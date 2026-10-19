//! The window's controller: wires the Slint callbacks to the tab and tree state, runs listings in
//! the background and keeps the models in sync. All commands and keys live here (the Java
//! `MainWindow`).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use slint::platform::Key;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use fileem_core::fs::listing::{self, SortKey};
use fileem_core::fs::measure::{self, Totals};
use fileem_core::fs::watcher::{self, WatchHandle};
use fileem_core::fs::{places, Category, FileEntry};
use fileem_core::fs::trash;
use fileem_core::ops::{names, Handle, Kind, State};
use fileem_core::session::Session;
use fileem_core::util::{formats, fs_util};

use crate::bg::{self, Job};
use crate::clipboard::{self, FileClipboard};
use crate::menu::{self, Command, Menu};
use crate::ops_ui::{OpsUi, Outcome};
use crate::properties;
use crate::shell;
use crate::tab::{Nav, Tab};
use crate::tree::{NodeKind, Reveal, Tree};
use crate::{AppWindow, FileRow, MenuRow, PropLine, PropertiesWindow, TreeRow};

pub struct App {
    ui: slint::Weak<AppWindow>,
    session: Rc<Session>,
    tab: Tab,
    tree: Tree,
    files: Rc<VecModel<FileRow>>,
    tree_rows: Rc<VecModel<TreeRow>>,
    load: Option<Job>,
    /// Drops listings that finished after a newer navigation started.
    generation: u64,
    /// Drops tree loads of an older reveal.
    reveal_token: u64,
    type_ahead: String,
    type_ahead_at: Option<Instant>,
    /// The watch on the shown folder; dropping it stops watching.
    watch: Option<(PathBuf, WatchHandle)>,
    /// Drops watcher reports for a folder no longer shown.
    watch_token: u64,
    /// A navigation (not a refresh) is loading: watcher refreshes wait, it lists fresh anyway.
    navigating: bool,
    /// A refresh is loading; further requests collapse into one follow-up.
    refreshing: bool,
    refresh_pending: bool,
    /// How long the last listing took: follow-up refreshes wait twice that.
    last_list: Duration,
    /// Selected once the next refresh is shown (a renamed or created item).
    select_after: Option<PathBuf>,
    ops: OpsUi,
    dialog: Dialog,
    clipboard: Box<dyn FileClipboard>,
    /// The commands of the menu shown last, by `MenuRow.id`.
    menu: Vec<Command>,
    /// Open Properties windows by id; the flag stops their folder walk.
    properties: Vec<(u64, PropertiesWindow, Arc<AtomicBool>)>,
    next_properties: u64,
}

pub type Shared = Rc<RefCell<App>>;

/// Creates the controller for `ui`, restores the state saved in `session`, wires the callbacks and
/// opens `start` (default: the folder open last time, else the home folder).
pub fn install(ui: &AppWindow, session: Rc<Session>, start: Option<PathBuf>) -> Shared {
    let files = Rc::new(VecModel::default());
    let tree_rows = Rc::new(VecModel::default());
    ui.set_files(ModelRc::from(files.clone()));
    ui.set_tree(ModelRc::from(tree_rows.clone()));
    let mut tab = Tab::new();
    tab.set_show_hidden(session.get_bool("showHidden", false));
    let sort = match session.get_or("sort.column", "name").as_str() {
        "size" => SortKey::Size,
        "type" => SortKey::Type,
        "modified" => SortKey::Modified,
        _ => SortKey::Name,
    };
    tab.set_sort(sort, session.get_bool("sort.ascending", true));
    let mut tree = Tree::new(&places::quick_access(), &places::volumes());
    tree.set_show_hidden(tab.show_hidden());
    ui.set_tree_width(session.get_f64(&format!("{SIDE}.treeWidth"), 240.0).clamp(120.0, 800.0) as f32);
    let saved = session
        .get(&format!("{SIDE}.tab.{}", session.get_int(&format!("{SIDE}.selected"), 0)))
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from);
    let app = Rc::new(RefCell::new(App {
        ui: ui.as_weak(),
        session,
        tab,
        tree,
        files,
        tree_rows,
        load: None,
        generation: 0,
        reveal_token: 0,
        type_ahead: String::new(),
        type_ahead_at: None,
        watch: None,
        watch_token: 0,
        navigating: false,
        refreshing: false,
        refresh_pending: false,
        last_list: Duration::ZERO,
        select_after: None,
        ops: OpsUi::new(),
        dialog: Dialog::None,
        clipboard: Box::new(clipboard::System::default()),
        menu: Vec::new(),
        properties: Vec::new(),
        next_properties: 0,
    }));
    wire(ui, &app);
    sync_tree(&app);
    sync_files(&app);
    match (start, saved) {
        (Some(dir), _) => go(&app, dir, Nav::Go),
        // the folder from last time may be gone: its nearest existing parent then
        (None, Some(dir)) => go_with(&app, dir, Nav::Go, true),
        (None, None) => go(&app, fileem_core::util::os::home(), Nav::Go),
    }
    app
}

/// The session side this window stands for. One side until the dual pane is ported; the right
/// side's keys are left as the Java app wrote them.
const SIDE: &str = "left";

