//! Navigation and listings per tab, directory watching, the folder trees, tabs and sides.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fileem_core::fs::listing;
use fileem_core::fs::watcher;
use fileem_core::fs::FileEntry;
use fileem_core::util::os;

use super::{
    ensure_visible, push_view, set_status, sync_files, sync_selection, sync_tabs, sync_tree, sync_window, Shared,
};
use crate::address;
use crate::bg;
use crate::tab::Nav;
use crate::tree::Reveal;

/// Test hook: runs in a navigation's job right after it listed its folder.
static AFTER_LISTING: std::sync::Mutex<Option<fn(&Path)>> = std::sync::Mutex::new(None);

/// Sets (or clears) a function each navigation calls right after listing its folder, in the
/// background job (for tests: a change made there must still show up).
pub fn set_after_listing(hook: Option<fn(&Path)>) {
    *AFTER_LISTING.lock().unwrap() = hook;
}

/// Lists `dir` in the background and shows it in tab `id` when done. File system calls stay off
/// the UI thread, even the check whether `dir` is a folder: on a dead network mount they would
/// hang.
pub fn go(app: &Shared, id: u64, dir: PathBuf, nav: Nav) {
    go_with(app, id, dir, nav, false);
}

/// Like [`go`]; with `nearest`, a folder that no longer exists is replaced by its nearest existing
/// parent (or home), looked up in the background.
pub fn go_with(app: &Shared, id: u64, dir: PathBuf, nav: Nav, nearest: bool) {
    let (generation, watch) = {
        let mut a = app.borrow_mut();
        // a navigation watches its folder before listing it, so a change in between is caught
        let watch = (nav != Nav::Refresh).then(|| (a.next_id(), a.ui.clone()));
        let Some(t) = a.tab_state(id) else { return };
        if nav == Nav::Refresh {
            if t.navigating {
                return; // the navigation in flight lists its folder fresh anyway: don't drop it
            }
            t.refreshing = true;
        } else {
            t.navigating = true;
            // a cancelled refresh never calls back, so it can't clear this itself
            t.refreshing = false;
        }
        if let Some(job) = t.load.take() {
            job.cancel();
        }
        t.generation += 1;
        if let Some((token, _)) = watch {
            t.next_watch_token = token;
            t.next_changed = false;
        }
        (t.generation, watch)
    };
    let token = watch.as_ref().map(|(token, _)| *token);
    let target = dir.clone();
    let done_app = app.clone();
    let job = bg::run(
        move |cancel| {
            let mut dir = std::path::absolute(&target).unwrap_or(target);
            if nearest {
                dir = dir.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf).unwrap_or_else(os::home);
            }
            let handle = watch.map(|(token, ui)| watch_folder(&dir, token, ui));
            let started = Instant::now();
            let listed = listing::list(&dir, cancel);
            if let Some(hook) = *AFTER_LISTING.lock().unwrap() {
                hook(&dir);
            }
            listed.map(|entries| (dir, entries, started.elapsed(), handle))
        },
        move |result: std::io::Result<(PathBuf, Vec<FileEntry>, Duration, Option<watcher::WatchHandle>)>| {
            let changed = {
                let mut a = done_app.borrow_mut();
                let Some(t) = a.tab_state(id) else { return };
                if t.generation != generation {
                    return;
                }
                t.load = None;
                t.navigating = false;
                if let Ok((_, _, took, _)) = &result {
                    t.last_list = *took;
                }
                t.next_watch_token = 0;
                std::mem::take(&mut t.next_changed)
            };
            if nav == Nav::Refresh {
                finish_refresh(&done_app, id);
            }
            match result {
                Ok((dir, entries, _, handle)) => {
                    // the folder's new watch replaces the old one; a failed navigation keeps the old
                    if let (Some(handle), Some(token)) = (handle, token) {
                        if let Some(t) = done_app.borrow_mut().tab_state(id) {
                            t.watch = Some(handle);
                            t.watch_token = token;
                        }
                    }
                    shown(&done_app, id, dir, entries, nav);
                    if changed {
                        request_refresh(&done_app, id); // changed while it was listed
                    }
                }
                // the shown folder is gone: its nearest existing parent
                Err(_) if nav == Nav::Refresh => go_with(&done_app, id, dir, Nav::Go, true),
                Err(e) => failed(&done_app, id, &dir, &e),
            }
        },
    );
    if let Some(t) = app.borrow_mut().tab_state(id) {
        t.load = Some(job);
    }
}

