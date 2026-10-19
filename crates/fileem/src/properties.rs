//! What the Properties window shows (the Java `PropertiesDialog`), toolkit-free. Reading it
//! touches the file system: build it in a background job. Folder sizes come later, from
//! [`fileem_core::fs::measure`].

use std::path::{Path, PathBuf};

use fileem_core::fs::measure::Totals;
use fileem_core::fs::FileEntry;
use fileem_core::util::{formats, fs_util};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub label: String,
    pub value: String,
    /// Long values people copy (name, location, link target) are selectable text.
    pub selectable: bool,
    /// A separator line above this one.
    pub group_start: bool,
}

pub struct Sheet {
    pub title: String,
    pub header: String,
    /// The item itself, for its icon (one item only).
    pub entry: Option<FileEntry>,
    pub lines: Vec<Line>,
    /// Size and contents are counted in the background ("Calculating…" until then).
    pub walk: bool,
}

pub const SIZE: &str = "Size:";
pub const CONTAINS: &str = "Contains:";
pub const CALCULATING: &str = "Calculating…";

fn line(label: &str, value: impl Into<String>) -> Line {
    Line { label: label.into(), value: value.into(), selectable: false, group_start: false }
}

fn text(label: &str, value: impl Into<String>) -> Line {
    Line { selectable: true, ..line(label, value) }
}

fn group(mut l: Line) -> Line {
    l.group_start = true;
    l
}

pub fn sheet(paths: &[PathBuf]) -> Sheet {
    match paths {
        [p] => single(p),
        many => several(many),
    }
}

fn single(p: &Path) -> Sheet {
    let e = FileEntry::read(p);
    let name = fs_util::display_name(p);
    let mut lines = vec![
        line("Type:", e.type_name()),
        text("Location:", p.parent().map(|d| d.display().to_string()).unwrap_or_default()),
    ];
    if e.is_symlink() {
        let target = std::fs::read_link(p).map(|t| t.display().to_string()).unwrap_or_else(|_| "(unreadable)".into());
        lines.push(text("Link target:", target));
    }
    let walk = e.is_dir();
    if walk {
        lines.push(line(SIZE, CALCULATING));
        lines.push(line(CONTAINS, ""));
    } else {
        lines.push(line(SIZE, formats::size_with_exact(e.size().unwrap_or(0))));
    }
    lines.push(group(line("Created:", formats::date_long(e.created()))));
    lines.push(line("Modified:", formats::date_long(e.modified())));
    lines.push(line("Accessed:", formats::date_long(e.accessed())));
    let attributes = e.attributes().to_string();
    if !attributes.is_empty() {
        lines.push(group(line("Attributes:", attributes)));
    }
    lines.push(line("Hidden:", if e.is_hidden() { "Yes" } else { "No" }));
    Sheet { title: format!("{name} Properties"), header: name, entry: Some(e), lines, walk }
}

fn several(paths: &[PathBuf]) -> Sheet {
    let files = paths.iter().filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_file())).count();
    let mut lines = Vec::new();
    let parent = paths.first().and_then(|p| p.parent());
    if let Some(parent) = parent.filter(|d| paths.iter().all(|p| p.parent() == Some(*d))) {
        lines.push(text("Location:", parent.display().to_string()));
    }
    lines.push(line("Selected:", format!("{files} files, {} folders", paths.len() - files)));
    lines.push(line(SIZE, CALCULATING));
    lines.push(line(CONTAINS, ""));
    Sheet {
        title: format!("{} items Properties", paths.len()),
        header: format!("{} items selected", paths.len()),
        entry: None,
        lines,
        walk: true,
    }
}

/// The Size and Contains values for counted totals.
pub fn totals_text(t: Totals) -> (String, String) {
    (formats::size_with_exact(t.bytes), format!("{} files, {} folders", formats::count(t.files), formats::count(t.folders)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn value<'a>(s: &'a Sheet, label: &str) -> Option<&'a str> {
        s.lines.iter().find(|l| l.label == label).map(|l| l.value.as_str())
    }

    #[test]
    fn a_file_shows_its_size_dates_and_location() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("info.txt");
        fs::write(&f, vec![b'x'; 2048]).unwrap();
        let s = sheet(std::slice::from_ref(&f));
        assert_eq!(s.title, "info.txt Properties");
        assert_eq!(s.header, "info.txt");
        assert!(!s.walk);
        assert_eq!(value(&s, SIZE), Some("2.00 KB (2,048 bytes)"));
        assert_eq!(value(&s, "Location:"), Some(tmp.path().display().to_string().as_str()));
        assert_eq!(value(&s, "Hidden:"), Some("No"));
        assert!(!value(&s, "Modified:").unwrap().is_empty());
        assert_eq!(value(&s, CONTAINS), None);
    }

    #[test]
    fn a_folder_is_counted_in_the_background() {
        let tmp = tempfile::tempdir().unwrap();
        let s = sheet(&[tmp.path().to_path_buf()]);
        assert!(s.walk);
        assert_eq!(value(&s, SIZE), Some(CALCULATING));
        assert_eq!(value(&s, CONTAINS), Some(""));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_shows_its_target() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("l");
        std::os::unix::fs::symlink("/some/where", &link).unwrap();
        assert_eq!(value(&sheet(&[link]), "Link target:"), Some("/some/where"));
    }

    #[test]
    fn several_items_show_counts_and_their_common_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a.txt"), tmp.path().join("sub"));
        fs::write(&a, "x").unwrap();
        fs::create_dir(&b).unwrap();
        let s = sheet(&[a, b.clone()]);
        assert_eq!(s.title, "2 items Properties");
        assert_eq!(value(&s, "Selected:"), Some("1 files, 1 folders"));
        assert_eq!(value(&s, "Location:"), Some(tmp.path().display().to_string().as_str()));
        assert!(s.walk);
        let elsewhere = sheet(&[tmp.path().join("a.txt"), b.join("x")]);
        assert_eq!(value(&elsewhere, "Location:"), None, "no common folder");
        assert_eq!(totals_text(Totals { bytes: 1, files: 1000, folders: 2 }), ("1 byte".into(), "1,000 files, 2 folders".into()));
    }
}