/// Copies the window's state into the session (autosave, close). Only keys this version knows
/// are written; the rest (other tabs, columns, theme, …) stays as the Java app left it, so both
/// versions can share the file during the migration.
pub fn collect_session(app: &Shared) {
    let a = app.borrow();
    let s = &a.session;
    s.put("showHidden", a.tab.show_hidden());
    let (key, asc) = a.tab.sort();
    let column = match key {
        SortKey::Name => "name",
        SortKey::Size => "size",
        SortKey::Type => "type",
        SortKey::Modified => "modified",
    };
    s.put("sort.column", column);
    s.put("sort.ascending", asc);
    if let Some(dir) = a.tab.dir() {
        let selected = s.get_int(&format!("{SIDE}.selected"), 0).max(0);
        if s.get_int(&format!("{SIDE}.tabs"), 0) <= selected {
            s.put(&format!("{SIDE}.tabs"), selected + 1);
        }
        s.put(&format!("{SIDE}.selected"), selected);
        s.put(&format!("{SIDE}.tab.{selected}"), dir.display());
    }
    if let Some(ui) = a.ui.upgrade() {
        s.put(&format!("{SIDE}.treeWidth"), ui.get_tree_width().round());
        let w = ui.window();
        s.put("window.maximized", w.is_maximized());
        if !w.is_maximized() && !w.is_minimized() {
            let size = w.size().to_logical(w.scale_factor());
            if size.width > 0.0 && size.height > 0.0 {
                // Java writes doubles; keep the same shape so it reads them back unchanged
                s.put("window.w", format!("{:.1}", size.width));
                s.put("window.h", format!("{:.1}", size.height));
            }
        }
    }
}

pub fn session(app: &Shared) -> Rc<Session> {
    app.borrow().session.clone()
}

fn wire(ui: &AppWindow, app: &Shared) {
    let a = app.clone();
    ui.on_back(move || back(&a));
    let a = app.clone();
    ui.on_forward(move || forward(&a));
    let a = app.clone();
    ui.on_up(move || up(&a));
    let a = app.clone();
    ui.on_refresh(move || refresh(&a));
    let a = app.clone();
    ui.on_toggle_hidden(move || toggle_hidden(&a));
    let a = app.clone();
    ui.on_navigate_address(move |text| navigate_address(&a, &text));
    let a = app.clone();
    ui.on_sort_by(move |col| sort_by(&a, col));
    let a = app.clone();
    ui.on_row_pressed(move |row, ctrl, shift| {
        a.borrow_mut().tab.click(row as usize, ctrl, shift);
        sync_selection(&a);
    });
    let a = app.clone();
    ui.on_row_activated(move |row| activate(&a, row as usize));
    let a = app.clone();
    ui.on_tree_clicked(move |row| tree_clicked(&a, row as usize));
    let a = app.clone();
    ui.on_tree_toggled(move |row| tree_toggled(&a, row as usize));
    let a = app.clone();
    ui.on_dialog_accepted(move |text| dialog_accepted(&a, &text));
    let a = app.clone();
    ui.on_dialog_cancelled(move || dialog_cancelled(&a));
    let a = app.clone();
    ui.on_dir_changed(move |token| dir_changed(&a, token));
    let a = app.clone();
    ui.on_key(move |text, ctrl, alt, shift| key(&a, &text, ctrl, alt, shift));
    let a = app.clone();
    ui.on_row_menu(move |row| row_menu(&a, row));
    let a = app.clone();
    ui.on_tree_menu(move |row| tree_menu(&a, row as usize));
    let a = app.clone();
    ui.on_keyboard_menu(move || {
        let row = a.borrow().tab.current_row().filter(|&r| a.borrow().tab.is_selected(r));
        show_file_menu(&a);
        row.map_or(-1, |r| r as i32)
    });
    let a = app.clone();
    ui.on_menu_activated(move |id| menu_activated(&a, id));
}

// ---- navigation ----

/// Lists `dir` in the background and shows it when done. File system calls stay off the UI
/// thread, even the check whether `dir` is a folder: on a dead network mount they would hang.
pub fn go(app: &Shared, dir: PathBuf, nav: Nav) {
    go_with(app, dir, nav, false);
}