fn shown(app: &Shared, id: u64, dir: PathBuf, entries: Vec<FileEntry>, nav: Nav) {
    let (side, same, folders_before, selected) = {
        let mut a = app.borrow_mut();
        let Some((side, _)) = a.find(id) else { return };
        let t = a.tab_state(id).expect("found above");
        let same = t.tab.dir() == Some(dir.as_path());
        let before = folder_set(t.tab.rows());
        t.tab.show(dir.clone(), entries, nav);
        let select = t.select_after.take();
        let selected = select.is_some_and(|item| t.tab.select_path(&item));
        (side, same, before, selected)
    };
    let visible = app.borrow().find(id).is_some_and(|(s, i)| app.borrow().sides[s].active == i);
    if !same {
        sync_tabs(app, side);
    }
    if visible {
        sync_files(app, side);
    }
    if same && nav == Nav::Refresh {
        // a watcher refresh keeps scroll and focus where the user left them; a renamed or
        // created item is shown
        if selected && visible {
            ensure_visible(app, side);
        }
        let folders_after = {
            let a = app.borrow();
            a.find(id).map(|(s, i)| folder_set(a.sides[s].tabs[i].tab.rows()))
        };
        if folders_after.is_some_and(|f| f != folders_before) {
            reload_tree_node(app, &dir);
        }
        return;
    }
    if visible {
        if !same {
            app.borrow_mut().sides[side].top_seq += 1;
        }
        ensure_visible(app, side);
        if app.borrow().active == side {
            if let Some(ui) = app.borrow().ui.upgrade() {
                ui.invoke_focus_list();
            }
        }
        reveal(app, side, dir);
    }
}

fn folder_set<'a>(rows: impl Iterator<Item = &'a FileEntry>) -> HashSet<PathBuf> {
    rows.filter(|e| e.is_dir()).map(|e| e.path().to_path_buf()).collect()
}

fn failed(app: &Shared, id: u64, dir: &Path, e: &std::io::Error) {
    if !app.borrow().is_shown(id) {
        return;
    }
    let reason = match e.kind() {
        std::io::ErrorKind::PermissionDenied => "Access denied".to_string(),
        std::io::ErrorKind::NotFound => "Not found".to_string(),
        std::io::ErrorKind::NotADirectory => "Not a folder".to_string(),
        _ => e.to_string(),
    };
    set_status(app, format!("Cannot open \u{201c}{}\u{201d}: {reason}", dir.display()));
}

// ---- toolbar and keys ----

pub fn back(app: &Shared) {
    let (id, target) = {
        let a = app.borrow();
        (a.active_tab_id(), a.tab().back_target().map(Path::to_path_buf))
    };
    if let Some(t) = target {
        go(app, id, t, Nav::Back);
    }
}

pub fn forward(app: &Shared) {
    let (id, target) = {
        let a = app.borrow();
        (a.active_tab_id(), a.tab().forward_target().map(Path::to_path_buf))
    };
    if let Some(t) = target {
        go(app, id, t, Nav::Forward);
    }
}

pub fn up(app: &Shared) {
    let (id, target) = {
        let a = app.borrow();
        (a.active_tab_id(), a.tab().up_target().map(Path::to_path_buf))
    };
    if let Some(t) = target {
        go(app, id, t, Nav::Go);
    }
}

/// Ctrl+R: the active tab and its folder in the tree.
pub fn refresh_active(app: &Shared) {
    let (id, dir) = {
        let a = app.borrow();
        (a.active_tab_id(), a.tab().dir().map(Path::to_path_buf))
    };
    if let Some(d) = dir {
        reload_tree_node(app, &d);
        go(app, id, d, Nav::Refresh);
    }
}

fn refresh_tab(app: &Shared, id: u64) {
    let dir = app.borrow_mut().tab_state(id).and_then(|t| t.tab.dir().map(Path::to_path_buf));
    if let Some(d) = dir {
        go(app, id, d, Nav::Refresh);
    }
}

