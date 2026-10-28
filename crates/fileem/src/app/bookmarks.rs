//! Bookmarks (the Java `Bookmarks` users in `MainWindow`, `FolderTree` and `BookmarksDialog`):
//! one list per window, shown in both trees' Bookmarks section, toggled with Ctrl+D or the
//! toolbar, changed from the menus and the Manage bookmarks window (Ctrl+Shift+B). Every change
//! is saved at once; renamed folders take their bookmarks along.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use fileem_core::bookmarks::Bookmarks;
use fileem_core::fs::places::Place;
use fileem_core::session::Session;

use super::commands::{self, Dialog, DialogHost, DialogSpec};
use super::{nav, sync_tree, sync_window, Shared};
use crate::bg;
use crate::tab::Nav;
use crate::{BookmarkRow, BookmarksWindow};

/// The open Manage bookmarks window.
pub struct Manage {
    pub(super) window: BookmarksWindow,
    rows: Rc<VecModel<BookmarkRow>>,
    selected: Option<PathBuf>,
    /// Bookmarked folders that aren't there (checked in the background).
    missing: HashSet<PathBuf>,
    /// Drops existence checks of an older list.
    check: u64,
    /// The bookmark being renamed in the window's own dialog.
    renaming: Option<PathBuf>,
    ensure_seq: i32,
}

/// Changes the list (saved at once) and shows the change everywhere.
fn change(app: &Shared, f: impl FnOnce(&mut Bookmarks, &Session)) {
    {
        let mut a = app.borrow_mut();
        let a = &mut *a;
        f(&mut a.bookmarks, &a.session);
    }
    sync(app);
}

/// Mirrors the list in both trees, the toolbar button and the Manage window.
pub fn sync(app: &Shared) {
    let places: Vec<Place> =
        app.borrow().bookmarks.items().iter().map(|b| Place { name: b.name.clone(), path: b.path.clone() }).collect();
    for side in 0..2 {
        app.borrow_mut().sides[side].tree.set_bookmarks(&places);
        sync_tree(app, side);
    }
    sync_window(app);
    if app.borrow().manage.is_some() {
        sync_rows(app);
        check_missing(app);
    }
}

pub fn add(app: &Shared, dir: PathBuf) {
    change(app, |b, s| {
        b.add(&dir, None, s);
    });
}

pub fn remove(app: &Shared, dir: &Path) {
    change(app, |b, s| b.remove(dir, s));
}

pub fn move_by(app: &Shared, dir: &Path, delta: isize) {
    change(app, |b, s| b.move_by(dir, delta, s));
}

/// Ctrl+D, the toolbar: bookmarks the active tab's folder, or removes its bookmark.
pub fn toggle(app: &Shared) {
    let Some(dir) = app.borrow().tab().dir().map(Path::to_path_buf) else { return };
    if app.borrow().bookmarks.contains(&dir) {
        remove(app, &dir);
    } else {
        add(app, dir);
    }
}

/// After a folder was renamed or moved: its bookmarks (and those inside it) follow.
pub fn relocate(app: &Shared, from: &Path, to: &Path) {
    let affected = app.borrow().bookmarks.items().iter().any(|b| b.path.starts_with(from));
    if affected {
        change(app, |b, s| b.relocate(from, to, s));
    }
}

fn rename_spec(app: &Shared, dir: &Path) -> Option<DialogSpec> {
    let name = app.borrow().bookmarks.find(dir)?.name.clone();
    Some(DialogSpec::ask("Rename bookmark", "Name:", &name, "Rename").select_all())
}

/// "Rename bookmark…" in the tree: asks over the main window.
pub fn ask_rename(app: &Shared, dir: PathBuf) {
    if let Some(spec) = rename_spec(app, &dir) {
        commands::open_dialog(app, Dialog::RenameBookmark { path: dir }, spec);
    }
}

/// The new name (blank: the folder's name).
pub fn rename(app: &Shared, dir: &Path, name: &str) {
    change(app, |b, s| b.rename(dir, name, s));
}

/// Opens the folder in the active tab.
fn open_folder(app: &Shared, dir: PathBuf) {
    let id = app.borrow().active_tab_id();
    nav::go(app, id, dir, Nav::Go);
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.invoke_focus_list();
    }
}

// ---- the Manage bookmarks window ----

/// Ctrl+Shift+B, "Manage bookmarks…": the window listing every bookmark (one per main window;
/// asking again brings it up).
pub fn manage(app: &Shared) -> Option<BookmarksWindow> {
    if let Some(w) = window(app) {
        let _ = w.show();
        return Some(w);
    }
    let w = BookmarksWindow::new().ok()?;
    super::theme::style(app, &w);
    let rows = Rc::new(VecModel::default());
    w.set_rows(ModelRc::from(rows.clone()));
    let selected = app.borrow().bookmarks.items().first().map(|b| b.path.clone());
    app.borrow_mut().manage = Some(Manage {
        window: w.clone_strong(),
        rows,
        selected,
        missing: HashSet::new(),
        check: 0,
        renaming: None,
        ensure_seq: 0,
    });
    wire(app, &w);
    sync_rows(app);
    check_missing(app);
    let _ = w.show();
    w.invoke_focus_list();
    Some(w)
}

/// The open Manage bookmarks window (tests).
pub fn window(app: &Shared) -> Option<BookmarksWindow> {
    app.borrow().manage.as_ref().map(|m| m.window.clone_strong())
}