/// Like [`go`]; with `nearest`, a folder that no longer exists is replaced by its nearest existing
/// parent (or home), looked up in the background.
fn go_with(app: &Shared, dir: PathBuf, nav: Nav, nearest: bool) {
    let generation = {
        let mut a = app.borrow_mut();
        if nav == Nav::Refresh {
            if a.navigating {
                return; // the navigation in flight lists its folder fresh anyway: don't drop it
            }
            a.refreshing = true;
        } else {
            a.navigating = true;
            // a cancelled refresh never calls back, so it can't clear this itself
            a.refreshing = false;
        }
        if let Some(job) = a.load.take() {
            job.cancel();
        }
        a.generation += 1;
        a.generation
    };
    let target = dir.clone();
    let done_app = app.clone();
    let job = bg::run(
        move |cancel| {
            let mut dir = std::path::absolute(&target).unwrap_or(target);
            if nearest {
                dir = dir.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf).unwrap_or_else(fileem_core::util::os::home);
            }
            let started = Instant::now();
            listing::list(&dir, cancel).map(|entries| (dir, entries, started.elapsed()))
        },
        move |result: std::io::Result<(PathBuf, Vec<FileEntry>, Duration)>| {
            if done_app.borrow().generation != generation {
                return;
            }
            {
                let mut a = done_app.borrow_mut();
                a.load = None;
                a.navigating = false;
            }
            if nav == Nav::Refresh {
                finish_refresh(&done_app);
            }
            match result {
                Ok((dir, entries, took)) => {
                    done_app.borrow_mut().last_list = took;
                    shown(&done_app, dir, entries, nav);
                }
                // the shown folder is gone: its nearest existing parent
                Err(_) if nav == Nav::Refresh => go_with(&done_app, dir, Nav::Go, true),
                Err(e) => failed(&done_app, &dir, &e),
            }
        },
    );
    app.borrow_mut().load = Some(job);
}

fn shown(app: &Shared, dir: PathBuf, entries: Vec<FileEntry>, nav: Nav) {
    let same = app.borrow().tab.dir() == Some(dir.as_path());
    let folders_before = folder_set(app.borrow().tab.rows());
    app.borrow_mut().tab.show(dir.clone(), entries, nav);
    let select = app.borrow_mut().select_after.take();
    let selected = select.is_some_and(|item| app.borrow_mut().tab.select_path(&item));
    sync_files(app);
    if same && nav == Nav::Refresh {
        // a renamed or created item is shown; otherwise the scroll stays where the user left it
        if let (true, Some(row)) = (selected, app.borrow().tab.current_row()) {
            if let Some(ui) = app.borrow().ui.upgrade() {
                ui.invoke_ensure_visible(row as i32);
            }
        }
        // a watcher refresh keeps scroll and focus where the user left them
        if folder_set(app.borrow().tab.rows()) != folders_before {
            reload_tree_node(app, &dir);
        }
        return;
    }
    if !same || app.borrow().watch.is_none() {
        watch_folder(app, &dir);
    }
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    ui.set_address(dir.display().to_string().into());
    ui.set_window_title(format!("{} - fileEm", fs_util::display_name(&dir)).into());
    if !same {
        ui.invoke_scroll_to_top();
    }
    if let Some(row) = app.borrow().tab.current_row() {
        ui.invoke_ensure_visible(row as i32);
    }
    ui.invoke_focus_list();
    if nav != Nav::Refresh || !same {
        reveal(app, dir);
    }
}

fn folder_set<'a>(rows: impl Iterator<Item = &'a FileEntry>) -> std::collections::HashSet<PathBuf> {
    rows.filter(|e| e.is_dir()).map(|e| e.path().to_path_buf()).collect()
}

// ---- watching ----

/// Watches the shown folder; changes refresh it. Registering touches the file system (it can
/// hang on a dead mount), so it happens in a background job.
fn watch_folder(app: &Shared, dir: &Path) {
    let (token, ui) = {
        let mut a = app.borrow_mut();
        a.watch = None;
        a.watch_token += 1;
        (a.watch_token, a.ui.clone())
    };
    let done_app = app.clone();
    let path = dir.to_path_buf();
    bg::run(
        move |_| {
            watcher::watch(&path, move || {
                // from the watcher's thread to the event loop; the window may be gone by then
                let _ = ui.upgrade_in_event_loop(move |ui| ui.invoke_dir_changed(token as i32));
            })
        },
        move |handle| {
            let mut a = done_app.borrow_mut();
            if a.watch_token == token {
                let dir = a.tab.dir().map(Path::to_path_buf).unwrap_or_default();
                a.watch = Some((dir, handle));
            }
        },
    );
}

/// A watcher report: refresh, coalesced (one follow-up at most while a refresh runs).
fn dir_changed(app: &Shared, token: i32) {
    if app.borrow().watch_token as i32 != token {
        return; // a report for a folder no longer shown
    }
    request_refresh(app);
}

fn request_refresh(app: &Shared) {
    let mut a = app.borrow_mut();
    if a.refreshing {
        a.refresh_pending = true;
        return;
    }
    drop(a);
    refresh(app);
}

/// After a refresh: a request that came in meanwhile runs after twice the listing time, so a big
/// folder under heavy writes isn't listed continuously.
fn finish_refresh(app: &Shared) {
    let mut a = app.borrow_mut();
    a.refreshing = false;
    if a.refresh_pending {
        a.refresh_pending = false;
        let delay = (a.last_list * 2).max(Duration::from_millis(250));
        let again = app.clone();
        slint::Timer::single_shot(delay, move || request_refresh(&again));
    }
}

/// Re-reads the subfolders of every loaded tree node showing `dir` (its set of subfolders changed).
fn reload_tree_node(app: &Shared, dir: &Path) {
    let ids = app.borrow().tree.loaded_nodes(dir);
    if ids.is_empty() {
        return;
    }
    let done_app = app.clone();
    let path = dir.to_path_buf();
    bg::run(
        move |_| listing::subfolders(&path, true),
        move |children: std::io::Result<Vec<FileEntry>>| {
            if let Ok(children) = children {
                for id in ids {
                    done_app.borrow_mut().tree.set_children(id, children.clone());
                }
                sync_tree(&done_app);
            }
        },
    );
}

