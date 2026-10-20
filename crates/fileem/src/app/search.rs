//! Search windows (the Java `SearchWindow`): one per search, each searching a folder and
//! everything below it. The search runs on a thread of its own; results queue up and reach the
//! window in batches (at most one pending UI update). Starting again, Stop, Escape and closing
//! stop it. The results' commands are the file list's, run so the results follow: a renamed
//! result (and what was found inside a renamed folder) is re-read at its new path, deleted ones
//! leave the list.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use slint::platform::Key;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use fileem_core::fs::search::{self, Listener};
use fileem_core::fs::{trash, FileEntry};
use fileem_core::ops::Kind;
use fileem_core::util::{formats, fs_util};

use super::commands::{self, Dialog, DialogHost, DialogSpec};
use super::{icon_of, nav, Shared};
use crate::bg;
use crate::menu::{self, Command};
use crate::search::{self as results, Column, Results, MAX_RESULTS};
use crate::shell;
use crate::{SearchRow, SearchWindow};

pub struct Search {
    id: u64,
    window: SearchWindow,
    results: Results,
    rows: Rc<VecModel<SearchRow>>,
    run: Option<Run>,
    dialog: Dialog,
    /// The commands of the menu shown last, by `MenuRow.id`.
    menu: Vec<Command>,
    ensure_seq: i32,
}

/// One search: its thread fills `found`, the UI thread takes from it.
struct Run {
    stop: Arc<AtomicBool>,
    found: Arc<Mutex<Found>>,
    started: Instant,
}

#[derive(Default)]
struct Found {
    batch: Vec<FileEntry>,
    /// A UI update is already on its way.
    queued: bool,
    count: usize,
    folders: u64,
    unreadable: u64,
    /// The search ended: whether it got through everything, or why it couldn't start.
    done: Option<Result<bool, String>>,
}

impl Found {
    fn summary(&self, shown: usize) -> String {
        results::summary(shown, self.folders)
    }
}

/// Reports from the searching thread.
struct Collector {
    found: Arc<Mutex<Found>>,
    window: slint::Weak<SearchWindow>,
}

impl Collector {
    fn post(&self, f: &mut Found) {
        if !f.queued {
            f.queued = true;
            let _ = self.window.upgrade_in_event_loop(|w| w.invoke_results_ready());
        }
    }
}

impl Listener for Collector {
    fn found(&mut self, entry: FileEntry) {
        let mut f = self.found.lock().unwrap();
        f.batch.push(entry);
        f.count += 1;
        self.post(&mut f);
    }

    fn entering(&mut self, _dir: &Path) {
        self.found.lock().unwrap().folders += 1;
    }

    fn failed(&mut self, _path: &Path) {
        self.found.lock().unwrap().unreadable += 1;
    }

    fn wants_more(&self) -> bool {
        self.found.lock().unwrap().count < MAX_RESULTS // `finished` tells why it stopped
    }
}

/// Opens a search window for `dir` and everything below it; hidden files are searched when the
/// file list shows them.
pub fn open(app: &Shared, dir: PathBuf) -> Option<SearchWindow> {
    let w = SearchWindow::new().ok()?;
    let id = app.borrow_mut().next_id();
    w.set_window_title(format!("Search in {}", fs_util::display_name(&dir)).into());
    w.set_root_path(dir.display().to_string().into());
    w.set_include_hidden(app.borrow().show_hidden);
    let rows = Rc::new(VecModel::default());
    w.set_rows(ModelRc::from(rows.clone()));
    wire(app, &w, id);
    app.borrow_mut().searches.push(Search {
        id,
        window: w.clone_strong(),
        results: Results::new(dir),
        rows,
        run: None,
        dialog: Dialog::None,
        menu: Vec::new(),
        ensure_seq: 0,
    });
    push_history(app);
    let _ = w.show();
    w.invoke_focus_pattern();
    Some(w)
}

