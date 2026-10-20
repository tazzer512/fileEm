//! Decoding images for thumbnails (and later the preview), free of any UI toolkit: the common
//! formats through the `image` crate, and our own readers for what it can't show: Photoshop
//! (`psd`), camera raws (`raw`), JPEG 2000 (`jpeg2000`) and Netpbm (`netpbm`). Each one reads
//! only what a picture of the asked size needs (a stored smaller resolution, an embedded
//! preview, sampled rows), so a thumbnail of a huge file stays cheap.

use std::io;
use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};

pub mod jpeg2000;
pub mod netpbm;
pub mod psd;
pub mod raw;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

/// The kinds of image files, by who decodes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// PNG, JPEG, GIF, BMP: the `image` crate.
    Common,
    Psd,
    Raw,
    Jpeg2000,
    Netpbm,
}

const COMMON: [&str; 5] = ["png", "jpg", "jpeg", "gif", "bmp"];

impl Format {
    /// The format of a file with this extension (any case), if it is an image we decode.
    pub fn of(extension: &str) -> Option<Format> {
        let ext = extension.to_ascii_lowercase();
        let ext = ext.as_str();
        if COMMON.contains(&ext) {
            Some(Format::Common)
        } else if psd::is_psd(ext) {
            Some(Format::Psd)
        } else if raw::is_raw(ext) {
            Some(Format::Raw)
        } else if jpeg2000::is_jpeg2000(ext) {
            Some(Format::Jpeg2000)
        } else if netpbm::is_netpbm(ext) {
            Some(Format::Netpbm)
        } else {
            None
        }
    }

    /// Bigger files are not worth decoding for a thumbnail. Photoshop and raw previews read only a
    /// part of the file and Netpbm streams, so even big ones are fine; JPEG 2000 needs the whole
    /// codestream in memory.
    pub fn max_file_size(self) -> u64 {
        match self {
            Format::Common => 40 << 20,
            Format::Jpeg2000 => 256 << 20,
            Format::Psd | Format::Raw | Format::Netpbm => 8 << 30,
        }
    }
}

/// Decoded RGBA pixels (8 bits per channel, row by row), and the size of the whole picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixels {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub full_width: u32,
    pub full_height: u32,
}

impl Pixels {
    /// Pixels shown at their full size.
    pub fn new(rgba: Vec<u8>, width: u32, height: u32) -> Pixels {
        Pixels { rgba, width, height, full_width: width, full_height: height }
    }

    /// The pixel at `x, y` as `[r, g, b, a]`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// Shrunk (averaging) so the longer side is at most `max_side`; never enlarged.
    pub fn fit(self, max_side: u32) -> Pixels {
        let (w, h) = (self.width, self.height);
        let max_side = max_side.max(1);
        if w.max(h) <= max_side {
            return self;
        }
        let scale = f64::from(max_side) / f64::from(w.max(h));
        let nw = ((f64::from(w) * scale).round() as u32).max(1);
        let nh = ((f64::from(h) * scale).round() as u32).max(1);
        Pixels { rgba: shrink(&self.rgba, w, h, nw, nh), width: nw, height: nh, ..self }
    }

    fn from_image(img: RgbaImage) -> Pixels {
        let (w, h) = img.dimensions();
        Pixels::new(img.into_raw(), w, h)
    }
}

/// Decodes `path` with its longer side at most `max_side` pixels (never enlarged), upright.
/// `Ok(None)` when it isn't an image we can show; errors only for files that can't be read.
/// A decoder that panics on a damaged file gives `None` too.
pub fn decode(path: &Path, max_side: u32) -> io::Result<Option<Pixels>> {
    std::panic::catch_unwind(|| decode_any(path, max_side)).unwrap_or(Ok(None))
}

fn decode_any(path: &Path, max_side: u32) -> io::Result<Option<Pixels>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let max_side = max_side.max(1);
    match Format::of(ext).unwrap_or(Format::Common) {
        Format::Common => Ok(decode_common(path, max_side)),
        Format::Psd => Ok(match psd::read(path, max_side)? {
            Some(psd::Preview::Pixels(p)) => Some(p),
            Some(psd::Preview::Jpeg(jpeg)) => decode_jpeg(&jpeg, max_side, Orientation::NoTransforms),
            None => None,
        }),
        Format::Raw => Ok(raw::read(path, max_side)?
            .and_then(|p| decode_jpeg(&p.jpeg, max_side, Orientation::from_exif(p.orientation).unwrap_or(Orientation::NoTransforms)))),
        Format::Jpeg2000 => jpeg2000::read(path, max_side),
        Format::Netpbm => netpbm::read(path, max_side),
    }
}

