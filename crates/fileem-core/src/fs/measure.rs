//! Total size and item counts of files and folders (the Properties dialog's background walk).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::util::fs_util;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
}

/// How often [`measure`] reports totals while it walks.
pub const REPORT_EVERY: Duration = Duration::from_millis(150);

/// Walks `paths` without following links (a link counts as a file of size 0; junctions aren't
/// entered) and returns the totals. A single folder doesn't count itself; with several paths every
/// folder does. Unreadable items are skipped. `report` gets the totals so far every
/// [`REPORT_EVERY`]; `stop` ends the walk early with what was counted.
pub fn measure(paths: &[PathBuf], stop: &AtomicBool, mut report: impl FnMut(Totals)) -> Totals {
    let mut t = Totals::default();
    let mut last = Instant::now();
    for root in paths {
        let mut stack: Vec<PathBuf> = vec![root.clone()];
        while let Some(p) = stack.pop() {
            if stop.load(Ordering::Relaxed) {
                return t;
            }
            let Ok(meta) = p.symlink_metadata() else { continue };
            if !meta.is_dir() {
                t.files += 1;
                if meta.is_file() {
                    t.bytes += meta.len();
                }
            } else {
                if p != *root || paths.len() > 1 {
                    t.folders += 1;
                }
                if !fs_util::is_junction(&p) {
                    push_children(&p, &mut stack);
                }
            }
            if last.elapsed() >= REPORT_EVERY {
                last = Instant::now();
                report(t);
            }
        }
    }
    t
}

fn push_children(dir: &Path, stack: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        stack.extend(rd.flatten().map(|e| e.path()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_files_folders_and_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("top");
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::write(d.join("one.txt"), "12345").unwrap();
        fs::write(d.join("a/two.txt"), "123").unwrap();
        fs::write(d.join("a/b/three.bin"), vec![0u8; 100]).unwrap();
        let stop = AtomicBool::new(false);
        let t = measure(std::slice::from_ref(&d), &stop, |_| {});
        assert_eq!(t, Totals { bytes: 108, files: 3, folders: 2 }, "the folder itself isn't counted");

        let both = measure(&[d.join("a"), d.join("one.txt")], &stop, |_| {});
        assert_eq!(both, Totals { bytes: 108, files: 3, folders: 2 }, "with several paths, each folder counts");
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("top");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&d).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("big"), vec![0u8; 1000]).unwrap();
        std::os::unix::fs::symlink(&outside, d.join("link")).unwrap();
        let t = measure(&[d], &AtomicBool::new(false), |_| {});
        assert_eq!(t, Totals { bytes: 0, files: 1, folders: 0 }, "the link counts as an item, its target doesn't");
    }

    #[test]
    fn stops_when_asked_and_skips_missing_paths() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("f"), "x").unwrap();
        let stopped = measure(&[tmp.path().to_path_buf()], &AtomicBool::new(true), |_| {});
        assert_eq!(stopped, Totals::default());
        let missing = measure(&[tmp.path().join("gone")], &AtomicBool::new(false), |_| {});
        assert_eq!(missing, Totals::default());
    }
}
