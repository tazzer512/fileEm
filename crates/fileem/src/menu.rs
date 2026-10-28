//! Right-click menus, built per request from what was clicked (the Java
//! `MainWindow.fileMenuItems` / `treeMenuItems`). Toolkit-free: app.slint shows the sections, the
//! controller runs the chosen [`Command`].

use std::path::{Path, PathBuf};

use fileem_core::fs::listing::SortKey;
use fileem_core::util::fs_util;

use crate::columns::{ColumnId, Columns};
use crate::shell;
use crate::theme::Family;
use crate::tree::NodeKind;
use crate::view_mode::ViewMode;

/// What a menu item does. Commands carry their paths, so the tree and the list share them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The list's selection: a single folder opens in the tab, files with their programs.
    OpenSelection,
    /// A folder from the tree opens in the tab.
    OpenFolder(PathBuf),
    OpenInNewTab(PathBuf),
    OpenInOtherSide(PathBuf),
    /// F5 / F6: asks for the target, prefilled with the other side's folder.
    CopyToOther(Vec<PathBuf>),
    MoveToOther(Vec<PathBuf>),
    OpenTerminal(PathBuf),
    Reveal(PathBuf),
    /// The folder shown, in the OS file manager.
    OpenInFileManager(PathBuf),
    Cut(Vec<PathBuf>),
    Copy(Vec<PathBuf>),
    PasteInto(PathBuf),
    Duplicate(Vec<PathBuf>),
    Compress(Vec<PathBuf>),
    Extract(PathBuf),
    /// Always into a folder named after the archive.
    ExtractToFolder(PathBuf),
    Rename(PathBuf),
    Trash(Vec<PathBuf>),
    Delete(Vec<PathBuf>),
    CopyPath(Vec<PathBuf>),
    Properties(Vec<PathBuf>),
    NewFolder(PathBuf),
    NewFile(PathBuf),
    SortBy(SortKey),
    SortAscending(bool),
    /// The active tab's view mode.
    SetView(ViewMode),
    ToggleHidden,
    Refresh,
    ToggleColumn(ColumnId),
    /// A tab of a side (side, index).
    DuplicateTab(usize, usize),
    TabToOtherSide(usize, usize),
    CloseTab(usize, usize),
    CloseOtherTabs(usize, usize),
    /// A search window for a folder and everything below it.
    Search(PathBuf),
    /// In a search window: open the selected results (folders in the main window's tab).
    OpenResults,
    /// In a search window: the result's folder in the main window's tab, the result selected.
    ShowInFolder(PathBuf),
    AddBookmark(PathBuf),
    RemoveBookmark(PathBuf),
    /// Asks for a bookmark's new name.
    RenameBookmark(PathBuf),
    /// A bookmark a place up (-1) or down (1).
    MoveBookmark(PathBuf, isize),
    /// Ctrl+Shift+B: the bookmarks dialog.
    ManageBookmarks,
    /// Safely removes a drive (its root and its label).
    Eject(PathBuf, String),
    /// The theme family; the mode stays.
    SetTheme(Family),
    /// Light (false) or dark (true); the family stays.
    SetDark(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub enabled: bool,
    /// A check mark (Some) or none.
    pub checked: Option<bool>,
    pub command: Command,
}

fn item(title: impl Into<String>, command: Command) -> Entry {
    Entry { title: title.into(), enabled: true, checked: None, command }
}

fn check(title: &str, checked: bool, command: Command) -> Entry {
    Entry { checked: Some(checked), ..item(title, command) }
}

impl Entry {
    fn enabled(mut self, on: bool) -> Entry {
        self.enabled = on;
        self
    }
}

/// Sections are separated by lines; the "View", "Sort by" and "New" submenus (when not empty)
/// follow the items of section 1.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Menu {
    pub sections: Vec<Vec<Entry>>,
    pub view: Vec<Entry>,
    pub sort: Vec<Entry>,
    pub new: Vec<Entry>,
}