/// PNG, JPEG, GIF, BMP (by content, whatever the name says), turned upright per EXIF.
fn decode_common(path: &Path, max_side: u32) -> Option<Pixels> {
    let mut decoder = ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let img = DynamicImage::from_decoder(decoder).ok()?;
    Some(finish(img, max_side, orientation))
}

/// An embedded JPEG (raw files, Photoshop thumbnails).
fn decode_jpeg(jpeg: &[u8], max_side: u32, orientation: Orientation) -> Option<Pixels> {
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?;
    Some(finish(img, max_side, orientation))
}

fn finish(img: DynamicImage, max_side: u32, orientation: Orientation) -> Pixels {
    let (fw, fh) = (img.width(), img.height());
    let mut img = if fw.max(fh) > max_side { img.thumbnail(max_side, max_side) } else { img };
    img.apply_orientation(orientation);
    let mut p = Pixels::from_image(img.into_rgba8());
    let swapped = matches!(orientation, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH);
    (p.full_width, p.full_height) = if swapped { (fh, fw) } else { (fw, fh) };
    p
}

/// Scales RGBA pixels down by averaging the source pixels under each target pixel.
pub fn shrink(src: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let (w, h, nw, nh) = (w as usize, h as usize, nw as usize, nh as usize);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let y0 = y * h / nh;
        let y1 = ((y + 1) * h / nh).max(y0 + 1);
        for x in 0..nw {
            let x0 = x * w / nw;
            let x1 = ((x + 1) * w / nw).max(x0 + 1);
            let mut sum = [0u64; 4];
            for yy in y0..y1 {
                let row = &src[(yy * w + x0) * 4..(yy * w + x1) * 4];
                for px in row.as_chunks::<4>().0 {
                    for c in 0..4 {
                        sum[c] += u64::from(px[c]);
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u64;
            let o = (y * nw + x) * 4;
            for c in 0..4 {
                out[o + c] = (sum[c] / count) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_by_extension() {
        assert_eq!(Format::of("PNG"), Some(Format::Common));
        assert_eq!(Format::of("psb"), Some(Format::Psd));
        assert_eq!(Format::of("CR2"), Some(Format::Raw));
        assert_eq!(Format::of("j2k"), Some(Format::Jpeg2000));
        assert_eq!(Format::of("pnm"), Some(Format::Netpbm));
        assert_eq!(Format::of("txt"), None);
        assert_eq!(Format::of(""), None);
    }

    #[test]
    fn shrinking_averages_the_pixels_under_each_target_pixel() {
        let px = |v: u32| v.to_be_bytes(); // 0xRRGGBBAA
        let src: Vec<u8> = [0x000000ff, 0x0000ffff, 0xffffffff, 0xffffffff, 0x000000ff, 0x0000ffff, 0xffffff00, 0xffffff00]
            .into_iter()
            .flat_map(px)
            .collect();
        let out = shrink(&src, 4, 2, 2, 1);
        assert_eq!(out[..4], [0, 0, 0x7f, 0xff], "black and blue: half blue");
        assert_eq!(out[4..], [0xff, 0xff, 0xff, 0x7f], "half transparent white");
    }

    #[test]
    fn common_images_shrink_turn_upright_and_damaged_ones_give_none() {
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big.png");
        RgbaImage::from_pixel(400, 200, image::Rgba([200, 30, 30, 255])).save(&big).unwrap();
        let p = decode(&big, 96).unwrap().unwrap();
        assert_eq!((p.width, p.height, p.full_width, p.full_height), (96, 48, 400, 200));
        let small = tmp.path().join("small.png");
        RgbaImage::from_pixel(20, 10, image::Rgba([1, 2, 3, 255])).save(&small).unwrap();
        assert_eq!(decode(&small, 96).unwrap().unwrap().width, 20, "never enlarged");
        let broken = tmp.path().join("broken.png");
        std::fs::write(&broken, "not a png").unwrap();
        assert_eq!(decode(&broken, 96).unwrap(), None);

        // a photo taken with the camera on its side: 90 × 60 stored, shown 60 × 90
        let photo = tmp.path().join("photo.jpg");
        std::fs::write(&photo, fixtures::with_exif_orientation(&fixtures::jpeg(90, 60), 6)).unwrap();
        let p = decode(&photo, 200).unwrap().unwrap();
        assert_eq!((p.width, p.height), (60, 90));
        assert_eq!((p.full_width, p.full_height), (60, 90));
        assert!(p.pixel(30, 5)[0] > 200, "the red left half is on top now");
        assert!(p.pixel(30, 85)[0] < 50);
    }
}
