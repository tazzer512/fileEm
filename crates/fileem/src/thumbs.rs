//! Image thumbnails for the icon views (the Java `Thumbnails`). Two background threads decode
//! what the grids ask for, newest wishes first; nothing touches the disk on the UI thread.
//! Kept in an LRU cache bounded by pixel count, keyed by file, size, date and load size (a
//! changed file gets a new entry). Thumbnails load at a few fixed sizes ([`load_size`]) and the
//! view scales them, so dragging the size slider doesn't decode every image again per pixel.
//! Decoding is `fileem_core::images` (the common formats, Photoshop, camera raws, JPEG 2000 and
//! Netpbm).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

use fileem_core::fs::FileEntry;
use fileem_core::images::{self, Format};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::view_mode::MAX_ICON;

/// About 32 MB of decoded RGBA pixels.
const PIXEL_BUDGET: u64 = 8 << 20;
const WORKERS: usize = 2;

/// Whether `e` is an image file of a size worth decoding for a thumbnail.
pub fn supports(e: &FileEntry) -> bool {
    !e.is_dir() && Format::of(e.extension()).is_some_and(|f| e.size().is_some_and(|s| s > 0 && s <= f.max_file_size()))
}

/// The size a thumbnail for `px` pixels is loaded at: the next multiple of 32, at least 48.
pub fn load_size(px: u32) -> u32 {
    px.div_ceil(32).saturating_mul(32).clamp(48, MAX_ICON)
}

/// Identifies one thumbnail: a changed file (size, date) or another load size is another one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    load: u32,
}

