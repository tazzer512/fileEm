//! Previews of camera raw files (the Java `RawReader`). Raw sensor data needs a full raw
//! developer, but cameras embed ready JPEG previews (often full size) in the file; this finds
//! them without reading the rest. TIFF based raws (Canon CR2, Nikon NEF, Sony ARW, Adobe DNG,
//! Pentax PEF, Olympus ORF, Panasonic RW2, Samsung SRW, …): JPEGs referenced from any IFD,
//! sub-IFD or RW2's JpgFromRaw tag. Canon CR3 (ISO media boxes): the PRVW preview. Fujifilm RAF:
//! the JPEG its header points to. Only reads headers and the chosen JPEG; JPEGs a normal decoder
//! can't show (lossless raw data, which also starts like a JPEG) are skipped.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const EXTENSIONS: [&str; 22] = [
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "pef", "orf", "rw2", "raf", "srw", "erf", "kdc", "dcr", "3fr", "mef", "mos",
    "iiq", "rwl", "raw",
];
/// Embedded JPEGs bigger than this are not believed (damaged offsets).
const MAX_JPEG: u64 = 64 << 20;
const MAX_IFDS: usize = 64;
const MAX_ENTRIES: usize = 1000;
const CR3_PREVIEW_UUID: [u8; 16] = [0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16];
const CR3_CANON_UUID: [u8; 16] = [0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];

/// Camera raw extensions this reader tries (lower case).
pub fn is_raw(extension: &str) -> bool {
    EXTENSIONS.contains(&extension)
}

/// An embedded JPEG: its bytes and pixel size, plus the orientation the photo was taken in
/// (EXIF values: 1 = as stored, 3 = upside down, 6 = turn 90° clockwise, 8 = 90° counter-
/// clockwise, 2/4/5/7 mirrored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    offset: u64,
    length: u64,
    width: u32,
    height: u32,
}

impl Candidate {
    fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// The smallest embedded JPEG at least `min_side` pixels on its longer side, or the largest one
/// when none is that big; `None` when the file has none this reader can find.
pub fn read(path: &Path, min_side: u32) -> io::Result<Option<Preview>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut src = Source::File(file, size);
    let mut found = Vec::new();
    let head = src.bytes(0, size.min(16) as usize)?;
    let mut orientation = if head.starts_with(b"FUJIFILMCCD-RAW") {
        raf(&mut src, &mut found)?;
        0
    } else if head.len() >= 12 && &head[4..12] == b"ftypcrx " {
        cr3(&mut src, &mut found)?
    } else {
        match Tiff::open(&mut src, 0)? {
            Some(tiff) => tiff.collect(&mut src, &mut found)?,
            None => return Ok(None),
        }
    };
    let Some(best) = choose(&found, min_side) else { return Ok(None) };
    let jpeg = src.bytes(best.offset, best.length as usize)?;
    if orientation == 0 {
        orientation = exif_orientation(&jpeg); // unknown so far: the JPEG's own EXIF says
    }
    Ok(Some(Preview { jpeg, width: best.width, height: best.height, orientation: if orientation == 0 { 1 } else { orientation } }))
}

fn choose(found: &[Candidate], min_side: u32) -> Option<Candidate> {
    let big_enough = found.iter().filter(|c| c.width.max(c.height) >= min_side).min_by_key(|c| c.pixels());
    big_enough.or_else(|| found.iter().max_by_key(|c| c.pixels())).copied()
}

/// Adds the JPEG at `offset` when it is one a normal decoder shows (baseline/progressive).
fn add_jpeg(src: &mut Source, offset: u64, length: u64, found: &mut Vec<Candidate>) -> io::Result<()> {
    if offset == 0 || !(4..=MAX_JPEG).contains(&length) || offset + length > src.size() || found.iter().any(|c| c.offset == offset) {
        return Ok(());
    }
    // cut off or damaged: not a usable preview
    if let Ok(Some((width, height))) = jpeg_size(src, offset, offset + length) {
        found.push(Candidate { offset, length, width, height });
    }
    Ok(())
}

