//! Commands on files: operations, the clipboard, dialogs, right-click menus and Properties.
//! They act on the active side's active tab, or on the paths a menu carries.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use fileem_core::fs::measure::{self, Totals};
use fileem_core::fs::trash;
use fileem_core::fs::FileEntry;
use fileem_core::ops::{names, Handle, Kind, State};
use fileem_core::util::{fs_util, os};

use super::nav::{self, refresh_selecting};
use super::{icon_of, push_columns, push_view, set_status, sync_files, sync_selection, views, Shared};
use crate::address;
use crate::bg;
use crate::clipboard::FileClipboard;
use crate::menu::{self, Command, Menu};
use crate::ops_ui::{OpsUi, Outcome};
use crate::properties;
use crate::shell;
use crate::tab::Nav;
use crate::tree::NodeKind;
use crate::{MenuRow, PropLine, PropertiesWindow};

// ---- opening ----

/// Double-click or Enter on a row: folders open in the tab, files with their programs.
pub fn activate(app: &Shared, row: usize) {
    let entry = {
        let a = app.borrow();
        if row >= a.tab().len() {
            return;
        }
        a.tab().row(row).clone()
    };
    if entry.is_dir() {
        let id = app.borrow().active_tab_id();
        nav::go(app, id, entry.path().to_path_buf(), Nav::Go);
    } else if let Err(e) = shell::open(entry.path()) {
        set_status(app, format!("Cannot open \u{201c}{}\u{201d}: {e}", entry.name()));
    }
}

/// "Open": one selected folder opens in the tab; files open with their programs.
fn open_selection(app: &Shared) {
    let (dirs, files): (Vec<FileEntry>, Vec<FileEntry>) =
        app.borrow().tab().selected_entries().into_iter().cloned().partition(|e| e.is_dir());
    if let ([dir], []) = (dirs.as_slice(), files.as_slice()) {
        let id = app.borrow().active_tab_id();
        return nav::go(app, id, dir.path().to_path_buf(), Nav::Go);
    }
    for f in files {
        if let Err(e) = shell::open(f.path()) {
            set_status(app, format!("Cannot open \u{201c}{}\u{201d}: {e}", f.name()));
        }
    }
}

/// Starts a program in the background (it may block briefly); a failure is shown in a dialog.
fn launch(app: &Shared, title: &str, start: impl FnOnce() -> std::io::Result<()> + Send + 'static) {
    let (done_app, title) = (app.clone(), title.to_string());
    bg::run(
        move |_| start().map_err(|e| e.to_string()),
        move |result: Result<(), String>| {
            if let Err(msg) = result {
                open_dialog(&done_app, Dialog::Info, DialogSpec::info(&title, &msg));
            }
        },
    );
}

// ---- operations ----

/// Asks, then moves the selection to the Trash or deletes it, with a row in the progress window.
pub fn delete_selection(app: &Shared, permanent: bool) {
    let paths = app.borrow().tab().selected_paths();
    delete(app, paths, permanent);
}

fn delete(app: &Shared, paths: Vec<PathBuf>, permanent: bool) {
    if paths.is_empty() {
        return;
    }
    let (dialog, spec) = delete_question(paths, permanent);
    open_dialog(app, dialog, spec);
}

/// The question before moving `paths` to the Trash (where there is one) or deleting them.
pub(super) fn delete_question(paths: Vec<PathBuf>, permanent: bool) -> (Dialog, DialogSpec) {
    let trash = !permanent && trash::is_supported();
    let what = describe(&paths);
    let (title, question, ok) = if trash {
        ("Move to Trash", format!("Move {what} to the Trash?"), "Move to Trash")
    } else {
        ("Delete permanently", format!("Permanently delete {what}?\n\nThis cannot be undone."), "Delete")
    };
    (Dialog::Delete { paths, trash }, DialogSpec::confirm(title, &question, ok))
}

/// "“a.txt”" for one item, "3 items" for more.
pub(super) fn describe(paths: &[PathBuf]) -> String {
    match paths {
        [one] => format!("\u{201c}{}\u{201d}", fs_util::display_name(one)),
        many => format!("{} items", many.len()),
    }
}