/// How many sections app.slint can show.
pub const SECTIONS: usize = 6;

impl Menu {
    /// Every entry, submenus included, in the order shown.
    pub fn entries(&self) -> Vec<&Entry> {
        let mut all = Vec::new();
        for (i, s) in self.sections.iter().enumerate() {
            all.extend(s);
            if i == 1 {
                all.extend(&self.view);
                all.extend(&self.sort);
                all.extend(&self.new);
            }
        }
        all
    }

    /// The first entry whose title starts with `prefix`.
    pub fn find(&self, prefix: &str) -> Option<&Entry> {
        self.entries().into_iter().find(|e| e.title.starts_with(prefix))
    }

    pub fn titles(&self) -> Vec<&str> {
        self.entries().into_iter().map(|e| e.title.as_str()).collect()
    }
}

/// An item the menu is about.
pub struct Item<'a> {
    pub path: &'a Path,
    pub is_dir: bool,
}

/// What the menus need to know about the window.
pub struct State {
    pub can_paste: bool,
    pub trash: bool,
    pub sort: (SortKey, bool),
    /// The active tab's view mode.
    pub view: ViewMode,
    pub show_hidden: bool,
    /// The second side is shown: copy / move to it.
    pub dual: bool,
    /// The bookmarked folders, in order.
    pub bookmarks: Vec<PathBuf>,
}

impl State {
    fn bookmark_index(&self, dir: &Path) -> Option<usize> {
        let d = fs_util::normalize(dir);
        self.bookmarks.iter().position(|b| *b == d)
    }
}

pub fn is_zip(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(".zip"))
}

/// The file list's menu: actions on the selection, or on the folder (`dir`) when nothing is selected.
pub fn file_menu(selection: &[Item], dir: Option<&Path>, st: &State) -> Menu {
    let Some(first) = selection.first() else { return background_menu(dir, st) };
    let sel: Vec<PathBuf> = selection.iter().map(|i| i.path.to_path_buf()).collect();
    let single = selection.len() == 1;
    let single_dir = single && first.is_dir;
    let p = first.path.to_path_buf();

    let mut open = vec![item("Open", Command::OpenSelection)];
    let terminal_dir = if single_dir { Some(p.clone()) } else { dir.map(Path::to_path_buf) };
    if let Some(d) = terminal_dir {
        open.push(item("Open in terminal", Command::OpenTerminal(d)));
    }
    if single_dir {
        open.insert(1, item("Open in new tab", Command::OpenInNewTab(p.clone())));
        open.insert(2, item("Open in other side", Command::OpenInOtherSide(p.clone())));
        open.push(search_item(&p));
    }
    if single {
        open.push(item(format!("Show in {}", shell::file_manager_name()), Command::Reveal(p.clone())));
    }
    let mut clip = vec![item("Cut", Command::Cut(sel.clone())), item("Copy", Command::Copy(sel.clone()))];
    if st.can_paste {
        clip.push(match (single_dir, dir) {
            (true, _) => item("Paste into folder", Command::PasteInto(p.clone())),
            (false, Some(d)) => item("Paste", Command::PasteInto(d.to_path_buf())),
            (false, None) => item("Paste", Command::PasteInto(PathBuf::new())).enabled(false),
        });
    }
    let transfer = vec![
        item("Copy to other side…  (F5)", Command::CopyToOther(sel.clone())).enabled(st.dual),
        item("Move to other side…  (F6)", Command::MoveToOther(sel.clone())).enabled(st.dual),
        item("Duplicate", Command::Duplicate(sel.clone())),
    ];
    let mut archive = vec![item("Compress to ZIP", Command::Compress(sel.clone()))];
    if single && !first.is_dir && is_zip(first.path) {
        archive.push(item("Extract here", Command::Extract(p.clone())));
        archive.push(item("Extract to folder", Command::ExtractToFolder(p.clone())));
    }
    let mut remove = vec![item("Rename  (F2)", Command::Rename(p.clone())).enabled(single)];
    if st.trash {
        remove.push(item("Move to Trash  (Del)", Command::Trash(sel.clone())));
    }
    remove.push(item("Delete permanently  (Shift+Del)", Command::Delete(sel.clone())));
    let mut info = vec![item("Copy path", Command::CopyPath(sel.clone())), item("Properties  (Alt+Enter)", Command::Properties(sel))];
    if single_dir {
        info.insert(0, bookmark_toggle(&p, st));
    }
    Menu { sections: vec![open, clip, transfer, archive, remove, info], ..Menu::default() }
}

