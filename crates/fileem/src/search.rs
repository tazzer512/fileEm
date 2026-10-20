//! A search window's results, toolkit-free (the Java `SearchWindow`'s table): the found items in
//! arrival order or sorted by a column, the selection and focus, and the status texts.
//! `app/search.rs` runs the search and shows these in `ui/search.slint`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use fileem_core::fs::FileEntry;
use fileem_core::util::{formats, natural_order};

/// More results than anyone reads; stops a pattern like "*" on a whole disk.
pub const MAX_RESULTS: usize = 50_000;

/// The result table's columns, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Folder,
    Size,
    Type,
    Modified,
}

impl Column {
    pub const ALL: [Column; 5] = [Column::Name, Column::Folder, Column::Size, Column::Type, Column::Modified];

    pub fn title(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::Folder => "Folder",
            Column::Size => "Size",
            Column::Type => "Type",
            Column::Modified => "Date modified",
        }
    }

    pub fn index(self) -> usize {
        Column::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Option<Column> {
        Column::ALL.get(i).copied()
    }
}

pub struct Results {
    root: PathBuf,
    /// In the order they were found.
    entries: Vec<FileEntry>,
    /// Indices into `entries`: the rows as shown.
    view: Vec<usize>,
    row_of: HashMap<PathBuf, usize>,
    /// Unsorted (as found) until a column header is clicked.
    sort: Option<(Column, bool)>,
    selected: HashSet<PathBuf>,
    current: Option<PathBuf>,
    anchor: Option<PathBuf>,
}

impl Results {
    pub fn new(root: PathBuf) -> Results {
        Results {
            root,
            entries: Vec::new(),
            view: Vec::new(),
            row_of: HashMap::new(),
            sort: None,
            selected: HashSet::new(),
            current: None,
            anchor: None,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn len(&self) -> usize {
        self.view.len()
    }

    pub fn is_empty(&self) -> bool {
        self.view.is_empty()
    }

    pub fn row(&self, row: usize) -> &FileEntry {
        &self.entries[self.view[row]]
    }

    pub fn rows(&self) -> impl Iterator<Item = &FileEntry> {
        self.view.iter().map(|&i| &self.entries[i])
    }

    /// The result's folder, relative to where the search started ("." for that folder itself).
    pub fn folder_of(&self, e: &FileEntry) -> String {
        let Some(parent) = e.path().parent() else { return String::new() };
        match parent.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
            Ok(rel) => rel.display().to_string(),
            Err(_) => parent.display().to_string(),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.selected.clear();
        self.current = None;
        self.anchor = None;
        self.rebuild();
    }

    /// Adds a batch of found items (sorted in when a column is sorted).
    pub fn append(&mut self, found: Vec<FileEntry>) {
        self.entries.extend(found);
        self.rebuild();
    }

    pub fn sort(&self) -> Option<(Column, bool)> {
        self.sort
    }

    /// A click on a column header: sorts by it, the second click reverses.
    pub fn sort_by(&mut self, col: Column) {
        self.sort = Some(match self.sort {
            Some((c, asc)) if c == col => (col, !asc),
            _ => (col, true),
        });
        self.rebuild();
    }

    /// Replaces the results at or under `src` after it was renamed to `target`: `updated` holds
    /// them re-read at their new paths, in the same order as [`Results::under`] returned them.
    pub fn renamed(&mut self, src: &Path, target: &Path, updated: Vec<FileEntry>) {
        let at: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].path().starts_with(src)).collect();
        for (i, e) in at.into_iter().zip(updated) {
            self.entries[i] = e;
        }
        let moved = |p: &PathBuf| match p.strip_prefix(src) {
            Ok(rest) if rest.as_os_str().is_empty() => target.to_path_buf(),
            Ok(rest) => target.join(rest),
            Err(_) => p.clone(),
        };
        self.selected = self.selected.iter().map(moved).collect();
        self.current = self.current.as_ref().map(moved);
        self.anchor = self.anchor.as_ref().map(moved);
        self.rebuild();
    }