/// Width and height from a JPEG's frame header, or `None` when it isn't a JPEG or uses a coding
/// decoders don't support (lossless, hierarchical, arithmetic).
fn jpeg_size(src: &mut Source, start: u64, end: u64) -> io::Result<Option<(u32, u32)>> {
    if src.bytes(start, 2)? != [0xff, 0xd8] {
        return Ok(None);
    }
    let mut pos = start + 2;
    for _ in 0..1000 {
        if pos + 4 > end {
            break;
        }
        let m = src.bytes(pos, 4)?;
        if m[0] != 0xff {
            return Ok(None);
        }
        let marker = m[1];
        if marker == 0xff {
            pos += 1; // fill byte
            continue;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            pos += 2;
            continue;
        }
        let len = u64::from(u16::from_be_bytes([m[2], m[3]]));
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            if marker > 0xc2 {
                return Ok(None);
            }
            let sof = src.bytes(pos + 5, 4)?;
            let h = u32::from(u16::from_be_bytes([sof[0], sof[1]]));
            let w = u32::from(u16::from_be_bytes([sof[2], sof[3]]));
            return Ok((w > 0 && h > 0).then_some((w, h)));
        }
        if marker == 0xda || marker == 0xd9 || len < 2 {
            return Ok(None); // image data or the end before any frame header
        }
        pos += 2 + len;
    }
    Ok(None)
}

/// The EXIF orientation inside a JPEG (APP1 "Exif"), or 0 when it has none.
pub fn exif_orientation(jpeg: &[u8]) -> u8 {
    let mut src = Source::Bytes(jpeg);
    let mut pos = 2;
    for _ in 0..64 {
        if pos + 4 > jpeg.len() || jpeg[pos] != 0xff {
            return 0;
        }
        let marker = jpeg[pos + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]));
        if marker == 0xe1 && len > 8 && jpeg.get(pos + 4..pos + 10) == Some(b"Exif\0\0") {
            return match Tiff::open(&mut src, pos as u64 + 10) {
                Ok(Some(t)) => t.orientation(&mut src).unwrap_or(0),
                _ => 0,
            };
        }
        if marker == 0xda || len < 2 {
            return 0;
        }
        pos += 2 + len;
    }
    0
}

// ---- Fujifilm RAF ----

fn raf(src: &mut Source, found: &mut Vec<Candidate>) -> io::Result<()> {
    let b = src.bytes(84, 8)?;
    let offset = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let length = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    add_jpeg(src, u64::from(offset), u64::from(length), found)
}

// ---- Canon CR3 ----

/// Finds the PRVW preview; returns the orientation from the CMT1 (TIFF IFD0) box, or 0.
fn cr3(src: &mut Source, found: &mut Vec<Candidate>) -> io::Result<u8> {
    let mut orientation = 0;
    let size = src.size();
    let mut top = Vec::new();
    boxes(src, 0, size, &mut top)?;
    let moov: Vec<IsoBox> = top.iter().filter(|b| &b.kind == b"moov").copied().collect();
    let mut candidates = top;
    for m in moov {
        boxes(src, m.content, m.end, &mut candidates)?; // Canon's uuid box is inside
    }
    for b in candidates.into_iter().filter(|b| &b.kind == b"uuid" && b.end - b.content >= 16) {
        let uuid = src.bytes(b.content, 16)?;
        let mut inner = Vec::new();
        if uuid == CR3_CANON_UUID {
            boxes(src, b.content + 16, b.end, &mut inner)?;
            for cmt1 in inner.iter().filter(|i| &i.kind == b"CMT1") {
                if let Some(tiff) = Tiff::open(src, cmt1.content)? {
                    orientation = tiff.orientation(src)?;
                }
            }
        } else if uuid == CR3_PREVIEW_UUID {
            // 8 bytes, then a PRVW box: a few size fields, then the JPEG
            boxes(src, b.content + 24, b.end, &mut inner)?;
            for prvw in inner.iter().filter(|i| &i.kind == b"PRVW") {
                let window = src.bytes(prvw.content, (prvw.end.min(prvw.content + 64) - prvw.content) as usize)?;
                if let Some(i) = window.windows(3).position(|w| w == [0xff, 0xd8, 0xff]) {
                    let jpeg = prvw.content + i as u64;
                    add_jpeg(src, jpeg, prvw.end - jpeg, found)?;
                }
            }
        }
    }
    Ok(orientation)
}

#[derive(Debug, Clone, Copy)]
struct IsoBox {
    kind: [u8; 4],
    content: u64,
    end: u64,
}

/// The ISO media boxes between `from` and `to` (one level).
fn boxes(src: &mut Source, from: u64, to: u64, out: &mut Vec<IsoBox>) -> io::Result<()> {
    let mut pos = from;
    for _ in 0..10_000 {
        if pos + 8 > to {
            break;
        }
        let h = src.bytes(pos, 8)?;
        let mut size = u64::from(u32::from_be_bytes([h[0], h[1], h[2], h[3]]));
        let kind = [h[4], h[5], h[6], h[7]];
        let mut content = pos + 8;
        if size == 1 {
            if pos + 16 > to {
                break;
            }
            size = u64::from_be_bytes(src.bytes(pos + 8, 8)?.try_into().unwrap());
            content = pos + 16;
        } else if size == 0 {
            size = to - pos;
        }
        if size < content - pos || pos.checked_add(size).is_none_or(|end| end > to) {
            break;
        }
        out.push(IsoBox { kind, content, end: pos + size });
        pos += size;
    }
    Ok(())
}

