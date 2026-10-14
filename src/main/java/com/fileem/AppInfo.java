package com.fileem;

import java.io.IOException;
import java.io.InputStream;
import java.util.Properties;

/** The app's version and build date, from {@code version.properties} (filled in by Maven). */
public final class AppInfo {
    private static final Properties PROPS = load();

    private AppInfo() {
    }

    private static Properties load() {
        Properties p = new Properties();
        try (InputStream in = AppInfo.class.getResourceAsStream("version.properties")) {
            if (in != null) {
                p.load(in);
            }
        } catch (IOException ignored) {
            // no version info: shown as "dev"
        }
        return p;
    }

    /** The version, like "0.1.0" (the pom's app.version, also the packaged app's version). */
    public static String version() {
        return value("version", "dev");
    }

    /** The build date (yyyy-MM-dd), or null when unknown. */
    public static String buildDate() {
        return value("built", null);
    }

    private static String value(String key, String fallback) {
        String v = PROPS.getProperty(key);
        return v == null || v.isBlank() || v.startsWith("${") ? fallback : v.strip();
    }

    /** What the About box shows: name and version, build date, Java/JavaFX and OS. */
    public static String about() {
        StringBuilder sb = new StringBuilder();
        sb.append("Version ").append(version());
        if (buildDate() != null) {
            sb.append("\nBuilt ").append(buildDate());
        }
        sb.append("\n\nJava ").append(System.getProperty("java.version"))
                .append("  ·  JavaFX ").append(System.getProperty("javafx.runtime.version", "?"))
                .append("\n").append(System.getProperty("os.name")).append(' ')
                .append(System.getProperty("os.version")).append(" (").append(System.getProperty("os.arch")).append(')');
        return sb.toString();
    }
}