fn wire(app: &Shared, w: &SearchWindow, id: u64) {
    macro_rules! on {
        ($setter:ident, $app:ident => |$($arg:ident),*| $body:expr) => {{
            let a = app.clone();
            w.$setter(move |$($arg),*| {
                let $app = &a;
                $body
            });
        }};
    }
    on!(on_search, app => |text| start(app, id, &text));
    on!(on_stop, app => | | stop(app, id));
    on!(on_history_picked, app => |text| {
        if let Some(w) = window(app, id) {
            w.set_pattern(text.clone());
        }
        start(app, id, &text);
    });
    on!(on_results_ready, app => | | drain(app, id));
    on!(on_row_pressed, app => |row, ctrl, shift| {
        with(app, id, |s| s.results.click(row as usize, ctrl, shift));
        sync_selection(app, id, false);
    });
    on!(on_row_activated, app => |row| {
        with(app, id, |s| s.results.click(row as usize, false, false));
        sync_selection(app, id, false);
        open_selection(app, id);
    });
    on!(on_row_menu, app => |row| {
        if let Ok(r) = usize::try_from(row) {
            with(app, id, |s| s.results.context_click(r));
            sync_selection(app, id, false);
        }
        show_menu(app, id);
    });
    on!(on_sort_by, app => |col| {
        if let Some(c) = Column::from_index(col as usize) {
            with(app, id, |s| s.results.sort_by(c));
            sync_rows(app, id, false);
        }
    });
    on!(on_key, app => |text, ctrl, alt, shift| key(app, id, &text, ctrl, alt, shift));
    on!(on_escape, app => | | {
        if with(app, id, |s| s.run.is_some()).unwrap_or(false) {
            stop(app, id);
        } else {
            close(app, id);
        }
    });
    on!(on_menu_activated, app => |item| {
        let command = with(app, id, |s| usize::try_from(item).ok().and_then(|i| s.menu.get(i).cloned())).flatten();
        if let Some(c) = command {
            run_command(app, id, c);
        }
    });
    on!(on_dialog_accepted, app => |text| dialog_accepted(app, id, &text));
    on!(on_dialog_cancelled, app => | | close_dialog(app, id));
    on!(on_dialog_alternative, app => | | close_dialog(app, id));
    let a = app.clone();
    w.window().on_close_requested(move || {
        close(&a, id);
        slint::CloseRequestResponse::HideWindow
    });
}

/// Runs `f` on the search window `id`, if it is still open.
fn with<T>(app: &Shared, id: u64, f: impl FnOnce(&mut Search) -> T) -> Option<T> {
    let mut a = app.borrow_mut();
    a.searches.iter_mut().find(|s| s.id == id).map(f)
}

fn window(app: &Shared, id: u64) -> Option<SearchWindow> {
    with(app, id, |s| s.window.clone_strong())
}

/// The open search windows (tests).
pub fn windows(app: &Shared) -> Vec<SearchWindow> {
    app.borrow().searches.iter().map(|s| s.window.clone_strong()).collect()
}

/// The paths of the results of the `index`th open search window, as shown, and the selected ones
/// (tests).
pub fn results(app: &Shared, index: usize) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let a = app.borrow();
    a.searches.get(index).map_or_else(Default::default, |s| {
        (s.results.rows().map(|e| e.path().to_path_buf()).collect(), s.results.selected_paths())
    })
}

/// The last searches go to every open search window.
fn push_history(app: &Shared) {
    let (items, windows): (Vec<SharedString>, Vec<SearchWindow>) = {
        let a = app.borrow();
        (a.history.items().iter().map(|p| p.as_str().into()).collect(), a.searches.iter().map(|s| s.window.clone_strong()).collect())
    };
    for w in windows {
        w.set_history(ModelRc::from(Rc::new(VecModel::from(items.clone()))));
    }
}

fn set_status(app: &Shared, id: u64, text: &str) {
    if let Some(w) = window(app, id) {
        w.set_status(text.into());
    }
}

// ---- searching ----

/// Starts searching for `text`, replacing the results of an earlier search.
fn start(app: &Shared, id: u64, text: &str) {
    stop_quietly(app, id);
    with(app, id, |s| s.results.clear());
    sync_rows(app, id, false);
    let Some(matcher) = search::matcher(text) else {
        set_status(app, id, "Type a name or a pattern to search for.");
        return;
    };
    {
        let mut a = app.borrow_mut();
        let a = &mut *a;
        a.history.add(text, &a.session);
    }
    push_history(app);
    let Some(w) = window(app, id) else { return };
    let root = with(app, id, |s| s.results.root().to_path_buf()).unwrap_or_default();
    let include_hidden = w.get_include_hidden();
    let run = Run { stop: Arc::new(AtomicBool::new(false)), found: Arc::new(Mutex::new(Found::default())), started: Instant::now() };
    let mut collector = Collector { found: run.found.clone(), window: w.as_weak() };
    let stop = run.stop.clone();
    std::thread::Builder::new()
        .name("fileem-search".into())
        .spawn(move || {
            let result = search::search(&root, &matcher, include_hidden, &mut collector, &stop)
                .map_err(|e| format!("Cannot search {}: {e}", root.display()));
            let mut f = collector.found.lock().unwrap();
            f.done = Some(result);
            collector.post(&mut f);
        })
        .expect("spawn the search");
    with(app, id, |s| s.run = Some(run));
    w.set_running(true);
    w.set_status("Searching…".into());
}