impl Key {
    /// The thumbnail of `e` for icons of `px` pixels.
    pub fn of(e: &FileEntry, px: u32) -> Key {
        Key { path: e.path().to_path_buf(), size: e.size().unwrap_or(0), modified: e.modified(), load: load_size(px) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Decodes the image at `path` and shrinks it to fit `load × load` (never enlarges); `None` when
/// it can't be read.
pub fn decode(path: &Path, load: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let p = images::decode(path, load).ok()??;
    Some(SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height))
}

type Decoded = (Key, Option<SharedPixelBuffer<Rgba8Pixel>>);

#[derive(Default)]
struct Queue {
    wanted: VecDeque<Key>,
    busy: HashSet<Key>,
    done: Vec<Decoded>,
    stop: bool,
}

struct Workers {
    queue: Mutex<Queue>,
    wake: Condvar,
    /// Called on a worker thread after each result (the window then collects them).
    notify: Box<dyn Fn() + Send + Sync>,
}

pub struct Thumbnails {
    cache: HashMap<Key, (Image, u64)>,
    /// Use counter for the LRU order.
    tick: u64,
    pixels: u64,
    budget: u64,
    /// Files that can't be shown: not tried again until they change.
    failed: HashSet<Key>,
    workers: Arc<Workers>,
    started: bool,
}

impl Thumbnails {
    /// `notify` runs on a decoder thread whenever a result is ready for [`Thumbnails::take_done`].
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Thumbnails {
        Thumbnails {
            cache: HashMap::new(),
            tick: 0,
            pixels: 0,
            budget: PIXEL_BUDGET,
            failed: HashSet::new(),
            workers: Arc::new(Workers { queue: Mutex::new(Queue::default()), wake: Condvar::new(), notify: Box::new(notify) }),
            started: false,
        }
    }

    /// The cached thumbnail, if any (and marks it used).
    pub fn get(&mut self, key: &Key) -> Option<Image> {
        self.tick += 1;
        let tick = self.tick;
        self.cache.get_mut(key).map(|(img, used)| {
            *used = tick;
            img.clone()
        })
    }

    /// Asks for these thumbnails, first ones first. Replaces the earlier wishes that haven't
    /// started: only what the grids show now is decoded.
    pub fn want(&mut self, keys: Vec<Key>) {
        let mut q = self.workers.queue.lock().unwrap();
        q.wanted = keys
            .into_iter()
            .filter(|k| !self.cache.contains_key(k) && !self.failed.contains(k) && !q.busy.contains(k))
            .collect();
        let any = !q.wanted.is_empty();
        drop(q);
        if any {
            self.start();
            self.workers.wake.notify_all();
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        for _ in 0..WORKERS {
            let w = self.workers.clone();
            std::thread::Builder::new()
                .name("fileem-thumbs".into())
                .spawn(move || work(&w))
                .expect("spawn a thumbnail thread");
        }
    }

    /// Takes the decoded results into the cache; returns the keys now available.
    pub fn take_done(&mut self) -> Vec<Key> {
        let done = std::mem::take(&mut self.workers.queue.lock().unwrap().done);
        let mut ready = Vec::new();
        for (key, pixels) in done {
            match pixels {
                Some(buf) => {
                    self.pixels += u64::from(buf.width()) * u64::from(buf.height());
                    self.tick += 1;
                    self.cache.insert(key.clone(), (Image::from_rgba8(buf), self.tick));
                    ready.push(key);
                }
                None => {
                    self.failed.insert(key);
                }
            }
        }
        self.trim();
        ready
    }

    /// Drops the least recently used thumbnails while over the pixel budget.
    fn trim(&mut self) {
        while self.pixels > self.budget {
            let Some(oldest) = self.cache.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else { break };
            if let Some((img, _)) = self.cache.remove(&oldest) {
                let s = img.size();
                self.pixels = self.pixels.saturating_sub(u64::from(s.width) * u64::from(s.height));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.workers.queue.lock().unwrap().stop = true;
        self.workers.wake.notify_all();
    }
}

fn work(w: &Workers) {
    loop {
        let key = {
            let mut q = w.queue.lock().unwrap();
            loop {
                if q.stop {
                    return;
                }
                if let Some(k) = q.wanted.pop_front() {
                    q.busy.insert(k.clone());
                    break k;
                }
                q = w.wake.wait(q).unwrap();
            }
        };
        let pixels = decode(&key.path, key.load);
        {
            let mut q = w.queue.lock().unwrap();
            q.busy.remove(&key);
            q.done.push((key, pixels));
        }
        (w.notify)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn png(path: &Path, w: u32, h: u32) {
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255])).save(path).unwrap();
    }

    #[test]
    fn load_sizes_are_multiples_of_32_within_limits() {
        assert_eq!(load_size(16), 48);
        assert_eq!(load_size(32), 48);
        assert_eq!(load_size(48), 64, "as in Java: (px + 31) / 32 * 32");
        assert_eq!(load_size(77), 96);
        assert_eq!(load_size(96), 96);
        assert_eq!(load_size(120), 128);
        assert_eq!(load_size(320), 320);
        assert_eq!(load_size(5000), 320);
    }

    #[test]
    fn only_image_files_of_sensible_size_are_supported() {
        let tmp = tempfile::tempdir().unwrap();
        png(&tmp.path().join("a.PNG"), 4, 4);
        fs::write(tmp.path().join("empty.png"), "").unwrap();
        fs::write(tmp.path().join("notes.txt"), "x").unwrap();
        fs::create_dir(tmp.path().join("pics.png")).unwrap();
        let ok = |n: &str| supports(&FileEntry::read(&tmp.path().join(n)));
        assert!(ok("a.PNG"));
        assert!(!ok("empty.png"));
        assert!(!ok("notes.txt"));
        assert!(!ok("pics.png"), "a folder");
        for name in ["art.psd", "IMG_1.CR2", "scan.jp2", "pixels.ppm"] {
            fs::write(tmp.path().join(name), "x").unwrap();
            assert!(ok(name), "{name}: our own readers");
        }
        // big files: the common formats decode all of it, raw previews read only a part
        for name in ["big.png", "big.cr2"] {
            fs::File::create(tmp.path().join(name)).unwrap().set_len(41 << 20).unwrap();
        }
        assert!(!ok("big.png"));
        assert!(ok("big.cr2"));
    }

    #[test]
    fn images_shrink_to_fit_and_damaged_files_give_none() {
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big.png");
        png(&big, 400, 200);
        let t = decode(&big, 96).unwrap();
        assert_eq!((t.width(), t.height()), (96, 48), "the aspect ratio stays");
        let small = tmp.path().join("small.png");
        png(&small, 20, 10);
        let t = decode(&small, 96).unwrap();
        assert_eq!((t.width(), t.height()), (20, 10), "never enlarged");
        let broken = tmp.path().join("broken.png");
        fs::write(&broken, "not a png").unwrap();
        assert!(decode(&broken, 96).is_none());
    }

    #[test]
    fn wanted_thumbnails_decode_in_the_background_and_stay_within_the_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let mut keys = Vec::new();
        for i in 0..3 {
            let p = tmp.path().join(format!("p{i}.png"));
            png(&p, 200, 200);
            keys.push(Key::of(&FileEntry::read(&p), 96));
        }
        let broken = tmp.path().join("broken.png");
        fs::write(&broken, "nope").unwrap();
        let broken = Key::of(&FileEntry::read(&broken), 96);
        keys.push(broken.clone());

        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let mut t = Thumbnails::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        t.budget = 2 * 96 * 96; // room for two
        t.want(keys.clone());
        let start = Instant::now();
        let mut ready = Vec::new();
        while ready.len() + usize::from(t.failed.contains(&broken)) < 4 {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(5));
            ready.extend(t.take_done());
        }
        assert_eq!(ready.len(), 3);
        assert!(calls.load(Ordering::SeqCst) >= 4, "notified for each result");
        assert_eq!(t.len(), 2, "the oldest was dropped");
        assert!(t.failed.contains(&broken));
        t.want(vec![broken.clone()]);
        assert!(t.workers.queue.lock().unwrap().wanted.is_empty(), "a failure is not tried again");
        let newest = ready.last().unwrap().clone();
        assert!(t.get(&newest).is_some_and(|img| img.size().width == 96));
    }
}
