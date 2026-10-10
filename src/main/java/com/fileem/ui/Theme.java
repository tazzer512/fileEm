package com.fileem.ui;

import atlantafx.base.theme.CupertinoDark;
import atlantafx.base.theme.CupertinoLight;
import atlantafx.base.theme.PrimerDark;
import atlantafx.base.theme.PrimerLight;
import javafx.application.Application;
import javafx.scene.Parent;
import javafx.scene.Scene;
import javafx.stage.Window;

import java.util.List;
import java.util.Objects;
import java.util.function.Supplier;

/**
 * The look of the app: a theme family (Classic, Windows 11, …) in a light or a dark mode.
 * A family is an AtlantaFX user-agent stylesheet plus optional author stylesheets from
 * {@code /com/fileem/themes} that redefine the looked-up colors ({@code -color-*}) and shapes.
 * Theme stylesheets always sit right after {@link #APP_CSS} so they can override it.
 */
public final class Theme {
    public static final String APP_CSS = url("/com/fileem/app.css");
    private static final String THEMES_DIR = "/com/fileem/themes/";

    /** A theme family; each has a light and a dark mode. */
    public enum Family {
        CLASSIC("classic", "Classic", false, () -> new PrimerLight().getUserAgentStylesheet(),
                () -> new PrimerDark().getUserAgentStylesheet()),
        WINDOWS("windows", "Windows 11", true, () -> new PrimerLight().getUserAgentStylesheet(),
                () -> new PrimerDark().getUserAgentStylesheet()),
        MACOS("macos", "macOS", true, () -> new CupertinoLight().getUserAgentStylesheet(),
                () -> new CupertinoDark().getUserAgentStylesheet()),
        ADWAITA("adwaita", "Adwaita (GNOME)", true, () -> new PrimerLight().getUserAgentStylesheet(),
                () -> new PrimerDark().getUserAgentStylesheet()),
        AURORA("aurora", "Aurora", true, () -> new PrimerLight().getUserAgentStylesheet(),
                () -> new PrimerDark().getUserAgentStylesheet());

        private final String id;
        private final String label;
        private final boolean styled;
        private final Supplier<String> lightBase;
        private final Supplier<String> darkBase;

        Family(String id, String label, boolean styled, Supplier<String> lightBase, Supplier<String> darkBase) {
            this.id = id;
            this.label = label;
            this.styled = styled;
            this.lightBase = lightBase;
            this.darkBase = darkBase;
        }

        public String id() {
            return id;
        }

        public String label() {
            return label;
        }

        /** The family with {@code id}, or Classic for unknown or missing ids. */
        public static Family byId(String id) {
            for (Family f : values()) {
                if (f.id.equals(id)) {
                    return f;
                }
            }
            return CLASSIC;
        }

        /** This family's author stylesheets for one mode: shapes first, then the mode's colors. */
        List<String> stylesheets(boolean dark) {
            if (!styled) {
                return List.of();
            }
            return List.of(url(THEMES_DIR + id + ".css"), url(THEMES_DIR + id + (dark ? "-dark.css" : "-light.css")));
        }
    }

    private static Family family = Family.CLASSIC;
    private static boolean dark;
    private static boolean applied;

    private Theme() {
    }

    /** Keeps the family, switches the mode. */
    public static void apply(boolean useDark) {
        apply(family, useDark);
    }

    /** Keeps the mode, switches the family. */
    public static void apply(Family f) {
        apply(f, dark);
    }

    /** Sets the theme; re-applying the current theme is a no-op (it would restyle every window). */
    public static void apply(Family f, boolean useDark) {
        Objects.requireNonNull(f);
        if (applied && family == f && dark == useDark) {
            return;
        }
        applied = true;
        family = f;
        dark = useDark;
        Application.setUserAgentStylesheet(useDark ? f.darkBase.get() : f.lightBase.get());
        for (Window w : List.copyOf(Window.getWindows())) {
            Scene scene = w.getScene();
            if (scene != null) {
                restyle(scene.getStylesheets());
                Parent root = scene.getRoot();
                if (root != null) {
                    restyle(root.getStylesheets());
                }
            }
        }
    }

    public static Family family() {
        return family;
    }

    public static boolean isDark() {
        return dark;
    }

    public static void toggle() {
        apply(!dark);
    }

    /** Adds the app stylesheet and the current theme's stylesheets to a scene or dialog pane. */
    public static void style(List<String> stylesheets) {
        if (!stylesheets.contains(APP_CSS)) {
            stylesheets.add(APP_CSS);
        }
        restyle(stylesheets);
    }

    /** Replaces the theme stylesheets in a list that holds {@link #APP_CSS}; other lists are left alone. */
    private static void restyle(List<String> stylesheets) {
        if (!stylesheets.contains(APP_CSS)) {
            return;
        }
        List<String> wanted = family.stylesheets(dark);
        List<String> current = stylesheets.stream().filter(Theme::isThemeSheet).toList();
        if (current.equals(wanted)) {
            return;
        }
        stylesheets.removeIf(Theme::isThemeSheet);
        stylesheets.addAll(stylesheets.indexOf(APP_CSS) + 1, wanted);
    }

    private static boolean isThemeSheet(String url) {
        return url.contains(THEMES_DIR);
    }

    private static String url(String resource) {
        return Objects.requireNonNull(Theme.class.getResource(resource), resource).toExternalForm();
    }
}
