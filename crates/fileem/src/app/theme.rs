//! Puts the theme ([`crate::theme`]) on every window: the `Theme` global of `ui/theme.slint` and
//! the std widgets' light or dark scheme. Globals belong to one window, so each window gets it
//! when it is created ([`style`]) and again on every change ([`set`]). Session: `theme.name`,
//! `theme.dark`, shared with the Java app.

use slint::{Color, ComponentHandle};

use crate::theme::{self, Family, Fill, Rgba};
use crate::{
    AppWindow, BookmarksWindow, Palette, PreviewWindow, ProgressWindow, PropertiesWindow, SearchWindow, Theme,
    ThemeData, ThemeFill,
};

use super::{commands, Shared};

/// A window that draws with the theme.
pub trait Themed {
    fn set_theme(&self, data: &ThemeData);
}

macro_rules! themed {
    ($($window:ty),*) => {$(
        impl Themed for $window {
            fn set_theme(&self, data: &ThemeData) {
                self.global::<Theme>().set_data(data.clone());
                self.global::<Palette>().set_color_scheme(if data.dark {
                    slint::language::ColorScheme::Dark
                } else {
                    slint::language::ColorScheme::Light
                });
            }
        }
    )*};
}

themed!(AppWindow, PreviewWindow, SearchWindow, BookmarksWindow, PropertiesWindow, ProgressWindow);

fn color(c: Rgba) -> Color {
    Color::from_argb_u8((c & 0xff) as u8, (c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8)
}

fn fill(f: Fill) -> ThemeFill {
    let (angle, stops): (f32, &[Rgba]) = match f {
        Fill::Solid(ref c) => (0.0, std::slice::from_ref(c)),
        Fill::Gradient(angle, stops) => (angle as f32, stops),
    };
    let at = |i: usize| color(stops[i.min(stops.len() - 1)]);
    ThemeFill { angle, stops: stops.len() as i32, c0: at(0), c1: at(1), c2: at(2), c3: at(3) }
}

/// What the windows' `Theme` global gets for one family and mode.
pub fn data(family: Family, dark: bool) -> ThemeData {
    let p = theme::palette(family, dark);
    ThemeData {
        dark: p.dark,
        font: p.font.into(),
        window: fill(p.window),
        toolbar: fill(p.toolbar),
        tabbar: fill(p.tabbar),
        tab_active: fill(p.tab_active),
        view: fill(p.view),
        sidebar: fill(p.sidebar),
        status: fill(p.status),
        field: color(p.field),
        fg: color(p.fg),
        muted: color(p.muted),
        subtle: color(p.subtle),
        border: color(p.border),
        divider: color(p.divider),
        accent: color(p.accent),
        accent_fg: color(p.accent_fg),
        accent_subtle: color(p.accent_subtle),
        highlight: fill(p.highlight),
        hover: color(p.hover),
        selection: color(p.selection),
        selection_focused: fill(p.selection_focused),
        selection_fg: color(p.selection_fg),
        tree_current: color(p.tree_current),
        danger: color(p.danger),
        radius: p.radius,
        control_radius: p.control_radius,
        tab_radius: p.tab_radius,
    }
}

/// The theme a new window of this app starts with.
pub(super) fn style(app: &Shared, window: &impl Themed) {
    let (family, dark) = app.borrow().theme;
    window.set_theme(&data(family, dark));
}

/// The theme of the session (Classic light without one).
pub(super) fn restore(session: &fileem_core::session::Session) -> (Family, bool) {
    (Family::by_id(&session.get_or("theme.name", "classic")), session.get_bool("theme.dark", false))
}

/// The current theme (tests).
pub fn current(app: &Shared) -> (Family, bool) {
    app.borrow().theme
}

/// Switches the theme in every open window of this app.
pub fn set(app: &Shared, family: Family, dark: bool) {
    app.borrow_mut().theme = (family, dark);
    let data = data(family, dark);
    let a = app.borrow();
    if let Some(ui) = a.ui.upgrade() {
        ui.set_theme(&data);
    }
    if let Some(w) = &a.preview.window {
        w.set_theme(&data);
    }
    for s in &a.searches {
        s.window.set_theme(&data);
    }
    if let Some(m) = &a.manage {
        m.window.set_theme(&data);
    }
    for (_, w, _) in &a.properties {
        w.set_theme(&data);
    }
    a.ops.set_theme(data);
}

/// The theme menu's families (toolbar).
pub fn menu(app: &Shared) {
    let (family, dark) = app.borrow().theme;
    commands::set_menu(app, crate::menu::theme_menu(family, dark));
}

/// The light / dark button.
pub fn toggle_dark(app: &Shared) {
    let (family, dark) = app.borrow().theme;
    set(app, family, !dark);
}

/// The About box (the toolbar's last button).
pub fn about(app: &Shared) {
    let spec = commands::DialogSpec::info(&format!("fileEm {}", crate::about::VERSION), &crate::about::text());
    commands::open_dialog(app, commands::Dialog::Info, spec);
}
