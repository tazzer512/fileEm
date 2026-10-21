//! Small image files for tests (the Java `RawFixtures`): JPEGs, TIFF structures (CR2, NEF,
//! RW2 …), CR3 boxes, RAF headers and Photoshop documents. Only built for tests and with the
//! `fixtures` feature (the UI tests use it).

use std::collections::HashMap;

/// A real baseline JPEG of `w × h`: the left half red, the rest black, so a turned image can be
/// told apart.
pub fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, _| if x < w / 2 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 0]) });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img).unwrap();
    out
}

/// A valid PNG whose header promises `w × h` pixels but holds only a few bytes of image data:
/// a decoder that trusts the header would allocate gigabytes for it.
pub fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    // IHDR's data follows the 8-byte signature and the chunk's length and type; its CRC after it
    out[16..20].copy_from_slice(&w.to_be_bytes());
    out[20..24].copy_from_slice(&h.to_be_bytes());
    let crc = crc32(&out[12..29]);
    out[29..33].copy_from_slice(&crc.to_be_bytes());
    out
}

/// A BMP header promising `w × h` 24-bit pixels, followed by almost none.
pub fn bmp_claiming(w: i32, h: i32) -> Vec<u8> {
    let mut b = b"BM".to_vec();
    b.extend(54u32.to_le_bytes()); // file size (not checked)
    b.extend([0; 4]);
    b.extend(54u32.to_le_bytes()); // pixel data offset
    b.extend(40u32.to_le_bytes());
    b.extend(w.to_le_bytes());
    b.extend(h.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(24u16.to_le_bytes());
    b.extend([0; 24]);
    b.resize(160, 0);
    b
}

/// The CRC-32 PNG chunks carry (bit by bit: only for test files).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The start of a lossless JPEG (SOF3), like the raw sensor data in CR2/DNG: not viewable.
pub fn lossless_jpeg(w: u16, h: u16) -> Vec<u8> {
    let mut b = vec![0xff, 0xd8, 0xff, 0xc3, 0, 11, 14];
    b.extend(h.to_be_bytes());
    b.extend(w.to_be_bytes());
    b.extend([1, 1, 0x11, 0, 0xff, 0xd9]);
    b.resize(64, 0);
    b
}

/// The same JPEG with an EXIF block (APP1) holding an orientation.
pub fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let tiff = tiff(false, 42, &[Ifd::new().short(0x112, orientation)]);
    let len = 2 + 6 + tiff.len();
    let mut out = jpeg[..2].to_vec();
    out.extend([0xff, 0xe1, (len >> 8) as u8, len as u8]);
    out.extend(b"Exif\0\0");
    out.extend(tiff);
    out.extend(&jpeg[2..]);
    out
}

#[derive(Clone)]
enum Value {
    Number(u32),
    /// Bytes laid out after the IFDs; the entry holds their offset.
    Data(Vec<u8>),
    /// Another IFD (by index into the list given to [`tiff`]); the entry holds its offset.
    Ifd(usize),
}

/// One TIFF IFD for [`tiff`]; values that need it are laid out after the IFDs.
#[derive(Clone, Default)]
pub struct Ifd {
    entries: Vec<(u16, u16, u32, Value)>,
    /// The next IFD (IFD0 → IFD1 …), by index.
    next: Option<usize>,
}

impl Ifd {
    pub fn new() -> Ifd {
        Ifd::default()
    }

    pub fn short(mut self, tag: u16, v: u16) -> Ifd {
        self.entries.push((tag, 3, 1, Value::Number(u32::from(v))));
        self
    }

    pub fn long(mut self, tag: u16, v: u32) -> Ifd {
        self.entries.push((tag, 4, 1, Value::Number(v)));
        self
    }

    /// `data` somewhere in the file: its offset under `offset_tag`, its length under `length_tag`.
    pub fn data(mut self, offset_tag: u16, length_tag: u16, data: &[u8]) -> Ifd {
        let len = data.len() as u32;
        self.entries.push((offset_tag, 4, 1, Value::Data(data.to_vec())));
        self.entries.push((length_tag, 4, 1, Value::Number(len)));
        self
    }

    /// An UNDEFINED value (bytes stored at an offset), like RW2's JpgFromRaw.
    pub fn undefined(mut self, tag: u16, data: &[u8]) -> Ifd {
        self.entries.push((tag, 7, data.len() as u32, Value::Data(data.to_vec())));
        self
    }

