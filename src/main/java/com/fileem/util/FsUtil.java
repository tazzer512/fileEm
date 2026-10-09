package com.fileem.util;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.util.Locale;
import java.util.function.Predicate;
import java.util.regex.Pattern;

public final class FsUtil {
    private FsUtil() {
    }

    public static String displayName(Path p) {
        Path name = p.getFileName();
        return name != null ? name.toString() : p.toString();
    }

    public static boolean isHidden(Path p) {
        if (OsUtil.isWindows()) {
            try {
                return Files.isHidden(p);
            } catch (IOException e) {
                return false;
            }
        }
        Path name = p.getFileName();
        return name != null && name.toString().startsWith(".");
    }

    /**
     * True for Windows directory junctions / mount points (reparse points that are not symlinks).
     * Java reports them as plain directories, so recursive operations must not descend into them.
     */
    public static boolean isJunction(Path p) {
        if (!OsUtil.isWindows()) {
            return false;
        }
        try {
            Object attrs = Files.getAttribute(p, "dos:attributes", LinkOption.NOFOLLOW_LINKS);
            return attrs instanceof Integer i && (i & 0x400) != 0 && !Files.isSymbolicLink(p);
        } catch (IOException | RuntimeException e) {
            return false;
        }
    }

    /** True when {@code target} equals {@code folder} or lies somewhere below it. */
    public static boolean isSameOrInside(Path target, Path folder) {
        return target.toAbsolutePath().normalize().startsWith(folder.toAbsolutePath().normalize());
    }

    /**
     * A free sibling name: "name (2).ext", "name (3).ext" ... or, with {@code copySuffix},
     * "name - Copy.ext", "name - Copy (2).ext" ...
     */
    public static Path uniqueName(Path wanted, boolean copySuffix) {
        String name = displayName(wanted);
        String base = name, ext = "";
        int dot = name.lastIndexOf('.');
        if (dot > 0 && !Files.isDirectory(wanted, LinkOption.NOFOLLOW_LINKS)) {
            base = name.substring(0, dot);
            ext = name.substring(dot);
        }
        Path parent = wanted.getParent();
        if (copySuffix) {
            base = base + " - Copy";
            Path first = parent.resolve(base + ext);
            if (!Files.exists(first, LinkOption.NOFOLLOW_LINKS)) {
                return first;
            }
        } else if (!Files.exists(wanted, LinkOption.NOFOLLOW_LINKS)) {
            return wanted;
        }
        for (int i = 2; ; i++) {
            Path candidate = parent.resolve(base + " (" + i + ")" + ext);
            if (!Files.exists(candidate, LinkOption.NOFOLLOW_LINKS)) {
                return candidate;
            }
        }
    }

    /** Returns an error message, or null when the name is a valid file name on this OS. */
    public static String validateName(String name) {
        if (name == null || name.isBlank()) {
            return "The name cannot be empty.";
        }
        if (name.equals(".") || name.equals("..")) {
            return "\"" + name + "\" is not a valid name.";
        }
        if (name.indexOf('/') >= 0 || name.indexOf('\0') >= 0) {
            return "A name cannot contain \"/\".";
        }
        if (OsUtil.isWindows()) {
            for (char c : "\\:*?\"<>|".toCharArray()) {
                if (name.indexOf(c) >= 0) {
                    return "A name cannot contain any of these characters: \\ / : * ? \" < > |";
                }
            }
            if (name.endsWith(".") || name.endsWith(" ")) {
                return "A name cannot end with a dot or a space.";
            }
        }
        return null;
    }

    /** Filter text to a name matcher: plain text matches as "contains", * and ? act as wildcards. */
    public static Predicate<String> nameMatcher(String filter) {
        if (filter == null || filter.isBlank()) {
            return n -> true;
        }
        String f = filter.trim();
        if (f.indexOf('*') < 0 && f.indexOf('?') < 0) {
            String lower = f.toLowerCase(Locale.ROOT);
            return n -> n.toLowerCase(Locale.ROOT).contains(lower);
        }
        StringBuilder rx = new StringBuilder();
        for (char c : f.toCharArray()) {
            switch (c) {
                case '*' -> rx.append(".*");
                case '?' -> rx.append('.');
                default -> rx.append(Pattern.quote(String.valueOf(c)));
            }
        }
        Pattern p = Pattern.compile(rx.toString(), Pattern.CASE_INSENSITIVE | Pattern.UNICODE_CASE);
        return n -> p.matcher(n).matches();
    }
}