    /// The paths of results at or under any of `roots`, in arrival order.
    pub fn under(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|e| roots.iter().any(|r| e.path().starts_with(r)))
            .map(|e| e.path().to_path_buf())
            .collect()
    }

    /// Drops the results whose paths are in `gone`.
    pub fn remove(&mut self, gone: &[PathBuf]) {
        let gone: HashSet<&PathBuf> = gone.iter().collect();
        self.entries.retain(|e| !gone.contains(&e.path().to_path_buf()));
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut view: Vec<usize> = (0..self.entries.len()).collect();
        if let Some((col, asc)) = self.sort {
            let e = &self.entries;
            let folder: Vec<String> = if col == Column::Folder { e.iter().map(|x| self.folder_of(x)).collect() } else { Vec::new() };
            view.sort_by(|&a, &b| {
                let (x, y) = (&e[a], &e[b]);
                let o = match col {
                    Column::Name => natural_order::compare(x.name(), y.name()),
                    Column::Folder => natural_order::compare(&folder[a], &folder[b]),
                    Column::Size => x.size().cmp(&y.size()),
                    Column::Type => natural_order::compare(x.type_name(), y.type_name()),
                    Column::Modified => x.modified().cmp(&y.modified()),
                };
                if asc {
                    o
                } else {
                    o.reverse()
                }
            });
        }
        self.row_of = view.iter().enumerate().map(|(row, &i)| (self.entries[i].path().to_path_buf(), row)).collect();
        self.view = view;
        let shown = &self.row_of;
        self.selected.retain(|p| shown.contains_key(p));
        for p in [&mut self.current, &mut self.anchor] {
            if p.as_ref().is_some_and(|p| !shown.contains_key(p)) {
                *p = None;
            }
        }
    }

    // ---- selection ----

    pub fn is_selected(&self, row: usize) -> bool {
        self.selected.contains(self.row(row).path())
    }

    pub fn current_row(&self) -> Option<usize> {
        self.current.as_ref().and_then(|p| self.row_of.get(p).copied())
    }

    /// Selected entries in row order.
    pub fn selected_entries(&self) -> Vec<&FileEntry> {
        self.rows().filter(|e| self.selected.contains(e.path())).collect()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_entries().into_iter().map(|e| e.path().to_path_buf()).collect()
    }

    /// A click on `row`: select only it; Ctrl/Cmd toggles it; Shift selects the range from the anchor.
    pub fn click(&mut self, row: usize, ctrl: bool, shift: bool) {
        if row >= self.len() {
            return;
        }
        let path = self.row(row).path().to_path_buf();
        if shift {
            let anchor = self.anchor_row().unwrap_or(row);
            if !ctrl {
                self.selected.clear();
            }
            self.select_range(anchor, row);
        } else if ctrl {
            if !self.selected.remove(&path) {
                self.selected.insert(path.clone());
            }
            self.anchor = Some(path.clone());
        } else {
            self.selected.clear();
            self.selected.insert(path.clone());
            self.anchor = Some(path.clone());
        }
        self.current = Some(path);
    }

    /// A right-click: a row outside the selection becomes the selection; a selected one keeps it.
    pub fn context_click(&mut self, row: usize) {
        if row < self.len() && !self.is_selected(row) {
            self.click(row, false, false);
        }
    }

    /// Arrow keys and friends: plain selects only the new row, Shift extends from the anchor,
    /// Ctrl/Cmd only moves the focus.
    pub fn move_focus(&mut self, delta: isize, ctrl: bool, shift: bool) {
        if self.is_empty() {
            return;
        }
        let from = self.current_row();
        let to = match from {
            Some(r) => (r as isize + delta).clamp(0, self.len() as isize - 1) as usize,
            None => 0,
        };
        let path = self.row(to).path().to_path_buf();
        if shift {
            let anchor = self.anchor_row().or(from).unwrap_or(to);
            self.anchor = Some(self.row(anchor).path().to_path_buf());
            self.selected.clear();
            self.select_range(anchor, to);
        } else if !ctrl {
            self.selected.clear();
            self.selected.insert(path.clone());
            self.anchor = Some(path.clone());
        }
        self.current = Some(path);
    }

    pub fn select_all(&mut self) {
        self.selected = self.rows().map(|e| e.path().to_path_buf()).collect();
    }

    /// Selects and focuses only the result at `path`, if it is shown.
    pub fn select_path(&mut self, path: &Path) -> bool {
        if !self.row_of.contains_key(path) {
            return false;
        }
        self.selected = HashSet::from([path.to_path_buf()]);
        self.anchor = Some(path.to_path_buf());
        self.current = Some(path.to_path_buf());
        true
    }

    fn anchor_row(&self) -> Option<usize> {
        self.anchor.as_ref().and_then(|p| self.row_of.get(p).copied())
    }

    fn select_range(&mut self, a: usize, b: usize) {
        for row in a.min(b)..=a.max(b) {
            let p = self.row(row).path().to_path_buf();
            self.selected.insert(p);
        }
    }
}

/// "3 items found in 12 folders".
pub fn summary(items: usize, folders: u64) -> String {
    format!(
        "{} {} found in {} {}",
        formats::count(items as u64),
        if items == 1 { "item" } else { "items" },
        formats::count(folders),
        if folders == 1 { "folder" } else { "folders" }
    )
}

