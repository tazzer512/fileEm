//! A small preview of a Photoshop file (PSD, and PSB for big documents) without decoding all of
//! it (the Java `PsdReader`). The preview comes from the merged ("composite") image stored after
//! the layers: only the rows and columns needed for the asked size are read, so a huge document
//! costs a few hundred small reads. Supported: RGB, grayscale/duotone, indexed, CMYK and bitmap,
//! 1/8/16 bits, raw or RLE (PackBits). When the composite can't be used (ZIP compression, Lab,
//! 32-bit, or a blank composite saved without "maximize compatibility"), the JPEG thumbnail
//! Photoshop embeds in the image resources is returned.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::{Pixels, check};

const RESOURCE_THUMBNAIL: u16 = 1036;
const MAX_SIDE_PSD: u32 = 30_000;
const MAX_SIDE_PSB: u32 = 300_000;

/// A preview: pixels, or the embedded JPEG's bytes.
#[derive(Debug, PartialEq, Eq)]
pub enum Preview {
    Pixels(Pixels),
    Jpeg(Vec<u8>),
}

pub fn is_psd(extension: &str) -> bool {
    extension == "psd" || extension == "psb"
}

/// A preview at most `max_side` pixels wide and high, or `None` when the file has none this
/// reader can use. Errors only for read errors and files that aren't Photoshop documents.
pub fn read(path: &Path, max_side: u32) -> io::Result<Option<Preview>> {
    read_until(path, max_side, &AtomicBool::new(false))
}

/// [`read`] that stops with an `Interrupted` error once `stop` is set (checked every row).
pub fn read_until(path: &Path, max_side: u32, stop: &AtomicBool) -> io::Result<Option<Preview>> {
    Parser { file: File::open(path)?, pos: 0, psb: false, channels: 0, width: 0, height: 0, depth: 0, mode: 0, palette: None, jpeg: None, stop }
        .read(max_side.max(1))
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

struct Parser<'a> {
    file: File,
    pos: u64,
    psb: bool,
    channels: usize,
    width: u32,
    height: u32,
    depth: u16,
    mode: u16,
    palette: Option<Vec<u8>>,
    jpeg: Option<Vec<u8>>,
    stop: &'a AtomicBool,
}

