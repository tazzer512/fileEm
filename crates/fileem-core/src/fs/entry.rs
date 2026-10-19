//! Immutable snapshot of one directory entry, as shown in the file list.

use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use super::file_types::{self, Category};

#[derive(Debug, Clone)]
pub struct FileEntry {
    path: PathBuf,
    name: String,
    directory: bool,
    symlink: bool,
    size: Option<u64>,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    accessed: Option<SystemTime>,
    hidden: bool,
    extension: String,
    category: Category,
    type_name: String,
    /// Raw attributes, kept so the (slow, owner lookup) attribute string is only built on demand.
    raw: Option<RawAttributes>,
    attributes: OnceLock<String>,
}

#[derive(Debug, Clone, Copy)]
enum RawAttributes {
    #[cfg(unix)]
    Posix { mode: u32, uid: u32 },
    #[cfg(windows)]
    Dos { flags: u32 },
    #[allow(dead_code)]
    None,
}

impl FileEntry {
    /// Reads the attributes of `p`. Never fails: unreadable entries still get a row.
    pub fn read(p: &Path) -> FileEntry {
        let file_name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let own = match p.symlink_metadata() {
            Ok(m) => m,
            Err(_) => return FileEntry::new(p, None, false, file_name.starts_with('.')),
        };
        let link = own.file_type().is_symlink();
        // a broken link keeps the link's own attributes
        let meta = if link { p.metadata().unwrap_or(own) } else { own };
        let hidden = hidden_attribute(&meta).unwrap_or_else(|| file_name.starts_with('.'));
        FileEntry::new(p, Some(&meta), link, hidden)
    }

    fn new(path: &Path, meta: Option<&Metadata>, symlink: bool, hidden: bool) -> FileEntry {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => path.to_string_lossy().into_owned(),
        };
        let directory = meta.is_some_and(|m| m.is_dir());
        let extension = file_types::extension(&name, directory);
        let mut category = file_types::category(&extension, directory);
        if symlink && !directory && category == Category::File {
            category = Category::Link;
        }
        let type_name =
            if symlink && directory { "Folder link".into() } else { file_types::description(&extension, category) };
        FileEntry {
            path: path.to_path_buf(),
            name,
            directory,
            symlink,
            size: meta.filter(|_| !directory).map(|m| m.len()),
            modified: meta.and_then(|m| m.modified().ok()),
            created: meta.and_then(|m| m.created().ok()),
            accessed: meta.and_then(|m| m.accessed().ok()),
            hidden,
            extension,
            category,
            type_name,
            raw: meta.map(raw_attributes),
            attributes: OnceLock::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_dir(&self) -> bool {
        self.directory
    }

    pub fn is_symlink(&self) -> bool {
        self.symlink
    }

    /// Size in bytes; `None` for folders and unreadable entries.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    pub fn created(&self) -> Option<SystemTime> {
        self.created
    }