fn wire(app: &Shared, w: &BookmarksWindow) {
    macro_rules! on {
        ($setter:ident, $app:ident => |$($arg:ident),*| $body:expr) => {{
            let a = app.clone();
            w.$setter(move |$($arg),*| {
                let $app = &a;
                $body
            });
        }};
    }
    on!(on_row_pressed, app => |row| select_row(app, row));
    on!(on_select_by, app => |delta| {
        let (index, n) = (selected_index(app), app.borrow().bookmarks.items().len() as i32);
        if n > 0 {
            select_row(app, index.map_or(0, |i| (i as i32 + delta).clamp(0, n - 1)));
        }
    });
    on!(on_open, app => | | {
        if let Some(dir) = selected(app) {
            close(app);
            open_folder(app, dir);
        }
    });
    on!(on_rename, app => | | {
        let Some(dir) = selected(app) else { return };
        let Some(spec) = rename_spec(app, &dir) else { return };
        let w = {
            let mut a = app.borrow_mut();
            let Some(m) = a.manage.as_mut() else { return };
            m.renaming = Some(dir);
            m.window.clone_strong()
        };
        w.show_dialog(spec);
    });
    on!(on_remove, app => | | {
        let Some(dir) = selected(app) else { return };
        let index = selected_index(app).unwrap_or(0);
        remove(app, &dir);
        // the next one (or the new last one) is selected
        let next = app.borrow().bookmarks.items().get(index).or(app.borrow().bookmarks.items().last()).map(|b| b.path.clone());
        if let Some(m) = app.borrow_mut().manage.as_mut() {
            m.selected = next;
        }
        sync_rows(app);
    });
    on!(on_move_by, app => |delta| {
        if let Some(dir) = selected(app) {
            move_by(app, &dir, delta as isize);
            ensure_selected_visible(app);
        }
    });
    on!(on_close_clicked, app => | | close(app));
    on!(on_dialog_accepted, app => |text| {
        let dir = app.borrow_mut().manage.as_mut().and_then(|m| m.renaming.take());
        if let Some(d) = dir {
            rename(app, &d, &text);
        }
        if let Some(w) = window(app) {
            w.hide_dialog();
        }
    });
    on!(on_dialog_cancelled, app => | | {
        if let Some(m) = app.borrow_mut().manage.as_mut() {
            m.renaming = None;
        }
        if let Some(w) = window(app) {
            w.hide_dialog();
        }
    });
    w.on_dialog_alternative(|| {});
    let a = app.clone();
    w.window().on_close_requested(move || {
        close(&a);
        slint::CloseRequestResponse::HideWindow
    });
}

pub fn close(app: &Shared) {
    let m = app.borrow_mut().manage.take();
    if let Some(m) = m {
        let _ = m.window.hide();
    }
}

fn selected(app: &Shared) -> Option<PathBuf> {
    app.borrow().manage.as_ref().and_then(|m| m.selected.clone())
}

fn selected_index(app: &Shared) -> Option<usize> {
    let a = app.borrow();
    a.manage.as_ref().and_then(|m| m.selected.as_deref()).and_then(|p| a.bookmarks.index_of(p))
}

fn select_row(app: &Shared, row: i32) {
    let path = usize::try_from(row).ok().and_then(|r| app.borrow().bookmarks.items().get(r).map(|b| b.path.clone()));
    if let Some(m) = app.borrow_mut().manage.as_mut() {
        m.selected = path;
    }
    sync_rows(app);
    ensure_selected_visible(app);
}

fn ensure_selected_visible(app: &Shared) {
    let index = selected_index(app);
    let mut a = app.borrow_mut();
    let Some(m) = a.manage.as_mut() else { return };
    if let Some(i) = index {
        m.ensure_seq += 1;
        m.window.set_ensure_row(i as i32);
        m.window.set_ensure_seq(m.ensure_seq);
    }
}

/// The window's rows from the list (names, paths, which is missing and selected).
fn sync_rows(app: &Shared) {
    let a = app.borrow();
    let Some(m) = a.manage.as_ref() else { return };
    let rows: Vec<BookmarkRow> = a
        .bookmarks
        .items()
        .iter()
        .map(|b| BookmarkRow {
            name: b.name.as_str().into(),
            path: b.path.display().to_string().into(),
            missing: m.missing.contains(&b.path),
            selected: m.selected.as_ref() == Some(&b.path),
        })
        .collect();
    let selected = rows.iter().position(|r| r.selected).map_or(-1, |i| i as i32);
    super::update_rows(&m.rows, rows); // a press in progress stays
    m.window.set_selected(selected);
}

/// Which bookmarked folders exist is checked in the background: a bookmark on a dead mount would
/// hang the window otherwise.
fn check_missing(app: &Shared) {
    let (token, paths) = {
        let mut a = app.borrow_mut();
        let paths: Vec<PathBuf> = a.bookmarks.items().iter().map(|b| b.path.clone()).collect();
        let Some(m) = a.manage.as_mut() else { return };
        m.check += 1;
        (m.check, paths)
    };
    let done_app = app.clone();
    bg::run(
        move |_| paths.into_iter().filter(|p| !p.is_dir()).collect::<HashSet<PathBuf>>(),
        move |gone: HashSet<PathBuf>| {
            let changed = {
                let mut a = done_app.borrow_mut();
                match a.manage.as_mut() {
                    Some(m) if m.check == token && m.missing != gone => {
                        m.missing = gone;
                        true
                    }
                    _ => false,
                }
            };
            if changed {
                sync_rows(&done_app);
            }
        },
    );
}
