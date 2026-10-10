package com.fileem;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertTrue;

/** Guards the jpackage setup (pom dist profile, tools/install-linux.sh) against renames. */
class PackagingTest {
    /**
     * JavaFX names the window class (WM_CLASS) after the Application class; the Linux launcher entry
     * must use that name, or the dock shows a generic icon and a second entry for running windows.
     */
    @Test
    void linuxLauncherEntryMatchesTheWindowClass() throws Exception {
        String script = Files.readString(Path.of("tools", "install-linux.sh"));
        assertTrue(script.contains("wm_class=\"" + FileEmApp.class.getName() + "\""),
                "install-linux.sh wm_class must be " + FileEmApp.class.getName());
    }

    @Test
    void everyPlatformHasAnAppIconAndTheDistProfileUsesThem() throws Exception {
        String pom = Files.readString(Path.of("pom.xml"));
        for (String icon : new String[]{"fileem.png", "fileem.ico", "fileem.icns"}) {
            Path p = Path.of("src", "packaging", icon);
            assertTrue(Files.size(p) > 0, p.toString());
            assertTrue(pom.contains("<app.icon>src/packaging/" + icon + "</app.icon>"), icon + " used in pom.xml");
        }
    }

    /** Zip extraction falls back to IBM437 names, which only jdk.charsets provides in a trimmed runtime. */
    @Test
    void trimmedRuntimeKeepsTheModulesJdepsCannotSee() throws Exception {
        String pom = Files.readString(Path.of("pom.xml"));
        int start = pom.indexOf("<dist.modules>");
        String modules = pom.substring(start, pom.indexOf("</dist.modules>", start));
        for (String m : new String[]{"java.desktop", "jdk.charsets", "jdk.localedata", "javafx.controls"}) {
            assertTrue(modules.contains(m), m);
        }
    }
}