/// Starts an operation with a row in the progress window; when it ends, the folders it touched
/// are refreshed in every tab and tree (the watcher is slow on some systems) and its problems are
/// listed.
pub fn run_operation(app: &Shared, kind: Kind, sources: Vec<PathBuf>, target: Option<PathBuf>) -> Handle {
    run_operation_then(app, kind, sources, target, |_| {})
}

/// [`run_operation`], and `then` once it ended (a search window drops the results that are gone).
pub(super) fn run_operation_then(
    app: &Shared,
    kind: Kind,
    sources: Vec<PathBuf>,
    target: Option<PathBuf>,
    then: impl FnOnce(&Shared) + 'static,
) -> Handle {
    let ops = app.borrow().ops.clone();
    let done_app = app.clone();
    ops.run(kind, sources, target, move |outcome| {
        operation_done(&done_app, outcome);
        then(&done_app);
    })
}

fn operation_done(app: &Shared, outcome: Outcome) {
    let shown = app.borrow().tab().dir().map(Path::to_path_buf);
    // what Compress / Extract made is selected when it lands in the folder shown
    let made = outcome.result.clone().filter(|r| outcome.state == State::Succeeded && r.parent() == shown.as_deref());
    if let Some(item) = made {
        refresh_selecting(app, item);
    }
    nav::refresh_dirs(app, &outcome.affected);
    let problem = match &outcome.state {
        State::Failed(msg) => Some((msg.clone(), Vec::new())),
        _ if !outcome.errors.is_empty() => {
            let n = outcome.errors.len();
            let text = if n == 1 { "1 item could not be processed.".to_string() } else { format!("{n} items could not be processed.") };
            Some((text, outcome.errors.clone()))
        }
        _ => None,
    };
    if let Some((message, lines)) = problem {
        let spec = DialogSpec { lines, ..DialogSpec::info(&outcome.title, &message) };
        open_dialog(app, Dialog::Info, spec);
    }
}

/// F5 / F6: copy or move to the other side's folder, or a typed one. Only with the second side
/// shown: without it there is nothing to copy to.
pub fn transfer_selection(app: &Shared, mv: bool) {
    let paths = app.borrow().tab().selected_paths();
    transfer(app, paths, mv);
}

fn transfer(app: &Shared, sources: Vec<PathBuf>, mv: bool) {
    let initial = {
        let a = app.borrow();
        if sources.is_empty() || !a.dual {
            return;
        }
        a.sides[1 - a.active].tab().dir().map(|d| d.display().to_string()).unwrap_or_default()
    };
    let verb = if mv { "Move" } else { "Copy" };
    let spec = DialogSpec::ask(verb, &format!("{verb} {} to:", describe(&sources)), &initial, verb).select_all();
    open_dialog(app, Dialog::Transfer { sources, mv }, spec);
}

// ---- clipboard ----

/// Ctrl+C / Ctrl+X: the selection goes on the OS clipboard (cut: a paste moves it).
pub fn clipboard_put(app: &Shared, cut: bool) {
    let paths = app.borrow().tab().selected_paths();
    put_files(app, paths, cut);
}

pub(super) fn put_files(app: &Shared, paths: Vec<PathBuf>, cut: bool) {
    if !paths.is_empty() {
        app.borrow_mut().clipboard.put(&paths, cut);
    }
}

/// Ctrl+V: copies (or, after a cut, moves) the clipboard's files into the shown folder.
pub fn paste(app: &Shared) {
    let Some(target) = app.borrow().tab().dir().map(Path::to_path_buf) else { return };
    paste_into(app, target);
}

fn paste_into(app: &Shared, target: PathBuf) {
    let Some(files) = app.borrow_mut().clipboard.get() else { return };
    if files.cut {
        // moved files are gone from where the clipboard says they are
        app.borrow_mut().clipboard.clear();
    }
    run_operation(app, if files.cut { Kind::Move } else { Kind::Copy }, files.paths, Some(target));
}