/// Takes what the search found since the last time into the list; ends the run when it is done.
fn drain(app: &Shared, id: u64) {
    let taken = with(app, id, |s| {
        let run = s.run.as_ref()?;
        let mut f = run.found.lock().unwrap();
        f.queued = false;
        let batch = std::mem::take(&mut f.batch);
        let done = f.done.take();
        let (folders, unreadable) = (f.folders, f.unreadable);
        let seconds = run.started.elapsed().as_secs();
        drop(f);
        s.results.append(batch);
        let shown = s.results.len();
        let status = match &done {
            None => format!("Searching…   {}", results::summary(shown, folders)),
            Some(Ok(complete)) => results::finished(shown, folders, *complete, seconds, unreadable),
            Some(Err(msg)) => msg.clone(),
        };
        if done.is_some() {
            s.run = None;
        }
        Some((status, done.is_some()))
    })
    .flatten();
    let Some((status, done)) = taken else { return };
    sync_rows(app, id, true);
    if let Some(w) = window(app, id) {
        w.set_status(status.into());
        if done {
            w.set_running(false);
        }
    }
}

/// Stops a running search; the results so far stay.
fn stop(app: &Shared, id: u64) {
    if let Some(summary) = stop_quietly(app, id) {
        sync_rows(app, id, true);
        set_status(app, id, &format!("Stopped.   {summary}"));
    }
}

/// Stops the run and takes its last results; the summary, if one was running.
fn stop_quietly(app: &Shared, id: u64) -> Option<String> {
    let summary = with(app, id, |s| {
        let run = s.run.take()?;
        run.stop.store(true, Ordering::Relaxed);
        let mut f = run.found.lock().unwrap();
        let batch = std::mem::take(&mut f.batch);
        s.results.append(batch);
        Some(f.summary(s.results.len()))
    })
    .flatten()?;
    if let Some(w) = window(app, id) {
        w.set_running(false);
    }
    Some(summary)
}

fn close(app: &Shared, id: u64) {
    stop_quietly(app, id);
    let closed = {
        let mut a = app.borrow_mut();
        a.searches.iter().position(|s| s.id == id).map(|at| a.searches.remove(at))
    };
    if let Some(s) = closed {
        let _ = s.window.hide();
    }
}

/// Closes every search window (the main window is closing).
pub fn close_all(app: &Shared) {
    let ids: Vec<u64> = app.borrow().searches.iter().map(|s| s.id).collect();
    for id in ids {
        close(app, id);
    }
}

// ---- rows ----

fn row_of(r: &Results, row: usize) -> SearchRow {
    let e = r.row(row);
    SearchRow {
        name: e.name().into(),
        folder: r.folder_of(e).into(),
        size: formats::size(e.size()).into(),
        kind: e.type_name().into(),
        modified: formats::date(e.modified()).into(),
        icon: icon_of(e),
        hidden: e.is_hidden(),
        selected: r.is_selected(row),
        current: r.current_row() == Some(row),
    }
}

/// Brings the window's rows up to date. With `added`, only results were added since: in found order
/// they go at the end; otherwise (sorted, renamed, removed) every row is built again.
fn sync_rows(app: &Shared, id: u64, added: bool) {
    let a = app.borrow();
    let Some(s) = a.searches.iter().find(|s| s.id == id) else { return };
    let r = &s.results;
    let have = s.rows.row_count();
    if added && r.sort().is_none() && have <= r.len() {
        for row in have..r.len() {
            s.rows.push(row_of(r, row));
        }
    } else {
        s.rows.set_vec((0..r.len()).map(|row| row_of(r, row)).collect::<Vec<_>>());
    }
    let (col, asc) = r.sort().map_or((-1, true), |(c, asc)| (c.index() as i32, asc));
    s.window.set_sort_column(col);
    s.window.set_sort_ascending(asc);
}