/// Right-click on empty space in the list: the folder shown.
fn background_menu(dir: Option<&Path>, st: &State) -> Menu {
    let paste = match dir {
        Some(d) => item("Paste", Command::PasteInto(d.to_path_buf())).enabled(st.can_paste),
        None => item("Paste", Command::PasteInto(PathBuf::new())).enabled(false),
    };
    let view = vec![check("Show hidden files", st.show_hidden, Command::ToggleHidden), item("Refresh  (Ctrl+R)", Command::Refresh)];
    let mut menu = Menu { sections: vec![vec![paste], view], view: view_entries(st.view), sort: sort_entries(st.sort), new: Vec::new() };
    if let Some(d) = dir {
        menu.new = new_entries(d);
        menu.sections.push(vec![
            search_item(d),
            item("Open in terminal", Command::OpenTerminal(d.to_path_buf())),
            item(format!("Open in {}", shell::file_manager_name()), Command::OpenInFileManager(d.to_path_buf())),
        ]);
        menu.sections.push(vec![
            bookmark_toggle(d, st),
            item("Copy path", Command::CopyPath(vec![d.to_path_buf()])),
            item("Properties  (Alt+Enter)", Command::Properties(vec![d.to_path_buf()])),
        ]);
    }
    menu
}

/// The "Sort by" submenu: the columns, then the direction, the current ones checked.
fn sort_entries((key, ascending): (SortKey, bool)) -> Vec<Entry> {
    let mut v: Vec<Entry> = SortKey::ALL.into_iter().map(|k| check(k.title(), k == key, Command::SortBy(k))).collect();
    v.push(check("Ascending", ascending, Command::SortAscending(true)));
    v.push(check("Descending", !ascending, Command::SortAscending(false)));
    v
}

/// Explorer's view modes with their shortcuts, the current one checked.
fn view_entries(current: ViewMode) -> Vec<Entry> {
    ViewMode::ALL
        .into_iter()
        .map(|m| check(&format!("{}  (Ctrl+Shift+{})", m.label(), m.index() + 1), m == current, Command::SetView(m)))
        .collect()
}

/// The view button's menu.
pub fn view_menu(current: ViewMode) -> Menu {
    Menu { sections: vec![view_entries(current)], ..Menu::default() }
}

/// The sort button's menu: the columns, then the direction.
pub fn sort_menu(sort: (SortKey, bool)) -> Menu {
    let mut entries = sort_entries(sort);
    let direction = entries.split_off(entries.len() - 2);
    Menu { sections: vec![entries, direction], ..Menu::default() }
}

fn new_entries(dir: &Path) -> Vec<Entry> {
    vec![item("Folder  (F7)", Command::NewFolder(dir.to_path_buf())), item("Text file", Command::NewFile(dir.to_path_buf()))]
}

/// A drive of the tree that can be ejected: its label, and whether an eject may start now (not
/// while one runs).
pub struct Eject<'a> {
    pub label: &'a str,
    pub enabled: bool,
}