/// Replaces the clipboard (tests use [`crate::clipboard::Memory`], never the user's clipboard).
pub fn set_clipboard(app: &Shared, clipboard: Box<dyn FileClipboard>) {
    app.borrow_mut().clipboard = clipboard;
}

/// The clipboard's text (tests).
pub fn clipboard_text(app: &Shared) -> Option<String> {
    app.borrow_mut().clipboard.text()
}

// ---- names ----

/// F2: asks for a new name; the old name is shown with the part before the extension selected.
pub fn rename_selection(app: &Shared) {
    let sel = app.borrow().tab().selected_paths();
    if let [src] = sel.as_slice() {
        rename(app, src.clone());
    }
}

fn rename(app: &Shared, src: PathBuf) {
    let name = fs_util::display_name(&src);
    let spec = DialogSpec::ask("Rename", "New name:", &name, "Rename");
    open_dialog(app, Dialog::Rename { src }, spec);
}

/// F7, Ctrl+Shift+N: a new folder in the shown folder, under a free name.
pub fn new_folder(app: &Shared) {
    let Some(dir) = app.borrow().tab().dir().map(Path::to_path_buf) else { return };
    new_item(app, dir, true);
}

/// Asks for the name of a new folder or (empty) text file in `dir`, offering a free name.
fn new_item(app: &Shared, dir: PathBuf, folder: bool) {
    let (default, title, prompt) =
        if folder { ("New folder", "New folder", "Folder name:") } else { ("New text file.txt", "New text file", "File name:") };
    let name = fs_util::display_name(&fs_util::unique_name(&dir.join(default), false));
    let spec = DialogSpec::ask(title, prompt, &name, "Create");
    open_dialog(app, Dialog::New { dir, folder }, spec);
}

// ---- right-click menus ----

fn menu_state(app: &Shared) -> menu::State {
    let can_paste = app.borrow_mut().clipboard.get().is_some();
    let a = app.borrow();
    let view = a.side().tabs[a.side().active].view.mode;
    menu::State {
        can_paste,
        trash: trash::is_supported(),
        sort: a.tab().sort(),
        view,
        show_hidden: a.show_hidden,
        dual: a.dual,
        bookmarks: a.bookmarks.items().iter().map(|b| b.path.clone()).collect(),
    }
}

/// A right-click on list row `row` (-1: empty space) of the active side: selects as Explorer
/// does, then builds the menu for the selection (or the folder).
pub fn row_menu(app: &Shared, row: i32) {
    app.borrow_mut().tab_mut().context_click(usize::try_from(row).ok());
    let side = app.borrow().active;
    sync_selection(app, side, false);
    show_file_menu(app);
}

/// The Menu key, Shift+F10: the selection's menu.
pub fn keyboard_menu(app: &Shared) {
    show_file_menu(app);
}

fn show_file_menu(app: &Shared) {
    let st = menu_state(app);
    let m = {
        let a = app.borrow();
        let items: Vec<menu::Item> =
            a.tab().selected_entries().into_iter().map(|e| menu::Item { path: e.path(), is_dir: e.is_dir() }).collect();
        menu::file_menu(&items, a.tab().dir(), &st)
    };
    set_menu(app, m);
}

/// A right-click in a tree: its menu, without navigating. False for rows without one.
pub fn tree_menu(app: &Shared, side: usize, row: usize) -> bool {
    let node = {
        let a = app.borrow();
        let t = &a.sides[side].tree;
        t.rows().get(row).map(|&id| t.node(id).clone())
    };
    let Some(node) = node else { return false };
    let m = match node.path {
        Some(path) => menu::tree_menu(&path, node.kind, &menu_state(app)),
        None if node.kind == NodeKind::BookmarkHeader => menu::bookmarks_header_menu(),
        None => return false,
    };
    set_menu(app, m);
    app.borrow_mut().sides[side].tree_menu_row = row as i32;
    push_view(app, side);
    true
}

/// A menu closed (the window's keys came back): the tree row it was for is no longer outlined.
pub fn menu_closed(app: &Shared) {
    for side in 0..2 {
        let outlined = std::mem::replace(&mut app.borrow_mut().sides[side].tree_menu_row, -1) >= 0;
        if outlined {
            push_view(app, side);
        }
    }
}

