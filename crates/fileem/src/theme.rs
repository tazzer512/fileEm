//! The look of the app (the Java `Theme` and its stylesheets): a family (Classic, Windows 11, …)
//! in a light or a dark mode. Each is a table of colors and corner radii that `ui/theme.slint`'s
//! `Theme` global hands to every window; the std widgets (buttons, fields, scroll bars) follow
//! the mode. The colors are the Java themes' (`themes/<id>-light.css` / `-dark.css`, Classic is
//! AtlantaFX Primer).

/// A theme family; each has a light and a dark mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Classic,
    Windows,
    MacOs,
    Adwaita,
    Aurora,
}

impl Family {
    pub const ALL: [Family; 5] = [Family::Classic, Family::Windows, Family::MacOs, Family::Adwaita, Family::Aurora];

    /// The session's `theme.name`.
    pub fn id(self) -> &'static str {
        match self {
            Family::Classic => "classic",
            Family::Windows => "windows",
            Family::MacOs => "macos",
            Family::Adwaita => "adwaita",
            Family::Aurora => "aurora",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Family::Classic => "Classic",
            Family::Windows => "Windows 11",
            Family::MacOs => "macOS",
            Family::Adwaita => "Adwaita (GNOME)",
            Family::Aurora => "Aurora",
        }
    }

    /// The family with `id`, Classic for unknown or missing ids.
    pub fn by_id(id: &str) -> Family {
        Family::ALL.into_iter().find(|f| f.id() == id).unwrap_or(Family::Classic)
    }
}

/// A color as 0xRRGGBBAA.
pub type Rgba = u32;

/// A fill: one color, or a linear gradient (angle in degrees as Slint counts them: 90 runs left
/// to right, 180 top to bottom; evenly spread stops).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Solid(Rgba),
    Gradient(u16, &'static [Rgba]),
}

impl Fill {
    /// The color a gradient starts with (contrast checks, places that take one color).
    pub fn first(self) -> Rgba {
        match self {
            Fill::Solid(c) => c,
            Fill::Gradient(_, stops) => stops[0],
        }
    }
}

/// Everything a window needs to draw itself in one family and mode.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// The font family ("" for the system's).
    pub font: &'static str,
    /// Window background (dialogs and the other windows too).
    pub window: Fill,
    pub toolbar: Fill,
    /// The strip behind the tabs, and the active tab.
    pub tabbar: Fill,
    pub tab_active: Fill,
    /// The file list and grid.
    pub view: Fill,
    /// The folder tree.
    pub sidebar: Fill,
    pub status: Fill,
    /// Text fields drawn by us (the address bar).
    pub field: Rgba,
    pub fg: Rgba,
    /// Secondary text (status line, headers, details).
    pub muted: Rgba,
    /// Faint text (section titles, separators in the address bar, free space).
    pub subtle: Rgba,
    pub border: Rgba,
    /// Lines between parts (splitters, under bars).
    pub divider: Rgba,
    /// The accent: current markers, drop targets, the selection box's outline.
    pub accent: Rgba,
    /// Accent text and icons (quick access, F-keys).
    pub accent_fg: Rgba,
    /// Faint accent fills (drop targets, the selection box).
    pub accent_subtle: Rgba,
    /// The bar marking the active side's tab (a gradient in Aurora).
    pub highlight: Fill,
    /// Row and item hover.
    pub hover: Rgba,
    /// Selected rows while the list has no focus, and while it has.
    pub selection: Rgba,
    pub selection_focused: Fill,
    /// Text on a focused selection.
    pub selection_fg: Rgba,
    /// The tree's current folder.
    pub tree_current: Rgba,
    /// Errors and missing things.
    pub danger: Rgba,
    /// Corner radii: rows and grid items, buttons and fields, tabs.
    pub radius: f32,
    pub control_radius: f32,
    pub tab_radius: f32,
}

const fn rgb(c: u32) -> Rgba {
    (c << 8) | 0xff
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | ((a * 255.0 + 0.5) as u32)
}

const fn solid(c: u32) -> Fill {
    Fill::Solid(rgb(c))
}

