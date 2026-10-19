//! The window's controller: wires the Slint callbacks to the tab and tree state, runs listings in
//! the background and keeps the models in sync. All commands and keys live here (the Java
//! `MainWindow`).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::Key;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use fileem_core::fs::listing::{self, SortKey};
use fileem_core::fs::watcher::{self, WatchHandle};
use fileem_core::fs::{places, Category, FileEntry};
use fileem_core::ops::{FileOperation, Kind, State};
use fileem_core::session::Session;
use fileem_core::util::{formats, fs_util};

use crate::bg::{self, Job};
use crate::shell;
use crate::tab::{Nav, Tab};
use crate::tree::{NodeKind, Reveal, Tree};
use crate::{AppWindow, FileRow, TreeRow};

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
    ui.on_dir_changed(move |token| dir_changed(&a, token));
    let a = app.clone();
    ui.on_key(move |text, ctrl, alt, shift| key(&a, &text, ctrl, alt, shift));
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
    sync_files(app);
    if same && nav == Nav::Refresh {
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

/// Moves the selection to the Trash on a worker thread; the folder is re-read when it ends.
fn trash_selection(app: &Shared) {
    let paths = app.borrow().tab.selected_paths();
    if paths.is_empty() {
        return;
    }
    let n = paths.len();
    set_status(app, format!("Moving {} {} to the Trash…", n, if n == 1 { "item" } else { "items" }));
    let done_app = app.clone();
    bg::run(
        move |_| {
            let h = FileOperation::new(Kind::Trash, paths, None).run();
            (h.state(), h.errors())
        },
        move |(state, errors): (State, Vec<String>)| {
            refresh(&done_app);
            let summary = match (state, errors.first()) {
                (State::Failed(msg), _) => Some(msg),
                (_, Some(first)) if errors.len() == 1 => Some(first.clone()),
                (_, Some(first)) => Some(format!("{first} (and {} more problems)", errors.len() - 1)),
                _ => None,
            };
            if let Some(s) = summary {
                // after the refresh's own status
                let a = done_app.clone();
                slint::Timer::single_shot(Duration::from_millis(300), move || set_status(&a, s));
            }
        },
    );
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
    } else if (is(Key::Return) || c == '\n') && !alt {
        let row = app.borrow().tab.current_row();
        if let Some(r) = row {
            activate(app, r);
        }
    } else if ctrl && c == ' ' {
        app.borrow_mut().tab.toggle_current();
        sync_selection(app);
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
    } else if !shift && (is(Key::Delete) || is(Key::F8)) {
        trash_selection(app);
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