    /// A sub-IFD: the IFD at `index` in the list given to [`tiff`].
    pub fn sub_ifd(mut self, index: usize) -> Ifd {
        self.entries.push((0x14a, 4, 1, Value::Ifd(index)));
        self
    }

    /// The IFD after this one: the one at `index`.
    pub fn next(mut self, index: usize) -> Ifd {
        self.next = Some(index);
        self
    }

    fn size(&self) -> usize {
        2 + 12 * self.entries.len() + 4
    }
}

/// A TIFF file: the header (`magic`: 42, or 0x55 for RW2), then `ifds` (the first is IFD0),
/// then their data. Big-endian ("MM", as Nikon) or little-endian ("II").
pub fn tiff(big_endian: bool, magic: u16, ifds: &[Ifd]) -> Vec<u8> {
    let u16b = |v: u16| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
    let u32b = |v: u32| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut at = Vec::new();
    let mut pos = 8;
    for ifd in ifds {
        at.push(pos as u32);
        pos += ifd.size();
    }
    let mut data_at = HashMap::new();
    let mut blobs = Vec::new();
    for (i, ifd) in ifds.iter().enumerate() {
        for (e, entry) in ifd.entries.iter().enumerate() {
            if let Value::Data(d) = &entry.3 {
                data_at.insert((i, e), pos as u32);
                pos += d.len();
                blobs.push(d.clone());
            }
        }
    }
    let mut b = Vec::with_capacity(pos);
    b.extend(if big_endian { b"MM" } else { b"II" });
    b.extend(u16b(magic));
    b.extend(u32b(8));
    for (i, ifd) in ifds.iter().enumerate() {
        b.extend(u16b(ifd.entries.len() as u16));
        for (e, (tag, ty, count, value)) in ifd.entries.iter().enumerate() {
            b.extend(u16b(*tag));
            b.extend(u16b(*ty));
            b.extend(u32b(*count));
            let v = match value {
                Value::Number(n) => *n,
                Value::Data(_) => data_at[&(i, e)],
                Value::Ifd(index) => at[*index],
            };
            if *ty == 3 {
                b.extend(u16b(v as u16));
                b.extend([0, 0]);
            } else {
                b.extend(u32b(v));
            }
        }
        b.extend(u32b(ifd.next.map_or(0, |n| at[n])));
    }
    for blob in blobs {
        b.extend(blob);
    }
    b
}

/// A CR2-like file: a full-size JPEG in IFD0's strip (with the orientation), a thumbnail in
/// IFD1, lossless raw data in IFD2.
pub fn cr2(preview: &[u8], thumbnail: &[u8], orientation: u16) -> Vec<u8> {
    let ifd0 = Ifd::new().short(0x103, 6).short(0x112, orientation).data(0x111, 0x117, preview).next(1);
    let thumb = Ifd::new().data(0x201, 0x202, thumbnail).next(2);
    let raw = Ifd::new().short(0x103, 6).data(0x111, 0x117, &lossless_jpeg(4000, 3000));
    tiff(false, 42, &[ifd0, thumb, raw])
}

/// A Fujifilm RAF: its header points to the JPEG.
pub fn raf(jpeg: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 100];
    b[..28].copy_from_slice(b"FUJIFILMCCD-RAW 0201FF383501");
    b[84..88].copy_from_slice(&100u32.to_be_bytes());
    b[88..92].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
    b.extend(jpeg);
    b
}