/// Re-reads `item`'s folder in the active tab and selects `item` there.
pub fn refresh_selecting(app: &Shared, item: PathBuf) {
    let id = app.borrow().active_tab_id();
    if let Some(t) = app.borrow_mut().tab_state(id) {
        t.select_after = Some(item);
    }
    refresh_tab(app, id);
}

/// The address bar's text: a folder opens; a file opens its folder with the file selected.
pub fn navigate_address(app: &Shared, text: &str) {
    let (id, current) = {
        let a = app.borrow();
        (a.active_tab_id(), a.tab().dir().map(Path::to_path_buf))
    };
    let Some(path) = address::resolve(text, current.as_deref(), &os::home()) else { return };
    let done_app = app.clone();
    bg::run(
        move |_| {
            let is_file = path.metadata().is_ok_and(|m| !m.is_dir());
            (path, is_file)
        },
        move |(path, is_file): (PathBuf, bool)| match (is_file, path.parent()) {
            (true, Some(parent)) => {
                if let Some(t) = done_app.borrow_mut().tab_state(id) {
                    t.select_after = Some(path.clone());
                }
                go(&done_app, id, parent.to_path_buf(), Nav::Go);
            }
            _ => go(&done_app, id, path, Nav::Go),
        },
    );
}

/// Opens the folder holding `item` in the active tab, with `item` selected (a search result's
/// "Open containing folder").
pub fn show_item(app: &Shared, item: PathBuf) {
    let Some(parent) = item.parent().map(Path::to_path_buf) else { return };
    let (id, same) = {
        let a = app.borrow();
        (a.active_tab_id(), a.tab().dir() == Some(parent.as_path()))
    };
    if let Some(t) = app.borrow_mut().tab_state(id) {
        t.select_after = Some(item);
    }
    go(app, id, parent, if same { Nav::Refresh } else { Nav::Go });
}

/// Ctrl+L: the address bar turns into a text field.
pub fn edit_address(app: &Shared) {
    let side = app.borrow().active;
    app.borrow_mut().sides[side].edit_seq += 1;
    push_view(app, side);
}

/// Ctrl+F: the filter box.
pub fn focus_filter(app: &Shared) {
    let side = app.borrow().active;
    app.borrow_mut().sides[side].filter_seq += 1;
    push_view(app, side);
}

pub fn toggle_hidden(app: &Shared) {
    {
        let mut a = app.borrow_mut();
        let show = !a.show_hidden;
        a.show_hidden = show;
        for side in &mut a.sides {
            side.tree.set_show_hidden(show);
            for t in &mut side.tabs {
                t.tab.set_show_hidden(show);
            }
        }
    }
    for s in 0..2 {
        sync_files(app, s);
        sync_tree(app, s);
    }
}

// ---- sides ----

/// F9: shows or hides the second side.
pub fn toggle_dual(app: &Shared) {
    {
        let mut a = app.borrow_mut();
        a.dual = !a.dual;
        if !a.dual {
            a.active = 0;
        }
    }
    sync_window(app);
    for s in 0..2 {
        push_view(app, s);
    }
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.invoke_focus_list();
    }
}

/// Makes `side` the one commands act on (a press anywhere in it). The right side only in dual mode.
pub fn set_active_side(app: &Shared, side: usize) {
    {
        let mut a = app.borrow_mut();
        if a.active == side || (side == 1 && !a.dual) {
            return;
        }
        a.active = side;
    }
    for s in 0..2 {
        push_view(app, s);
    }
    sync_window(app);
}

/// Tab: the other side, in dual mode.
pub fn switch_side(app: &Shared) {
    let (dual, active) = (app.borrow().dual, app.borrow().active);
    if dual {
        set_active_side(app, 1 - active);
        if let Some(ui) = app.borrow().ui.upgrade() {
            ui.invoke_focus_list();
        }
    }
}

// ---- tabs ----

/// Opens a tab for `dir` at the end of `side`'s tabs (selected with `select`); returns its id.
/// Ctrl+T, the toolbar: a new tab on the active side, on the folder shown.
pub fn new_tab_here(app: &Shared) {
    let (side, dir) = {
        let a = app.borrow();
        (a.active, a.tab().dir().map(Path::to_path_buf))
    };
    new_tab(app, side, dir.unwrap_or_else(fileem_core::util::os::home), true);
}