/// A folder or drive in the tree. Quick access folders, bookmarks and drives are shortcuts: no
/// cut, rename, delete or duplicate (drives can't be copied or zipped either). Drives people plug
/// in can be ejected.
pub fn tree_menu(dir: &Path, kind: NodeKind, eject: Option<Eject>, st: &State) -> Menu {
    let d = dir.to_path_buf();
    let plain = kind == NodeKind::Folder;
    let volume = kind == NodeKind::Volume;
    let open = vec![
        item("Open", Command::OpenFolder(d.clone())),
        item("Open in new tab", Command::OpenInNewTab(d.clone())),
        item("Open in other side", Command::OpenInOtherSide(d.clone())),
        item("Open in terminal", Command::OpenTerminal(d.clone())),
        item(format!("Show in {}", shell::file_manager_name()), Command::Reveal(d.clone())),
        search_item(dir),
    ];
    let mut clip = Vec::new();
    if plain {
        clip.push(item("Cut", Command::Cut(vec![d.clone()])));
    }
    if !volume {
        clip.push(item("Copy", Command::Copy(vec![d.clone()])));
    }
    clip.push(item("Paste into folder", Command::PasteInto(d.clone())).enabled(st.can_paste));
    let mut change = Vec::new();
    if plain {
        change.push(item("Duplicate", Command::Duplicate(vec![d.clone()])));
    }
    if !volume {
        change.push(item("Compress to ZIP", Command::Compress(vec![d.clone()])));
    }
    if plain {
        change.push(item("Rename…", Command::Rename(d.clone())));
        if st.trash {
            change.push(item("Move to Trash", Command::Trash(vec![d.clone()])));
        }
        change.push(item("Delete permanently", Command::Delete(vec![d.clone()])));
    }
    let bookmark = match (kind, st.bookmark_index(dir)) {
        (NodeKind::Bookmark, Some(i)) => vec![
            item("Rename bookmark…", Command::RenameBookmark(d.clone())),
            item("Remove bookmark", Command::RemoveBookmark(d.clone())),
            item("Move up", Command::MoveBookmark(d.clone(), -1)).enabled(i > 0),
            item("Move down", Command::MoveBookmark(d.clone(), 1)).enabled(i + 1 < st.bookmarks.len()),
            manage_item(),
        ],
        _ => vec![bookmark_toggle(dir, st)],
    };
    let mut info = vec![item("Copy path", Command::CopyPath(vec![d.clone()])), item("Properties", Command::Properties(vec![d.clone()]))];
    if let Some(e) = eject.filter(|_| kind == NodeKind::Volume) {
        info.push(item("Eject", Command::Eject(d.clone(), e.label.to_string())).enabled(e.enabled));
    }
    Menu { sections: vec![open, clip, change, bookmark, info], new: new_entries(dir), ..Menu::default() }
}

/// The Bookmarks section's title.
pub fn bookmarks_header_menu() -> Menu {
    Menu { sections: vec![vec![manage_item()]], ..Menu::default() }
}

fn manage_item() -> Entry {
    item("Manage bookmarks…  (Ctrl+Shift+B)", Command::ManageBookmarks)
}

/// "Add to bookmarks", or "Remove from bookmarks" for a bookmarked folder.
fn bookmark_toggle(dir: &Path, st: &State) -> Entry {
    let d = dir.to_path_buf();
    match st.bookmark_index(dir) {
        Some(_) => item("Remove from bookmarks", Command::RemoveBookmark(d)),
        None => item("Add to bookmarks  (Ctrl+D)", Command::AddBookmark(d)),
    }
}

fn search_item(dir: &Path) -> Entry {
    item("Search in folder…  (Ctrl+Shift+F)", Command::Search(dir.to_path_buf()))
}

/// A search window's results: open them, their folder, the clipboard, rename and delete (the
/// window runs these itself, so the results follow), the path and Properties.
pub fn search_menu(selection: &[PathBuf], trash: bool) -> Menu {
    let Some(first) = selection.first() else { return Menu::default() };
    let sel = selection.to_vec();
    let mut remove = vec![item("Rename  (F2)", Command::Rename(first.clone())).enabled(selection.len() == 1)];
    if trash {
        remove.push(item("Move to Trash  (Del)", Command::Trash(sel.clone())));
    }
    remove.push(item("Delete permanently  (Shift+Del)", Command::Delete(sel.clone())));
    Menu {
        sections: vec![
            vec![item("Open  (Enter)", Command::OpenResults), item("Open containing folder", Command::ShowInFolder(first.clone()))],
            vec![item("Cut  (Ctrl+X)", Command::Cut(sel.clone())), item("Copy  (Ctrl+C)", Command::Copy(sel.clone()))],
            remove,
            vec![item("Copy path", Command::CopyPath(sel.clone())), item("Properties  (Alt+Enter)", Command::Properties(sel))],
        ],
        ..Menu::default()
    }
}

