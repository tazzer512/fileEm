//! Mouse selection and drag & drop inside the window (the Java `Marquee`, `Dnd` and
//! `MainWindow.drop`). Slint tracks the gesture; the rules and the selection live here.
//!
//! Drags to and from other applications are not possible yet: Slint 1.18's winit backend only
//! drags inside the window. The clipboard (Ctrl+C / Ctrl+V) works with other file managers.

use std::path::PathBuf;
use std::time::Duration;

use fileem_core::ops::Kind;
use fileem_core::util::fs_util;

use super::commands::{self, describe, Dialog, DialogSpec};
use super::{nav, sync_selection, Shared};
use crate::tab;
use crate::tree::NodeKind;

/// Where a drag was picked up or is over: the file list or the folder tree.
const LIST: i32 = 0;

/// What is being dragged, and the last drop target asked about (asked on every mouse move).
#[derive(Default)]
pub struct Drag {
    sources: Vec<PathBuf>,
    last_check: Option<((usize, i32, i32), i32)>,
}

/// A left press on a row (Shift/Ctrl as for a click). A press on a row of the selection keeps
/// the selection until the release, so it can be dragged.
pub fn row_pressed(app: &Shared, side: usize, row: usize, ctrl: bool, shift: bool) {
    nav::set_active_side(app, side);
    let late = app.borrow_mut().tab_mut().press(row, ctrl, shift);
    app.borrow_mut().late_press = late.then_some((side, row));
    sync_selection(app, side, false);
}

/// The release of a press that didn't drag: a kept selection becomes the row alone.
pub fn row_released(app: &Shared, side: usize, row: usize) {
    if app.borrow_mut().late_press.take() != Some((side, row)) {
        return;
    }
    app.borrow_mut().sides[side].tab_mut().click(row, false, false);
    sync_selection(app, side, false);
}

pub fn box_start(app: &Shared, side: usize, ctrl: bool, shift: bool) {
    nav::set_active_side(app, side);
    app.borrow_mut().sides[side].tab_mut().start_box(ctrl, shift);
}

/// The box now reaches from `anchor` to `mouse` (content offsets, 0 = top of the first row).
pub fn box_moved(app: &Shared, side: usize, anchor: f32, mouse: f32, row_height: f32) {
    {
        let mut a = app.borrow_mut();
        a.late_press = None;
        let t = a.sides[side].tab_mut();
        let rows = tab::box_rows(anchor, mouse, row_height, t.len());
        let focus = (mouse.max(0.0) / row_height.max(1.0)).floor() as usize;
        t.box_select(rows, focus);
    }
    sync_selection(app, side, false);
}

pub fn box_ended(app: &Shared, side: usize, drawn: bool) {
    let mut a = app.borrow_mut();
    a.sides[side].tab_mut().end_box();
    if drawn {
        a.late_press = None;
    }
}

/// A drag starts: from the list it carries the side's selection, from the tree that folder.
pub fn drag_started(app: &Shared, side: usize, area: i32, row: usize) {
    let mut a = app.borrow_mut();
    a.late_press = None;
    let s = &a.sides[side];
    let sources = if area == LIST {
        s.tab().selected_paths()
    } else {
        s.tree
            .rows()
            .get(row)
            .map(|&id| s.tree.node(id))
            .filter(|n| matches!(n.kind, NodeKind::Folder | NodeKind::Quick))
            .and_then(|n| n.path.clone())
            .into_iter()
            .collect()
    };
    a.drag = Drag { sources, last_check: None };
}

/// The folder a drop at `row` of `area` would go into, and the row to mark for it (-1: none).
fn target(app: &Shared, side: usize, area: i32, row: i32) -> Option<(PathBuf, i32)> {
    let a = app.borrow();
    let s = &a.sides[side];
    let sources = &a.drag.sources;
    if area == LIST {
        let t = s.tab();
        let on_row = usize::try_from(row)
            .ok()
            .filter(|&r| r < t.len() && t.row(r).is_dir())
            .map(|r| t.row(r).path().to_path_buf())
            .filter(|p| fs_util::can_drop(sources, p));
        match on_row {
            Some(p) => Some((p, row)),
            None => t.dir().filter(|d| fs_util::can_drop(sources, d)).map(|d| (d.to_path_buf(), -1)),
        }
    } else {
        let id = *s.tree.rows().get(usize::try_from(row).ok()?)?;
        let path = s.tree.node(id).path.clone()?;
        fs_util::can_drop(sources, &path).then_some((path, row))
    }
}

/// While dragging over `row` of `area`: the row to mark, -1 to accept without one, -2 to refuse.
pub fn drop_check(app: &Shared, side: usize, area: i32, row: i32) -> i32 {
    let key = (side, area, row);
    if let Some((k, answer)) = app.borrow().drag.last_check {
        if k == key {
            return answer;
        }
    }
    let answer = target(app, side, area, row).map_or(-2, |(_, mark)| mark);
    app.borrow_mut().drag.last_check = Some((key, answer));
    answer
}

/// A drop: asks Move (the default) / Copy / Cancel once the drag has ended, never inside the
/// drop event itself.
pub fn dropped(app: &Shared, side: usize, area: i32, row: i32) {
    let found = target(app, side, area, row);
    let sources = std::mem::take(&mut app.borrow_mut().drag).sources;
    let Some((target, _)) = found else { return };
    if sources.is_empty() {
        return;
    }
    let app = app.clone();
    slint::Timer::single_shot(Duration::ZERO, move || ask_move_or_copy(&app, sources, target));
}

fn ask_move_or_copy(app: &Shared, sources: Vec<PathBuf>, target: PathBuf) {
    let name = fs_util::display_name(&target);
    let question = format!(
        "Move {} to \u{201c}{name}\u{201d}?\n\nTo: {}\n\nChoose Copy to keep the originals where they are.",
        describe(&sources),
        target.display()
    );
    let mut spec = DialogSpec::confirm("Move or copy", &question, "Move");
    spec.alt = "Copy".into();
    commands::open_dialog(app, Dialog::Drop { sources, target }, spec);
}

/// The dialog's Copy button.
pub fn copy_instead(app: &Shared) {
    let dialog = std::mem::replace(&mut app.borrow_mut().dialog, Dialog::None);
    commands::close_dialog(app);
    if let Dialog::Drop { sources, target } = dialog {
        commands::run_operation(app, Kind::Copy, sources, Some(target));
    }
}