pub fn new_tab(app: &Shared, side: usize, dir: PathBuf, select: bool) -> u64 {
    let id = {
        let mut a = app.borrow_mut();
        // new tabs sort like the one you're looking at, and open in its view
        let sort = a.sides[side].tab().sort();
        let mode = a.sides[side].tabs[a.sides[side].active].view.mode;
        let t = a.new_tab_state(sort, mode);
        let id = t.id;
        a.sides[side].tabs.push(t);
        if select {
            a.sides[side].active = a.sides[side].tabs.len() - 1;
        }
        id
    };
    sync_tabs(app, side);
    if select {
        sync_files(app, side);
    }
    go(app, id, dir, Nav::Go);
    id
}

/// Closes a tab; the last one of a side stays.
pub fn close_tab(app: &Shared, side: usize, index: usize) {
    {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        if s.tabs.len() <= 1 || index >= s.tabs.len() {
            return;
        }
        let closed = s.tabs.remove(index);
        if let Some(job) = closed.load {
            job.cancel();
        }
        if index < s.active || s.active >= s.tabs.len() {
            s.active = s.active.saturating_sub(1);
        }
    }
    tab_changed(app, side);
}

/// The tab at `from` was dragged into `gap` (0: before the first tab); the shown tab stays shown.
pub fn move_tab(app: &Shared, side: usize, from: usize, gap: usize) {
    {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        if from >= s.tabs.len() {
            return;
        }
        let Some(to) = crate::tab::reordered(from, gap.min(s.tabs.len())) else { return };
        let active = s.tabs[s.active].id;
        let tab = s.tabs.remove(from);
        s.tabs.insert(to, tab);
        s.active = s.tabs.iter().position(|t| t.id == active).unwrap_or(0);
    }
    sync_tabs(app, side);
}

pub fn close_other_tabs(app: &Shared, side: usize, keep: usize) {
    {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        if keep >= s.tabs.len() {
            return;
        }
        let kept = s.tabs.remove(keep);
        for t in s.tabs.drain(..) {
            if let Some(job) = t.load {
                job.cancel();
            }
        }
        s.tabs.push(kept);
        s.active = 0;
    }
    tab_changed(app, side);
}

pub fn select_tab(app: &Shared, side: usize, index: usize) {
    {
        let mut a = app.borrow_mut();
        if index >= a.sides[side].tabs.len() || a.sides[side].active == index {
            return;
        }
        a.sides[side].active = index;
    }
    tab_changed(app, side);
}

/// After the shown tab of `side` changed: its rows, the tab bar, and the tree follows.
fn tab_changed(app: &Shared, side: usize) {
    app.borrow_mut().sides[side].top_seq += 1;
    sync_tabs(app, side);
    sync_files(app, side);
    ensure_visible(app, side);
    let dir = app.borrow().sides[side].tab().dir().map(Path::to_path_buf);
    if let Some(d) = dir {
        reveal(app, side, d);
    }
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.invoke_focus_list();
    }
}

/// Opens `dir` in the other side's tab (turning dual mode on) and makes that side active.
pub fn open_in_other_side(app: &Shared, dir: PathBuf) {
    if !app.borrow().dual {
        toggle_dual(app);
    }
    let other = 1 - app.borrow().active;
    let id = {
        let a = app.borrow();
        a.sides[other].tabs[a.sides[other].active].id
    };
    set_active_side(app, other);
    go(app, id, dir, Nav::Go);
}

/// Tabs inside a renamed folder follow it to its new name.
pub fn tabs_follow_rename(app: &Shared, from: &Path, to: &Path) {
    let moves: Vec<(u64, PathBuf)> = {
        let a = app.borrow();
        a.sides
            .iter()
            .flat_map(|s| s.tabs.iter())
            .filter_map(|t| {
                let dir = t.tab.dir()?;
                let rest = dir.strip_prefix(from).ok()?;
                Some((t.id, to.join(rest)))
            })
            .collect()
    };
    for (id, dir) in moves {
        go(app, id, dir, Nav::Refresh);
    }
}

// ---- watching and refreshing ----

/// Watches `dir` for a tab (or a tree folder): its reports reach [`dir_changed`] with `token`.
/// Registering touches the file system (it can hang on a dead mount): call it in a background job.
fn watch_folder(dir: &Path, token: u64, ui: slint::Weak<crate::AppWindow>) -> watcher::WatchHandle {
    watcher::watch(dir, move || {
        // from the watcher's thread to the event loop; the window may be gone by then
        let _ = ui.upgrade_in_event_loop(move |ui| ui.invoke_dir_changed(token as i32));
    })
}