// ---- TIFF ----

/// A TIFF structure at `base` (offsets in it are relative to base).
struct Tiff {
    base: u64,
    big_endian: bool,
    first_ifd: u64,
}

/// An IFD entry; values that don't fit in its 4 bytes live at an offset.
struct Entry {
    tag: u16,
    kind: u16,
    count: u64,
    raw: [u8; 4],
}

impl Tiff {
    /// Reads the header: II/MM, then 42 (or ORF's and RW2's own magic numbers).
    fn open(src: &mut Source, base: u64) -> io::Result<Option<Tiff>> {
        if src.size() < base + 8 {
            return Ok(None);
        }
        let h = src.bytes(base, 8)?;
        let big_endian = match &h[..2] {
            b"II" => false,
            b"MM" => true,
            _ => return Ok(None),
        };
        let t = Tiff { base, big_endian, first_ifd: 0 };
        let magic = t.u16(&h[2..4]);
        if !matches!(magic, 42 | 0x4f52 | 0x5352 | 0x55) {
            return Ok(None);
        }
        Ok(Some(Tiff { first_ifd: u64::from(t.u32(&h[4..8])), ..t }))
    }

    fn u16(&self, b: &[u8]) -> u16 {
        let b = [b[0], b[1]];
        if self.big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    }

    fn u32(&self, b: &[u8]) -> u32 {
        let b = [b[0], b[1], b[2], b[3]];
        if self.big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    }

    fn orientation(&self, src: &mut Source) -> io::Result<u8> {
        for e in self.ifd(src, self.first_ifd)? {
            if e.tag == 0x112 {
                return Ok(self.first(src, &e).min(255) as u8);
            }
        }
        Ok(0)
    }

    /// Adds every embedded JPEG; returns IFD0's orientation (0: none).
    fn collect(&self, src: &mut Source, found: &mut Vec<Candidate>) -> io::Result<u8> {
        let mut orientation = 0;
        let mut seen = HashSet::new();
        let mut todo = vec![self.first_ifd];
        let mut first = true;
        while !todo.is_empty() && seen.len() < MAX_IFDS {
            let at = todo.remove(0);
            if at == 0 || !seen.insert(at) {
                continue;
            }
            let entries = self.ifd(src, at)?;
            let (mut jpeg_offset, mut jpeg_length, mut strip, mut strip_length, mut strips) = (0, 0, 0, 0, 0);
            for e in &entries {
                match e.tag {
                    0x112 if first => orientation = self.first(src, e).min(255) as u8,
                    0x201 => jpeg_offset = self.first(src, e),
                    0x202 => jpeg_length = self.first(src, e),
                    0x111 => {
                        strip = self.first(src, e);
                        strips = e.count;
                    }
                    0x117 => strip_length = self.first(src, e),
                    0x14a => todo.extend(self.values(src, e, 16)?),
                    // Panasonic RW2: JpgFromRaw, stored inline as undefined bytes
                    0x2e if e.kind == 7 => add_jpeg(src, self.base + u64::from(self.u32(&e.raw)), e.count, found)?,
                    _ => {}
                }
            }
            if jpeg_offset > 0 && jpeg_length > 0 {
                add_jpeg(src, self.base + jpeg_offset, jpeg_length, found)?;
            }
            if strips == 1 && strip > 0 && strip_length > 0 {
                add_jpeg(src, self.base + strip, strip_length, found)?; // checked to be a viewable JPEG
            }
            first = false;
            let next = self.next_ifd(src, at, entries.len())?;
            if next > 0 {
                todo.push(next);
            }
        }
        Ok(orientation)
    }

    fn next_ifd(&self, src: &mut Source, at: u64, count: usize) -> io::Result<u64> {
        let pos = self.base + at + 2 + 12 * count as u64;
        if pos + 4 > src.size() {
            return Ok(0);
        }
        Ok(u64::from(self.u32(&src.bytes(pos, 4)?)))
    }

