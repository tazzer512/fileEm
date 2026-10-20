//! Right-click menus, built per request from what was clicked (the Java
//! `MainWindow.fileMenuItems` / `treeMenuItems`). Toolkit-free: app.slint shows the sections, the
//! controller runs the chosen [`Command`].
//!
//! Not yet ported (they need later stages): search, bookmarks, eject.

use std::path::{Path, PathBuf};

use fileem_core::fs::listing::SortKey;

use crate::columns::{ColumnId, Columns};
use crate::shell;
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
    }
    let mut remove = vec![item("Rename  (F2)", Command::Rename(p.clone())).enabled(single)];
    if st.trash {
        remove.push(item("Move to Trash  (Del)", Command::Trash(sel.clone())));
    }
    remove.push(item("Delete permanently  (Shift+Del)", Command::Delete(sel.clone())));
    let info = vec![item("Copy path", Command::CopyPath(sel.clone())), item("Properties  (Alt+Enter)", Command::Properties(sel))];
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
            item("Open in terminal", Command::OpenTerminal(d.to_path_buf())),
            item(format!("Open in {}", shell::file_manager_name()), Command::OpenInFileManager(d.to_path_buf())),
        ]);
        menu.sections.push(vec![
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

/// A folder or drive in the tree. Quick access folders and drives are shortcuts: no cut, rename,
/// delete or duplicate (drives can't be copied or zipped either).
pub fn tree_menu(dir: &Path, kind: NodeKind, st: &State) -> Menu {
    let d = dir.to_path_buf();
    let plain = kind == NodeKind::Folder;
    let volume = kind == NodeKind::Volume;
    let open = vec![
        item("Open", Command::OpenFolder(d.clone())),
        item("Open in new tab", Command::OpenInNewTab(d.clone())),
        item("Open in other side", Command::OpenInOtherSide(d.clone())),
        item("Open in terminal", Command::OpenTerminal(d.clone())),
        item(format!("Show in {}", shell::file_manager_name()), Command::Reveal(d.clone())),
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
    let info = vec![item("Copy path", Command::CopyPath(vec![d.clone()])), item("Properties", Command::Properties(vec![d.clone()]))];
    Menu { sections: vec![open, clip, change, info], new: new_entries(dir), ..Menu::default() }
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
        State { can_paste, trash: true, sort: (SortKey::Name, true), view: ViewMode::Details, show_hidden: false, dual: false }
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
        assert!(!has(&file, "Extract here"), "only zips can be extracted");
        assert!(!has(&file, "Open in new tab"), "files don't open in tabs");
        assert!(!file.find("Copy to other side").unwrap().enabled, "needs the second side");
        let dual = file_menu(&[Item { path: &doc, is_dir: false }], Some(dir), &State { dual: true, ..state(false) });
        assert!(dual.find("Move to other side").unwrap().enabled);
        assert!(!has(&file, "Paste"), "nothing to paste");
        assert_eq!(file.find("Open in terminal").unwrap().command, Command::OpenTerminal(dir.into()), "a file's terminal opens in its folder");

        let zip = dir.join("pack.ZIP");
        assert!(has(&file_menu(&[Item { path: &zip, is_dir: false }], Some(dir), &state(false)), "Extract here"));

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
        assert!(!has(&tree_menu(p, NodeKind::Folder, &st), "Move to Trash"));
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
        let folder = tree_menu(dir, NodeKind::Folder, &state(true));
        for expected in ["Open", "Open in terminal", "Cut", "Copy", "Paste into folder", "Folder", "Text file", "Duplicate", "Compress to ZIP", "Rename…", "Delete permanently", "Properties"] {
            assert!(has(&folder, expected), "{expected} in {:?}", folder.titles());
        }
        assert!(has(&folder, "Open in new tab") && has(&folder, "Open in other side"));
        let quick = tree_menu(dir, NodeKind::Quick, &state(true));
        assert!(has(&quick, "Copy") && has(&quick, "Paste into folder"));
        for forbidden in ["Cut", "Rename…", "Move to Trash", "Delete permanently", "Duplicate"] {
            assert!(!offers(&quick, forbidden), "{forbidden} must not be offered for quick access");
        }
        let volume = tree_menu(dir, NodeKind::Volume, &state(false));
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
    fn the_last_tab_cant_be_closed_from_its_menu() {
        assert!(!tab_menu(0, 0, 1).find("Close tab").unwrap().enabled);
        let m = tab_menu(1, 2, 3);
        assert!(m.find("Close other tabs").unwrap().enabled);
        assert_eq!(m.find("Duplicate tab").unwrap().command, Command::DuplicateTab(1, 2));
    }
}
