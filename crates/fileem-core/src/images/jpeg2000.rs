//! JPEG 2000 (`.jp2`, `.j2k` and the other variants) with the pure-Rust `hayro-jpeg2000`
//! decoder (the Java app used jai-imageio). The format stores the picture at several
//! resolutions, each half the next; decoding the smallest one at least the asked size skips most
//! of the work, so a thumbnail of a huge scan takes a fraction of the time of the whole picture.

use std::io;
use std::path::Path;

use hayro_jpeg2000::{ColorSpace, DecodeSettings, DecoderContext, Image};

use super::{Format, Pixels};

const EXTENSIONS: [&str; 6] = ["jp2", "j2k", "j2c", "jpc", "jpf", "jpx"];

pub fn is_jpeg2000(extension: &str) -> bool {
    EXTENSIONS.contains(&extension)
}

/// Decodes `path` with its longer side at most `max_side` (never enlarged); `None` when it isn't
/// a JPEG 2000 image or can't be decoded.
pub fn read(path: &Path, max_side: u32) -> io::Result<Option<Pixels>> {
    if std::fs::metadata(path)?.len() > Format::Jpeg2000.max_file_size() {
        return Ok(None); // the decoder needs it all in memory
    }
    let data = std::fs::read(path)?;
    Ok(decode(&data, max_side.max(1)))
}

fn decode(data: &[u8], limit: u32) -> Option<Pixels> {
    let full = Image::new(data, &DecodeSettings::default()).ok()?;
    let (fw, fh) = (full.width(), full.height());
    if fw == 0 || fh == 0 {
        return None;
    }
    // the decoder skips as many halvings as keep both sides at least this
    let scale = f64::from(limit) / f64::from(fw.max(fh));
    let target = (((f64::from(fw) * scale).ceil() as u32).max(1), ((f64::from(fh) * scale).ceil() as u32).max(1));
    let settings = DecodeSettings { target_resolution: (scale < 1.0).then_some(target), ..DecodeSettings::default() };
    let image = Image::new(data, &settings).ok()?;
    let (w, h) = (image.width(), image.height());
    let (color, alpha) = (image.color_space().clone(), image.has_alpha());
    let mut ctx = DecoderContext::default();
    let samples = image.decode(&mut ctx).ok()?.data_u8();
    let channels = color.num_channels() as usize + usize::from(alpha);
    if samples.len() != w as usize * h as usize * channels {
        return None;
    }
    let rgba: Vec<u8> = samples.chunks_exact(channels).flat_map(|s| to_rgba(&color, alpha, s)).collect();
    Some(Pixels { rgba, width: w, height: h, full_width: fw, full_height: fh }.fit(limit))
}

fn to_rgba(color: &ColorSpace, alpha: bool, s: &[u8]) -> [u8; 4] {
    let a = if alpha { s[s.len() - 1] } else { 255 };
    let colors = s.len() - usize::from(alpha);
    match (color, colors) {
        (ColorSpace::CMYK, 4) => {
            let k = 255 - u32::from(s[3]);
            let ink = |c: u8| ((255 - u32::from(c)) * k / 255) as u8;
            [ink(s[0]), ink(s[1]), ink(s[2]), a]
        }
        (_, 1 | 2) => [s[0], s[0], s[0], a],
        // RGB, or an ICC profile's three channels taken as RGB
        _ => [s[0], s[1], s[2], a],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A test file, made with OpenJPEG (through Pillow, lossless). red-blue: 120 × 80, left half
    /// red, right half blue (also as a bare .j2k codestream); big: the same at 1000 × 600, five
    /// decomposition levels; gray: 8-bit grey 200; gray16: 16-bit grey 51400 (= 200 in 8 bits).
    pub(crate) fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/jpeg2000").join(name)
    }

    #[test]
    fn knows_the_extensions() {
        for ext in ["jp2", "j2k", "J2K", "j2c", "jpc", "jpf", "jpx"] {
            assert_eq!(Format::of(ext), Some(Format::Jpeg2000), "{ext}");
            assert_eq!(crate::fs::file_types::category(&ext.to_lowercase(), false), crate::fs::Category::Image, "{ext}");
        }
        assert!(!is_jpeg2000("jpg") && !is_jpeg2000(""));
    }

    #[test]
    fn reads_jp2_files_and_bare_codestreams() {
        for name in ["red-blue.jp2", "red-blue.j2k"] {
            let p = read(&fixture(name), 4096).unwrap().expect(name);
            assert_eq!((p.width, p.height), (120, 80));
            assert_eq!(p.pixel(10, 40), [255, 0, 0, 255], "{name}: left is red");
            assert_eq!(p.pixel(110, 40), [0, 0, 255, 255], "{name}: right is blue");
        }
    }

    #[test]
    fn big_images_decode_a_stored_smaller_resolution_then_shrink_to_the_limit() {
        let data = std::fs::read(fixture("big.jp2")).unwrap();
        let settings = DecodeSettings { target_resolution: Some((256, 154)), ..DecodeSettings::default() };
        assert_eq!(Image::new(&data, &settings).unwrap().width(), 500, "the 500 px level is decoded");
        let p = read(&fixture("big.jp2"), 256).unwrap().unwrap();
        assert_eq!((p.width, p.height), (256, 154), "shrunk to the limit");
        assert_eq!((p.full_width, p.full_height), (1000, 600));
        assert_eq!(p.pixel(5, 77), [255, 0, 0, 255], "left is red");
        assert_eq!(p.pixel(250, 77), [0, 0, 255, 255], "right is blue");
        assert_eq!(read(&fixture("big.jp2"), 5000).unwrap().unwrap().width, 1000, "never enlarged");
    }

    #[test]
    fn grey_images_keep_their_values_at_8_and_16_bits() {
        for name in ["gray.jp2", "gray16.jp2"] {
            let p = read(&fixture(name), 4096).unwrap().expect(name);
            assert_eq!(p.width, 64);
            assert_eq!(p.pixel(36, 1), [200, 200, 200, 255], "{name}: grey 200, opaque");
        }
    }

    #[test]
    fn damaged_or_other_files_give_none_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let write = |name: &str, data: &[u8]| {
            let p = tmp.path().join(name);
            std::fs::write(&p, data).unwrap();
            p
        };
        assert_eq!(read(&write("text.jp2", b"not an image"), 100).unwrap(), None);
        assert_eq!(read(&write("empty.j2k", b""), 100).unwrap(), None);
        let whole = std::fs::read(fixture("big.jp2")).unwrap();
        assert_eq!(read(&write("cut.jp2", &whole[..60]), 100).unwrap(), None, "header only");
        let cut = write("half.jp2", &whole[..whole.len() / 2]);
        let _ = read(&cut, 100); // a cut off codestream: a picture or none, no panic
    }
}