    fn ifd(&self, src: &mut Source, at: u64) -> io::Result<Vec<Entry>> {
        let pos = self.base + at;
        if pos + 2 > src.size() {
            return Ok(Vec::new());
        }
        let count = usize::from(self.u16(&src.bytes(pos, 2)?));
        if count > MAX_ENTRIES || pos + 2 + 12 * count as u64 > src.size() {
            return Ok(Vec::new());
        }
        let b = src.bytes(pos + 2, 12 * count)?;
        Ok(b.as_chunks::<12>().0.iter()
            .map(|e| Entry { tag: self.u16(&e[0..2]), kind: self.u16(&e[2..4]), count: u64::from(self.u32(&e[4..8])), raw: [e[8], e[9], e[10], e[11]] })
            .collect())
    }

    fn type_size(e: &Entry) -> u64 {
        match e.kind {
            3 | 8 => 2,
            4 | 9 | 13 => 4,
            _ => 1,
        }
    }

    /// The first value as a number (SHORT or LONG); 0 when it can't be read.
    fn first(&self, src: &mut Source, e: &Entry) -> u64 {
        let size = Tiff::type_size(e);
        let value = |b: &[u8]| match size {
            2 => u64::from(self.u16(b)),
            4 => u64::from(self.u32(b)),
            _ => u64::from(b[0]),
        };
        if e.count == 1 || size * e.count <= 4 {
            return value(&e.raw);
        }
        src.bytes(self.base + u64::from(self.u32(&e.raw)), size as usize).map_or(0, |b| value(&b))
    }

    /// Up to `max` LONG values (sub-IFD offsets).
    fn values(&self, src: &mut Source, e: &Entry, max: u64) -> io::Result<Vec<u64>> {
        let n = e.count.min(max) as usize;
        if Tiff::type_size(e) != 4 || n == 0 {
            return Ok(Vec::new());
        }
        let b = if n == 1 { e.raw.to_vec() } else { src.bytes(self.base + u64::from(self.u32(&e.raw)), 4 * n)? };
        Ok(b.as_chunks::<4>().0.iter().map(|v| u64::from(self.u32(v))).collect())
    }
}

// ---- reading ----

/// Random access to the file (or a byte slice); reads past the end fail.
enum Source<'a> {
    File(File, u64),
    Bytes(&'a [u8]),
}

