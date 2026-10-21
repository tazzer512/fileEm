//! The single keyboard handler (the Java `MainWindow.onKey`). Text fields (address, filter,
//! dialogs) take their own keys first; what they don't use arrives here.

use std::path::Path;
use std::time::{Duration, Instant};

use slint::platform::Key;

use super::{bookmarks, commands, nav, preview, search, sync_selection, views, Shared};
use crate::grid::Move;
use crate::view_mode::ViewMode;

/// Returns true when the key was handled.
pub fn key(app: &Shared, text: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
    let Some(c) = text.chars().next() else { return false };
    let is = |k: Key| c == char::from(k);
    let letter = |l: char| ctrl && !alt && c.eq_ignore_ascii_case(&l);
    let page = 20;
    let side = app.borrow().active;
    let grid_move = |m: Move| views::move_key(app, m, ctrl, shift);
    if let Some(mode) = view_shortcut(c).filter(|_| ctrl && shift && !alt) {
        views::set_view(app, side, mode);
    } else if ctrl && !alt && (c == '=' || c == '+') {
        views::resize(app, side, 1);
    } else if ctrl && !alt && (c == '-' || c == '_') {
        views::resize(app, side, -1);
    } else if letter('t') {
        nav::new_tab_here(app);
    } else if letter('w') {
        let (side, index) = (app.borrow().active, app.borrow().side().active);
        nav::close_tab(app, side, index);
    } else if letter('l') {
        nav::edit_address(app);
    } else if letter('f') && shift {
        let dir = app.borrow().tab().dir().map(Path::to_path_buf);
        if let Some(d) = dir {
            search::open(app, d);
        }
    } else if letter('f') {
        nav::focus_filter(app);
    } else if letter('d') && !shift {
        bookmarks::toggle(app);
    } else if letter('b') && shift {
        bookmarks::manage(app);
    } else if is(Key::F3) && !ctrl && !alt {
        preview::toggle(app);
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
    } else if is(Key::LeftArrow) && !alt {
        // only the grid moves sideways
        return grid_move(Move::Left);
    } else if is(Key::RightArrow) && !alt {
        return grid_move(Move::Right);
    } else if is(Key::UpArrow) {
        if !grid_move(Move::Up) {
            nav::move_focus(app, -1, ctrl, shift);
        }
    } else if is(Key::DownArrow) {
        if !grid_move(Move::Down) {
            nav::move_focus(app, 1, ctrl, shift);
        }
    } else if is(Key::PageUp) {
        if !grid_move(Move::PageUp) {
            nav::move_focus(app, -page, ctrl, shift);
        }
    } else if is(Key::PageDown) {
        if !grid_move(Move::PageDown) {
            nav::move_focus(app, page, ctrl, shift);
        }
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
        sync_selection(app, side, false);
    } else if letter('c') && !shift {
        commands::clipboard_put(app, false);
    } else if letter('x') && !shift {
        commands::clipboard_put(app, true);
    } else if letter('v') && !shift {
        commands::paste(app);
    } else if letter('a') {
        app.borrow_mut().tab_mut().select_all();
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

/// Ctrl+Shift+1…8: Explorer's views. With Shift held, keyboards send the shifted character:
/// the digits (AZERTY) or the US layout's symbols are understood.
fn view_shortcut(c: char) -> Option<ViewMode> {
    let digit = match c {
        '1'..='8' => c as usize - '0' as usize,
        _ => "!@#$%^&*".chars().position(|s| s == c)? + 1,
    };
    ViewMode::from_index(digit - 1)
}

/// Slint sends special keys (arrows, F-keys) as private-use characters.
fn is_special(c: char) -> bool {
    ('\u{F700}'..='\u{F8FF}').contains(&c)
}

/// Whether `name` lowercased starts with `prefix` (lowercase), without copying the name: type-ahead
/// checks every row up to the match.
fn starts_with_lowercase(name: &str, prefix: &str) -> bool {
    let mut lower = name.chars().flat_map(char::to_lowercase);
    prefix.chars().all(|p| lower.next() == Some(p))
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
        let path = a.tab().rows().find(|e| starts_with_lowercase(e.name(), &prefix)).map(|e| e.path().to_path_buf());
        path.is_some_and(|p| a.tab_mut().select_path(&p))
    };
    if found {
        let side = app.borrow().active;
        sync_selection(app, side, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_ahead_matches_the_start_of_a_name_in_any_case() {
        assert!(starts_with_lowercase("Notes.txt", "no"));
        assert!(starts_with_lowercase("ÉTÉ.txt", "été"));
        assert!(starts_with_lowercase("a", ""));
        assert!(!starts_with_lowercase("No", "not"));
        assert!(!starts_with_lowercase("big.bin", "i"));
    }
}