/// The status line after a search ended: the counts, why it stopped early, how long it took (from
/// a second on) and how many folders could not be read.
pub fn finished(items: usize, folders: u64, complete: bool, seconds: u64, unreadable: u64) -> String {
    let mut text = summary(items, folders);
    if !complete && items >= MAX_RESULTS {
        text = format!("Stopped at {} items: use a narrower pattern.   {text}", formats::count(MAX_RESULTS as u64));
    }
    if seconds > 0 {
        text.push_str(&format!(" ({seconds} s)"));
    }
    if unreadable > 0 {
        text.push_str(&format!("   \u{b7}   {} could not be read", formats::count(unreadable)));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(r: &Results) -> Vec<&str> {
        r.rows().map(|e| e.name()).collect()
    }

    fn results() -> (tempfile::TempDir, Results) {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        fs::create_dir_all(d.join("a/b")).unwrap();
        for (p, n) in [("z10.txt", 5), ("a/z9.txt", 1), ("a/b/m.jpg", 30)] {
            fs::write(d.join(p), "x".repeat(n)).unwrap();
        }
        let mut r = Results::new(d.to_path_buf());
        r.append(vec![FileEntry::read(&d.join("z10.txt")), FileEntry::read(&d.join("a/z9.txt"))]);
        r.append(vec![FileEntry::read(&d.join("a/b/m.jpg")), FileEntry::read(&d.join("a/b"))]);
        (t, r)
    }

    #[test]
    fn rows_come_as_found_until_a_column_is_clicked() {
        let (_t, mut r) = results();
        assert_eq!(names(&r), ["z10.txt", "z9.txt", "m.jpg", "b"]);
        r.sort_by(Column::Name);
        assert_eq!(names(&r), ["b", "m.jpg", "z9.txt", "z10.txt"], "natural order");
        r.sort_by(Column::Name);
        assert_eq!(names(&r), ["z10.txt", "z9.txt", "m.jpg", "b"], "the second click reverses");
        r.sort_by(Column::Size);
        assert_eq!(names(&r), ["b", "z9.txt", "z10.txt", "m.jpg"], "a folder has no size: first");
        r.sort_by(Column::Folder);
        assert_eq!(r.rows().map(|e| r.folder_of(e)).collect::<Vec<_>>(), [".", "a", "a", "a/b"]);
        r.sort_by(Column::Name);
        r.append(vec![FileEntry::read(&r.root().join("a"))]);
        assert_eq!(names(&r)[..2], ["a", "b"], "later results are sorted in");
    }

    #[test]
    fn the_selection_survives_sorting_and_follows_a_rename() {
        let (t, mut r) = results();
        r.click(1, false, false);
        r.click(3, false, true);
        assert_eq!(r.selected_paths().len(), 3);
        r.sort_by(Column::Name);
        let selected: Vec<&str> = r.selected_entries().into_iter().map(|e| e.name()).collect();
        assert_eq!(selected, ["b", "m.jpg", "z9.txt"], "in the new row order");

        // a folder renamed: it and what was found inside follow, still selected
        let d = t.path();
        fs::rename(d.join("a/b"), d.join("a/c")).unwrap();
        let under = r.under(&[d.join("a/b")]);
        assert_eq!(under, [d.join("a/b/m.jpg"), d.join("a/b")]);
        let updated = under.iter().map(|p| FileEntry::read(&d.join("a/c").join(p.strip_prefix(d.join("a/b")).unwrap()))).collect();
        r.renamed(&d.join("a/b"), &d.join("a/c"), updated);
        assert!(r.rows().any(|e| e.path() == d.join("a/c/m.jpg")));
        assert!(r.selected_paths().contains(&d.join("a/c")));
        assert_eq!(r.selected_paths().len(), 3);

        r.remove(&[d.join("a/c"), d.join("a/c/m.jpg")]);
        assert_eq!(names(&r), ["z9.txt", "z10.txt"]);
        assert_eq!(r.selected_paths(), [d.join("a/z9.txt")], "removed results leave the selection");
        r.clear();
        assert!(r.is_empty() && r.current_row().is_none());
    }

    #[test]
    fn keys_and_right_clicks_select_like_the_file_list() {
        let (_t, mut r) = results();
        r.move_focus(1, false, false);
        assert_eq!((r.current_row(), r.selected_paths().len()), (Some(0), 1), "the first arrow lands on the first row");
        r.move_focus(2, false, true);
        assert_eq!(r.selected_paths().len(), 3);
        r.context_click(1);
        assert_eq!(r.selected_paths().len(), 3, "a selected row keeps the selection");
        r.context_click(3);
        assert_eq!(r.selected_paths().len(), 1);
        r.select_all();
        assert_eq!(r.selected_paths().len(), 4);
        let third = r.row(2).path().to_path_buf();
        assert!(r.select_path(&third));
        assert_eq!(r.current_row(), Some(2));
    }

    #[test]
    fn the_status_says_what_was_found_and_why_it_stopped() {
        assert_eq!(summary(1, 1), "1 item found in 1 folder");
        assert_eq!(finished(3, 12, true, 0, 0), "3 items found in 12 folders");
        assert_eq!(finished(3, 12, true, 4, 2), "3 items found in 12 folders (4 s)   \u{b7}   2 could not be read");
        assert!(finished(MAX_RESULTS, 9, false, 0, 0).starts_with("Stopped at 50,000 items: use a narrower pattern."));
        assert_eq!(finished(7, 9, false, 0, 0), "7 items found in 9 folders", "stopped by hand: no reason given");
    }
}
