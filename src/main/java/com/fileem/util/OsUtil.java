package com.fileem.util;

import java.nio.file.FileSystems;
import java.nio.file.Path;
import java.util.Locale;

public final class OsUtil {
    private static final String OS = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
    private static final boolean POSIX = FileSystems.getDefault().supportedFileAttributeViews().contains("posix");

    private OsUtil() {
    }

    public static boolean isWindows() {
        return OS.startsWith("windows");
    }

    public static boolean isMac() {
        return OS.startsWith("mac");
    }

    public static boolean isLinux() {
        return !isWindows() && !isMac();
    }

    public static boolean isPosix() {
        return POSIX;
    }

    public static Path home() {
        return Path.of(System.getProperty("user.home"));
    }

    public static Path configDir() {
        if (isWindows()) {
            String appData = System.getenv("APPDATA");
            return (appData != null ? Path.of(appData) : home()).resolve("fileEm");
        }
        if (isMac()) {
            return home().resolve("Library").resolve("Application Support").resolve("fileEm");
        }
        String xdg = System.getenv("XDG_CONFIG_HOME");
        Path base = xdg != null && !xdg.isBlank() ? Path.of(xdg) : home().resolve(".config");
        return base.resolve("fileem");
    }
}