fn failed(app: &Shared, dir: &Path, e: &std::io::Error) {
    let a = app.borrow();
    let Some(ui) = a.ui.upgrade() else { return };
    let reason = match e.kind() {
        std::io::ErrorKind::PermissionDenied => "Access denied".to_string(),
        std::io::ErrorKind::NotFound => "Not found".to_string(),
        std::io::ErrorKind::NotADirectory => "Not a folder".to_string(),
        _ => e.to_string(),
    };
    ui.set_status(format!("Cannot open \u{201c}{}\u{201d}: {reason}", dir.display()).into());
    // the address bar goes back to the folder still shown
    if let Some(cur) = a.tab.dir() {
        ui.set_address(cur.display().to_string().into());
    }
}

fn back(app: &Shared) {
    let target = app.borrow().tab.back_target().map(Path::to_path_buf);
    if let Some(t) = target {
        go(app, t, Nav::Back);
    }
}

fn forward(app: &Shared) {
    let target = app.borrow().tab.forward_target().map(Path::to_path_buf);
    if let Some(t) = target {
        go(app, t, Nav::Forward);
    }
}

fn up(app: &Shared) {
    let target = app.borrow().tab.up_target().map(Path::to_path_buf);
    if let Some(t) = target {
        go(app, t, Nav::Go);
    }
}

fn refresh(app: &Shared) {
    let dir = app.borrow().tab.dir().map(Path::to_path_buf);
    if let Some(d) = dir {
        go(app, d, Nav::Refresh);
    }
}

fn navigate_address(app: &Shared, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let path = match text.strip_prefix('~') {
        Some(rest) => fileem_core::util::os::home().join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(text),
    };
    go(app, path, Nav::Go);
}

fn activate(app: &Shared, row: usize) {
    let entry = {
        let a = app.borrow();
        if row >= a.tab.len() {
            return;
        }
        a.tab.row(row).clone()
    };
    if entry.is_dir() {
        go(app, entry.path().to_path_buf(), Nav::Go);
    } else if let Err(e) = shell::open(entry.path()) {
        set_status(app, format!("Cannot open \u{201c}{}\u{201d}: {e}", entry.name()));
    }
}

fn sort_by(app: &Shared, col: i32) {
    let key = match col {
        1 => SortKey::Size,
        2 => SortKey::Type,
        3 => SortKey::Modified,
        _ => SortKey::Name,
    };
    app.borrow_mut().tab.sort_by(key);
    sync_files(app);
}

fn toggle_hidden(app: &Shared) {
    {
        let mut a = app.borrow_mut();
        let show = !a.tab.show_hidden();
        a.tab.set_show_hidden(show);
        a.tree.set_show_hidden(show);
    }
    sync_files(app);
    sync_tree(app);
}

// ---- operations ----

/// Asks, then moves the selection to the Trash or deletes it, with a row in the progress window.
fn delete_selection(app: &Shared, permanent: bool) {
    let paths = app.borrow().tab.selected_paths();
    delete(app, paths, permanent);
}

fn delete(app: &Shared, paths: Vec<PathBuf>, permanent: bool) {
    if paths.is_empty() {
        return;
    }
    let trash = !permanent && trash::is_supported();
    let what = describe(&paths);
    let (title, question, ok) = if trash {
        ("Move to Trash", format!("Move {what} to the Trash?"), "Move to Trash")
    } else {
        ("Delete permanently", format!("Permanently delete {what}?\n\nThis cannot be undone."), "Delete")
    };
    open_dialog(app, Dialog::Delete { paths, trash }, DialogSpec::confirm(title, &question, ok));
}

/// "“a.txt”" for one item, "3 items" for more.
fn describe(paths: &[PathBuf]) -> String {
    match paths {
        [one] => format!("\u{201c}{}\u{201d}", fs_util::display_name(one)),
        many => format!("{} items", many.len()),
    }
}

/// Starts an operation with a row in the progress window; when it ends, the folders it touched
/// are refreshed (the watcher is slow on some systems) and its problems are listed.
pub fn run_operation(app: &Shared, kind: Kind, sources: Vec<PathBuf>, target: Option<PathBuf>) -> Handle {
    let ops = app.borrow().ops.clone();
    let done_app = app.clone();
    ops.run(kind, sources, target, move |outcome| operation_done(&done_app, outcome))
}

