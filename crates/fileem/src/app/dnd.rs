//! Mouse selection and drag & drop inside the window (the Java `Marquee`, `Dnd` and
//! `MainWindow.drop`). Slint tracks the gesture; the rules and the selection live here.
//!
//! Drags to and from other applications are not possible yet: Slint 1.18's winit backend only
//! drags inside the window. The clipboard (Ctrl+C / Ctrl+V) works with other file managers.
//! The dragged item's picture follows the mouse (`show_drag_image`).

use std::path::PathBuf;
use std::time::Duration;

use fileem_core::ops::Kind;
use fileem_core::util::fs_util;
use slint::ComponentHandle;

use super::commands::{self, describe, Dialog, DialogSpec};
use super::{nav, sync_selection, views, Shared};
use crate::tab;
use crate::tree::NodeKind;

/// Where a drag was picked up or is over: the file list (or grid) or the folder tree.
const LIST: i32 = 0;
/// The Details list's row height (side.slint's `row-height`).
const ROW_HEIGHT: f32 = 24.0;

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

/// The box now reaches from `anchor` to `mouse` (content coordinates: 0,0 is the top left of
/// the first row or item). In a grid it selects the items it touches, in Details the rows.
pub fn box_moved(app: &Shared, side: usize, anchor: (f32, f32), mouse: (f32, f32)) {
    {
        let mut a = app.borrow_mut();
        a.late_press = None;
        let grid = views::grid_of(&a.sides[side]);
        let t = a.sides[side].tab_mut();
        match grid {
            Some(g) => t.box_select_rows(&g.box_hits(anchor, mouse), g.index_at(mouse.0, mouse.1)),
            None => {
                let rows = tab::box_rows(anchor.1, mouse.1, ROW_HEIGHT, t.len());
                let focus = (mouse.1.max(0.0) / ROW_HEIGHT).floor() as usize;
                t.box_select(rows, focus);
            }
        }
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

/// The dragged item's picture under the mouse: the part of the window it sits in (`rect`, window
/// coordinates), faded a little, like the Java app's snapshot of the row. Needs a renderer that
/// can take snapshots (both of ours can); without one there is no picture.
pub fn show_drag_image(app: &Shared, rect: (f32, f32, f32, f32)) {
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    let window = ui.window();
    let image = window.take_snapshot().ok().and_then(|shot| {
        let scale = window.scale_factor();
        let px = |v: f32| (v * scale).round().max(0.0) as u32;
        let (rgba, w, h) = drag_picture(shot.as_bytes(), shot.width(), shot.height(), (px(rect.0), px(rect.1), px(rect.2), px(rect.3)))?;
        // drawn at one image pixel per logical pixel
        let (lw, lh) = (((w as f32 / scale).round() as u32).max(1), ((h as f32 / scale).round() as u32).max(1));
        let rgba = if (lw, lh) == (w, h) { rgba } else { fileem_core::images::shrink(&rgba, w, h, lw, lh) };
        Some(slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&rgba, lw, lh)))
    });
    ui.set_drag_image(image.unwrap_or_default());
}

/// `rect` (x, y, width, height) cut out of RGBA pixels `width × height` wide and made 75 %
/// opaque; `None` when it lies outside.
pub fn drag_picture(rgba: &[u8], width: u32, height: u32, rect: (u32, u32, u32, u32)) -> Option<(Vec<u8>, u32, u32)> {
    let (x0, y0) = (rect.0.min(width), rect.1.min(height));
    let (x1, y1) = (rect.0.saturating_add(rect.2).min(width), rect.1.saturating_add(rect.3).min(height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for y in y0..y1 {
        let row = &rgba[((y * width + x0) * 4) as usize..((y * width + x1) * 4) as usize];
        for px in row.as_chunks::<4>().0 {
            out.extend([px[0], px[1], px[2], (u16::from(px[3]) * 3 / 4) as u8]);
        }
    }
    Some((out, x1 - x0, y1 - y0))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drag_picture_is_the_rectangle_cut_out_and_faded() {
        // 3 × 2 pixels, each with its index as red
        let rgba: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0, 255]).collect();
        let (px, w, h) = drag_picture(&rgba, 3, 2, (1, 0, 2, 2)).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(px.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), [1, 2, 4, 5]);
        assert!(px.chunks(4).all(|p| p[3] == 191), "75 % opaque");
        let (_, w, h) = drag_picture(&rgba, 3, 2, (2, 1, 10, 10)).unwrap();
        assert_eq!((w, h), (1, 1), "clipped to the window");
        assert_eq!(drag_picture(&rgba, 3, 2, (5, 0, 2, 2)), None, "outside");
        assert_eq!(drag_picture(&rgba, 3, 2, (0, 0, 0, 2)), None, "empty");
    }
}
