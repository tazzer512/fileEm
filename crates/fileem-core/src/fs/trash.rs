//! Move to the OS recycle bin / Trash.
//!
//! Linux: `gio trash` (knows per-volume trash folders), else a minimal freedesktop.org home trash.
//! Windows and macOS: the `trash` crate (recycle bin / Finder).

use std::io;
use std::path::Path;

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU8, Ordering};

    use crate::util::os;

    /// 0 unknown, 1 gio works, 2 gio missing.
    static GIO: AtomicU8 = AtomicU8::new(0);

    pub fn is_supported() -> bool {
        true
    }

    pub fn move_to_trash(p: &Path) -> io::Result<()> {
        if !gio_trash(p)? {
            freedesktop_trash(p)?;
        }
        Ok(())
    }

    pub fn move_all_to_trash(paths: &[PathBuf]) -> bool {
        if GIO.load(Ordering::Relaxed) == 2 || paths.is_empty() {
            return false;
        }
        let status = Command::new("gio")
            .args(["trash", "--"])
            .args(paths)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(s) => {
                GIO.store(1, Ordering::Relaxed);
                s.success()
            }
            Err(_) => {
                GIO.store(2, Ordering::Relaxed);
                false
            }
        }
    }

    /// Returns false when gio is not installed.
    fn gio_trash(p: &Path) -> io::Result<bool> {
        if GIO.load(Ordering::Relaxed) == 2 {
            return Ok(false);
        }
        let out = match Command::new("gio").args(["trash", "--"]).arg(p).stdin(Stdio::null()).output() {
            Ok(out) => out,
            Err(_) => {
                GIO.store(2, Ordering::Relaxed);
                return Ok(false);
            }
        };
        GIO.store(1, Ordering::Relaxed);
        if out.status.success() {
            return Ok(true);
        }
        let mut msg = String::from_utf8_lossy(&out.stdout).into_owned();
        msg.push_str(&String::from_utf8_lossy(&out.stderr));
        let msg = msg.trim();
        Err(io::Error::other(if msg.is_empty() { "gio trash failed" } else { msg }))
    }

    /// Minimal freedesktop.org trash implementation for the home trash.
    fn freedesktop_trash(p: &Path) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let trash = match std::env::var_os("XDG_DATA_HOME") {
            Some(x) if !x.is_empty() => PathBuf::from(x),
            _ => os::home().join(".local").join("share"),
        }
        .join("Trash");
        let files = trash.join("files");
        let info = trash.join("info");
        fs::create_dir_all(&files)?;
        fs::create_dir_all(&info)?;
        // the volume of the item itself, not of a link's target: links are trashed as links
        let abs = std::path::absolute(p)?;
        let same_volume = abs.parent().map(|parent| -> io::Result<bool> {
            Ok(fs::metadata(parent)?.dev() == fs::metadata(&files)?.dev())
        });
        if !matches!(same_volume, Some(Ok(true))) {
            return Err(io::Error::other(
                "Trash is not available on this volume. Use Shift+Delete to delete permanently.",
            ));
        }
        let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut trash_name = name.clone();
        let mut info_file;
        let mut i = 2;
        loop {
            info_file = info.join(format!("{trash_name}.trashinfo"));
            match fs::OpenOptions::new().write(true).create_new(true).open(&info_file) {
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    trash_name = format!("{name}.{i}");
                    i += 1;
                }
                Err(e) => return Err(e),
            }
        }
        let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
        fs::write(&info_file, format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", encode_path(&abs)))?;
        if let Err(e) = fs::rename(&abs, files.join(&trash_name)) {
            let _ = fs::remove_file(&info_file);
            return Err(e);
        }
        Ok(())
    }

    /// Percent-encodes a path as a URI path (RFC 2396 unreserved characters and '/' stay).
    pub(super) fn encode_path(p: &Path) -> String {
        use std::os::unix::ffi::OsStrExt;
        let mut out = String::new();
        for &b in p.as_os_str().as_bytes() {
            if b.is_ascii_alphanumeric() || b"/-_.!~*'()".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        if out.len() > 1 && out.ends_with('/') {
            out.pop();
        }
        out
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    pub fn is_supported() -> bool {
        true
    }

    pub fn move_to_trash(p: &Path) -> io::Result<()> {
        trash::delete(p).map_err(|e| io::Error::other(format!("Could not move to Trash: {e}")))
    }

    pub fn move_all_to_trash(paths: &[PathBuf]) -> bool {
        !paths.is_empty() && trash::delete_all(paths).is_ok()
    }
}

pub fn is_supported() -> bool {
    imp::is_supported()
}

pub fn move_to_trash(p: &Path) -> io::Result<()> {
    if p.symlink_metadata().is_err() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Not found"));
    }
    imp::move_to_trash(p)
}

/// Trashes several items at once where the platform allows it. Returns false when nothing or not
/// everything was trashed; callers then fall back to [`move_to_trash`] per remaining item to get
/// individual errors.
pub fn move_all_to_trash(paths: &[std::path::PathBuf]) -> bool {
    imp::move_all_to_trash(paths)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn trash_info_paths_are_uri_encoded() {
        assert_eq!(imp::encode_path(Path::new("/home/a b/ü.txt")), "/home/a%20b/%C3%BC.txt");
        assert_eq!(imp::encode_path(Path::new("/")), "/");
    }

    #[test]
    fn a_missing_item_is_not_found_and_never_reaches_the_trash() {
        let dir = tempfile::tempdir().unwrap();
        let err = move_to_trash(&dir.path().join("does-not-exist")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
