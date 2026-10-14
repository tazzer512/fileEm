package com.fileem;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

class AppInfoTest {
    @Test
    void theVersionIsThePackagedAppVersionFromThePom() throws Exception {
        Matcher m = Pattern.compile("<app.version>([^<]+)</app.version>").matcher(Files.readString(Path.of("pom.xml")));
        assertTrue(m.find(), "pom defines app.version");
        assertEquals(m.group(1), AppInfo.version(), "Maven filled in version.properties");
    }

    @Test
    void theAboutTextHasVersionBuildDateJavaAndOs() {
        assertNotNull(AppInfo.buildDate());
        assertTrue(AppInfo.buildDate().matches("\\d{4}-\\d{2}-\\d{2}"), AppInfo.buildDate());
        String about = AppInfo.about();
        assertTrue(about.startsWith("Version " + AppInfo.version() + "\nBuilt " + AppInfo.buildDate()), about);
        assertTrue(about.contains("Java " + System.getProperty("java.version")), about);
        assertTrue(about.contains(System.getProperty("os.name")), about);
    }
}
