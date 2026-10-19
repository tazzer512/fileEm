//! Reading a folder into rows, and the file list's default order.

use std::cmp::Ordering;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use super::entry::FileEntry;
use crate::util::natural_order;

/// Reads every entry of `dir` (hidden ones too; the view filters them). Stops with
/// [`io::ErrorKind::Interrupted`] once `cancel` is set, so a stale listing ends early.
pub fn list(dir: &Path, cancel: &AtomicBool) -> io::Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        if cancel.load(AtomicOrdering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "listing cancelled"));
        }
        // an entry that vanished or can't be read still gets a row from its path
        match item {
            Ok(e) => out.push(FileEntry::read(&e.path())),
            Err(_) => continue,
        }
    }
    Ok(out)
}

/// Only the subfolders of `dir` (following links to folders), naturally sorted: the tree's children.
pub fn subfolders(dir: &Path, show_hidden: bool) -> io::Result<Vec<FileEntry>> {
    let mut out: Vec<FileEntry> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| FileEntry::read(&e.path()))
        .filter(|e| show_hidden || !e.is_hidden())
        .collect();
    out.sort_by(|a, b| natural_order::compare(a.name(), b.name()));
    Ok(out)
}

/// Sortable columns of the details view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Type,
    Modified,
}

/// Folders first, then the chosen column, then the natural name order (also the tie-breaker).
/// `ascending` flips the column and name order, never folders-first.
pub fn compare(a: &FileEntry, b: &FileEntry, key: SortKey, ascending: bool) -> Ordering {
    let folders = b.is_dir().cmp(&a.is_dir());
    if folders != Ordering::Equal {
        return folders;
    }
    let by_key = match key {
        SortKey::Name => Ordering::Equal,
        SortKey::Size => a.size().cmp(&b.size()),
        SortKey::Type => natural_order::compare(a.type_name(), b.type_name()),
        SortKey::Modified => a.modified().cmp(&b.modified()),
    };
    let ord = by_key.then_with(|| natural_order::compare(a.name(), b.name()));
    if ascending {
        ord
    } else {
        ord.reverse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(FileEntry::name).collect()
    }

    #[test]
    fn lists_everything_and_sorts_folders_first_naturally() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for f in ["b10.txt", "b2.txt", ".hidden", "A.txt"] {
            fs::write(d.join(f), "x").unwrap();
        }
        fs::create_dir(d.join("zeta")).unwrap();
        fs::create_dir(d.join("alpha")).unwrap();
        let mut list = list(d, &AtomicBool::new(false)).unwrap();
        list.sort_by(|a, b| compare(a, b, SortKey::Name, true));
        assert_eq!(names(&list), ["alpha", "zeta", ".hidden", "A.txt", "b2.txt", "b10.txt"]);
        list.sort_by(|a, b| compare(a, b, SortKey::Name, false));
        assert_eq!(names(&list), ["zeta", "alpha", "b10.txt", "b2.txt", "A.txt", ".hidden"], "folders stay first");
    }

    #[test]
    fn size_sort_breaks_ties_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::write(d.join("big"), "xxxx").unwrap();
        fs::write(d.join("b"), "x").unwrap();
        fs::write(d.join("a"), "x").unwrap();
        let mut list = list(d, &AtomicBool::new(false)).unwrap();
        list.sort_by(|a, b| compare(a, b, SortKey::Size, true));
        assert_eq!(names(&list), ["a", "b", "big"]);
    }

    #[test]
    fn a_cancelled_listing_stops() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), "x").unwrap();
        let err = list(dir.path(), &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn subfolders_are_sorted_and_hidden_ones_optional() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for f in ["x10", "x9", ".git"] {
            fs::create_dir(d.join(f)).unwrap();
        }
        fs::write(d.join("file"), "x").unwrap();
        assert_eq!(names(&subfolders(d, false).unwrap()), ["x9", "x10"]);
        assert_eq!(names(&subfolders(d, true).unwrap()), [".git", "x9", "x10"]);
    }
}