/// A watcher report: refresh the tab watching with `token`, coalesced (one follow-up at most
/// while a refresh runs).
pub fn dir_changed(app: &Shared, token: u64) {
    let (id, tree_dir) = {
        let mut a = app.borrow_mut();
        let id = a.sides.iter().flat_map(|s| s.tabs.iter()).find(|t| t.watch_token == token).map(|t| t.id);
        // a navigation still listing its folder refreshes once it is shown
        if let Some(t) = a.sides.iter_mut().flat_map(|s| s.tabs.iter_mut()).find(|t| t.next_watch_token == token) {
            t.next_changed = true;
        }
        let tree_dir = a.sides.iter().flat_map(|s| s.tree_watches.iter()).find(|(_, w)| w.token == token);
        (id, tree_dir.map(|(dir, _)| dir.clone()))
    };
    if let Some(id) = id {
        request_refresh(app, id);
    }
    if let Some(dir) = tree_dir {
        reload_tree_node(app, &dir);
    }
}

fn request_refresh(app: &Shared, id: u64) {
    {
        let mut a = app.borrow_mut();
        let Some(t) = a.tab_state(id) else { return };
        if t.refreshing {
            t.refresh_pending = true;
            return;
        }
    }
    refresh_tab(app, id);
}

/// After a refresh: a request that came in meanwhile runs after twice the listing time, so a big
/// folder under heavy writes isn't listed continuously.
fn finish_refresh(app: &Shared, id: u64) {
    let mut a = app.borrow_mut();
    let Some(t) = a.tab_state(id) else { return };
    t.refreshing = false;
    if t.refresh_pending {
        t.refresh_pending = false;
        let delay = (t.last_list * 2).max(Duration::from_millis(250));
        let again = app.clone();
        slint::Timer::single_shot(delay, move || request_refresh(&again, id));
    }
}

/// Refreshes every tab and tree item that shows one of `dirs` (after our own operations: the
/// watcher is slow on some systems).
pub fn refresh_dirs(app: &Shared, dirs: &[PathBuf]) {
    let ids: Vec<u64> = {
        let a = app.borrow();
        a.sides
            .iter()
            .flat_map(|s| s.tabs.iter())
            .filter(|t| t.tab.dir().is_some_and(|d| dirs.iter().any(|x| x == d)))
            .map(|t| t.id)
            .collect()
    };
    for id in ids {
        request_refresh(app, id);
    }
    for d in dirs {
        reload_tree_node(app, d);
    }
}

// ---- tree ----

/// Re-reads the subfolders of every loaded tree node showing `dir`, in both trees. Only the
/// latest reload of a folder is shown: an older one finishing later read it before the change.
pub fn reload_tree_node(app: &Shared, dir: &Path) {
    for side in 0..2 {
        let (ids, generation) = {
            let mut a = app.borrow_mut();
            let ids = a.sides[side].tree.loaded_nodes(dir);
            if ids.is_empty() {
                continue;
            }
            let generation = a.next_id();
            a.sides[side].tree_reloads.insert(dir.to_path_buf(), generation);
            (ids, generation)
        };
        let done_app = app.clone();
        let path = dir.to_path_buf();
        bg::run(
            {
                let path = path.clone();
                move |_| listing::subfolders(&path, true)
            },
            move |children: std::io::Result<Vec<FileEntry>>| {
                {
                    let mut a = done_app.borrow_mut();
                    let reloads = &mut a.sides[side].tree_reloads;
                    if reloads.get(&path) != Some(&generation) {
                        return;
                    }
                    reloads.remove(&path);
                }
                if let Ok(children) = children {
                    for id in ids {
                        done_app.borrow_mut().sides[side].tree.set_children(id, children.clone());
                    }
                    sync_tree(&done_app, side);
                }
            },
        );
    }
}

/// A watch on an open tree folder; `handle` arrives from a background job.
pub struct TreeWatch {
    pub token: u64,
    handle: Option<watcher::WatchHandle>,
}