impl Parser<'_> {
    fn read(mut self, max_side: u32) -> io::Result<Option<Preview>> {
        if self.bytes(4)? != b"8BPS" {
            return Err(bad("Not a Photoshop file"));
        }
        let version = self.u16()?;
        if version != 1 && version != 2 {
            return Err(bad("Unknown Photoshop version"));
        }
        self.psb = version == 2;
        self.pos += 6; // reserved
        self.channels = usize::from(self.u16()?);
        self.height = self.u32()?;
        self.width = self.u32()?;
        self.depth = self.u16()?;
        self.mode = self.u16()?;
        let max_dim = if self.psb { MAX_SIDE_PSB } else { MAX_SIDE_PSD };
        if !(1..=56).contains(&self.channels) || !(1..=max_dim).contains(&self.width) || !(1..=max_dim).contains(&self.height) {
            return Err(bad("Unsupported Photoshop dimensions"));
        }
        let color_data = u64::from(self.u32()?);
        if self.mode == 2 && color_data >= 768 {
            self.palette = Some(self.bytes(768)?);
            self.pos += color_data - 768;
        } else {
            self.pos += color_data;
        }
        let resources_end = u64::from(self.u32()?) + self.pos;
        self.read_resources(resources_end)?;
        self.pos = resources_end;
        let layers = if self.psb { self.u64()? } else { u64::from(self.u32()?) };
        self.pos += layers;

        // a broken composite falls back to the embedded thumbnail
        let composite = match self.composite(max_side) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
            r => r.ok().flatten(),
        };
        Ok(match (composite, self.jpeg.take()) {
            (Some(p), jpeg) if jpeg.is_none() || !uniform(&p.rgba) => Some(Preview::Pixels(p)),
            (_, Some(jpeg)) => Some(Preview::Jpeg(jpeg)),
            (composite, None) => composite.map(Preview::Pixels),
        })
    }

    /// Finds the JPEG thumbnail (resource 1036) among the image resources.
    fn read_resources(&mut self, end: u64) -> io::Result<()> {
        while self.pos + 12 <= end {
            if self.bytes(4)? != b"8BIM" {
                return Ok(());
            }
            let id = self.u16()?;
            let name_len = u64::from(self.bytes(1)?[0]);
            self.pos += name_len + if (name_len + 1) % 2 == 0 { 0 } else { 1 }; // Pascal string padded to even
            let size = u64::from(self.u32()?);
            let next = self.pos + size + size % 2;
            if id == RESOURCE_THUMBNAIL && size > 28 {
                let format = self.u32()?;
                self.pos += 24; // width, height, row bytes, total size, compressed size, bpp + planes
                if format == 1 && size - 28 < 16 << 20 {
                    self.jpeg = Some(self.bytes((size - 28) as usize)?);
                }
            }
            self.pos = next;
        }
        Ok(())
    }

    fn composite(&mut self, max_side: u32) -> io::Result<Option<Pixels>> {
        let compression = self.u16()?;
        if compression > 1 || !matches!(self.depth, 1 | 8 | 16) {
            return Ok(None); // ZIP is rare for the composite: use the thumbnail
        }
        let ch = self.channels;
        // the channels each output component reads
        let used: &[usize] = match self.mode {
            3 if ch >= 4 => &[0, 1, 2, 3],
            3 if ch >= 3 => &[0, 1, 2],
            1 | 8 if ch >= 2 => &[0, 1],
            1 | 8 => &[0],
            2 if self.palette.is_some() => &[0],
            4 if ch >= 4 => &[0, 1, 2, 3],
            0 if self.depth == 1 => &[0],
            _ => return Ok(None), // Lab, multichannel
        };
        let (w, h) = (self.width as usize, self.height as usize);
        let scale = (f64::from(max_side) / f64::from(self.width.max(self.height))).min(1.0);
        let ow = ((w as f64 * scale).round() as usize).max(1);
        let oh = ((h as f64 * scale).round() as usize).max(1);
        let row_bytes = (w * usize::from(self.depth)).div_ceil(8);
        // RLE: where each (channel, row) starts, and its length
        let mut rle = Vec::new();
        let data_start = self.pos;
        if compression == 1 {
            let rows = ch * h;
            let entry = if self.psb { 4 } else { 2 };
            let table = self.at(self.pos, rows * entry)?;
            let mut offset = self.pos + (rows * entry) as u64;
            for e in table.chunks_exact(entry) {
                let len = if self.psb { u32::from_be_bytes([e[0], e[1], e[2], e[3]]) as usize } else { usize::from(u16::from_be_bytes([e[0], e[1]])) };
                rle.push((offset, len));
                offset += len as u64;
            }
        }
        let src_cols: Vec<usize> = (0..ow).map(|x| (((x as f64 + 0.5) * w as f64 / ow as f64) as usize).min(w - 1)).collect();
        let mut out = Vec::with_capacity(ow * oh * 4);
        let mut comp = vec![vec![0u8; ow]; used.len()];
        let mut row = vec![0u8; row_bytes];
        for y in 0..oh {
            check(self.stop)?;
            let sy = (((y as f64 + 0.5) * h as f64 / oh as f64) as usize).min(h - 1);
            for (c, &channel) in used.iter().enumerate() {
                let index = channel * h + sy;
                if compression == 1 {
                    let (at, len) = rle[index];
                    unpack_bits(&self.at(at, len)?, &mut row)?;
                } else {
                    row.copy_from_slice(&self.at(data_start + (index * row_bytes) as u64, row_bytes)?);
                }
                for (x, &sx) in src_cols.iter().enumerate() {
                    comp[c][x] = self.sample(&row, sx);
                }
            }
            for x in 0..ow {
                out.extend(self.to_rgba(&comp, x));
            }
        }
        let mut p = Pixels::new(out, ow as u32, oh as u32);
        (p.full_width, p.full_height) = (self.width, self.height);
        Ok(Some(p))
    }

    /// The 8-bit value of column `x` in a decoded row.
    fn sample(&self, row: &[u8], x: usize) -> u8 {
        match self.depth {
            1 => {
                if (row[x >> 3] >> (7 - (x & 7))) & 1 == 1 {
                    0 // bitmap: 1 is black
                } else {
                    255
                }
            }
            16 => row[x * 2], // big-endian: the high byte
            _ => row[x],
        }
    }

    fn to_rgba(&self, comp: &[Vec<u8>], x: usize) -> [u8; 4] {
        let v = |c: usize| comp[c][x];
        match self.mode {
            3 => [v(0), v(1), v(2), if comp.len() > 3 { v(3) } else { 255 }],
            4 => {
                // Photoshop stores CMYK inverted: 255 means no ink
                let k = u32::from(v(3));
                let ink = |c: usize| (u32::from(v(c)) * k / 255) as u8;
                [ink(0), ink(1), ink(2), 255]
            }
            2 => {
                let p = self.palette.as_deref().unwrap_or(&[]);
                let i = usize::from(v(0));
                [p[i], p[256 + i], p[512 + i], 255]
            }
            _ => [v(0), v(0), v(0), if comp.len() > 1 { v(1) } else { 255 }], // grey, duotone, bitmap
        }
    }

    // ---- reading ----

    fn at(&mut self, pos: u64, len: usize) -> io::Result<Vec<u8>> {
        if len > i32::MAX as usize {
            return Err(bad("Corrupt length"));
        }
        let mut b = vec![0; len];
        self.file.seek(SeekFrom::Start(pos))?;
        self.file.read_exact(&mut b)?;
        Ok(b)
    }

    fn bytes(&mut self, n: usize) -> io::Result<Vec<u8>> {
        let b = self.at(self.pos, n)?;
        self.pos += n as u64;
        Ok(b)
    }

    fn u16(&mut self) -> io::Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> io::Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> io::Result<u64> {
        let b = self.bytes(8)?;
        let v = u64::from_be_bytes(b.try_into().unwrap());
        if v > i64::MAX as u64 {
            return Err(bad("Corrupt length"));
        }
        Ok(v)
    }
}