/// One family in one mode.
pub fn palette(family: Family, dark: bool) -> Palette {
    match (family, dark) {
        (Family::Classic, false) => Palette {
            dark,
            font: "",
            window: solid(0xffffff),
            toolbar: solid(0xf6f8fa),
            tabbar: solid(0xf6f8fa),
            tab_active: solid(0xffffff),
            view: solid(0xffffff),
            sidebar: solid(0xf6f8fa),
            status: solid(0xf6f8fa),
            field: rgb(0xffffff),
            fg: rgb(0x24292f),
            muted: rgb(0x57606a),
            subtle: rgb(0x6e7781),
            border: rgb(0xd0d7de),
            divider: rgb(0xd0d7de),
            accent: rgb(0x0969da),
            accent_fg: rgb(0x0969da),
            accent_subtle: rgb(0xddf4ff),
            highlight: solid(0x0969da),
            hover: rgb(0xf6f8fa),
            selection: rgb(0xeaeef2),
            selection_focused: Fill::Solid(rgba(84, 174, 255, 0.4)),
            selection_fg: rgb(0x24292f),
            tree_current: rgb(0xddf4ff),
            danger: rgb(0xcf222e),
            radius: 4.0,
            control_radius: 6.0,
            tab_radius: 4.0,
        },
        (Family::Classic, true) => Palette {
            dark,
            font: "",
            window: solid(0x0d1117),
            toolbar: solid(0x161b22),
            tabbar: solid(0x161b22),
            tab_active: solid(0x0d1117),
            view: solid(0x0d1117),
            sidebar: solid(0x161b22),
            status: solid(0x161b22),
            field: rgb(0x0d1117),
            fg: rgb(0xc9d1d9),
            muted: rgb(0x8b949e),
            subtle: rgb(0x6e7681),
            border: rgb(0x30363d),
            divider: rgb(0x21262d),
            accent: rgb(0x1f6feb),
            accent_fg: rgb(0x58a6ff),
            accent_subtle: rgba(56, 139, 253, 0.15),
            highlight: solid(0x1f6feb),
            hover: rgb(0x161b22),
            selection: rgb(0x21262d),
            selection_focused: Fill::Solid(rgba(56, 139, 253, 0.4)),
            selection_fg: rgb(0xe6edf3),
            tree_current: rgba(56, 139, 253, 0.15),
            danger: rgb(0xf85149),
            radius: 4.0,
            control_radius: 6.0,
            tab_radius: 4.0,
        },
        (Family::Windows, false) => Palette {
            dark,
            font: "Segoe UI Variable Text",
            window: solid(0xeeeeee),
            toolbar: solid(0xf9f9f9),
            tabbar: solid(0xeeeeee),
            tab_active: solid(0xf9f9f9),
            view: solid(0xf9f9f9),
            sidebar: solid(0xf9f9f9),
            status: solid(0xf9f9f9),
            field: rgb(0xffffff),
            fg: rgb(0x1b1b1b),
            muted: rgb(0x5d5d5d),
            subtle: rgb(0x8a8a8a),
            border: rgb(0xe0e0e0),
            divider: rgb(0xe5e5e5),
            accent: rgb(0x005fb8),
            accent_fg: rgb(0x005fb8),
            accent_subtle: rgb(0xe0eef9),
            highlight: solid(0x005fb8),
            hover: rgb(0xe5f3ff),
            selection: rgb(0xd9e8f5),
            selection_focused: solid(0xcce8ff),
            selection_fg: rgb(0x1b1b1b),
            tree_current: rgb(0xebebeb),
            danger: rgb(0xc42b1c),
            radius: 4.0,
            control_radius: 4.0,
            tab_radius: 8.0,
        },
        (Family::Windows, true) => Palette {
            dark,
            font: "Segoe UI Variable Text",
            window: solid(0x1c1c1c),
            toolbar: solid(0x272727),
            tabbar: solid(0x1c1c1c),
            tab_active: solid(0x272727),
            view: solid(0x272727),
            sidebar: solid(0x272727),
            status: solid(0x272727),
            field: rgb(0x2d2d2d),
            fg: rgb(0xffffff),
            muted: rgb(0xc5c5c5),
            subtle: rgb(0x9a9a9a),
            border: rgb(0x3a3a3a),
            divider: rgb(0x353535),
            accent: rgb(0x4cc2ff),
            accent_fg: rgb(0x60cdff),
            accent_subtle: rgba(96, 205, 255, 0.15),
            highlight: solid(0x4cc2ff),
            hover: rgb(0x2f3338),
            selection: rgb(0x3a3d41),
            selection_focused: solid(0x3d4a57),
            selection_fg: rgb(0xffffff),
            tree_current: rgb(0x363636),
            danger: rgb(0xff99a4),
            radius: 4.0,
            control_radius: 4.0,
            tab_radius: 8.0,
        },
        (Family::MacOs, false) => Palette {
            dark,
            font: "SF Pro Text",
            window: solid(0xf6f6f6),
            toolbar: solid(0xf6f6f6),
            tabbar: solid(0xe3e3e3),
            tab_active: solid(0xffffff),
            view: solid(0xffffff),
            sidebar: solid(0xececec),
            status: solid(0xffffff),
            field: rgb(0xffffff),
            fg: rgb(0x262626),
            muted: rgb(0x6e6e73),
            subtle: rgb(0x8e8e93),
            border: rgb(0xdcdcdc),
            divider: rgb(0xdcdcdc),
            accent: rgb(0x0a64e1),
            accent_fg: rgb(0x0a64e1),
            accent_subtle: rgb(0xdbe8fb),
            highlight: solid(0x0a64e1),
            hover: rgba(0, 0, 0, 0.06),
            selection: rgb(0xdcdcdc),
            selection_focused: solid(0x0a64e1),
            selection_fg: rgb(0xffffff),
            tree_current: rgba(0, 0, 0, 0.09),
            danger: rgb(0xd70015),
            radius: 5.0,
            control_radius: 6.0,
            tab_radius: 6.0,
        },
        (Family::MacOs, true) => Palette {
            dark,
            font: "SF Pro Text",
            window: solid(0x2b2b2d),
            toolbar: solid(0x2d2d2f),
            tabbar: solid(0x1f1f21),
            tab_active: solid(0x1e1e1e),
            view: solid(0x1e1e1e),
            sidebar: solid(0x262628),
            status: solid(0x1e1e1e),
            field: rgb(0x2c2c2e),
            fg: rgb(0xe5e5e7),
            muted: rgb(0x98989d),
            subtle: rgb(0x8d8d93),
            border: rgb(0x3a3a3c),
            divider: rgb(0x3a3a3c),
            accent: rgb(0x0a64d8),
            accent_fg: rgb(0x3d9bff),
            accent_subtle: rgba(10, 132, 255, 0.18),
            highlight: solid(0x0a64d8),
            hover: rgba(255, 255, 255, 0.07),
            selection: rgb(0x464648),
            selection_focused: solid(0x0a64d8),
            selection_fg: rgb(0xffffff),
            tree_current: rgba(255, 255, 255, 0.1),
            danger: rgb(0xff6961),
            radius: 5.0,
            control_radius: 6.0,
            tab_radius: 6.0,
        },
        (Family::Adwaita, false) => Palette {
            dark,
            font: "Cantarell",
            window: solid(0xfafafa),
            toolbar: solid(0xebebeb),
            tabbar: solid(0xebebeb),
            tab_active: Fill::Solid(rgba(0, 0, 6, 0.1)),
            view: solid(0xffffff),
            sidebar: solid(0xf2f2f4),
            status: solid(0xffffff),
            field: rgba(0, 0, 6, 0.06),
            fg: rgb(0x2e2e32),
            muted: rgb(0x6b6b70),
            subtle: rgb(0x8e8e93),
            border: rgb(0xdadadc),
            divider: rgb(0xe3e3e5),
            accent: rgb(0x3584e4),
            accent_fg: rgb(0x1c71d8),
            accent_subtle: rgba(53, 132, 228, 0.15),
            highlight: solid(0x3584e4),
            hover: rgba(0, 0, 6, 0.05),
            selection: rgba(53, 132, 228, 0.12),
            selection_focused: Fill::Solid(rgba(53, 132, 228, 0.22)),
            selection_fg: rgb(0x2e2e32),
            tree_current: rgba(0, 0, 6, 0.1),
            danger: rgb(0xc01c28),
            radius: 6.0,
            control_radius: 6.0,
            tab_radius: 6.0,
        },
        (Family::Adwaita, true) => Palette {
            dark,
            font: "Cantarell",
            window: solid(0x242424),
            toolbar: solid(0x2e2e32),
            tabbar: solid(0x2e2e32),
            tab_active: Fill::Solid(rgba(255, 255, 255, 0.11)),
            view: solid(0x1e1e1e),
            sidebar: solid(0x28282c),
            status: solid(0x1e1e1e),
            field: rgba(255, 255, 255, 0.08),
            fg: rgb(0xffffff),
            muted: rgb(0xb5b5b9),
            subtle: rgb(0x8e8e93),
            border: rgb(0x3a3a3e),
            divider: rgb(0x303034),
            accent: rgb(0x3584e4),
            accent_fg: rgb(0x78aeed),
            accent_subtle: rgba(120, 174, 237, 0.15),
            highlight: solid(0x3584e4),
            hover: rgba(255, 255, 255, 0.06),
            selection: rgba(120, 174, 237, 0.15),
            selection_focused: Fill::Solid(rgba(120, 174, 237, 0.27)),
            selection_fg: rgb(0xffffff),
            tree_current: rgba(255, 255, 255, 0.11),
            danger: rgb(0xff7b63),
            radius: 6.0,
            control_radius: 6.0,
            tab_radius: 6.0,
        },
        (Family::Aurora, false) => Palette {
            dark,
            font: "Inter",
            window: Fill::Gradient(135, const { &[rgb(0xf1edff), rgb(0xeef6ff), rgb(0xecfbf6)] }),
            toolbar: solid(0xffffff),
            tabbar: Fill::Gradient(135, const { &[rgb(0xf1edff), rgb(0xeef6ff), rgb(0xecfbf6)] }),
            tab_active: solid(0xffffff),
            view: solid(0xffffff),
            sidebar: Fill::Gradient(180, const { &[rgb(0xf5f2ff), rgb(0xf0f5ff), rgb(0xeffbf7)] }),
            status: solid(0xffffff),
            field: rgb(0xfaf9ff),
            fg: rgb(0x231c46),
            muted: rgb(0x5f5884),
            subtle: rgb(0x7a7499),
            border: rgb(0xe4defa),
            divider: rgb(0xece8fb),
            accent: rgb(0x8b5cf6),
            accent_fg: rgb(0x7c3aed),
            accent_subtle: rgba(139, 92, 246, 0.12),
            highlight: Fill::Gradient(90, const { &[rgb(0xff7a59), rgb(0xf472b6), rgb(0x8b5cf6), rgb(0x14b8a6)] }),
            hover: rgba(124, 58, 237, 0.07),
            selection: rgba(139, 92, 246, 0.12),
            selection_focused: Fill::Gradient(90, const { &[rgba(139, 92, 246, 0.2), rgba(20, 184, 166, 0.13)] }),
            selection_fg: rgb(0x231c46),
            tree_current: rgba(124, 58, 237, 0.14),
            danger: rgb(0xdc2626),
            radius: 10.0,
            control_radius: 14.0,
            tab_radius: 12.0,
        },
        (Family::Aurora, true) => Palette {
            dark,
            font: "Inter",
            window: Fill::Gradient(180, const { &[rgb(0x070b16), rgb(0x0b1226), rgb(0x101a33)] }),
            toolbar: solid(0x0f1629),
            tabbar: Fill::Gradient(180, const { &[rgb(0x070b16), rgb(0x0b1226)] }),
            tab_active: solid(0x0f1629),
            view: solid(0x0f1629),
            sidebar: Fill::Gradient(180, const { &[rgb(0x0a1020), rgb(0x0d1730), rgb(0x0f2235)] }),
            status: solid(0x0f1629),
            field: rgb(0x0b1224),
            fg: rgb(0xe2e8ff),
            muted: rgb(0x9aa6c8),
            subtle: rgb(0x7f8db3),
            border: rgb(0x1f2a48),
            divider: rgb(0x18213b),
            accent: rgb(0x22d3ee),
            accent_fg: rgb(0x5eead4),
            accent_subtle: rgba(34, 211, 238, 0.14),
            highlight: Fill::Gradient(90, const { &[rgb(0x34d399), rgb(0x22d3ee), rgb(0xa78bfa), rgb(0xf472b6)] }),
            hover: rgba(148, 163, 255, 0.08),
            selection: rgba(99, 102, 241, 0.2),
            selection_focused: Fill::Gradient(90, const { &[rgba(34, 211, 238, 0.22), rgba(167, 139, 250, 0.2)] }),
            selection_fg: rgb(0xe2e8ff),
            tree_current: rgba(148, 163, 255, 0.16),
            danger: rgb(0xf87171),
            radius: 10.0,
            control_radius: 14.0,
            tab_radius: 12.0,
        },
    }
}

