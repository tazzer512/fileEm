//! One tab's state, free of the UI toolkit: folder, history, rows (filtered + sorted) and selection.
//! The window turns it into Slint models; tests drive it directly.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use fileem_core::fs::listing::{self, SortKey};
use fileem_core::fs::FileEntry;
use fileem_core::util::{formats, fs_util};

/// Where an item at `from` ends up when it is dropped into `gap` (0: before the first item, `n`:
/// after the last); `None` when that leaves it where it is.
pub fn reordered(from: usize, gap: usize) -> Option<usize> {
    if gap == from || gap == from + 1 {
        None
    } else {
        Some(if gap > from { gap - 1 } else { gap })
    }
}

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
    /// The filter box: `*` / `?` wildcards, else "contains".
    filter: String,
    matcher: Box<dyn Fn(&str) -> bool + Send + Sync>,
    selected: HashSet<PathBuf>,
    current: Option<PathBuf>,
    anchor: Option<PathBuf>,
    /// While a selection box is drawn: the selection it started from, and whether it toggles
    /// (Ctrl/Cmd) rather than adds.
    boxing: Option<(HashSet<PathBuf>, bool)>,
}

/// The rows a selection box touches, from content offsets `a` and `b` (0 = top of the first
/// row): `None` when it touches none (above the first row, or below the last).
pub fn box_rows(a: f32, b: f32, row_height: f32, count: usize) -> Option<(usize, usize)> {
    let (top, bottom) = (a.min(b), a.max(b));
    if bottom < 0.0 || count == 0 || row_height <= 0.0 {
        return None;
    }
    let lo = (top.max(0.0) / row_height).floor() as usize;
    let hi = ((bottom / row_height).floor() as usize).min(count - 1);
    (lo <= hi).then_some((lo, hi))
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
            filter: String::new(),
            matcher: fs_util::name_matcher(""),
            selected: HashSet::new(),
            current: None,
            anchor: None,
            boxing: None,
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
        if !same_folder {
            self.filter.clear();
            self.matcher = fs_util::name_matcher("");
        }
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
        let mut view: Vec<usize> = (0..self.entries.len())
            .filter(|&i| (show_hidden || !self.entries[i].is_hidden()) && (self.matcher)(self.entries[i].name()))
            .collect();
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

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Narrows the rows to names matching `text`; the selection keeps only what stays visible.
    pub fn set_filter(&mut self, text: &str) {
        self.filter = text.to_string();
        self.matcher = fs_util::name_matcher(text);
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

    /// Rows of the selected items, in row order. Costs the selection's size, not the folder's:
    /// arrow keys in a folder of 100,000 files call it on every press.
    pub fn selected_rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.selected.iter().filter_map(|p| self.row_of.get(p).copied()).collect();
        rows.sort_unstable();
        rows
    }

    /// Selected paths in row order.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_rows().into_iter().map(|r| self.row(r).path().to_path_buf()).collect()
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

    /// A left press on `row`. On a row of a larger selection without modifiers it only moves the
    /// focus and returns true: the selection stays, so it can be dragged, and a `click` on release
    /// (when no drag happened) selects the row alone. Otherwise it is a `click`.
    pub fn press(&mut self, row: usize, ctrl: bool, shift: bool) -> bool {
        if !ctrl && !shift && row < self.len() && self.is_selected(row) && self.selected.len() > 1 {
            self.current = Some(self.row(row).path().to_path_buf());
            return true;
        }
        self.click(row, ctrl, shift);
        false
    }

    /// A selection box starts: Shift adds to the current selection, Ctrl/Cmd toggles rows in
    /// it, plain replaces it.
    pub fn start_box(&mut self, ctrl: bool, shift: bool) {
        let base = if ctrl || shift { self.selected.clone() } else { HashSet::new() };
        self.boxing = Some((base, ctrl));
    }

    pub fn is_boxing(&self) -> bool {
        self.boxing.is_some()
    }

    /// The box now touches `rows` (see [`box_rows`]); the focus goes to the row under the mouse.
    pub fn box_select(&mut self, rows: Option<(usize, usize)>, focus: usize) {
        let rows: Vec<usize> = rows.map(|(lo, hi)| (lo..=hi).collect()).unwrap_or_default();
        self.box_select_rows(&rows, Some(focus));
    }

    /// The box now touches `rows` (in a grid, any set); the focus goes to `focus`, if given.
    pub fn box_select_rows(&mut self, rows: &[usize], focus: Option<usize>) {
        let Some((base, toggle)) = &self.boxing else { return };
        let mut sel = base.clone();
        for &r in rows.iter().filter(|&&r| r < self.len()) {
            let p = self.row(r).path();
            // toggling takes away what was selected; anything else is added
            if !(*toggle && sel.remove(p)) {
                sel.insert(p.to_path_buf());
            }
        }
        self.selected = sel;
        if let (false, Some(f)) = (self.is_empty(), focus) {
            let p = self.row(f.min(self.len() - 1)).path().to_path_buf();
            self.anchor = Some(p.clone());
            self.current = Some(p);
        }
    }

    pub fn end_box(&mut self) {
        self.boxing = None;
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

    /// Space (with or without Ctrl): toggles the focused row.
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

    /// "12 items", "12 items (2 hidden)", "3 items (9 filtered out), 3 selected (4.50 KB)".
    pub fn status(&self) -> String {
        let n = self.len();
        let mut s = format!("{} {}", formats::count(n as u64), if n == 1 { "item" } else { "items" });
        let left_out = self.entries.len() - n;
        if left_out > 0 {
            let why = if self.filter.trim().is_empty() { "hidden" } else { "filtered out" };
            s.push_str(&format!(" ({} {why})", formats::count(left_out as u64)));
        }
        let sel: Vec<&FileEntry> = self.selected_rows().into_iter().map(|r| self.row(r)).collect();
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

    #[test]
    fn a_dragged_item_lands_in_the_gap_it_was_dropped_into() {
        // a b c d: gaps 0 … 4
        assert_eq!(reordered(0, 0), None, "before itself");
        assert_eq!(reordered(0, 1), None, "after itself");
        assert_eq!(reordered(0, 4), Some(3), "a to the end");
        assert_eq!(reordered(3, 0), Some(0), "d to the front");
        assert_eq!(reordered(1, 3), Some(2), "b between c and d");
        assert_eq!(reordered(2, 1), Some(1), "c between a and b");
    }
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
    fn the_filter_narrows_rows_and_a_new_folder_clears_it() {
        let tmp = folder();
        let mut tab = Tab::new();
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Go);
        tab.click(3, false, false); // b.txt
        tab.set_filter("a*.txt");
        assert_eq!(names(&tab), ["a9.txt", "a10.txt"]);
        assert!(tab.selected_paths().is_empty(), "b.txt is filtered out, and so out of the selection");
        assert_eq!(tab.status(), "2 items (3 filtered out)");
        tab.set_filter("SUB");
        assert_eq!(names(&tab), ["sub"], "plain text: contains, any case");
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Refresh);
        assert_eq!(tab.filter(), "SUB", "a refresh keeps the filter");
        tab.show(tmp.path().join("sub"), listing(&tmp.path().join("sub")), Nav::Go);
        assert_eq!((tab.filter(), tab.len()), ("", 1), "another folder clears it");
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Back);
        assert_eq!(tab.status(), "4 items (1 hidden), 1 selected", "back selects the folder we came from");
    }

    #[test]
    fn pressing_a_row_of_a_selection_keeps_it_for_dragging() {
        let tmp = folder();
        let mut tab = Tab::new();
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Go);
        tab.click(1, false, false);
        tab.click(3, false, true);
        assert!(tab.press(2, false, false), "deferred: the selection may be dragged");
        assert_eq!(tab.selected_paths().len(), 3);
        assert_eq!(tab.current_row(), Some(2));
        tab.click(2, false, false); // released without dragging
        assert_eq!(tab.selected_paths(), [tab.row(2).path().to_path_buf()]);
        assert!(!tab.press(2, false, false), "a lone selected row: a plain click");
        assert!(!tab.press(1, true, false), "Ctrl toggles right away");
        assert_eq!(tab.selected_paths().len(), 2);
    }

    #[test]
    fn a_box_selects_the_rows_it_touches_adding_or_toggling_with_modifiers() {
        assert_eq!(box_rows(5.0, 50.0, 24.0, 10), Some((0, 2)));
        assert_eq!(box_rows(50.0, 5.0, 24.0, 10), Some((0, 2)), "either direction");
        assert_eq!(box_rows(-30.0, 30.0, 24.0, 10), Some((0, 1)), "above the first row counts from it");
        assert_eq!(box_rows(-30.0, -5.0, 24.0, 10), None, "entirely above");
        assert_eq!(box_rows(500.0, 30.0, 24.0, 3), Some((1, 2)), "below the last row stops at it");
        assert_eq!(box_rows(500.0, 600.0, 24.0, 3), None, "entirely below");

        let tmp = folder();
        let mut tab = Tab::new();
        tab.show(tmp.path().to_path_buf(), listing(tmp.path()), Nav::Go);
        let path = |tab: &Tab, r: usize| tab.row(r).path().to_path_buf();
        tab.click(0, false, false);
        tab.start_box(false, false);
        tab.box_select(Some((1, 2)), 2);
        assert_eq!(tab.selected_paths(), [path(&tab, 1), path(&tab, 2)], "plain replaces");
        assert_eq!(tab.current_row(), Some(2));
        tab.box_select(Some((2, 2)), 2);
        assert_eq!(tab.selected_paths(), [path(&tab, 2)], "shrinking the box deselects");
        tab.end_box();
        assert!(!tab.is_boxing());

        tab.start_box(false, true);
        tab.box_select(Some((0, 0)), 0);
        assert_eq!(tab.selected_paths(), [path(&tab, 0), path(&tab, 2)], "Shift adds");
        tab.end_box();
        tab.start_box(true, false);
        tab.box_select(Some((0, 1)), 1);
        assert_eq!(tab.selected_paths(), [path(&tab, 1), path(&tab, 2)], "Ctrl toggles");
        tab.box_select(None, 0);
        assert_eq!(tab.selected_paths(), [path(&tab, 0), path(&tab, 2)], "touching nothing: back to the start");
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
        assert_eq!(tab.status(), "4 items (1 hidden), 1 selected (3 bytes)");
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
        assert_eq!(tab.status(), "4 items (1 hidden), 4 selected (6 bytes)");
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
