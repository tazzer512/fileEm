//! The single keyboard handler (the Java `MainWindow.onKey`). Text fields (address, filter,
//! dialogs) take their own keys first; what they don't use arrives here.

use std::path::Path;
use std::time::{Duration, Instant};

use slint::platform::Key;

use super::{commands, nav, sync_selection, Shared};

/// Returns true when the key was handled.
pub fn key(app: &Shared, text: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
    let Some(c) = text.chars().next() else { return false };
    let is = |k: Key| c == char::from(k);
    let letter = |l: char| ctrl && !alt && c.eq_ignore_ascii_case(&l);
    let page = 20;
    if letter('t') {
        let (side, dir) = {
            let a = app.borrow();
            (a.active, a.tab().dir().map(Path::to_path_buf))
        };
        nav::new_tab(app, side, dir.unwrap_or_else(fileem_core::util::os::home), true);
    } else if letter('w') {
        let (side, index) = (app.borrow().active, app.borrow().side().active);
        nav::close_tab(app, side, index);
    } else if letter('l') {
        nav::edit_address(app);
    } else if letter('f') && !shift {
        nav::focus_filter(app);
    } else if is(Key::F9) {
        nav::toggle_dual(app);
    } else if is(Key::Tab) && !ctrl && !alt && !shift {
        if !app.borrow().dual {
            return false;
        }
        nav::switch_side(app);
    } else if (alt && is(Key::LeftArrow)) || (ctrl && c == '[') {
        nav::back(app);
    } else if (alt && is(Key::RightArrow)) || (ctrl && c == ']') {
        nav::forward(app);
    } else if (alt && is(Key::UpArrow)) || (!ctrl && !alt && is(Key::Backspace)) {
        nav::up(app);
    } else if is(Key::UpArrow) {
        nav::move_focus(app, -1, ctrl, shift);
    } else if is(Key::DownArrow) {
        nav::move_focus(app, 1, ctrl, shift);
    } else if is(Key::PageUp) {
        nav::move_focus(app, -page, ctrl, shift);
    } else if is(Key::PageDown) {
        nav::move_focus(app, page, ctrl, shift);
    } else if is(Key::Home) {
        nav::move_focus(app, isize::MIN / 2, ctrl, shift);
    } else if is(Key::End) {
        nav::move_focus(app, isize::MAX / 2, ctrl, shift);
    } else if (is(Key::Return) || c == '\n') && alt {
        commands::properties_of_selection(app);
    } else if is(Key::Return) || c == '\n' {
        let row = app.borrow().tab().current_row();
        if let Some(r) = row {
            commands::activate(app, r);
        }
    } else if ctrl && c == ' ' {
        app.borrow_mut().tab_mut().toggle_current();
        let side = app.borrow().active;
        sync_selection(app, side, false);
    } else if letter('c') && !shift {
        commands::clipboard_put(app, false);
    } else if letter('x') && !shift {
        commands::clipboard_put(app, true);
    } else if letter('v') && !shift {
        commands::paste(app);
    } else if letter('a') {
        app.borrow_mut().tab_mut().select_all();
        let side = app.borrow().active;
        sync_selection(app, side, false);
    } else if letter('r') {
        nav::refresh_active(app);
    } else if letter('h') || (ctrl && shift && (c == '.' || c == '>')) {
        nav::toggle_hidden(app);
    } else if is(Key::Delete) || is(Key::F8) {
        commands::delete_selection(app, shift);
    } else if is(Key::F2) && !ctrl && !alt {
        commands::rename_selection(app);
    } else if is(Key::F5) && !ctrl && !alt {
        commands::transfer_selection(app, false);
    } else if is(Key::F6) && !ctrl && !alt {
        commands::transfer_selection(app, true);
    } else if is(Key::F7) || (ctrl && shift && c.eq_ignore_ascii_case(&'n')) {
        commands::new_folder(app);
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
        let path = a.tab().rows().find(|e| e.name().to_lowercase().starts_with(&prefix)).map(|e| e.path().to_path_buf());
        path.is_some_and(|p| a.tab_mut().select_path(&p))
    };
    if found {
        let side = app.borrow().active;
        sync_selection(app, side, true);
    }
}