fn operation_done(app: &Shared, outcome: Outcome) {
    let shown = app.borrow().tab.dir().map(Path::to_path_buf);
    // what Compress / Extract made is selected when it lands in the folder shown
    let made = outcome.result.clone().filter(|r| outcome.state == State::Succeeded && r.parent() == shown.as_deref());
    if let Some(item) = made {
        refresh_selecting(app, item);
    } else if shown.as_ref().is_some_and(|d| outcome.affected.contains(d)) {
        refresh(app);
    }
    for dir in &outcome.affected {
        reload_tree_node(app, dir);
    }
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

// ---- clipboard ----

/// Ctrl+C / Ctrl+X: the selection goes on the OS clipboard (cut: a paste moves it).
fn clipboard_put(app: &Shared, cut: bool) {
    let paths = app.borrow().tab.selected_paths();
    put_files(app, paths, cut);
}

fn put_files(app: &Shared, paths: Vec<PathBuf>, cut: bool) {
    if !paths.is_empty() {
        app.borrow_mut().clipboard.put(&paths, cut);
    }
}

/// Ctrl+V: copies (or, after a cut, moves) the clipboard's files into the shown folder.
fn paste(app: &Shared) {
    let Some(target) = app.borrow().tab.dir().map(Path::to_path_buf) else { return };
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

/// Replaces the clipboard (tests use [`clipboard::Memory`], never the user's clipboard).
pub fn set_clipboard(app: &Shared, clipboard: Box<dyn FileClipboard>) {
    app.borrow_mut().clipboard = clipboard;
}

/// The clipboard's text (tests).
pub fn clipboard_text(app: &Shared) -> Option<String> {
    app.borrow_mut().clipboard.text()
}

/// F2: asks for a new name; the old name is shown with the part before the extension selected.
fn rename_selection(app: &Shared) {
    let sel = app.borrow().tab.selected_paths();
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
fn new_folder(app: &Shared) {
    let Some(dir) = app.borrow().tab.dir().map(Path::to_path_buf) else { return };
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

/// Re-reads the shown folder and selects `item` in it.
fn refresh_selecting(app: &Shared, item: PathBuf) {
    app.borrow_mut().select_after = Some(item);
    refresh(app);
}

// ---- right-click menus ----

fn menu_state(app: &Shared) -> menu::State {
    let can_paste = app.borrow_mut().clipboard.get().is_some();
    let a = app.borrow();
    menu::State { can_paste, trash: trash::is_supported(), sort: a.tab.sort(), show_hidden: a.tab.show_hidden() }
}

/// A right-click on list row `row` (-1: empty space): selects as Explorer does, then builds the
/// menu for the selection (or the folder).
fn row_menu(app: &Shared, row: i32) {
    app.borrow_mut().tab.context_click(usize::try_from(row).ok());
    sync_selection(app);
    show_file_menu(app);
}

fn show_file_menu(app: &Shared) {
    let st = menu_state(app);
    let m = {
        let a = app.borrow();
        let items: Vec<menu::Item> = a.tab.selected_entries().into_iter().map(|e| menu::Item { path: e.path(), is_dir: e.is_dir() }).collect();
        menu::file_menu(&items, a.tab.dir(), &st)
    };
    set_menu(app, m);
}

/// A right-click in the tree: its menu, without navigating. False for rows without one.
fn tree_menu(app: &Shared, row: usize) -> bool {
    let node = {
        let a = app.borrow();
        a.tree.rows().get(row).map(|&id| a.tree.node(id).clone())
    };
    let Some((path, kind)) = node.and_then(|n| n.path.map(|p| (p, n.kind))) else { return false };
    let st = menu_state(app);
    set_menu(app, menu::tree_menu(&path, kind, &st));
    true
}

/// Hands `m` to app.slint's sections; the commands stay here by id.
fn set_menu(app: &Shared, m: Menu) {
    let mut commands = Vec::new();
    let mut rows = |entries: &[menu::Entry]| -> ModelRc<MenuRow> {
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
    };
    let section = |i: usize| m.sections.get(i).map(Vec::as_slice).unwrap_or_default();
    debug_assert!(m.sections.len() <= menu::SECTIONS);
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    ui.set_menu_s0(rows(section(0)));
    ui.set_menu_s1(rows(section(1)));
    ui.set_menu_sort(rows(&m.sort));
    ui.set_menu_new(rows(&m.new));
    ui.set_menu_s2(rows(section(2)));
    ui.set_menu_s3(rows(section(3)));
    ui.set_menu_s4(rows(section(4)));
    ui.set_menu_s5(rows(section(5)));
    app.borrow_mut().menu = commands;
}

fn menu_activated(app: &Shared, id: i32) {
    let command = usize::try_from(id).ok().and_then(|i| app.borrow().menu.get(i).cloned());
    if let Some(c) = command {
        run_command(app, c);
    }
}

/// Runs a menu command (tests call it with the commands of [`menu`]'s menus).
pub fn run_command(app: &Shared, command: Command) {
    let parent = |paths: &[PathBuf]| paths.first().and_then(|p| p.parent()).map(Path::to_path_buf);
    match command {
        Command::OpenSelection => open_selection(app),
        Command::OpenFolder(dir) => go(app, dir, Nav::Go),
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
            let asc = app.borrow().tab.sort().1;
            app.borrow_mut().tab.set_sort(key, asc);
            sync_files(app);
        }
        Command::SortAscending(asc) => {
            let key = app.borrow().tab.sort().0;
            app.borrow_mut().tab.set_sort(key, asc);
            sync_files(app);
        }
        Command::ToggleHidden => toggle_hidden(app),
        Command::Refresh => refresh(app),
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

/// "Open": one selected folder opens in the tab; files open with their programs.
fn open_selection(app: &Shared) {
    let (dirs, files): (Vec<FileEntry>, Vec<FileEntry>) =
        app.borrow().tab.selected_entries().into_iter().cloned().partition(|e| e.is_dir());
    if let ([dir], []) = (dirs.as_slice(), files.as_slice()) {
        return go(app, dir.path().to_path_buf(), Nav::Go);
    }
    for f in files {
        if let Err(e) = shell::open(f.path()) {
            set_status(app, format!("Cannot open \u{201c}{}\u{201d}: {e}", f.name()));
        }
    }
}

// ---- properties ----

/// Alt+Enter: the selection's properties, or the folder's when nothing is selected.
fn properties_of_selection(app: &Shared) {
    let (sel, dir) = {
        let a = app.borrow();
        (a.tab.selected_paths(), a.tab.dir().map(Path::to_path_buf))
    };
    match (sel.is_empty(), dir) {
        (false, _) => open_properties(app, sel),
        (true, Some(d)) => open_properties(app, vec![d]),
        (true, None) => {}
    }
}

/// Opens a Properties window (not modal; several can be open). Reading the items happens in the
/// background, and so does counting folders, which stops when the window closes.
fn open_properties(app: &Shared, paths: Vec<PathBuf>) {
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
    let id = {
        let mut a = app.borrow_mut();
        a.next_properties += 1;
        a.next_properties
    };
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
    New { dir: PathBuf, folder: bool },
    Quit,
}

/// How a dialog looks.
pub struct DialogSpec {
    title: String,
    message: String,
    input: Option<String>,
    lines: Vec<String>,
    ok: String,
    can_cancel: bool,
}

impl DialogSpec {
    fn info(title: &str, message: &str) -> DialogSpec {
        DialogSpec {
            title: title.into(),
            message: message.into(),
            input: None,
            lines: Vec::new(),
            ok: "OK".into(),
            can_cancel: false,
        }
    }

    fn confirm(title: &str, question: &str, ok: &str) -> DialogSpec {
        DialogSpec { ok: ok.into(), can_cancel: true, ..DialogSpec::info(title, question) }
    }

    fn ask(title: &str, prompt: &str, initial: &str, ok: &str) -> DialogSpec {
        DialogSpec { input: Some(initial.into()), ..DialogSpec::confirm(title, prompt, ok) }
    }
}

pub fn open_dialog(app: &Shared, dialog: Dialog, spec: DialogSpec) {
    app.borrow_mut().dialog = dialog;
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    ui.set_dialog_title(spec.title.into());
    ui.set_dialog_message(spec.message.into());
    ui.set_dialog_input(spec.input.is_some());
    let text = spec.input.unwrap_or_default();
    // select the name without its extension, as Explorer does
    let end = match text.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => text.len(),
    };
    ui.set_dialog_select_end(end as i32);
    ui.set_dialog_text(text.into());
    ui.set_dialog_error("".into());
    ui.set_dialog_lines(ModelRc::from(Rc::new(VecModel::from(
        spec.lines.into_iter().map(SharedString::from).collect::<Vec<_>>(),
    ))));
    ui.set_dialog_ok(spec.ok.into());
    ui.set_dialog_can_cancel(spec.can_cancel);
    ui.set_dialog_open(true);
}

fn close_dialog(app: &Shared) {
    app.borrow_mut().dialog = Dialog::None;
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_dialog_open(false);
        ui.invoke_focus_list();
    }
}

fn dialog_error(app: &Shared, text: String) {
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_dialog_error(text.into());
    }
}

fn dialog_cancelled(app: &Shared) {
    close_dialog(app);
}

fn dialog_accepted(app: &Shared, text: &str) {
    let dialog = std::mem::replace(&mut app.borrow_mut().dialog, Dialog::None);
    match dialog {
        Dialog::None | Dialog::Info => close_dialog(app),
        Dialog::Delete { paths, trash } => {
            close_dialog(app);
            run_operation(app, if trash { Kind::Trash } else { Kind::Delete }, paths, None);
        }
        Dialog::Rename { src } => {
            let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
            if let Some(problem) = names::name_problem(&dir, text, Some(&src)) {
                app.borrow_mut().dialog = Dialog::Rename { src };
                return dialog_error(app, problem);
            }
            match names::rename(&src, text) {
                Ok(target) => {
                    close_dialog(app);
                    if app.borrow().tab.dir() == Some(dir.as_path()) {
                        refresh_selecting(app, target);
                    }
                    // a folder renamed from the tree may sit elsewhere
                    reload_tree_node(app, &dir);
                }
                Err(e) => {
                    let name = fs_util::display_name(&src);
                    app.borrow_mut().dialog = Dialog::Rename { src };
                    dialog_error(app, format!("Cannot rename \u{201c}{name}\u{201d}: {}", reason(&e)));
                }
            }
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
                    if app.borrow().tab.dir() == Some(dir.as_path()) {
                        refresh_selecting(app, created);
                    }
                    if folder {
                        reload_tree_node(app, &dir);
                    }
                }
                Err(e) => {
                    app.borrow_mut().dialog = Dialog::New { dir, folder };
                    dialog_error(app, format!("Cannot create \u{201c}{}\u{201d}: {}", text.trim(), reason(&e)));
                }
            }
        }
        Dialog::Quit => {
            close_dialog(app);
            quit_after_cancelling(app);
        }
    }
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

// ---- tree ----

/// Reveals `dir` in the tree, loading folders along the way in the background.
fn reveal(app: &Shared, dir: PathBuf) {
    let token = {
        let mut a = app.borrow_mut();
        a.reveal_token += 1;
        a.reveal_token
    };
    reveal_step(app, dir, token);
}

fn reveal_step(app: &Shared, dir: PathBuf, token: u64) {
    let step = app.borrow_mut().tree.reveal(&dir);
    match step {
        Reveal::Load(id, path) => {
            let done_app = app.clone();
            bg::run(
                move |_| listing::subfolders(&path, true),
                move |children: std::io::Result<Vec<FileEntry>>| {
                    if done_app.borrow().reveal_token != token {
                        return;
                    }
                    // an unreadable folder shows no children rather than asking again forever
                    done_app.borrow_mut().tree.set_children(id, children.unwrap_or_default());
                    reveal_step(&done_app, dir, token);
                },
            );
        }
        Reveal::Done(row) => {
            sync_tree(app);
            if let Some(ui) = app.borrow().ui.upgrade() {
                ui.set_tree_scroll_target(row as i32);
                ui.invoke_apply_tree_scroll();
            }
        }
        Reveal::NotFound => sync_tree(app),
    }
}

fn tree_clicked(app: &Shared, row: usize) {
    let path = {
        let a = app.borrow();
        a.tree.rows().get(row).and_then(|&id| a.tree.node(id).path.clone())
    };
    if let Some(p) = path {
        go(app, p, Nav::Go);
    }
}

fn tree_toggled(app: &Shared, row: usize) {
    let load = app.borrow_mut().tree.toggle(row);
    sync_tree(app);
    if let Some((id, path)) = load {
        let done_app = app.clone();
        bg::run(
            move |_| listing::subfolders(&path, true),
            move |children: std::io::Result<Vec<FileEntry>>| {
                done_app.borrow_mut().tree.set_children(id, children.unwrap_or_default());
                sync_tree(&done_app);
            },
        );
    }
}

// ---- keys ----

/// The single keyboard handler (the Java `MainWindow.onKey`). Returns true when handled.
fn key(app: &Shared, text: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
    let Some(c) = text.chars().next() else { return false };
    let is = |k: Key| c == char::from(k);
    let page = 20;
    if alt && is(Key::LeftArrow) {
        back(app);
    } else if alt && is(Key::RightArrow) {
        forward(app);
    } else if (alt && is(Key::UpArrow)) || (!ctrl && !alt && is(Key::Backspace)) {
        up(app);
    } else if is(Key::UpArrow) {
        move_focus(app, -1, ctrl, shift);
    } else if is(Key::DownArrow) {
        move_focus(app, 1, ctrl, shift);
    } else if is(Key::PageUp) {
        move_focus(app, -page, ctrl, shift);
    } else if is(Key::PageDown) {
        move_focus(app, page, ctrl, shift);
    } else if is(Key::Home) {
        move_focus(app, isize::MIN / 2, ctrl, shift);
    } else if is(Key::End) {
        move_focus(app, isize::MAX / 2, ctrl, shift);
    } else if (is(Key::Return) || c == '\n') && alt {
        properties_of_selection(app);
    } else if is(Key::Return) || c == '\n' {
        let row = app.borrow().tab.current_row();
        if let Some(r) = row {
            activate(app, r);
        }
    } else if ctrl && c == ' ' {
        app.borrow_mut().tab.toggle_current();
        sync_selection(app);
    } else if ctrl && !alt && !shift && c.eq_ignore_ascii_case(&'c') {
        clipboard_put(app, false);
    } else if ctrl && !alt && !shift && c.eq_ignore_ascii_case(&'x') {
        clipboard_put(app, true);
    } else if ctrl && !alt && !shift && c.eq_ignore_ascii_case(&'v') {
        paste(app);
    } else if ctrl && !alt && c.eq_ignore_ascii_case(&'a') {
        app.borrow_mut().tab.select_all();
        sync_selection(app);
    } else if ctrl && !alt && c.eq_ignore_ascii_case(&'r') {
        refresh(app);
    } else if ctrl && !alt && c.eq_ignore_ascii_case(&'h') {
        toggle_hidden(app);
    } else if ctrl && !alt && c.eq_ignore_ascii_case(&'l') {
        if let Some(ui) = app.borrow().ui.upgrade() {
            ui.invoke_focus_address();
        }
    } else if is(Key::Delete) || is(Key::F8) {
        delete_selection(app, shift);
    } else if is(Key::F2) && !ctrl && !alt {
        rename_selection(app);
    } else if is(Key::F7) || (ctrl && shift && c.eq_ignore_ascii_case(&'n')) {
        new_folder(app);
    } else if !ctrl && !alt && !c.is_control() && !is_special(c) {
        type_ahead(app, c);
    } else {
        return false;
    }
    true
}

/// Slint sends special keys (arrows, F-keys) as private-use characters.
fn is_special(c: char) -> bool {
    ('\u{F700}'..='\u{F8FF}').contains(&c)
}

fn move_focus(app: &Shared, delta: isize, ctrl: bool, shift: bool) {
    app.borrow_mut().tab.move_focus(delta, ctrl, shift);
    sync_selection(app);
}

/// Typing a name's first letters jumps to it; a pause of a second starts a new search.
fn type_ahead(app: &Shared, c: char) {
    let found = {
        let mut a = app.borrow_mut();
        let now = Instant::now();
        if a.type_ahead_at.is_none_or(|t| now - t > Duration::from_secs(1)) {
            a.type_ahead.clear();
        }
        a.type_ahead_at = Some(now);
        a.type_ahead.extend(c.to_lowercase());
        let prefix = a.type_ahead.clone();
        let path = a.tab.rows().find(|e| e.name().to_lowercase().starts_with(&prefix)).map(|e| e.path().to_path_buf());
        path.is_some_and(|p| a.tab.select_path(&p))
    };
    if found {
        sync_selection(app);
    }
}

// ---- models ----

fn set_status(app: &Shared, text: String) {
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_status(text.into());
    }
}