/// PackBits: n ≥ 0 copies n + 1 bytes, n < 0 repeats the next byte 1 − n times (−128 is a no-op).
fn unpack_bits(input: &[u8], out: &mut [u8]) -> io::Result<()> {
    let (mut i, mut o) = (0, 0);
    while o < out.len() && i < input.len() {
        let n = input[i] as i8;
        i += 1;
        if n >= 0 {
            let n = n as usize + 1;
            let count = n.min(out.len() - o);
            if input.len() - i < count {
                return Err(bad("Truncated RLE row"));
            }
            out[o..o + count].copy_from_slice(&input[i..i + count]);
            o += count;
            i += n;
        } else if n != -128 {
            let Some(&v) = input.get(i) else { return Err(bad("Truncated RLE row")) };
            i += 1;
            let count = ((1 - i32::from(n)) as usize).min(out.len() - o);
            out[o..o + count].fill(v);
            o += count;
        }
    }
    Ok(())
}

fn uniform(rgba: &[u8]) -> bool {
    rgba.as_chunks::<4>().0.iter().all(|p| p == &rgba[..4])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::fixtures::{plane, psd, psd_jpeg_thumbnail, Psd};
    use std::fs;
    use std::path::PathBuf;

    fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, data).unwrap();
        p
    }

    fn pixels(path: &Path, max_side: u32) -> Pixels {
        match read(path, max_side).unwrap() {
            Some(Preview::Pixels(p)) => p,
            other => panic!("pixels expected, got {other:?}"),
        }
    }

    const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
        [r, g, b, 255]
    }

    #[test]
    fn raw_rgb_at_full_size() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (4, 3);
        let planes = [plane(w, h, |x, _| (x * 60) as u8), plane(w, h, |_, y| (y * 100) as u8), plane(w, h, |_, _| 7)];
        let p = pixels(&write(tmp.path(), "raw.psd", &psd(&Psd { width: w, height: h, planes: &planes, ..Psd::default() })), 256);
        assert_eq!((p.width, p.height), (4, 3));
        assert_eq!(p.pixel(0, 0), rgb(0, 0, 7));
        assert_eq!(p.pixel(3, 2), rgb(180, 200, 7));
    }

    #[test]
    fn rle_rgb_is_scaled_down_reading_only_the_rows_it_needs() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (200, 100);
        let planes = [
            plane(w, h, |x, _| if x < 100 { 255 } else { 0 }),
            plane(w, h, |_, _| 0),
            plane(w, h, |x, _| if x < 100 { 0 } else { 255 }),
        ];
        let f = write(tmp.path(), "rle.psd", &psd(&Psd { width: w, height: h, planes: &planes, compression: 1, ..Psd::default() }));
        let p = pixels(&f, 50);
        assert_eq!((p.width, p.height, p.full_width, p.full_height), (50, 25, 200, 100));
        assert_eq!(p.pixel(5, 10), rgb(255, 0, 0), "left half red");
        assert_eq!(p.pixel(45, 10), rgb(0, 0, 255), "right half blue");
    }

    #[test]
    fn a_fourth_rgb_channel_is_transparency() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (2, 1);
        let planes = [plane(w, h, |_, _| 255), plane(w, h, |_, _| 255), plane(w, h, |_, _| 255), plane(w, h, |x, _| if x == 0 { 0 } else { 128 })];
        let p = pixels(&write(tmp.path(), "alpha.psd", &psd(&Psd { width: w, height: h, planes: &planes, compression: 1, ..Psd::default() })), 10);
        assert_eq!(p.pixel(0, 0), [255, 255, 255, 0]);
        assert_eq!(p.pixel(1, 0), [255, 255, 255, 128]);
    }

    #[test]
    fn sixteen_bit_grayscale_uses_the_high_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let planes = [vec![0x12, 0x34, 0xfe, 0xdc]]; // big-endian 0x1234, 0xfedc
        let d = Psd { mode: 1, depth: 16, width: 2, height: 1, planes: &planes, ..Psd::default() };
        let p = pixels(&write(tmp.path(), "gray16.psd", &psd(&d)), 10);
        assert_eq!(p.pixel(0, 0), rgb(0x12, 0x12, 0x12));
        assert_eq!(p.pixel(1, 0), rgb(0xfe, 0xfe, 0xfe));
    }

    #[test]
    fn cmyk_is_stored_inverted_and_converted_to_rgb() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (2, 1);
        let planes = [plane(w, h, |_, _| 255), plane(w, h, |_, _| 255), plane(w, h, |_, _| 255), plane(w, h, |x, _| if x == 0 { 255 } else { 0 })];
        let d = Psd { mode: 4, width: w, height: h, planes: &planes, compression: 1, ..Psd::default() };
        let p = pixels(&write(tmp.path(), "cmyk.psd", &psd(&d)), 10);
        assert_eq!(p.pixel(0, 0), rgb(255, 255, 255), "no ink: white");
        assert_eq!(p.pixel(1, 0), rgb(0, 0, 0), "full black ink");
    }

    #[test]
    fn indexed_colors_come_from_the_palette() {
        let tmp = tempfile::tempdir().unwrap();
        let mut palette = vec![0u8; 768];
        (palette[3], palette[256 + 3], palette[512 + 3]) = (10, 20, 30);
        let planes = [vec![3]];
        let d = Psd { mode: 2, planes: &planes, color_data: &palette, ..Psd::default() };
        assert_eq!(pixels(&write(tmp.path(), "indexed.psd", &psd(&d)), 10).pixel(0, 0), rgb(10, 20, 30));
    }

    #[test]
    fn big_documents_in_psb_format_are_read_too() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (3, 2);
        let planes = [plane(w, h, |_, _| 1), plane(w, h, |_, _| 2), plane(w, h, |_, _| 3)];
        let d = Psd { version: 2, width: w, height: h, planes: &planes, compression: 1, ..Psd::default() };
        assert_eq!(pixels(&write(tmp.path(), "big.psb", &psd(&d)), 10).pixel(2, 1), rgb(1, 2, 3));
    }

    #[test]
    fn without_a_usable_composite_the_embedded_jpeg_is_used() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (2, 2);
        let thumb = psd_jpeg_thumbnail();
        let zeros = [vec![0; 4], vec![0; 4], vec![0; 4]];
        let zip = Psd { width: w, height: h, planes: &zeros, compression: 2, resources: &thumb, ..Psd::default() };
        match read(&write(tmp.path(), "zip.psd", &psd(&zip)), 64).unwrap() {
            Some(Preview::Jpeg(j)) => assert_eq!(j[..2], [0xff, 0xd8], "a JPEG"),
            other => panic!("the thumbnail expected, got {other:?}"),
        }
        // a blank composite (saved without "maximize compatibility") also prefers the thumbnail
        let white = [plane(w, h, |_, _| 255), plane(w, h, |_, _| 255), plane(w, h, |_, _| 255)];
        let blank = Psd { width: w, height: h, planes: &white, resources: &thumb, ..Psd::default() };
        assert!(matches!(read(&write(tmp.path(), "blank.psd", &psd(&blank)), 64).unwrap(), Some(Preview::Jpeg(_))));
        // ... but a genuinely plain image without a thumbnail is still shown
        let nines = [plane(w, h, |_, _| 9), plane(w, h, |_, _| 9), plane(w, h, |_, _| 9)];
        let plain = Psd { width: w, height: h, planes: &nines, ..Psd::default() };
        assert_eq!(pixels(&write(tmp.path(), "plain.psd", &psd(&plain)), 64).pixel(0, 0), rgb(9, 9, 9));
        // and the thumbnail decodes as the file's picture
        let p = crate::images::decode(&write(tmp.path(), "zip2.psd", &psd(&zip)), 64).unwrap().unwrap();
        assert_eq!((p.width, p.height), (16, 8));
    }

    #[test]
    fn damaged_or_unsupported_files_give_no_preview_instead_of_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (10, 10);
        let planes = [plane(w, h, |x, _| x as u8), plane(w, h, |_, y| y as u8), plane(w, h, |_, _| 1)];
        let all = psd(&Psd { width: w, height: h, planes: &planes, compression: 1, ..Psd::default() });
        let cut = write(tmp.path(), "truncated.psd", &all[..all.len() - 40]);
        assert_eq!(read(&cut, 10).unwrap(), None, "truncated image data");

        let lab_planes = [vec![1], vec![2], vec![3]];
        let lab = Psd { mode: 9, planes: &lab_planes, ..Psd::default() };
        assert_eq!(read(&write(tmp.path(), "lab.psd", &psd(&lab)), 10).unwrap(), None, "Lab without a thumbnail");

        assert!(read(&write(tmp.path(), "fake.psd", b"not a psd"), 10).is_err());
        let ones = [vec![1], vec![1], vec![1]];
        let mut huge = psd(&Psd { planes: &ones, ..Psd::default() });
        huge[14] = 0x7f; // height 2 billion
        assert!(read(&write(tmp.path(), "huge.psd", &huge), 10).is_err());
    }

    #[test]
    fn a_set_stop_flag_interrupts_reading_the_composite() {
        let tmp = tempfile::tempdir().unwrap();
        let red = [vec![255], vec![0], vec![0]];
        let f = write(tmp.path(), "red.psd", &psd(&Psd { planes: &red, ..Psd::default() }));
        let err = read_until(&f, 10, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted, "not the JPEG fallback, not a damaged file");
        assert!(read_until(&f, 10, &AtomicBool::new(false)).unwrap().is_some());
        let decoded = crate::images::decode_until(&f, 10, &AtomicBool::new(true));
        assert_eq!(decoded.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn psd_and_psb_are_recognised() {
        assert!(is_psd("psd") && is_psd("psb"));
        assert!(!is_psd("png") && !is_psd(""));
    }
}