/// `top` (0xRRGGBBAA) drawn over the opaque color `under` (red, green, blue in 0…1).
pub fn over(top: Rgba, under: [f32; 3]) -> [f32; 3] {
    let a = (top & 0xff) as f32 / 255.0;
    let ch = |i: usize| ((top >> (24 - 8 * i)) & 0xff) as f32 / 255.0;
    [0, 1, 2].map(|i| ch(i) * a + under[i] * (1.0 - a))
}

/// WCAG contrast ratio of two opaque colors (1 to 21).
pub fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let lum = |c: [f32; 3]| {
        let lin = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
    };
    let (x, y) = (lum(a) + 0.05, lum(b) + 0.05);
    x.max(y) / x.min(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<(Family, bool, Palette)> {
        Family::ALL.into_iter().flat_map(|f| [false, true].map(|d| (f, d, palette(f, d)))).collect()
    }

    #[test]
    fn ids_round_trip_and_unknown_ones_are_classic() {
        for f in Family::ALL {
            assert_eq!(Family::by_id(f.id()), f);
        }
        assert_eq!(Family::by_id("nope"), Family::Classic);
        assert_eq!(Family::by_id(""), Family::Classic);
        // the Java app's ids and labels, so both read the same session
        let ids: Vec<_> = Family::ALL.iter().map(|f| f.id()).collect();
        assert_eq!(ids, ["classic", "windows", "macos", "adwaita", "aurora"]);
        assert_eq!(Family::Adwaita.label(), "Adwaita (GNOME)");
    }

    #[test]
    fn every_family_has_both_modes_and_they_differ() {
        for f in Family::ALL {
            let (light, dark) = (palette(f, false), palette(f, true));
            assert!(!light.dark && dark.dark, "{f:?}");
            assert_ne!(light.fg, dark.fg, "{f:?}");
            assert_ne!(light.view, dark.view, "{f:?}");
        }
        // and the families differ from each other
        for (i, a) in Family::ALL.iter().enumerate() {
            for b in &Family::ALL[i + 1..] {
                assert_ne!(palette(*a, false), palette(*b, false), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn text_is_readable_on_every_surface() {
        for (f, d, p) in all() {
            let base = if d { [0.0; 3] } else { [1.0; 3] };
            for (name, surface) in
                [("view", p.view), ("sidebar", p.sidebar), ("toolbar", p.toolbar), ("status", p.status), ("window", p.window)]
            {
                let bg = over(surface.first(), base);
                assert!(contrast(over(p.fg, bg), bg) >= 7.0, "{f:?} dark={d} text on {name}");
                assert!(contrast(over(p.muted, bg), bg) >= 3.0, "{f:?} dark={d} muted text on {name}");
            }
            let selected = over(p.selection_focused.first(), over(p.view.first(), base));
            assert!(contrast(over(p.selection_fg, selected), selected) >= 4.5, "{f:?} dark={d} selected text");
        }
    }

    #[test]
    fn colors_mix_and_contrast_like_wcag() {
        assert_eq!(over(rgb(0x000000), [1.0; 3]), [0.0, 0.0, 0.0]);
        let half = over(rgba(0, 0, 0, 0.5), [1.0; 3]);
        assert!((half[0] - 0.5).abs() < 0.01);
        assert!((contrast([0.0; 3], [1.0; 3]) - 21.0).abs() < 0.01);
        assert!((contrast([0.5; 3], [0.5; 3]) - 1.0).abs() < 0.001);
    }
}
