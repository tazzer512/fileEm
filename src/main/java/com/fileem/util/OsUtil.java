package com.fileem.util;

import java.io.File;
import java.nio.file.FileSystems;
import java.nio.file.Files;
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

    /** Whether {@code program} is an executable in one of the PATH folders. */
    public static boolean onPath(String program) {
        String path = System.getenv("PATH");
        if (path == null) {
            return false;
        }
        for (String dir : path.split(File.pathSeparator)) {
            try {
                if (!dir.isEmpty() && Files.isExecutable(Path.of(dir, program))) {
                    return true;
                }
            } catch (RuntimeException ignored) {
                // invalid PATH entry
            }
        }
        return false;
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