/// Watches the tree's open folders (and stops watching the ones closed), so subfolders other
/// programs create or delete show up. Not on a drive being ejected: a watch can keep it busy.
pub fn watch_tree_folders(app: &Shared, side: usize) {
    let (start, ui) = {
        let mut a = app.borrow_mut();
        let ejecting: Vec<PathBuf> = a.ejecting.iter().cloned().collect();
        let mut wanted = a.sides[side].tree.open_folders();
        wanted.retain(|p| !ejecting.iter().any(|r| p.starts_with(r)));
        let watches = &mut a.sides[side].tree_watches;
        if watches.len() == wanted.len() && wanted.iter().all(|p| watches.contains_key(p)) {
            return;
        }
        watches.retain(|p, _| wanted.contains(p));
        let new: Vec<PathBuf> = wanted.into_iter().filter(|p| !watches.contains_key(p)).collect();
        let start: Vec<(PathBuf, u64)> = new.into_iter().map(|p| (p, a.next_id())).collect();
        for (p, token) in &start {
            a.sides[side].tree_watches.insert(p.clone(), TreeWatch { token: *token, handle: None });
        }
        (start, a.ui.clone())
    };
    for (dir, token) in start {
        let done_app = app.clone();
        let ui = ui.clone();
        bg::run(
            {
                let dir = dir.clone();
                move |_| watch_folder(&dir, token, ui)
            },
            move |handle| {
                let mine = {
                    let mut a = done_app.borrow_mut();
                    match a.sides[side].tree_watches.get_mut(&dir) {
                        Some(w) if w.token == token => {
                            w.handle = Some(handle);
                            true
                        }
                        _ => false, // closed (or reopened) meanwhile
                    }
                };
                if mine {
                    reload_tree_node(&done_app, &dir); // what changed while it was being registered
                }
            },
        );
    }
}

/// Reveals `dir` in `side`'s tree, loading folders along the way in the background.
pub fn reveal(app: &Shared, side: usize, dir: PathBuf) {
    let token = {
        let mut a = app.borrow_mut();
        a.sides[side].reveal_token += 1;
        a.sides[side].reveal_token
    };
    reveal_step(app, side, dir, token);
}

fn reveal_step(app: &Shared, side: usize, dir: PathBuf, token: u64) {
    let step = app.borrow_mut().sides[side].tree.reveal(&dir);
    match step {
        Reveal::Load(id, path) => {
            let done_app = app.clone();
            bg::run(
                move |_| listing::subfolders(&path, true),
                move |children: std::io::Result<Vec<FileEntry>>| {
                    if done_app.borrow().sides[side].reveal_token != token {
                        return;
                    }
                    // an unreadable folder shows no children rather than asking again forever
                    done_app.borrow_mut().sides[side].tree.set_children(id, children.unwrap_or_default());
                    reveal_step(&done_app, side, dir, token);
                },
            );
        }
        Reveal::Done(row) => {
            sync_tree(app, side);
            {
                let mut a = app.borrow_mut();
                a.sides[side].tree_row = row as i32;
                a.sides[side].tree_seq += 1;
            }
            push_view(app, side);
        }
        Reveal::NotFound => sync_tree(app, side),
    }
}

pub fn tree_clicked(app: &Shared, side: usize, row: usize) {
    set_active_side(app, side);
    let path = {
        let a = app.borrow();
        let t = &a.sides[side].tree;
        t.rows().get(row).and_then(|&id| t.node(id).path.clone())
    };
    if let Some(p) = path {
        let id = app.borrow().active_tab_id();
        go(app, id, p, Nav::Go);
    }
}

pub fn tree_toggled(app: &Shared, side: usize, row: usize) {
    let load = app.borrow_mut().sides[side].tree.toggle(row);
    sync_tree(app, side);
    if let Some((id, path)) = load {
        let done_app = app.clone();
        bg::run(
            move |_| listing::subfolders(&path, true),
            move |children: std::io::Result<Vec<FileEntry>>| {
                done_app.borrow_mut().sides[side].tree.set_children(id, children.unwrap_or_default());
                sync_tree(&done_app, side);
            },
        );
    }
}

/// Moves the focus in the active list (arrow keys and friends).
pub fn move_focus(app: &Shared, delta: isize, ctrl: bool, shift: bool) {
    app.borrow_mut().tab_mut().move_focus(delta, ctrl, shift);
    let side = app.borrow().active;
    sync_selection(app, side, true);
}