pub fn header_menu(app: &Shared) {
    let m = menu::header_menu(&app.borrow().columns);
    set_menu(app, m);
}

/// The view button next to the address bar.
pub fn view_menu(app: &Shared) {
    let mode = {
        let a = app.borrow();
        a.side().tabs[a.side().active].view.mode
    };
    set_menu(app, menu::view_menu(mode));
}

/// The sort button next to the view button.
pub fn sort_menu(app: &Shared) {
    let sort = app.borrow().tab().sort();
    set_menu(app, menu::sort_menu(sort));
}

pub fn tab_menu(app: &Shared, side: usize, index: usize) {
    let n = app.borrow().sides[side].tabs.len();
    set_menu(app, menu::tab_menu(side, index, n));
}

/// A menu section as Slint rows; the commands go to `commands`, by `MenuRow.id`.
pub(super) fn menu_rows(entries: &[menu::Entry], commands: &mut Vec<Command>) -> ModelRc<MenuRow> {
    let rows: Vec<MenuRow> = entries
        .iter()
        .map(|e| {
            commands.push(e.command.clone());
            MenuRow {
                id: commands.len() as i32 - 1,
                title: e.title.as_str().into(),
                enabled: e.enabled,
                checkable: e.checked.is_some(),
                checked: e.checked == Some(true),
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// Hands `m` to app.slint's sections; the commands stay here by id.
fn set_menu(app: &Shared, m: Menu) {
    let mut commands = Vec::new();
    let mut rows = |entries: &[menu::Entry]| menu_rows(entries, &mut commands);
    let section = |i: usize| m.sections.get(i).map(Vec::as_slice).unwrap_or_default();
    debug_assert!(m.sections.len() <= menu::SECTIONS);
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    ui.set_menu_s0(rows(section(0)));
    ui.set_menu_s1(rows(section(1)));
    ui.set_menu_view(rows(&m.view));
    ui.set_menu_sort(rows(&m.sort));
    ui.set_menu_new(rows(&m.new));
    ui.set_menu_s2(rows(section(2)));
    ui.set_menu_s3(rows(section(3)));
    ui.set_menu_s4(rows(section(4)));
    ui.set_menu_s5(rows(section(5)));
    app.borrow_mut().menu = commands;
}

pub fn menu_activated(app: &Shared, id: i32) {
    let command = usize::try_from(id).ok().and_then(|i| app.borrow().menu.get(i).cloned());
    if let Some(c) = command {
        run_command(app, c);
    }
}

/// Runs a menu command (tests call it with the commands of [`menu`]'s menus).
pub fn run_command(app: &Shared, command: Command) {
    let parent = |paths: &[PathBuf]| paths.first().and_then(|p| p.parent()).map(Path::to_path_buf);
    let side = app.borrow().active;
    match command {
        Command::OpenSelection => open_selection(app),
        Command::OpenFolder(dir) => {
            let id = app.borrow().active_tab_id();
            nav::go(app, id, dir, Nav::Go);
        }
        Command::OpenInNewTab(dir) => {
            nav::new_tab(app, side, dir, true);
        }
        Command::OpenInOtherSide(dir) => nav::open_in_other_side(app, dir),
        Command::CopyToOther(paths) => transfer(app, paths, false),
        Command::MoveToOther(paths) => transfer(app, paths, true),
        Command::OpenTerminal(dir) => launch(app, "Open in terminal", move || shell::open_terminal(&dir)),
        Command::Reveal(p) => launch(app, &format!("Show in {}", shell::file_manager_name()), move || shell::reveal(&p)),
        Command::OpenInFileManager(dir) => launch(app, "Open", move || shell::open(&dir)),
        Command::Cut(paths) => put_files(app, paths, true),
        Command::Copy(paths) => put_files(app, paths, false),
        Command::PasteInto(dir) => paste_into(app, dir),
        Command::Duplicate(paths) => {
            // copying onto itself makes "name - Copy"
            if let Some(dir) = parent(&paths) {
                run_operation(app, Kind::Copy, paths, Some(dir));
            }
        }
        Command::Compress(paths) => {
            if let Some(dir) = parent(&paths) {
                run_operation(app, Kind::Compress, paths, Some(dir));
            }
        }
        Command::Extract(zip) => {
            if let Some(dir) = zip.parent().map(Path::to_path_buf) {
                run_operation(app, Kind::Extract, vec![zip], Some(dir));
            }
        }
        Command::Rename(p) => rename(app, p),
        Command::Trash(paths) => delete(app, paths, false),
        Command::Delete(paths) => delete(app, paths, true),
        Command::CopyPath(paths) => {
            let text = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
            app.borrow_mut().clipboard.put_text(&text);
        }
        Command::Properties(paths) => open_properties(app, paths),
        Command::NewFolder(dir) => new_item(app, dir, true),
        Command::NewFile(dir) => new_item(app, dir, false),
        Command::SortBy(key) => {
            let asc = app.borrow().tab().sort().1;
            app.borrow_mut().tab_mut().set_sort(key, asc);
            sync_files(app, side);
        }
        Command::SortAscending(asc) => {
            let key = app.borrow().tab().sort().0;
            app.borrow_mut().tab_mut().set_sort(key, asc);
            sync_files(app, side);
        }
        Command::SetView(mode) => views::set_view(app, side, mode),
        Command::ToggleHidden => nav::toggle_hidden(app),
        Command::Refresh => nav::refresh_active(app),
        Command::ToggleColumn(c) => {
            app.borrow_mut().columns.toggle(c);
            push_columns(app);
            // the extra columns' texts are only filled in while shown
            sync_files(app, 0);
            sync_files(app, 1);
        }
        Command::DuplicateTab(s, i) => {
            let dir = app.borrow().sides[s].tabs.get(i).and_then(|t| t.tab.dir().map(Path::to_path_buf));
            if let Some(d) = dir {
                nav::new_tab(app, s, d, true);
            }
        }
        Command::TabToOtherSide(s, i) => {
            let dir = app.borrow().sides[s].tabs.get(i).and_then(|t| t.tab.dir().map(Path::to_path_buf));
            if let Some(d) = dir {
                nav::set_active_side(app, s);
                nav::open_in_other_side(app, d);
            }
        }
        Command::CloseTab(s, i) => nav::close_tab(app, s, i),
        Command::CloseOtherTabs(s, i) => nav::close_other_tabs(app, s, i),
        Command::Search(dir) => {
            super::search::open(app, dir);
        }
        // only in a search window's menu, which runs them itself
        Command::OpenResults | Command::ShowInFolder(_) => {}
        Command::AddBookmark(dir) => super::bookmarks::add(app, dir),
        Command::RemoveBookmark(dir) => super::bookmarks::remove(app, &dir),
        Command::RenameBookmark(dir) => super::bookmarks::ask_rename(app, dir),
        Command::MoveBookmark(dir, delta) => super::bookmarks::move_by(app, &dir, delta),
        Command::ManageBookmarks => {
            super::bookmarks::manage(app);
        }
    }
}

// ---- properties ----

/// Alt+Enter: the selection's properties, or the folder's when nothing is selected.
pub fn properties_of_selection(app: &Shared) {
    let (sel, dir) = {
        let a = app.borrow();
        (a.tab().selected_paths(), a.tab().dir().map(Path::to_path_buf))
    };
    match (sel.is_empty(), dir) {
        (false, _) => open_properties(app, sel),
        (true, Some(d)) => open_properties(app, vec![d]),
        (true, None) => {}
    }
}

/// Opens a Properties window (not modal; several can be open). Reading the items happens in the
/// background, and so does counting folders, which stops when the window closes.
pub(super) fn open_properties(app: &Shared, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let done_app = app.clone();
    let read = paths.clone();
    bg::run(move |_| properties::sheet(&read), move |sheet| show_properties(&done_app, paths, sheet));
}

fn show_properties(app: &Shared, paths: Vec<PathBuf>, sheet: properties::Sheet) {
    let Ok(w) = PropertiesWindow::new() else { return };
    w.set_window_title(sheet.title.into());
    w.set_header(sheet.header.into());
    w.set_item_icon(sheet.entry.as_ref().map_or(-1, icon_of));
    let lines: Vec<PropLine> = sheet
        .lines
        .into_iter()
        .map(|l| PropLine { label: l.label.into(), value: l.value.into(), selectable: l.selectable, group_start: l.group_start })
        .collect();
    w.set_lines(ModelRc::from(Rc::new(VecModel::from(lines))));
    let stop = Arc::new(AtomicBool::new(false));
    let id = app.borrow_mut().next_id();
    let close_app = app.clone();
    w.on_close_clicked(move || close_properties(&close_app, id));
    let close_app = app.clone();
    w.window().on_close_requested(move || {
        close_properties(&close_app, id);
        slint::CloseRequestResponse::HideWindow
    });
    if sheet.walk {
        let (weak, stop) = (w.as_weak(), stop.clone());
        std::thread::Builder::new()
            .name("fileem-properties".into())
            .spawn(move || {
                let post = |t: Totals| {
                    let _ = weak.upgrade_in_event_loop(move |w| set_totals(&w, t));
                };
                let total = measure::measure(&paths, &stop, post);
                post(total);
            })
            .expect("spawn the properties walk");
    }
    let _ = w.show();
    app.borrow_mut().properties.push((id, w, stop));
}

fn set_totals(w: &PropertiesWindow, t: Totals) {
    let (size, contains) = properties::totals_text(t);
    let lines = w.get_lines();
    for i in 0..lines.row_count() {
        let Some(mut l) = lines.row_data(i) else { continue };
        let value = match l.label.as_str() {
            properties::SIZE => &size,
            properties::CONTAINS => &contains,
            _ => continue,
        };
        l.value = value.as_str().into();
        lines.set_row_data(i, l);
    }
}

fn close_properties(app: &Shared, id: u64) {
    let closed = {
        let mut a = app.borrow_mut();
        a.properties.iter().position(|(i, ..)| *i == id).map(|at| a.properties.remove(at))
    };
    if let Some((_, w, stop)) = closed {
        stop.store(true, Ordering::Relaxed);
        let _ = w.hide();
    }
}

/// The open Properties windows (tests).
pub fn properties_windows(app: &Shared) -> Vec<PropertiesWindow> {
    app.borrow().properties.iter().map(|(_, w, _)| w.clone_strong()).collect()
}

/// Closes the windows that belong to this one (the main window is closing).
pub fn close_windows(app: &Shared) {
    super::preview::hide_window(app);
    super::search::close_all(app);
    super::bookmarks::close(app);
    let ids: Vec<u64> = app.borrow().properties.iter().map(|(i, ..)| *i).collect();
    for id in ids {
        close_properties(app, id);
    }
}

// ---- dialogs ----

/// What the open dialog is for.
pub enum Dialog {
    None,
    Info,
    Delete { paths: Vec<PathBuf>, trash: bool },
    Rename { src: PathBuf },
    RenameBookmark { path: PathBuf },
    New { dir: PathBuf, folder: bool },
    Transfer { sources: Vec<PathBuf>, mv: bool },
    /// After a drop: Move (OK) or Copy (the alternative).
    Drop { sources: Vec<PathBuf>, target: PathBuf },
    Quit,
}

/// How a dialog looks.
pub struct DialogSpec {
    title: String,
    message: String,
    input: Option<String>,
    /// Select all of the input (a path), not just the name before its extension.
    select_all: bool,
    lines: Vec<String>,
    ok: String,
    /// A second choice next to OK, empty for none.
    pub alt: String,
    can_cancel: bool,
}

impl DialogSpec {
    pub fn info(title: &str, message: &str) -> DialogSpec {
        DialogSpec {
            title: title.into(),
            message: message.into(),
            input: None,
            select_all: false,
            lines: Vec::new(),
            ok: "OK".into(),
            alt: String::new(),
            can_cancel: false,
        }
    }

    pub fn confirm(title: &str, question: &str, ok: &str) -> DialogSpec {
        DialogSpec { ok: ok.into(), can_cancel: true, ..DialogSpec::info(title, question) }
    }

    pub fn ask(title: &str, prompt: &str, initial: &str, ok: &str) -> DialogSpec {
        DialogSpec { input: Some(initial.into()), ..DialogSpec::confirm(title, prompt, ok) }
    }

    /// The input starts all selected (a path, a bookmark's name), not just the part before a dot.
    pub fn select_all(self) -> DialogSpec {
        DialogSpec { select_all: true, ..self }
    }
}

/// A window that draws the overlay dialog (`ui/dialog.slint`) through its `dialog-*` properties.
pub(super) trait DialogHost {
    fn show_dialog(&self, spec: DialogSpec);
    fn hide_dialog(&self);
    fn set_dialog_problem(&self, text: String);
}

// AppWindow and SearchWindow have the same `dialog-*` properties (generated as separate types)
macro_rules! dialog_host {
    ($window:ty, $focus:ident) => {
        impl DialogHost for $window {
            fn show_dialog(&self, spec: DialogSpec) {
                self.set_dialog_title(spec.title.into());
                self.set_dialog_message(spec.message.into());
                self.set_dialog_input(spec.input.is_some());
                let text = spec.input.unwrap_or_default();
                // select the name without its extension, as Explorer does
                let end = match text.rfind('.') {
                    Some(dot) if dot > 0 && !spec.select_all => dot,
                    _ => text.len(),
                };
                self.set_dialog_select_end(end as i32);
                self.set_dialog_text(text.into());
                self.set_dialog_error("".into());
                let lines: Vec<SharedString> = spec.lines.into_iter().map(SharedString::from).collect();
                self.set_dialog_lines(ModelRc::from(Rc::new(VecModel::from(lines))));
                self.set_dialog_ok(spec.ok.into());
                self.set_dialog_alt(spec.alt.into());
                self.set_dialog_can_cancel(spec.can_cancel);
                self.set_dialog_open(true);
            }

            fn hide_dialog(&self) {
                self.set_dialog_open(false);
                self.$focus();
            }

            fn set_dialog_problem(&self, text: String) {
                self.set_dialog_error(text.into());
            }
        }
    };
}

dialog_host!(crate::AppWindow, invoke_focus_list);
dialog_host!(crate::SearchWindow, invoke_focus_results);
dialog_host!(crate::BookmarksWindow, invoke_focus_list);

pub fn open_dialog(app: &Shared, dialog: Dialog, spec: DialogSpec) {
    app.borrow_mut().dialog = dialog;
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    ui.show_dialog(spec);
}

pub fn close_dialog(app: &Shared) {
    app.borrow_mut().dialog = Dialog::None;
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.hide_dialog();
    }
}

fn dialog_error(app: &Shared, text: String) {
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_dialog_problem(text);
    }
}

pub fn dialog_accepted(app: &Shared, text: &str) {
    let dialog = std::mem::replace(&mut app.borrow_mut().dialog, Dialog::None);
    match dialog {
        Dialog::None | Dialog::Info => close_dialog(app),
        Dialog::Delete { paths, trash } => {
            close_dialog(app);
            run_operation(app, if trash { Kind::Trash } else { Kind::Delete }, paths, None);
        }
        Dialog::Rename { src } => match rename_to(app, &src, text) {
            Ok(_) => close_dialog(app),
            Err(problem) => {
                app.borrow_mut().dialog = Dialog::Rename { src };
                dialog_error(app, problem);
            }
        },
        Dialog::RenameBookmark { path } => {
            close_dialog(app);
            super::bookmarks::rename(app, &path, text);
        }
        Dialog::New { dir, folder } => {
            if let Some(problem) = names::name_problem(&dir, text, None) {
                app.borrow_mut().dialog = Dialog::New { dir, folder };
                return dialog_error(app, problem);
            }
            let made = if folder { names::create_folder(&dir, text) } else { names::create_file(&dir, text) };
            match made {
                Ok(created) => {
                    close_dialog(app);
                    if app.borrow().tab().dir() == Some(dir.as_path()) {
                        refresh_selecting(app, created);
                    }
                    if folder {
                        nav::reload_tree_node(app, &dir);
                    }
                }
                Err(e) => {
                    app.borrow_mut().dialog = Dialog::New { dir, folder };
                    dialog_error(app, format!("Cannot create \u{201c}{}\u{201d}: {}", text.trim(), reason(&e)));
                }
            }
        }
        Dialog::Transfer { sources, mv } => {
            let current = app.borrow().tab().dir().map(Path::to_path_buf);
            let Some(target) = address::resolve(text, current.as_deref(), &os::home()) else {
                app.borrow_mut().dialog = Dialog::Transfer { sources, mv };
                return dialog_error(app, "Enter a target folder.".into());
            };
            close_dialog(app);
            // checked in the background: a typed folder may be on a mount that hangs
            let done_app = app.clone();
            let check = target.clone();
            bg::run(
                move |_| check.is_dir(),
                move |is_dir: bool| {
                    let verb = if mv { "Move" } else { "Copy" };
                    if is_dir {
                        run_operation(&done_app, if mv { Kind::Move } else { Kind::Copy }, sources, Some(target));
                    } else {
                        let msg = format!("The target folder does not exist:\n{}", target.display());
                        open_dialog(&done_app, Dialog::Info, DialogSpec::info(verb, &msg));
                    }
                },
            );
        }
        Dialog::Drop { sources, target } => {
            close_dialog(app);
            run_operation(app, Kind::Move, sources, Some(target));
        }
        Dialog::Quit => {
            close_dialog(app);
            quit_after_cancelling(app);
        }
    }
}

/// Renames `src` to the name `text` (checked first) and lets the window follow: the tab showing
/// its folder selects it, the tree reloads, tabs inside a renamed folder move along. The problem
/// to show in the dialog otherwise.
pub(super) fn rename_to(app: &Shared, src: &Path, text: &str) -> Result<PathBuf, String> {
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    if let Some(problem) = names::name_problem(&dir, text, Some(src)) {
        return Err(problem);
    }
    let target = names::rename(src, text)
        .map_err(|e| format!("Cannot rename \u{201c}{}\u{201d}: {}", fs_util::display_name(src), reason(&e)))?;
    if app.borrow().tab().dir() == Some(dir.as_path()) {
        refresh_selecting(app, target.clone());
    }
    // a folder renamed from the tree (or a search) may sit elsewhere; tabs inside it follow
    nav::reload_tree_node(app, &dir);
    nav::tabs_follow_rename(app, src, &target);
    super::bookmarks::relocate(app, src, &target);
    Ok(target)
}

fn reason(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "Access denied".into(),
        std::io::ErrorKind::AlreadyExists => "An item with that name already exists".into(),
        std::io::ErrorKind::NotFound => "Not found".into(),
        _ => e.to_string(),
    }
}

// ---- closing ----

/// The window's close button: with operations running it asks first. Returns whether the window
/// may close now.
pub fn close_requested(app: &Shared) -> bool {
    let n = app.borrow().ops.running().len();
    if n == 0 {
        return true;
    }
    let what = if n == 1 { "An operation is".to_string() } else { format!("{n} operations are") };
    let spec = DialogSpec::confirm(
        "Operations running",
        &format!("{what} still running. Cancel and close fileEm?\n\nUnfinished copies are removed; nothing is lost."),
        "Cancel and close",
    );
    open_dialog(app, Dialog::Quit, spec);
    false
}

/// A termination signal (logout, `kill`): saves the session, cancels every operation and waits
/// up to 3 s for their cleanup (as the Java app's shutdown hook), closes the other windows. The
/// caller quits.
pub fn terminate(app: &Shared) {
    super::collect_session(app);
    app.borrow().session.save();
    let handles = app.borrow().ops.running();
    OpsUi::cancel_all_and_wait(&handles, Duration::from_secs(3));
    close_windows(app);
}

/// Cancels every operation, waits (off the UI thread) until their cleanup is done, then quits.
fn quit_after_cancelling(app: &Shared) {
    let handles = app.borrow().ops.running();
    set_status(app, "Stopping operations…".into());
    std::thread::spawn(move || {
        OpsUi::cancel_all_and_wait(&handles, Duration::from_secs(10));
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
    });
}
