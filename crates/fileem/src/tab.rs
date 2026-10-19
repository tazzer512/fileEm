//! One tab's state, free of the UI toolkit: folder, history, rows (filtered + sorted) and selection.
//! The window turns it into Slint models; tests drive it directly.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use fileem_core::fs::listing::{self, SortKey};
use fileem_core::fs::FileEntry;
use fileem_core::util::formats;

/// How a finished listing changes the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    /// A new folder: the old one goes on the back stack, forward is cleared.
    Go,
    Back,
    Forward,
    /// The same folder re-read (watcher, F5): selection and focus are kept.
    Refresh,
}

pub struct Tab {
    dir: Option<PathBuf>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    entries: Vec<FileEntry>,
    /// Indices into `entries`: the visible rows, filtered and sorted.
    view: Vec<usize>,
    /// Row of each visible path.
    row_of: HashMap<PathBuf, usize>,
    sort: SortKey,
    ascending: bool,
    show_hidden: bool,
    selected: HashSet<PathBuf>,
    current: Option<PathBuf>,
    anchor: Option<PathBuf>,
}

impl Default for Tab {
    fn default() -> Self {
        Tab::new()
    }
}

impl Tab {
    pub fn new() -> Tab {
        Tab {
            dir: None,
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            view: Vec::new(),
            row_of: HashMap::new(),
            sort: SortKey::Name,
            ascending: true,
            show_hidden: false,
            selected: HashSet::new(),
            current: None,
            anchor: None,
        }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn back_target(&self) -> Option<&Path> {
        self.back.last().map(PathBuf::as_path)
    }

    pub fn forward_target(&self) -> Option<&Path> {
        self.forward.last().map(PathBuf::as_path)
    }

    pub fn up_target(&self) -> Option<&Path> {
        self.dir.as_deref().and_then(Path::parent)
    }

    /// Applies a finished listing of `dir`. Going up selects the folder we came from.
    pub fn show(&mut self, dir: PathBuf, entries: Vec<FileEntry>, nav: Nav) {
        let old = self.dir.take();
        match nav {
            Nav::Go => {
                if let Some(old) = old.clone().filter(|o| *o != dir) {
                    self.back.push(old);
                    self.forward.clear();
                }
            }
            Nav::Back => {
                self.back.pop();
                if let Some(old) = old.clone() {
                    self.forward.push(old);
                }
            }
            Nav::Forward => {
                self.forward.pop();
                if let Some(old) = old.clone() {
                    self.back.push(old);
                }
            }
            Nav::Refresh => {}
        }
        self.entries = entries;
        let same_folder = old.as_deref() == Some(dir.as_path());
        self.dir = Some(dir);
        if nav != Nav::Refresh || !same_folder {
            self.selected.clear();
            self.anchor = None;
            self.current = None;
            // coming up out of a child: focus and select it, as Explorer does
            if let Some(child) = old.filter(|o| o.parent() == self.dir.as_deref()) {
                self.selected.insert(child.clone());
                self.anchor = Some(child.clone());
                self.current = Some(child);
            }
        }
        self.rebuild();
        // keep only what still exists and is visible
        let visible = &self.row_of;
        self.selected.retain(|p| visible.contains_key(p));
        if self.current.as_ref().is_none_or(|c| !visible.contains_key(c)) {
            self.current = self.view.first().map(|&i| self.entries[i].path().to_path_buf());
        }
    }

    fn rebuild(&mut self) {
        let show_hidden = self.show_hidden;
        let mut view: Vec<usize> = (0..self.entries.len()).filter(|&i| show_hidden || !self.entries[i].is_hidden()).collect();
        let (key, asc) = (self.sort, self.ascending);
        view.sort_by(|&a, &b| listing::compare(&self.entries[a], &self.entries[b], key, asc));
        self.row_of = view.iter().enumerate().map(|(row, &i)| (self.entries[i].path().to_path_buf(), row)).collect();
        self.view = view;
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

    pub fn sort(&self) -> (SortKey, bool) {
        (self.sort, self.ascending)
    }

    pub fn set_sort(&mut self, key: SortKey, ascending: bool) {
        self.sort = key;
        self.ascending = ascending;
        self.rebuild();
    }

    /// Sorts by `key`; the same key again flips the direction.
    pub fn sort_by(&mut self, key: SortKey) {
        if self.sort == key {
            self.ascending = !self.ascending;
        } else {
            self.sort = key;
            self.ascending = true;
        }
        self.rebuild();
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Hidden entries stay loaded and are only filtered, so toggling needs no reload.
    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
        self.rebuild();
        let visible = &self.row_of;
        self.selected.retain(|p| visible.contains_key(p));
        if self.current.as_ref().is_none_or(|c| !visible.contains_key(c)) {
            self.current = self.view.first().map(|&i| self.entries[i].path().to_path_buf());
        }
    }

    // ---- selection ----

    pub fn is_selected(&self, row: usize) -> bool {
        self.selected.contains(self.row(row).path())
    }

    pub fn current_row(&self) -> Option<usize> {
        self.current.as_ref().and_then(|p| self.row_of.get(p).copied())
    }

    /// Selected paths in row order.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.rows().filter(|e| self.selected.contains(e.path())).map(|e| e.path().to_path_buf()).collect()
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

    /// A right-click: on a row that isn't selected it selects only that row (a selected row keeps
    /// the selection, so the menu acts on all of it); on empty space (`None`) it clears the
    /// selection, so the menu acts on the folder.
    pub fn context_click(&mut self, row: Option<usize>) {
        match row {
            Some(r) if r < self.len() => {
                if !self.is_selected(r) {
                    self.click(r, false, false);
                }
            }
            _ => self.selected.clear(),
        }
    }

    /// Selected entries in row order.
    pub fn selected_entries(&self) -> Vec<&FileEntry> {
        self.rows().filter(|e| self.selected.contains(e.path())).collect()
    }

    /// Arrow keys and friends: moves the focus by `delta` rows (clamped). Plain selects only the
    /// new row, Shift extends from the anchor, Ctrl/Cmd only moves the focus.
    pub fn move_focus(&mut self, delta: isize, ctrl: bool, shift: bool) {
        if self.view.is_empty() {
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

    /// Space with Ctrl: toggles the focused row.
    pub fn toggle_current(&mut self) {
        if let Some(p) = self.current.clone() {
            if !self.selected.remove(&p) {
                self.selected.insert(p);
            }
        }
    }

    pub fn select_all(&mut self) {
        self.selected = self.rows().map(|e| e.path().to_path_buf()).collect();
    }

    /// Focuses and selects the row of `path`, if it is shown.
    pub fn select_path(&mut self, path: &Path) -> bool {
        if !self.row_of.contains_key(path) {
            return false;
        }
        self.selected.clear();
        self.selected.insert(path.to_path_buf());
        self.anchor = Some(path.to_path_buf());
        self.current = Some(path.to_path_buf());
        true
    }

    fn anchor_row(&self) -> Option<usize> {
        self.anchor.as_ref().and_then(|p| self.row_of.get(p).copied())
    }

    fn select_range(&mut self, a: usize, b: usize) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        for row in lo..=hi {
            let p = self.row(row).path().to_path_buf();
            self.selected.insert(p);
        }
    }

    /// "12 items", "12 items, 3 selected (4.50 KB)".
    pub fn status(&self) -> String {
        let n = self.len();
        let mut s = format!("{} {}", formats::count(n as u64), if n == 1 { "item" } else { "items" });
        let sel: Vec<&FileEntry> = self.rows().filter(|e| self.selected.contains(e.path())).collect();
        if !sel.is_empty() {
            s.push_str(&format!(", {} selected", formats::count(sel.len() as u64)));
            let bytes: u64 = sel.iter().filter_map(|e| e.size()).sum();
            if sel.iter().any(|e| !e.is_dir()) {
                s.push_str(&format!(" ({})", formats::size(Some(bytes))));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;

    fn listing(dir: &Path) -> Vec<FileEntry> {
        listing::list(dir, &AtomicBool::new(false)).unwrap()
    }

    fn names(tab: &Tab) -> Vec<String> {
        tab.rows().map(|e| e.name().to_string()).collect()
    }

    fn folder() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::create_dir(d.join("sub")).unwrap();
        fs::create_dir(d.join("sub/inner")).unwrap();
        for (n, size) in [("b.txt", 3), ("a10.txt", 1), ("a9.txt", 2), (".hidden", 1)] {
            fs::write(d.join(n), "x".repeat(size)).unwrap();
        }
        tmp
    }

    #[test]
    fn right_clicks_select_unselected_rows_and_clear_on_empty_space() {
        let tmp = folder();
        let mut tab = Tab::new();
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Go);
        tab.click(1, false, false);
        tab.click(2, true, false);
        tab.context_click(Some(2));
        assert_eq!(tab.selected_entries().len(), 2, "a selected row keeps the selection");
        tab.context_click(Some(3));
        assert_eq!(tab.selected_paths(), [tmp.path().join("b.txt")], "an unselected row is selected alone");
        assert_eq!(tab.current_row(), Some(3));
        tab.context_click(None);
        assert!(tab.selected_paths().is_empty(), "empty space: the menu is about the folder");
    }

    #[test]
    fn rows_are_folders_first_natural_and_hidden_is_a_filter() {
        let tmp = folder();
        let mut tab = Tab::new();
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Go);
        assert_eq!(names(&tab), ["sub", "a9.txt", "a10.txt", "b.txt"]);
        tab.set_show_hidden(true);
        assert_eq!(names(&tab), ["sub", ".hidden", "a9.txt", "a10.txt", "b.txt"]);
        tab.set_show_hidden(false);
        assert_eq!(tab.len(), 4);
    }

    #[test]
    fn sorting_flips_on_the_same_column_and_keeps_folders_first() {
        let tmp = folder();
        let mut tab = Tab::new();
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Go);
        tab.sort_by(SortKey::Size);
        assert_eq!(names(&tab), ["sub", "a10.txt", "a9.txt", "b.txt"]);
        tab.sort_by(SortKey::Size);
        assert_eq!(tab.sort(), (SortKey::Size, false));
        assert_eq!(names(&tab), ["sub", "b.txt", "a9.txt", "a10.txt"]);
    }

    #[test]
    fn history_back_forward_and_up_selects_the_child() {
        let tmp = folder();
        let root = tmp.path().to_path_buf();
        let sub = root.join("sub");
        let mut tab = Tab::new();
        tab.show(root.clone(), listing(&root), Nav::Go);
        assert_eq!(tab.back_target(), None);
        tab.show(sub.clone(), listing(&sub), Nav::Go);
        assert_eq!(tab.back_target(), Some(root.as_path()));
        assert_eq!(tab.up_target(), Some(root.as_path()));

        tab.show(root.clone(), listing(&root), Nav::Back);
        assert_eq!(tab.back_target(), None);
        assert_eq!(tab.forward_target(), Some(sub.as_path()));
        assert_eq!(tab.selected_paths(), std::slice::from_ref(&sub), "the folder we came out of is selected");
        assert_eq!(tab.current_row(), Some(0));

        tab.show(sub.clone(), listing(&sub), Nav::Forward);
        assert_eq!(tab.forward_target(), None);
        assert_eq!(tab.back_target(), Some(root.as_path()));

        tab.show(root.clone(), listing(&root), Nav::Go);
        assert_eq!(tab.forward_target(), None, "a new navigation clears forward");
    }

    #[test]
    fn refresh_keeps_selection_of_what_still_exists() {
        let tmp = folder();
        let root = tmp.path().to_path_buf();
        let mut tab = Tab::new();
        tab.show(root.clone(), listing(&root), Nav::Go);
        tab.click(1, false, false);
        tab.click(3, true, false);
        assert_eq!(tab.selected_paths(), [root.join("a9.txt"), root.join("b.txt")]);
        fs::remove_file(root.join("b.txt")).unwrap();
        tab.show(root.clone(), listing(&root), Nav::Refresh);
        assert_eq!(tab.selected_paths(), [root.join("a9.txt")]);
        assert_eq!(tab.back_target(), None, "a refresh is no navigation");
    }

    #[test]
    fn click_ctrl_and_shift_selection() {
        let tmp = folder();
        let root = tmp.path().to_path_buf();
        let mut tab = Tab::new();
        tab.show(root.clone(), listing(&root), Nav::Go);
        tab.click(0, false, false);
        tab.click(2, false, true);
        assert_eq!(tab.selected_paths().len(), 3, "shift selects the range");
        tab.click(1, true, false);
        assert_eq!(tab.selected_paths(), [root.join("sub"), root.join("a10.txt")], "ctrl toggles");
        tab.click(3, false, false);
        assert_eq!(tab.selected_paths(), [root.join("b.txt")]);
        assert_eq!(tab.status(), "4 items, 1 selected (3 bytes)");
    }

    #[test]
    fn keyboard_moves_focus_and_extends_with_shift() {
        let tmp = folder();
        let root = tmp.path().to_path_buf();
        let mut tab = Tab::new();
        tab.show(root.clone(), listing(&root), Nav::Go);
        assert_eq!(tab.current_row(), Some(0), "the first row has the focus");
        tab.move_focus(1, false, false);
        assert_eq!(tab.current_row(), Some(1));
        assert_eq!(tab.selected_paths(), [root.join("a9.txt")]);
        tab.move_focus(5, false, true);
        assert_eq!(tab.current_row(), Some(3), "clamped to the last row");
        assert_eq!(tab.selected_paths().len(), 3);
        tab.move_focus(-10, true, false);
        assert_eq!(tab.current_row(), Some(0));
        assert_eq!(tab.selected_paths().len(), 3, "ctrl moves only the focus");
        tab.toggle_current();
        assert_eq!(tab.selected_paths().len(), 4);
        tab.select_all();
        assert_eq!(tab.status(), "4 items, 4 selected (6 bytes)");
    }

    #[test]
    fn empty_folders_have_no_focus() {
        let tmp = tempfile::tempdir().unwrap();
        let mut tab = Tab::new();
        tab.show(tmp.path().to_path_buf(), Vec::new(), Nav::Go);
        tab.move_focus(1, false, false);
        assert_eq!(tab.current_row(), None);
        assert_eq!(tab.status(), "0 items");
    }
}