/// Category to the icon numbers `FileIcon` in app.slint draws.
fn icon_of(e: &FileEntry) -> i32 {
    match e.category() {
        Category::Folder => 0,
        Category::File => 1,
        Category::Image => 2,
        Category::Audio => 3,
        Category::Video => 4,
        Category::Archive => 5,
        Category::Pdf => 6,
        Category::Word => 7,
        Category::Spreadsheet => 8,
        Category::Presentation => 9,
        Category::Code => 10,
        Category::Text => 11,
        Category::Executable => 12,
        Category::Link => 13,
    }
}

fn file_row(tab: &Tab, row: usize) -> FileRow {
    let e = tab.row(row);
    FileRow {
        name: e.name().into(),
        size: formats::size(e.size()).into(),
        kind: e.type_name().into(),
        modified: formats::date(e.modified()).into(),
        icon: icon_of(e),
        selected: tab.is_selected(row),
        current: tab.current_row() == Some(row),
        hidden: e.is_hidden(),
    }
}

/// Rebuilds the file rows (after a listing, sort or filter change).
fn sync_files(app: &Shared) {
    let a = app.borrow();
    let rows: Vec<FileRow> = (0..a.tab.len()).map(|r| file_row(&a.tab, r)).collect();
    a.files.set_vec(rows);
    let (key, asc) = a.tab.sort();
    if let Some(ui) = a.ui.upgrade() {
        ui.set_sort_column(match key {
            SortKey::Name => 0,
            SortKey::Size => 1,
            SortKey::Type => 2,
            SortKey::Modified => 3,
        });
        ui.set_sort_ascending(asc);
        ui.set_show_hidden(a.tab.show_hidden());
        ui.set_can_back(a.tab.back_target().is_some());
        ui.set_can_forward(a.tab.forward_target().is_some());
        ui.set_can_up(a.tab.up_target().is_some());
        ui.set_status(a.tab.status().into());
        ui.set_message(SharedString::from(if a.tab.is_empty() { "This folder is empty." } else { "" }));
    }
}