fn iso_box(kind: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let mut b = ((8 + content.len()) as u32).to_be_bytes().to_vec();
    b.extend(kind);
    b.extend(content);
    b
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// A Canon CR3: ftyp, moov with Canon's uuid box (CMT1 = TIFF with the orientation), the PRVW box.
pub fn cr3(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut out = iso_box(b"ftyp", &[b"crx ".as_slice(), &[0; 4], b"crx isom"].concat());
    let cmt1 = iso_box(b"CMT1", &tiff(false, 42, &[Ifd::new().short(0x112, orientation)]));
    out.extend(iso_box(b"moov", &iso_box(b"uuid", &[hex("85c0b687820f11e08111f4ce462b6a48"), cmt1].concat())));
    let mut prvw = vec![0, 0, 0, 0, 0, 1];
    prvw.extend(1620u16.to_be_bytes());
    prvw.extend(1080u16.to_be_bytes());
    prvw.extend(1u16.to_be_bytes());
    prvw.extend((jpeg.len() as u32).to_be_bytes());
    prvw.extend(jpeg);
    out.extend(iso_box(b"uuid", &[hex("eaf42b5e1c984b88b9fbb7dc406e4d16"), vec![0; 8], iso_box(b"PRVW", &prvw)].concat()));
    out.extend(iso_box(b"mdat", &[0; 32]));
    out
}

/// PackBits with both runs (3+ equal bytes) and literals, to exercise both decoder branches.
pub fn pack_bits(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && row[i + run] == row[i] && run < 128 {
            run += 1;
        }
        if run >= 3 {
            out.push((1 - run as i32) as u8);
            out.push(row[i]);
            i += run;
        } else {
            let start = i;
            while i < row.len() && i - start < 128 && !(i + 2 < row.len() && row[i] == row[i + 1] && row[i] == row[i + 2]) {
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend(&row[start..i]);
        }
    }
    out
}

/// A Photoshop document's parts for [`psd`].
pub struct Psd<'a> {
    /// 1: PSD, 2: PSB.
    pub version: u16,
    /// 0 bitmap, 1 grey, 2 indexed, 3 RGB, 4 CMYK, 9 Lab.
    pub mode: u16,
    pub depth: u16,
    pub width: u32,
    pub height: u32,
    /// Channel planes, `planes[c][y * row_bytes + x]`.
    pub planes: &'a [Vec<u8>],
    /// 0 raw, 1 RLE, 2 ZIP (written as junk).
    pub compression: u16,
    pub color_data: &'a [u8],
    pub resources: &'a [u8],
}

impl Default for Psd<'_> {
    fn default() -> Self {
        Psd { version: 1, mode: 3, depth: 8, width: 1, height: 1, planes: &[], compression: 0, color_data: &[], resources: &[] }
    }
}

/// A minimal Photoshop file: header, color data, resources, no layers, the composite image.
pub fn psd(d: &Psd<'_>) -> Vec<u8> {
    let row_bytes = (d.width as usize * d.depth as usize).div_ceil(8);
    let mut out = b"8BPS".to_vec();
    out.extend(d.version.to_be_bytes());
    out.extend([0; 6]);
    out.extend((d.planes.len() as u16).to_be_bytes());
    out.extend(d.height.to_be_bytes());
    out.extend(d.width.to_be_bytes());
    out.extend(d.depth.to_be_bytes());
    out.extend(d.mode.to_be_bytes());
    out.extend((d.color_data.len() as u32).to_be_bytes());
    out.extend(d.color_data);
    out.extend((d.resources.len() as u32).to_be_bytes());
    out.extend(d.resources);
    if d.version == 2 {
        out.extend(0u64.to_be_bytes()); // no layers
    } else {
        out.extend(0u32.to_be_bytes());
    }
    out.extend(d.compression.to_be_bytes());
    match d.compression {
        0 => d.planes.iter().for_each(|p| out.extend(p)),
        1 => {
            let rows: Vec<Vec<u8>> =
                d.planes.iter().flat_map(|p| p.chunks(row_bytes.max(1)).map(pack_bits).collect::<Vec<_>>()).collect();
            for r in &rows {
                if d.version == 2 {
                    out.extend((r.len() as u32).to_be_bytes());
                } else {
                    out.extend((r.len() as u16).to_be_bytes());
                }
            }
            rows.iter().for_each(|r| out.extend(r));
        }
        _ => out.extend([1, 2, 3, 4]), // pretend ZIP data
    }
    out
}

/// One plane of `w × h` with the value `f(x, y)`.
pub fn plane(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// Image resource 1036: a 16 × 8 JPEG thumbnail.
pub fn psd_jpeg_thumbnail() -> Vec<u8> {
    let data = jpeg(16, 8);
    let mut out = b"8BIM".to_vec();
    out.extend(1036u16.to_be_bytes());
    out.extend([0, 0]); // empty name, padded to even
    out.extend((28 + data.len() as u32).to_be_bytes());
    for v in [1u32, 16, 8, 48, 16 * 8 * 3, data.len() as u32] {
        out.extend(v.to_be_bytes()); // kJpegRGB, width, height, row bytes, total, compressed size
    }
    out.extend(24u16.to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend(&data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}