/// Right-click on the column headers: which columns to show (Name always stays).
pub fn header_menu(columns: &Columns) -> Menu {
    let entries = columns
        .all()
        .iter()
        .map(|c| check(c.id.title(), c.visible, Command::ToggleColumn(c.id)).enabled(c.id != ColumnId::Name))
        .collect();
    Menu { sections: vec![entries], ..Menu::default() }
}

/// The toolbar's theme menu: the families, then light and dark.
pub fn theme_menu(family: Family, dark: bool) -> Menu {
    let families = Family::ALL.iter().map(|&f| check(f.label(), f == family, Command::SetTheme(f))).collect();
    let modes = vec![check("Light", !dark, Command::SetDark(false)), check("Dark", dark, Command::SetDark(true))];
    Menu { sections: vec![families, modes], ..Menu::default() }
}

/// Right-click on a tab.
pub fn tab_menu(side: usize, index: usize, tabs: usize) -> Menu {
    Menu {
        sections: vec![
            vec![
                item("Duplicate tab", Command::DuplicateTab(side, index)),
                item("Open in other side", Command::TabToOtherSide(side, index)),
            ],
            vec![
                item("Close tab  (Ctrl+W)", Command::CloseTab(side, index)).enabled(tabs > 1),
                item("Close other tabs", Command::CloseOtherTabs(side, index)).enabled(tabs > 1),
            ],
        ],
        ..Menu::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(can_paste: bool) -> State {
        State {
            can_paste,
            trash: true,
            sort: (SortKey::Name, true),
            view: ViewMode::Details,
            show_hidden: false,
            dual: false,
            bookmarks: Vec::new(),
        }
    }

    fn has(m: &Menu, prefix: &str) -> bool {
        m.find(prefix).is_some()
    }

    /// Exactly this item ("Copy", not "Copy path").
    fn offers(m: &Menu, title: &str) -> bool {
        m.titles().contains(&title)
    }

    #[test]
    fn file_and_folder_menus_offer_the_usual_actions() {
        let dir = Path::new("/w");
        let doc = dir.join("doc.txt");
        let file = file_menu(&[Item { path: &doc, is_dir: false }], Some(dir), &state(false));
        for expected in ["Open", "Open in terminal", "Show in", "Cut", "Copy", "Duplicate", "Compress to ZIP", "Rename", "Move to Trash", "Delete permanently", "Copy path", "Properties"] {
            assert!(has(&file, expected), "{expected} in {:?}", file.titles());
        }
        assert!(!has(&file, "Extract here") && !has(&file, "Extract to folder"), "only zips can be extracted");
        assert!(!has(&file, "Open in new tab"), "files don't open in tabs");
        assert!(!file.find("Copy to other side").unwrap().enabled, "needs the second side");
        let dual = file_menu(&[Item { path: &doc, is_dir: false }], Some(dir), &State { dual: true, ..state(false) });
        assert!(dual.find("Move to other side").unwrap().enabled);
        assert!(!has(&file, "Paste"), "nothing to paste");
        assert_eq!(file.find("Open in terminal").unwrap().command, Command::OpenTerminal(dir.into()), "a file's terminal opens in its folder");

        let zip = dir.join("pack.ZIP");
        let archive = file_menu(&[Item { path: &zip, is_dir: false }], Some(dir), &state(false));
        assert_eq!(archive.find("Extract here").unwrap().command, Command::Extract(zip.clone()));
        assert_eq!(archive.find("Extract to folder").unwrap().command, Command::ExtractToFolder(zip.clone()));

        let sub = dir.join("sub");
        let folder = file_menu(&[Item { path: &sub, is_dir: true }], Some(dir), &state(true));
        assert_eq!(folder.find("Paste into folder").unwrap().command, Command::PasteInto(sub.clone()));
        assert_eq!(folder.find("Open in terminal").unwrap().command, Command::OpenTerminal(sub.clone()));
        assert_eq!(folder.find("Open in new tab").unwrap().command, Command::OpenInNewTab(sub.clone()));
        assert!(has(&folder, "Open in other side"));

        let two = file_menu(&[Item { path: &doc, is_dir: false }, Item { path: &sub, is_dir: true }], Some(dir), &state(true));
        assert!(!two.find("Rename").unwrap().enabled, "rename needs one item");
        assert!(!has(&two, "Show in"));
        assert_eq!(two.find("Paste").unwrap().command, Command::PasteInto(dir.into()), "several items: paste into the folder shown");
        assert_eq!(two.find("Delete permanently").unwrap().command, Command::Delete(vec![doc, sub]));
    }

    #[test]
    fn without_trash_there_is_no_trash_item() {
        let p = Path::new("/w/a");
        let st = State { trash: false, ..state(false) };
        assert!(!has(&file_menu(&[Item { path: p, is_dir: false }], None, &st), "Move to Trash"));
        assert!(!has(&tree_menu(p, NodeKind::Folder, None, &st), "Move to Trash"));
    }

    #[test]
    fn the_empty_space_menu_and_the_buttons_offer_the_views_and_sorting() {
        let m = file_menu(&[], Some(Path::new("/w")), &State { view: ViewMode::Tiles, ..state(false) });
        let views: Vec<&str> = m.view.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(views[0], "Extra large icons  (Ctrl+Shift+1)");
        assert_eq!(views[7], "Content  (Ctrl+Shift+8)");
        assert_eq!(m.find("Tiles").unwrap().checked, Some(true));
        assert_eq!(m.find("Details").unwrap().command, Command::SetView(ViewMode::Details));
        assert!(file_menu(&[Item { path: Path::new("/w/a"), is_dir: false }], None, &state(false)).view.is_empty(), "items: no views");

        let v = view_menu(ViewMode::List);
        assert_eq!(v.entries().len(), 8);
        assert_eq!(v.find("List").unwrap().checked, Some(true));
        let s = sort_menu((SortKey::Size, false));
        assert_eq!(s.sections[1].iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Ascending", "Descending"]);
        assert_eq!(s.find("Size").unwrap().checked, Some(true));
        assert_eq!(s.find("Descending").unwrap().checked, Some(true));
    }

    #[test]
    fn the_empty_space_menu_offers_new_sort_and_hidden_files() {
        let dir = Path::new("/w");
        let m = file_menu(&[], Some(dir), &State { sort: (SortKey::Size, false), show_hidden: true, ..state(false) });
        for expected in ["Paste", "Folder", "Text file", "Name", "Ascending", "Show hidden files", "Refresh", "Open in terminal", "Open in", "Copy path", "Properties"] {
            assert!(has(&m, expected), "{expected} in {:?}", m.titles());
        }
        assert!(!m.find("Paste").unwrap().enabled, "nothing to paste");
        assert_eq!(m.find("Size").unwrap().checked, Some(true));
        assert_eq!(m.find("Name").unwrap().checked, Some(false));
        assert_eq!(m.find("Descending").unwrap().checked, Some(true));
        assert_eq!(m.find("Show hidden files").unwrap().checked, Some(true));
        assert_eq!(m.find("Properties").unwrap().command, Command::Properties(vec![dir.into()]));
        assert!(m.sections.len() <= SECTIONS);
    }

    #[test]
    fn tree_menus_protect_shortcuts_and_volumes() {
        let dir = Path::new("/w/plain");
        let folder = tree_menu(dir, NodeKind::Folder, None, &state(true));
        for expected in ["Open", "Open in terminal", "Cut", "Copy", "Paste into folder", "Folder", "Text file", "Duplicate", "Compress to ZIP", "Rename…", "Delete permanently", "Properties"] {
            assert!(has(&folder, expected), "{expected} in {:?}", folder.titles());
        }
        assert!(has(&folder, "Open in new tab") && has(&folder, "Open in other side"));
        let quick = tree_menu(dir, NodeKind::Quick, None, &state(true));
        assert!(has(&quick, "Copy") && has(&quick, "Paste into folder"));
        for forbidden in ["Cut", "Rename…", "Move to Trash", "Delete permanently", "Duplicate"] {
            assert!(!offers(&quick, forbidden), "{forbidden} must not be offered for quick access");
        }
        let volume = tree_menu(dir, NodeKind::Volume, None, &state(false));
        for forbidden in ["Cut", "Copy", "Rename…", "Delete permanently", "Compress to ZIP"] {
            assert!(!offers(&volume, forbidden), "{forbidden} must not be offered for a drive");
        }
        assert!(!volume.find("Paste into folder").unwrap().enabled);
    }

    #[test]
    fn the_header_menu_lists_every_column_and_keeps_name() {
        let m = header_menu(&Columns::default());
        assert_eq!(m.titles(), ["Name", "Size", "Type", "Date modified", "Date created", "Date accessed", "Extension", "Attributes"]);
        assert!(!m.find("Name").unwrap().enabled);
        assert_eq!(m.find("Size").unwrap().checked, Some(true));
        assert_eq!(m.find("Extension").unwrap().checked, Some(false));
        assert_eq!(m.find("Extension").unwrap().command, Command::ToggleColumn(ColumnId::Extension));
    }

    #[test]
    fn folders_offer_a_search_in_them_and_results_have_their_own_menu() {
        let dir = Path::new("/w");
        let sub = dir.join("sub");
        let search = Command::Search(sub.clone());
        assert_eq!(file_menu(&[Item { path: &sub, is_dir: true }], Some(dir), &state(false)).find("Search in folder").unwrap().command, search);
        assert!(!has(&file_menu(&[Item { path: &dir.join("a.txt"), is_dir: false }], Some(dir), &state(false)), "Search in folder"), "not on a file");
        assert_eq!(file_menu(&[], Some(dir), &state(false)).find("Search in folder").unwrap().command, Command::Search(dir.into()));
        assert_eq!(tree_menu(&sub, NodeKind::Volume, None, &state(false)).find("Search in folder").unwrap().command, search);

        let a = dir.join("a.txt");
        let one = search_menu(std::slice::from_ref(&a), true);
        assert_eq!(one.titles(), ["Open  (Enter)", "Open containing folder", "Cut  (Ctrl+X)", "Copy  (Ctrl+C)", "Rename  (F2)", "Move to Trash  (Del)", "Delete permanently  (Shift+Del)", "Copy path", "Properties  (Alt+Enter)"]);
        assert_eq!(one.find("Open containing").unwrap().command, Command::ShowInFolder(a.clone()));
        let two = search_menu(&[a.clone(), sub.clone()], false);
        assert!(!two.find("Rename").unwrap().enabled);
        assert!(!has(&two, "Move to Trash"));
        assert_eq!(two.find("Delete").unwrap().command, Command::Delete(vec![a, sub]));
        assert!(search_menu(&[], true).entries().is_empty());
    }

    #[test]
    fn the_last_tab_cant_be_closed_from_its_menu() {
        assert!(!tab_menu(0, 0, 1).find("Close tab").unwrap().enabled);
        let m = tab_menu(1, 2, 3);
        assert!(m.find("Close other tabs").unwrap().enabled);
        assert_eq!(m.find("Duplicate tab").unwrap().command, Command::DuplicateTab(1, 2));
    }

    #[test]
    fn folders_can_be_bookmarked_and_bookmarks_have_their_own_items() {
        let dir = Path::new("/w");
        let sub = dir.join("sub");
        let add = |m: &Menu| m.find("Add to bookmarks").map(|e| e.command.clone());
        assert_eq!(add(&file_menu(&[Item { path: &sub, is_dir: true }], Some(dir), &state(false))), Some(Command::AddBookmark(sub.clone())));
        assert_eq!(add(&file_menu(&[Item { path: &dir.join("a.txt"), is_dir: false }], Some(dir), &state(false))), None, "not a file");
        assert_eq!(add(&file_menu(&[], Some(dir), &state(false))), Some(Command::AddBookmark(dir.into())), "the folder shown");
        assert_eq!(add(&tree_menu(&sub, NodeKind::Folder, None, &state(false))), Some(Command::AddBookmark(sub.clone())));

        // bookmarks are kept absolute ("/w" is C:\w on Windows)
        let st = State { bookmarks: vec![fs_util::normalize(dir), fs_util::normalize(&sub)], ..state(false) };
        let marked = file_menu(&[Item { path: &sub, is_dir: true }], Some(dir), &st);
        assert_eq!(marked.find("Remove from bookmarks").unwrap().command, Command::RemoveBookmark(sub.clone()));
        assert!(!has(&marked, "Add to bookmarks"));

        let last = tree_menu(&sub, NodeKind::Bookmark, None, &st);
        for expected in ["Rename bookmark…", "Remove bookmark", "Move up", "Move down", "Manage bookmarks…"] {
            assert!(has(&last, expected), "{expected} in {:?}", last.titles());
        }
        for forbidden in ["Cut", "Rename…", "Move to Trash", "Delete permanently", "Duplicate"] {
            assert!(!offers(&last, forbidden), "{forbidden} must not be offered for a bookmark");
        }
        assert!(last.find("Move up").unwrap().enabled);
        assert!(!last.find("Move down").unwrap().enabled, "the last one can't go down");
        assert_eq!(last.find("Move up").unwrap().command, Command::MoveBookmark(sub.clone(), -1));
        let first = tree_menu(dir, NodeKind::Bookmark, None, &st);
        assert!(!first.find("Move up").unwrap().enabled && first.find("Move down").unwrap().enabled);
        assert!(first.sections.len() <= SECTIONS);
        assert_eq!(bookmarks_header_menu().titles(), ["Manage bookmarks…  (Ctrl+Shift+B)"]);
    }

    #[test]
    fn only_drives_people_plug_in_offer_eject() {
        let root = Path::new("/media/me/STICK");
        let eject = |enabled| Some(Eject { label: "STICK", enabled });
        let m = tree_menu(root, NodeKind::Volume, eject(true), &state(false));
        let e = m.find("Eject").unwrap();
        assert_eq!(e.command, Command::Eject(root.into(), "STICK".into()));
        assert!(e.enabled);
        assert!(!tree_menu(root, NodeKind::Volume, eject(false), &state(false)).find("Eject").unwrap().enabled, "not twice at once");
        assert!(!has(&tree_menu(root, NodeKind::Volume, None, &state(false)), "Eject"), "an internal disk");
        assert!(!has(&tree_menu(root, NodeKind::Folder, eject(true), &state(false)), "Eject"), "a folder on it");
    }

    #[test]
    fn theme_menu_marks_the_family_and_the_mode() {
        let m = theme_menu(Family::Adwaita, true);
        assert_eq!(m.titles(), ["Classic", "Windows 11", "macOS", "Adwaita (GNOME)", "Aurora", "Light", "Dark"]);
        let checked: Vec<&str> = m.entries().into_iter().filter(|e| e.checked == Some(true)).map(|e| e.title.as_str()).collect();
        assert_eq!(checked, ["Adwaita (GNOME)", "Dark"]);
        assert_eq!(m.find("Aurora").unwrap().command, Command::SetTheme(Family::Aurora));
        assert_eq!(m.find("Light").unwrap().command, Command::SetDark(false));
        assert_eq!(m.sections.len(), 2);
    }

}
