//! Netpbm images (the Java `NetpbmReader`): PBM (P1 text, P4 binary bitmap), PGM (P2/P5 grey)
//! and PPM (P3/P6 colour), 8 or 16 bits per sample. Big images are read with whole-pixel steps
//! (skipped rows are only read past), then shrunk to the limit.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use super::Pixels;

/// Larger sides are refused: a damaged header shouldn't make us allocate gigabytes.
const MAX_SIDE: u32 = 1 << 18;

pub fn is_netpbm(extension: &str) -> bool {
    matches!(extension, "ppm" | "pgm" | "pbm" | "pnm")
}

/// Decodes `path` with its longer side at most `max_side`; `None` when it isn't a valid
/// (or is a cut off) Netpbm file.
pub fn read(path: &Path, max_side: u32) -> io::Result<Option<Pixels>> {
    let mut input = Input { r: BufReader::with_capacity(1 << 16, File::open(path)?) };
    Ok(decode(&mut input, max_side).ok().flatten())
}

struct Input<R: Read> {
    r: R,
}

impl<R: Read> Input<R> {
    /// The next byte, `None` at the end.
    fn byte(&mut self) -> io::Result<Option<u8>> {
        let mut b = [0];
        Ok((self.r.read(&mut b)? == 1).then_some(b[0]))
    }

    fn skip_space(&mut self) -> io::Result<u8> {
        let mut c = self.byte()?;
        loop {
            match c {
                Some(b'#') => {
                    while !matches!(c, Some(b'\n' | b'\r') | None) {
                        c = self.byte()?;
                    }
                }
                Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b) => c = self.byte()?,
                Some(c) => return Ok(c),
                None => return Err(io::ErrorKind::UnexpectedEof.into()),
            }
        }
    }

    /// The next decimal number, skipping whitespace and # comments; eats the one character
    /// after it.
    fn number(&mut self) -> io::Result<u32> {
        let mut c = Some(self.skip_space()?);
        if !c.is_some_and(|c| c.is_ascii_digit()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "number expected"));
        }
        let mut n: u64 = 0;
        while let Some(d @ b'0'..=b'9') = c {
            n = n * 10 + u64::from(d - b'0');
            if n > i32::MAX as u64 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "number too big"));
            }
            c = self.byte()?;
        }
        Ok(n as u32)
    }

    /// A P1 pixel: a single 0 or 1 (they need no spaces between them).
    fn bit(&mut self) -> io::Result<u32> {
        match self.skip_space()? {
            c @ (b'0' | b'1') => Ok(u32::from(c - b'0')),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "0 or 1 expected")),
        }
    }
}

