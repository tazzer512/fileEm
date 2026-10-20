//! View modes per tab (the Java `FileTab.setViewMode` and the size controls), the grid's
//! geometry, its keys, and the thumbnails it shows. Slint reports the grid's size and the slots
//! in view; the layout itself is `grid.rs`.

use fileem_core::fs::FileEntry;
use slint::Model;

use super::{push_view, sync_files, sync_selection, Shared, Side};
use crate::grid::{self, Grid, Move};
use crate::thumbs::{self, Key};
use crate::view_mode::{ViewMode, THUMBNAIL_MIN};

/// Ctrl+wheel turns this much (logical pixels) before the view changes.
const WHEEL_STEP: f32 = 30.0;

/// The grid of a side's shown tab, or `None` in Details.
pub(super) fn grid_of(s: &Side) -> Option<Grid> {
    let t = &s.tabs[s.active];
    let mode = t.view.mode;
    if mode == ViewMode::Details {
        return None;
    }
    let text = if s.measured_width > 0.0 { s.measured_width } else { s.rough_width };
    let list_width = grid::list_width(text);
    Some(Grid::layout(mode, t.view.icon_px(), s.grid_size.0, s.grid_size.1, t.tab.len(), list_width))
}

/// The thumbnail key of `e` in a view with icons of `px`, when it shows one.
pub(super) fn thumb_key(mode: ViewMode, px: u32, e: &FileEntry) -> Option<Key> {
    (mode != ViewMode::Details && px >= THUMBNAIL_MIN && thumbs::supports(e)).then(|| Key::of(e, px))
}

/// Switches `side`'s shown tab to `mode`; selection, focus and sort carry over.
pub fn set_view(app: &Shared, side: usize, mode: ViewMode) {
    {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        let t = &mut s.tabs[s.active];
        if t.view.mode == mode {
            return;
        }
        t.view.mode = mode;
        s.wheel = 0.0;
        s.top_seq += 1;
    }
    sync_files(app, side);
    sync_selection(app, side, true);
}

/// One size step bigger (+1) or smaller (-1) in the active tab's view (icon views and Tiles).
pub fn resize(app: &Shared, side: usize, direction: i32) {
    change_size(app, side, |v| {
        v.resize(direction);
    });
}

/// The size slider moved (-1 … 1, the view's own size in the middle).
pub fn slider_moved(app: &Shared, side: usize, position: f32) {
    change_size(app, side, |v| v.set_slider(position as f64));
}

/// The slider was let go: a second time within a double-click's time resets the view's size.
pub fn slider_released(app: &Shared, side: usize) {
    let now = std::time::Instant::now();
    let double = {
        let mut a = app.borrow_mut();
        let last = &mut a.sides[side].slider_released;
        let double = last.is_some_and(|t| now.duration_since(t) < DOUBLE_CLICK);
        *last = (!double).then_some(now);
        double
    };
    if double {
        change_size(app, side, |v| v.set_steps(0.0));
    }
}

/// Slint's own double-click interval.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(500);

fn change_size(app: &Shared, side: usize, change: impl FnOnce(&mut crate::view_mode::ViewState)) {
    let (before, after) = {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        let v = &mut s.tabs[s.active].view;
        let before = v.icon_px();
        change(v);
        (before, v.icon_px())
    };
    if before == after {
        push_view(app, side); // the slider snaps back where it belongs
        return;
    }
    // thumbnails load at fixed sizes: rows only need new ones when that size changes
    let load = |px: u32| (px >= THUMBNAIL_MIN).then(|| thumbs::load_size(px));
    if load(before) != load(after) {
        sync_files(app, side);
    } else {
        push_view(app, side);
    }
    super::ensure_visible(app, side);
}

/// Ctrl+wheel: up (positive) toward bigger icons, down toward Content, as in Explorer.
pub fn wheel(app: &Shared, side: usize, delta: f32) {
    let mode = {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        s.wheel += delta;
        if s.wheel.abs() < WHEEL_STEP {
            return;
        }
        let up = s.wheel > 0.0;
        s.wheel = 0.0;
        s.tabs[s.active].view.mode.step(if up { -1 } else { 1 })
    };
    set_view(app, side, mode);
}

/// Slint measured the widest of `side`'s [`grid::names_to_measure`]: List's columns fit it.
pub fn list_measured(app: &Shared, side: usize, width: f32) {
    let list = {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        if (s.measured_width - width).abs() < 0.5 {
            return;
        }
        s.measured_width = width;
        s.tabs[s.active].view.mode == ViewMode::List
    };
    if list {
        push_view(app, side);
    }
}

/// Slint laid out the grid of `side` at this size.
pub fn grid_resized(app: &Shared, side: usize, width: f32, height: f32) {
    {
        let mut a = app.borrow_mut();
        let s = &mut a.sides[side];
        if s.grid_size == (width, height) {
            return;
        }
        s.grid_size = (width, height);
    }
    push_view(app, side);
}

/// The grid of `side` shows the slots `first .. first + count` now.
pub fn grid_range(app: &Shared, side: usize, first: usize, count: usize) {
    app.borrow_mut().sides[side].grid_range = (first, count);
    fill_thumbs(app, side);
    request_thumbs(app);
}

/// Arrow and page keys in a grid view; false in Details (the list handles them itself).
pub fn move_key(app: &Shared, m: Move, ctrl: bool, shift: bool) -> bool {
    let delta = {
        let a = app.borrow();
        match grid_of(a.side()) {
            Some(g) => g.delta(m),
            None => return false,
        }
    };
    super::nav::move_focus(app, delta, ctrl, shift);
    true
}

/// Asks for the thumbnails of the slots in view, in both shown sides.
pub(super) fn request_thumbs(app: &Shared) {
    let mut a = app.borrow_mut();
    let mut keys = Vec::new();
    for (i, s) in a.sides.iter().enumerate() {
        if i == 1 && !a.dual {
            continue;
        }
        let t = &s.tabs[s.active];
        let (mode, px) = (t.view.mode, t.view.icon_px());
        let (first, count) = s.grid_range;
        for r in first..(first + count).min(t.tab.len()) {
            keys.extend(thumb_key(mode, px, t.tab.row(r)));
        }
    }
    a.thumbs.want(keys);
}

/// Puts cached thumbnails into the rows in view that don't show theirs yet.
fn fill_thumbs(app: &Shared, side: usize) {
    let mut a = app.borrow_mut();
    let a = &mut *a;
    let s = &a.sides[side];
    let t = &s.tabs[s.active];
    let (mode, px) = (t.view.mode, t.view.icon_px());
    let (first, count) = s.grid_range;
    for r in first..(first + count).min(t.tab.len()).min(s.files.row_count()) {
        let Some(key) = thumb_key(mode, px, t.tab.row(r)) else { continue };
        let Some(mut row) = s.files.row_data(r) else { continue };
        if row.thumb.size().width > 0 {
            continue;
        }
        if let Some(img) = a.thumbs.get(&key) {
            row.thumb = img;
            s.files.set_row_data(r, row);
        }
    }
}

/// Decoded thumbnails arrived (the decoder threads' `thumbs-ready`).
pub fn thumbs_ready(app: &Shared) {
    let ready = app.borrow_mut().thumbs.take_done();
    if !ready.is_empty() {
        for side in 0..2 {
            fill_thumbs(app, side);
        }
    }
}
