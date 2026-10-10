package com.fileem.ui;

import com.fileem.FxTestSupport;
import javafx.css.CssParser;
import javafx.scene.Scene;
import javafx.scene.control.Alert;
import javafx.scene.layout.Pane;
import javafx.stage.Stage;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.testfx.util.WaitForAsyncUtils;

import java.io.IOException;
import java.io.InputStream;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.TreeSet;
import java.util.concurrent.Callable;
import java.util.concurrent.TimeUnit;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class ThemeTest {
    private static final Pattern FE_USE = Pattern.compile("(-fe-[a-z0-9-]++)(?!\\s*:)");
    private static final Pattern FE_DEFINE = Pattern.compile("(-fe-[a-z0-9-]+)\\s*:");

    @BeforeAll
    static void startFx() {
        FxTestSupport.startToolkit();
    }

    @AfterEach
    void backToClassic() {
        fx(() -> {
            Theme.apply(Theme.Family.CLASSIC, false);
            return null;
        });
    }

    private static <T> T fx(Callable<T> work) {
        try {
            return WaitForAsyncUtils.asyncFx(work).get(10, TimeUnit.SECONDS);
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    private static String read(String url) throws IOException {
        try (InputStream in = new URL(url).openStream()) {
            return new String(in.readAllBytes(), StandardCharsets.UTF_8);
        }
    }

    private static Set<String> matches(Pattern p, String css) {
        Set<String> found = new TreeSet<>();
        Matcher m = p.matcher(css.replaceAll("(?s)/\\*.*?\\*/", ""));
        while (m.find()) {
            found.add(m.group(1));
        }
        return found;
    }

    @Test
    void unknownOrMissingIdsFallBackToClassic() {
        assertEquals(Theme.Family.CLASSIC, Theme.Family.byId(null));
        assertEquals(Theme.Family.CLASSIC, Theme.Family.byId("no-such-theme"));
        for (Theme.Family f : Theme.Family.values()) {
            assertEquals(f, Theme.Family.byId(f.id()));
        }
    }

    @Test
    void classicAddsNoStylesheetsAndEveryOtherFamilyHasALightAndADarkMode() {
        assertEquals(List.of(), Theme.Family.CLASSIC.stylesheets(false));
        assertEquals(List.of(), Theme.Family.CLASSIC.stylesheets(true));
        for (Theme.Family f : Theme.Family.values()) {
            if (f == Theme.Family.CLASSIC) {
                continue;
            }
            List<String> light = f.stylesheets(false);
            List<String> dark = f.stylesheets(true);
            assertEquals(2, light.size(), f + " light");
            assertEquals(2, dark.size(), f + " dark");
            assertEquals(light.get(0), dark.get(0), f + " shares its shapes between modes");
            assertTrue(light.get(1).endsWith("-light.css") && dark.get(1).endsWith("-dark.css"), f.toString());
        }
    }

    @Test
    void themeStylesheetsParseWithoutErrors() throws IOException {
        for (Theme.Family f : Theme.Family.values()) {
            for (boolean dark : new boolean[]{false, true}) {
                for (String url : f.stylesheets(dark)) {
                    CssParser.errorsProperty().clear();
                    new CssParser().parse(new URL(url));
                    assertEquals(List.of(), List.copyOf(CssParser.errorsProperty()), url);
                }
            }
        }
    }

    /** A looked-up color that a mode doesn't define only shows up as a runtime warning, so catch it here. */
    @Test
    void bothModesDefineEveryColorTheShapesUse() throws IOException {
        for (Theme.Family f : Theme.Family.values()) {
            if (f == Theme.Family.CLASSIC) {
                continue;
            }
            Set<String> used = matches(FE_USE, read(f.stylesheets(false).get(0)));
            for (boolean dark : new boolean[]{false, true}) {
                String modeCss = read(f.stylesheets(dark).get(1));
                Set<String> defined = matches(FE_DEFINE, modeCss);
                used.addAll(matches(FE_USE, modeCss));
                Set<String> missing = new TreeSet<>(used);
                missing.removeAll(defined);
                assertEquals(Set.of(), missing, f + (dark ? " dark" : " light") + " misses colors");
            }
        }
    }

    @Test
    void switchingRestylesOpenWindowsAndKeepsThemeSheetsRightAfterTheAppSheet() {
        Stage stage = fx(() -> {
            Theme.apply(Theme.Family.CLASSIC, false);
            Stage s = new Stage();
            Scene scene = new Scene(new Pane(), 100, 100);
            scene.getStylesheets().add("other.css");
            Theme.style(scene.getStylesheets());
            s.setScene(scene);
            s.show();
            return s;
        });
        try {
            assertEquals(List.of("other.css", Theme.APP_CSS), fx(() -> List.copyOf(stage.getScene().getStylesheets())));

            List<String> windows = new ArrayList<>(List.of("other.css", Theme.APP_CSS));
            windows.addAll(Theme.Family.WINDOWS.stylesheets(false));
            fx(() -> {
                Theme.apply(Theme.Family.WINDOWS);
                return null;
            });
            assertEquals(windows, fx(() -> List.copyOf(stage.getScene().getStylesheets())));

            fx(() -> {
                Theme.toggle();
                return null;
            });
            assertTrue(fx(Theme::isDark));
            assertEquals(Theme.Family.WINDOWS, fx(Theme::family), "toggling the mode keeps the family");
            List<String> auroraDark = new ArrayList<>(List.of("other.css", Theme.APP_CSS));
            auroraDark.addAll(Theme.Family.AURORA.stylesheets(true));
            fx(() -> {
                Theme.apply(Theme.Family.AURORA);
                return null;
            });
            assertEquals(auroraDark, fx(() -> List.copyOf(stage.getScene().getStylesheets())),
                    "switching the family keeps the mode and replaces the old theme's sheets");

            fx(() -> {
                Theme.apply(Theme.Family.CLASSIC, false);
                return null;
            });
            assertEquals(List.of("other.css", Theme.APP_CSS), fx(() -> List.copyOf(stage.getScene().getStylesheets())));
        } finally {
            fx(() -> {
                stage.close();
                return null;
            });
        }
    }

    @Test
    void dialogsGetTheCurrentTheme() {
        List<String> sheets = fx(() -> {
            Theme.apply(Theme.Family.MACOS, true);
            Alert a = new Alert(Alert.AlertType.INFORMATION);
            Dialogs.prepare(a, null);
            return List.copyOf(a.getDialogPane().getStylesheets());
        });
        List<String> expected = new ArrayList<>(List.of(Theme.APP_CSS));
        expected.addAll(Theme.Family.MACOS.stylesheets(true));
        assertEquals(expected, sheets);
    }
}
