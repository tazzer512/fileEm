package com.fileem.ui;

import atlantafx.base.theme.PrimerDark;
import atlantafx.base.theme.PrimerLight;
import javafx.application.Application;

import java.util.Objects;

public final class Theme {
    public static final String APP_CSS = Objects.requireNonNull(
            Theme.class.getResource("/com/fileem/app.css")).toExternalForm();
    private static boolean dark;
    private static boolean applied;

    private Theme() {
    }

    /** Sets the theme; re-applying the current theme is a no-op (it would restyle every window). */
    public static void apply(boolean useDark) {
        if (applied && dark == useDark) {
            return;
        }
        applied = true;
        dark = useDark;
        Application.setUserAgentStylesheet(useDark
                ? new PrimerDark().getUserAgentStylesheet()
                : new PrimerLight().getUserAgentStylesheet());
    }

    public static boolean isDark() {
        return dark;
    }

    public static void toggle() {
        apply(!dark);
    }
}