/// Updates the selection flags of the rows that changed; with `scroll`, shows the focused row.
fn sync_selection(app: &Shared, id: u64, scroll: bool) {
    let _ = with(app, id, |s| {
        let current = s.results.current_row();
        for row in 0..s.rows.row_count().min(s.results.len()) {
            let Some(mut r) = s.rows.row_data(row) else { continue };
            let (sel, cur) = (s.results.is_selected(row), current == Some(row));
            if r.selected != sel || r.current != cur {
                r.selected = sel;
                r.current = cur;
                s.rows.set_row_data(row, r);
            }
        }
        if let (true, Some(row)) = (scroll, current) {
            s.ensure_seq += 1;
            s.window.set_ensure_row(row as i32);
            s.window.set_ensure_seq(s.ensure_seq);
        }
    });
}

// ---- commands ----

fn selected(app: &Shared, id: u64) -> Vec<PathBuf> {
    with(app, id, |s| s.results.selected_paths()).unwrap_or_default()
}

/// The results' keys, like the file list's: Enter, F2, Del / F8, Shift+Del, Ctrl+C / X / A,
/// Alt+Enter and moving through the rows.
fn key(app: &Shared, id: u64, text: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
    let Some(c) = text.chars().next() else { return false };
    let is = |k: Key| c == char::from(k);
    let letter = |l: char| ctrl && !alt && c.eq_ignore_ascii_case(&l);
    let moved = |delta: isize| {
        with(app, id, |s| s.results.move_focus(delta, ctrl, shift));
        sync_selection(app, id, true);
    };
    let paths = selected(app, id);
    if is(Key::UpArrow) {
        moved(-1);
    } else if is(Key::DownArrow) {
        moved(1);
    } else if is(Key::PageUp) {
        moved(-20);
    } else if is(Key::PageDown) {
        moved(20);
    } else if is(Key::Home) {
        moved(isize::MIN / 2);
    } else if is(Key::End) {
        moved(isize::MAX / 2);
    } else if letter('a') {
        with(app, id, |s| s.results.select_all());
        sync_selection(app, id, false);
    } else if paths.is_empty() {
        return false;
    } else if (is(Key::Return) || c == '\n') && alt {
        commands::open_properties(app, paths);
    } else if (is(Key::Return) || c == '\n') && !ctrl {
        open_selection(app, id);
    } else if is(Key::F2) && paths.len() == 1 {
        rename(app, id, paths[0].clone());
    } else if is(Key::Delete) || is(Key::F8) {
        delete(app, id, paths, shift);
    } else if letter('c') {
        commands::put_files(app, paths, false);
    } else if letter('x') {
        commands::put_files(app, paths, true);
    } else {
        return false;
    }
    true
}

fn show_menu(app: &Shared, id: u64) {
    let m = menu::search_menu(&selected(app, id), trash::is_supported());
    let _ = with(app, id, |s| {
        let mut commands = Vec::new();
        let section = |i: usize| m.sections.get(i).map(Vec::as_slice).unwrap_or_default();
        s.window.set_menu_s0(commands::menu_rows(section(0), &mut commands));
        s.window.set_menu_s1(commands::menu_rows(section(1), &mut commands));
        s.window.set_menu_s2(commands::menu_rows(section(2), &mut commands));
        s.window.set_menu_s3(commands::menu_rows(section(3), &mut commands));
        s.menu = commands;
    });
}

/// A command from the results' menu (tests call it with [`menu::search_menu`]'s commands).
pub fn run_command(app: &Shared, id: u64, command: Command) {
    match command {
        Command::OpenResults => open_selection(app, id),
        Command::ShowInFolder(p) => nav::show_item(app, p),
        Command::Rename(p) => rename(app, id, p),
        Command::Trash(paths) => delete(app, id, paths, false),
        Command::Delete(paths) => delete(app, id, paths, true),
        other => commands::run_command(app, other),
    }
}

/// Opens the selected results: folders in the main window's active tab, files with their
/// programs (a failure is shown over the search window).
fn open_selection(app: &Shared, id: u64) {
    let entries: Vec<FileEntry> = with(app, id, |s| s.results.selected_entries().into_iter().cloned().collect()).unwrap_or_default();
    for e in entries {
        if e.is_dir() {
            let tab = app.borrow().active_tab_id();
            nav::go(app, tab, e.path().to_path_buf(), crate::tab::Nav::Go);
        } else if let Err(err) = shell::open(e.path()) {
            let msg = format!("Cannot open {}:\n{err}", e.name());
            open_dialog(app, id, Dialog::Info, DialogSpec::info("Open", &msg));
        }
    }
}

