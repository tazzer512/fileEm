package com.fileem.ui;

import atlantafx.base.theme.PrimerDark;
import atlantafx.base.theme.PrimerLight;
import javafx.application.Application;

import java.util.Objects;

public final class Theme {
    public static final String APP_CSS = Objects.requireNonNull(
            Theme.class.getResource("/com/fileem/app.css")).toExternalForm();
    private static boolean dark;

    private Theme() {
    }

    public static void apply(boolean useDark) {
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