fn decode<R: Read>(input: &mut Input<R>, max_side: u32) -> io::Result<Option<Pixels>> {
    if input.byte()? != Some(b'P') {
        return Ok(None);
    }
    let kind = match input.byte()? {
        Some(c @ b'1'..=b'6') => c - b'0',
        _ => return Ok(None),
    };
    let (w, h) = (input.number()?, input.number()?);
    let bitmap = kind == 1 || kind == 4;
    let maxval = if bitmap { 1 } else { input.number()? };
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || maxval == 0 || maxval > 65535 {
        return Ok(None);
    }
    // after the header comes exactly one whitespace character before binary data (number() ate it)
    let text = kind <= 3;
    let channels = if kind == 3 || kind == 6 { 3 } else { 1 };
    let bytes_per_sample = if maxval > 255 { 2 } else { 1 };

    let limit = max_side.max(1);
    let step = (w.max(h) / limit).max(1); // the result stays at least the limit, then shrinks
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = vec![0u8; ow as usize * oh as usize * 4];
    let row_bytes = if bitmap { w.div_ceil(8) as usize } else { (w * channels * bytes_per_sample) as usize };
    let mut row = vec![0u8; if text { 0 } else { row_bytes }];
    let mut samples = [0u32; 3];
    let channels = channels as usize;
    let scale = |v: u32| if maxval == 255 { v.min(255) as u8 } else { (f64::from(v.min(maxval)) * 255.0 / f64::from(maxval)).round() as u8 };
    for y in 0..h {
        let keep = y % step == 0;
        if !text {
            input.r.read_exact(&mut row)?;
            if !keep {
                continue;
            }
        }
        for x in 0..w {
            if text {
                for s in samples.iter_mut().take(channels) {
                    *s = if kind == 1 { input.bit()? } else { input.number()? };
                }
            } else if bitmap {
                samples[0] = u32::from((row[x as usize >> 3] >> (7 - (x & 7))) & 1);
            } else {
                for (c, s) in samples.iter_mut().enumerate().take(channels) {
                    let at = (x as usize * channels + c) * bytes_per_sample as usize;
                    *s = if bytes_per_sample == 2 { u32::from(row[at]) << 8 | u32::from(row[at + 1]) } else { u32::from(row[at]) };
                }
            }
            if !keep || x % step != 0 {
                continue;
            }
            let rgba = if bitmap {
                if samples[0] == 1 { [0, 0, 0, 255] } else { [255, 255, 255, 255] } // 1 is black
            } else if channels == 1 {
                let v = scale(samples[0]);
                [v, v, v, 255]
            } else {
                [scale(samples[0]), scale(samples[1]), scale(samples[2]), 255]
            };
            let o = ((y / step) * ow + x / step) as usize * 4;
            out[o..o + 4].copy_from_slice(&rgba);
        }
    }
    let p = Pixels { rgba: out, width: ow, height: oh, full_width: w, full_height: h };
    Ok(Some(p.fit(limit)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn write(dir: &Path, name: &str, header: &str, pixels: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, [header.as_bytes(), pixels].concat()).unwrap();
        p
    }

    /// `p` is 2 × 1 with these two pixels.
    fn assert_two(p: Option<Pixels>, left: [u8; 4], right: [u8; 4]) {
        let p = p.expect("pixels");
        assert_eq!((p.width, p.height), (2, 1));
        assert_eq!(p.pixel(0, 0), left, "left");
        assert_eq!(p.pixel(1, 0), right, "right");
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const GREY: [u8; 4] = [200, 200, 200, 255];

    #[test]
    fn binary_text_grey_and_bitmap_forms() {
        for ext in ["ppm", "pgm", "pbm", "pnm"] {
            assert!(is_netpbm(ext), "{ext}");
            assert_eq!(crate::images::Format::of(&ext.to_uppercase()), Some(crate::images::Format::Netpbm));
            assert_eq!(crate::fs::file_types::category(ext, false), crate::fs::Category::Image, "{ext}");
        }
        assert!(!is_netpbm("jp2"));
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        // binary PPM (P6): red then blue
        assert_two(read(&write(d, "a.ppm", "P6\n2 1\n255\n", &[255, 0, 0, 0, 0, 255]), 100).unwrap(), RED, BLUE);
        // text PPM (P3) with a comment
        assert_two(read(&write(d, "t.ppm", "P3\n# made by hand\n2 1\n255\n255 0 0  0 0 255\n", &[]), 100).unwrap(), RED, BLUE);
        // grey PGM (P5), 8 and 16 bits: values kept, not brightened
        assert_two(read(&write(d, "g.pgm", "P5\n2 1\n255\n", &[200, 0]), 100).unwrap(), GREY, BLACK);
        assert_two(read(&write(d, "g16.pgm", "P5\n2 1\n65535\n", &[0xc8, 0xc8, 0, 0]), 100).unwrap(), GREY, BLACK);
        // bitmap PBM (P4): 1 is black
        assert_two(read(&write(d, "b.pbm", "P4\n2 1\n", &[0b0100_0000]), 100).unwrap(), WHITE, BLACK);
    }

    #[test]
    fn text_bitmaps_and_grey_and_damaged_files() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        // P1 pixels need no spaces between them; P2 is text grey with a maxval of 15
        assert_two(read(&write(d, "t.pbm", "P1 2 1\n10", &[]), 100).unwrap(), BLACK, WHITE);
        assert_two(read(&write(d, "t.pgm", "P2 2 1 15\n15 0", &[]), 100).unwrap(), WHITE, BLACK);

        assert_eq!(read(&write(d, "cut.ppm", "P6\n4 4\n255\n", &[0; 10]), 100).unwrap(), None, "cut off");
        assert_eq!(read(&write(d, "bad.ppm", "P9\n1 1\n255\n", &[]), 100).unwrap(), None, "no such type");
        assert_eq!(read(&write(d, "zero.pgm", "P5\n1 1\n0\n ", &[]), 100).unwrap(), None, "maxval 0");
        assert_eq!(
            read(&write(d, "huge.ppm", "P6\n2000000000 2000000000\n255\n", &[]), 100).unwrap(),
            None,
            "a damaged header can't make it allocate gigabytes"
        );
        assert_eq!(read(&write(d, "text.pnm", "hello", &[]), 100).unwrap(), None);
        assert_eq!(read(&write(d, "empty.pbm", "", &[]), 100).unwrap(), None);
    }

    #[test]
    fn big_images_are_decoded_smaller() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, h) = (900u32, 300u32);
        let px: Vec<u8> = (0..w * h).flat_map(|i| if i % w < w / 2 { [255, 0, 0] } else { [0, 0, 255] }).collect();
        let p = read(&write(tmp.path(), "big.ppm", &format!("P6\n{w} {h}\n255\n"), &px), 128).unwrap().unwrap();
        assert_eq!((p.width, p.height, p.full_width, p.full_height), (128, 43, 900, 300));
        assert_eq!(p.pixel(3, 20), RED, "left is red");
        assert_eq!(p.pixel(124, 20), BLUE, "right is blue");
    }
}