// ---- rename and delete, with a dialog over the search window ----

fn open_dialog(app: &Shared, id: u64, dialog: Dialog, spec: DialogSpec) {
    if let Some(w) = with(app, id, |s| {
        s.dialog = dialog;
        s.window.clone_strong()
    }) {
        w.show_dialog(spec);
    }
}

fn close_dialog(app: &Shared, id: u64) {
    if let Some(w) = with(app, id, |s| {
        s.dialog = Dialog::None;
        s.window.clone_strong()
    }) {
        w.hide_dialog();
    }
}

fn rename(app: &Shared, id: u64, src: PathBuf) {
    let name = fs_util::display_name(&src);
    open_dialog(app, id, Dialog::Rename { src }, DialogSpec::ask("Rename", "New name:", &name, "Rename"));
}

fn delete(app: &Shared, id: u64, paths: Vec<PathBuf>, permanent: bool) {
    if !paths.is_empty() {
        let (dialog, spec) = commands::delete_question(paths, permanent);
        open_dialog(app, id, dialog, spec);
    }
}

fn dialog_accepted(app: &Shared, id: u64, text: &str) {
    let Some(dialog) = with(app, id, |s| std::mem::replace(&mut s.dialog, Dialog::None)) else { return };
    match dialog {
        Dialog::Rename { src } => match commands::rename_to(app, &src, text) {
            Ok(target) => {
                close_dialog(app, id);
                follow_rename(app, id, src, target);
            }
            Err(problem) => {
                if let Some(w) = with(app, id, |s| {
                    s.dialog = Dialog::Rename { src };
                    s.window.clone_strong()
                }) {
                    w.set_dialog_problem(problem);
                }
            }
        },
        Dialog::Delete { paths, trash } => {
            close_dialog(app, id);
            let gone = paths.clone();
            let kind = if trash { Kind::Trash } else { Kind::Delete };
            commands::run_operation_then(app, kind, paths, None, move |app| remove_gone(app, id, gone));
        }
        _ => close_dialog(app, id),
    }
}

/// The results at or under a renamed item are read again at their new paths; the renamed one
/// stays selected.
fn follow_rename(app: &Shared, id: u64, src: PathBuf, target: PathBuf) {
    let Some(affected) = with(app, id, |s| s.results.under(std::slice::from_ref(&src))) else { return };
    let done_app = app.clone();
    let (from, to) = (src.clone(), target.clone());
    bg::run(
        move |_| {
            affected
                .iter()
                .map(|p| match p.strip_prefix(&from) {
                    // joining "" would add a separator: "new.txt/" isn't a file
                    Ok(rest) if !rest.as_os_str().is_empty() => FileEntry::read(&to.join(rest)),
                    _ => FileEntry::read(&to),
                })
                .collect::<Vec<_>>()
        },
        move |updated: Vec<FileEntry>| {
            with(&done_app, id, |s| {
                s.results.renamed(&src, &target, updated);
                s.results.select_path(&target);
            });
            sync_rows(&done_app, id, false);
            sync_selection(&done_app, id, true);
        },
    );
}

/// After a delete: the results that are gone leave the list, including what was found inside a
/// deleted folder; ones that failed stay.
fn remove_gone(app: &Shared, id: u64, roots: Vec<PathBuf>) {
    let Some(candidates) = with(app, id, |s| s.results.under(&roots)) else { return };
    let done_app = app.clone();
    bg::run(
        move |_| candidates.into_iter().filter(|p| !fs_util::exists_nofollow(p)).collect::<Vec<_>>(),
        move |gone: Vec<PathBuf>| {
            if gone.is_empty() {
                return;
            }
            let Some((left, running)) = with(&done_app, id, |s| {
                s.results.remove(&gone);
                (s.results.len(), s.run.is_some())
            }) else {
                return;
            };
            sync_rows(&done_app, id, false);
            if !running {
                let n = gone.len();
                let text = format!(
                    "{} {} removed   \u{b7}   {} left",
                    formats::count(n as u64),
                    if n == 1 { "item" } else { "items" },
                    formats::count(left as u64)
                );
                set_status(&done_app, id, &text);
            }
        },
    );
}