    pub fn accessed(&self) -> Option<SystemTime> {
        self.accessed
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    pub fn extension(&self) -> &str {
        &self.extension
    }

    pub fn category(&self) -> Category {
        self.category
    }

    /// "PNG Image", "Folder", "Folder link".
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// "drwxr-xr-x  owner" on POSIX, "RHSA" style flags on Windows. Computed lazily: the owner
    /// lookup is about 80% of the cost of a listing.
    pub fn attributes(&self) -> &str {
        self.attributes.get_or_init(|| self.format_attributes())
    }

    fn format_attributes(&self) -> String {
        match self.raw {
            #[cfg(unix)]
            Some(RawAttributes::Posix { mode, uid }) => {
                let kind = if self.symlink { 'l' } else if self.directory { 'd' } else { '-' };
                let mut s = String::with_capacity(32);
                s.push(kind);
                for (bit, c) in [
                    (0o400, 'r'), (0o200, 'w'), (0o100, 'x'),
                    (0o040, 'r'), (0o020, 'w'), (0o010, 'x'),
                    (0o004, 'r'), (0o002, 'w'), (0o001, 'x'),
                ] {
                    s.push(if mode & bit != 0 { c } else { '-' });
                }
                s.push_str("  ");
                s.push_str(&owner_name(uid));
                s
            }
            #[cfg(windows)]
            Some(RawAttributes::Dos { flags }) => {
                let mut s = String::new();
                for (bit, c) in [(0x1, 'R'), (0x2, 'H'), (0x4, 'S'), (0x20, 'A')] {
                    if flags & bit != 0 {
                        s.push(c);
                    }
                }
                if self.symlink {
                    s.push('L');
                }
                s
            }
            _ => String::new(),
        }
    }
}

#[cfg(unix)]
fn raw_attributes(m: &Metadata) -> RawAttributes {
    use std::os::unix::fs::MetadataExt;
    RawAttributes::Posix { mode: m.mode(), uid: m.uid() }
}

#[cfg(windows)]
fn raw_attributes(m: &Metadata) -> RawAttributes {
    use std::os::windows::fs::MetadataExt;
    RawAttributes::Dos { flags: m.file_attributes() }
}

#[cfg(not(any(unix, windows)))]
fn raw_attributes(_: &Metadata) -> RawAttributes {
    RawAttributes::None
}

#[cfg(windows)]
fn hidden_attribute(m: &Metadata) -> Option<bool> {
    use std::os::windows::fs::MetadataExt;
    Some(m.file_attributes() & 0x2 != 0)
}

#[cfg(not(windows))]
fn hidden_attribute(_: &Metadata) -> Option<bool> {
    None
}

/// The user name for `uid`, or the number when it has none.
#[cfg(unix)]
pub(crate) fn owner_name(uid: u32) -> String {
    use std::collections::HashMap;
    use std::ffi::CStr;
    use std::sync::Mutex;

    // getpwuid_r reads /etc/passwd (or NSS) on every call; a listing shows the same few owners
    static CACHE: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(name) = cache.lock().unwrap().get(&uid) {
        return name.clone();
    }
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: all pointers are valid for the call; buf outlives the use of pwd's string fields.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    let name = if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
        unsafe { CStr::from_ptr(pwd.pw_name) }.to_string_lossy().into_owned()
    } else {
        uid.to_string()
    };
    cache.lock().unwrap().insert(uid, name.clone());
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn reads_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("Report.PDF");
        fs::write(&f, "12345").unwrap();
        let time = UNIX_EPOCH + Duration::from_millis(1_000_000_000_000);
        fs::File::options().write(true).open(&f).unwrap().set_modified(time).unwrap();
        let e = FileEntry::read(&f);
        assert_eq!(e.name(), "Report.PDF");
        assert_eq!(e.size(), Some(5));
        assert!(!e.is_dir());
        assert!(!e.is_symlink());
        assert_eq!(e.extension(), "pdf");
        assert_eq!(e.category(), Category::Pdf);
        assert_eq!(e.modified(), Some(time));
    }

    #[test]
    fn folders_have_no_size() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub.d");
        fs::create_dir(&sub).unwrap();
        let e = FileEntry::read(&sub);
        assert!(e.is_dir());
        assert_eq!(e.size(), None);
        assert_eq!(e.type_name(), "Folder");
        assert_eq!(e.extension(), "");
    }

    #[test]
    fn missing_path_still_produces_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let e = FileEntry::read(&dir.path().join("gone"));
        assert_eq!(e.name(), "gone");
        assert_eq!(e.modified(), None);
        assert_eq!(e.attributes(), "");
    }

    #[cfg(unix)]
    #[test]
    fn posix_dot_files_are_hidden_and_attributes_show_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(".secret");
        fs::write(&f, "x").unwrap();
        let e = FileEntry::read(&f);
        assert!(e.is_hidden());
        let a = e.attributes();
        assert!(a.starts_with('-') && a[1..10].chars().all(|c| "rwx-".contains(c)) && &a[10..12] == "  ", "{a}");
        assert!(FileEntry::read(dir.path()).attributes().starts_with('d'));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_report_the_target_and_broken_links_do_not_fail() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let target = d.join("t.txt");
        fs::write(&target, "abc").unwrap();
        symlink(&target, d.join("link")).unwrap();
        let e = FileEntry::read(&d.join("link"));
        assert!(e.is_symlink());
        assert_eq!(e.size(), Some(3));
        assert!(e.attributes().starts_with('l'));

        symlink(d.join(".."), d.join("dlink")).unwrap();
        let dl = FileEntry::read(&d.join("dlink"));
        assert!(dl.is_dir());
        assert_eq!(dl.type_name(), "Folder link");

        symlink(d.join("nothing"), d.join("broken")).unwrap();
        let b = FileEntry::read(&d.join("broken"));
        assert!(b.is_symlink());
        assert!(!b.is_dir());
    }
}