impl Source<'_> {
    fn size(&self) -> u64 {
        match self {
            Source::File(_, size) => *size,
            Source::Bytes(b) => b.len() as u64,
        }
    }

    fn bytes(&mut self, pos: u64, len: usize) -> io::Result<Vec<u8>> {
        if pos.checked_add(len as u64).is_none_or(|end| end > self.size()) {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "past the end of the file"));
        }
        match self {
            Source::File(f, _) => {
                let mut b = vec![0; len];
                f.seek(SeekFrom::Start(pos))?;
                f.read_exact(&mut b)?;
                Ok(b)
            }
            Source::Bytes(b) => Ok(b[pos as usize..pos as usize + len].to_vec()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::fixtures::{self, Ifd};
    use std::fs;
    use std::path::PathBuf;

    fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, data).unwrap();
        p
    }

    fn assert_decodes(p: &Option<Preview>, w: u32, h: u32) {
        let p = p.as_ref().expect("a preview");
        assert_eq!((p.width, p.height), (w, h));
        let img = image::load_from_memory(&p.jpeg).expect("the bytes are the whole JPEG");
        assert_eq!((img.width(), img.height()), (w, h));
    }

    #[test]
    fn knows_raw_extensions() {
        for ext in ["cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2", "pef"] {
            assert!(is_raw(ext), "{ext}");
            assert_eq!(crate::fs::file_types::category(ext, false), crate::fs::Category::Image, "{ext}");
        }
        assert!(!is_raw("jpg") && !is_raw(""));
    }

    /// CR2 keeps a full-size JPEG in IFD0's strip, a small thumbnail in IFD1 and the sensor data
    /// as a lossless JPEG; the biggest viewable one is the preview, the smallest big-enough one
    /// the thumbnail.
    #[test]
    fn cr2_picks_the_right_embedded_jpeg_and_skips_the_raw_data() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write(tmp.path(), "IMG_1.CR2", &fixtures::cr2(&fixtures::jpeg(640, 480), &fixtures::jpeg(160, 120), 8));
        let big = read(&f, u32::MAX).unwrap();
        assert_decodes(&big, 640, 480);
        assert_eq!(big.unwrap().orientation, 8);
        assert_decodes(&read(&f, 100).unwrap(), 160, 120);
        assert_decodes(&read(&f, 161).unwrap(), 640, 480);
    }

    #[test]
    fn big_endian_sub_ifds_as_in_nikon_files() {
        let tmp = tempfile::tempdir().unwrap();
        let ifd0 = Ifd::new().short(0x112, 6).sub_ifd(1);
        let sub = Ifd::new().data(0x201, 0x202, &fixtures::jpeg(300, 200));
        let p = read(&write(tmp.path(), "a.nef", &fixtures::tiff(true, 42, &[ifd0, sub])), 0).unwrap();
        assert_decodes(&p, 300, 200);
        assert_eq!(p.unwrap().orientation, 6);
    }

    #[test]
    fn panasonic_rw2_keeps_its_jpeg_in_a_tag_of_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let ifd0 = Ifd::new().undefined(0x2e, &fixtures::jpeg(120, 80));
        let p = read(&write(tmp.path(), "a.rw2", &fixtures::tiff(false, 0x55, &[ifd0])), 0).unwrap();
        assert_decodes(&p, 120, 80);
        assert_eq!(p.unwrap().orientation, 1, "no orientation: as stored");
    }

    #[test]
    fn fujifilm_raf_takes_the_orientation_from_the_jpeg() {
        let tmp = tempfile::tempdir().unwrap();
        let jpeg = fixtures::with_exif_orientation(&fixtures::jpeg(90, 60), 3);
        let p = read(&write(tmp.path(), "a.raf", &fixtures::raf(&jpeg)), 0).unwrap();
        assert_decodes(&p, 90, 60);
        let p = p.unwrap();
        assert_eq!(p.orientation, 3);
        assert_eq!(p.jpeg, jpeg);
    }

    #[test]
    fn canon_cr3_preview_box_and_orientation() {
        let tmp = tempfile::tempdir().unwrap();
        let p = read(&write(tmp.path(), "a.cr3", &fixtures::cr3(&fixtures::jpeg(162, 108), 6)), 0).unwrap();
        assert_decodes(&p, 162, 108);
        assert_eq!(p.unwrap().orientation, 6);
    }

    #[test]
    fn files_without_a_preview_or_damaged_ones_give_none_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(read(&write(dir, "text.cr2", b"not a raw file at all"), 0).unwrap(), None);
        assert_eq!(read(&write(dir, "empty.nef", b""), 0).unwrap(), None);
        // macOS resource forks (._IMG.CR2) are not raws
        assert_eq!(read(&write(dir, "._IMG.CR2", &[0, 5, 22, 7, 0, 2, 0, 0, b'M', b'a', b'c']), 0).unwrap(), None);
        // only raw sensor data (like a merged HDR DNG): nothing to show
        let hdr = Ifd::new().short(0x103, 7).data(0x111, 0x117, &fixtures::lossless_jpeg(3000, 2000));
        assert_eq!(read(&write(dir, "hdr.dng", &fixtures::tiff(false, 42, &[hdr])), 0).unwrap(), None);
        // a JPEG pointer past the end of the file
        let bad = Ifd::new().long(0x201, 1_000_000).long(0x202, 5000);
        assert_eq!(read(&write(dir, "bad.cr2", &fixtures::tiff(false, 42, &[bad])), 0).unwrap(), None);
        // cut off halfway through the JPEG: no panic
        let whole = fixtures::cr2(&fixtures::jpeg(640, 480), &fixtures::jpeg(160, 120), 1);
        let _ = read(&write(dir, "cut.cr2", &whole[..whole.len() / 3]), 0);
        // an IFD that links to itself ends
        let lp = Ifd::new().data(0x201, 0x202, &fixtures::jpeg(50, 40)).next(0);
        assert_decodes(&read(&write(dir, "loop.cr2", &fixtures::tiff(false, 42, &[lp])), 0).unwrap(), 50, 40);
    }

    #[test]
    fn exif_orientation_of_plain_jpegs() {
        let jpeg = fixtures::jpeg(20, 10);
        assert_eq!(exif_orientation(&jpeg), 0, "no EXIF");
        assert_eq!(exif_orientation(&fixtures::with_exif_orientation(&jpeg, 8)), 8);
        assert_eq!(exif_orientation(&[0xff, 0xd8, 0xff, 0xe1, 0, 4]), 0, "cut off: no orientation, no panic");
    }

    #[test]
    fn raw_files_decode_upright_and_shrunk() {
        let tmp = tempfile::tempdir().unwrap();
        // turned 90° clockwise: the red left half ends up on top
        let f = write(tmp.path(), "IMG_2.CR2", &fixtures::cr2(&fixtures::jpeg(200, 150), &fixtures::jpeg(40, 30), 6));
        let p = crate::images::decode(&f, 64).unwrap().unwrap();
        assert_eq!((p.width, p.height), (48, 64), "the 200 × 150 preview: the thumbnail is too small");
        assert!(p.pixel(24, 4)[0] > 200 && p.pixel(24, 60)[0] < 50);
    }
}