/// Updates only the selection flags of rows that changed, then scrolls the focus into view.
fn sync_selection(app: &Shared) {
    let a = app.borrow();
    let current = a.tab.current_row();
    for r in 0..a.files.row_count() {
        let Some(mut row) = a.files.row_data(r) else { continue };
        let (sel, cur) = (a.tab.is_selected(r), current == Some(r));
        if row.selected != sel || row.current != cur {
            row.selected = sel;
            row.current = cur;
            a.files.set_row_data(r, row);
        }
    }
    if let Some(ui) = a.ui.upgrade() {
        ui.set_status(a.tab.status().into());
        if let Some(r) = current {
            ui.invoke_ensure_visible(r as i32);
        }
    }
}

fn sync_tree(app: &Shared) {
    let a = app.borrow();
    let t = &a.tree;
    let rows: Vec<TreeRow> = t
        .rows()
        .iter()
        .map(|&id| {
            let n = t.node(id);
            TreeRow {
                name: n.name.clone().into(),
                depth: n.depth as i32,
                header: n.kind == NodeKind::Header,
                expandable: t.expandable(id),
                expanded: n.expanded,
                current: t.current() == Some(id),
                on_path: t.on_path(id),
                icon: match n.kind {
                    NodeKind::Quick if n.depth == 0 && n.name == "Home" => 20,
                    NodeKind::Volume => 21,
                    _ => 0,
                },
            }
        })
        .collect();
    a.tree_rows.set_vec(rows);
}

/// The operations of this window (tests start their own through it).
pub fn ops(app: &Shared) -> OpsUi {
    app.borrow().ops.clone()
}
