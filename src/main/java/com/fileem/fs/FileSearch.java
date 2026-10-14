package com.fileem.fs;

import java.io.IOException;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.NotDirectoryException;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.DosFileAttributes;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Predicate;
import java.util.regex.Pattern;

/**
 * Finds files and folders by name in a folder and everything below it. Patterns use the wildcards
 * {@code *} (any text) and {@code ?} (one character), ignore case, and several can be given separated
 * by {@code ;} ({@code *.jpg;*.png}). A pattern without wildcards matches anywhere in the name, like
 * Explorer's search box. Links are not followed; unreadable folders are skipped and counted.
 */
public final class FileSearch {
    private FileSearch() {
    }

    /** What a search reports, on the searching thread. */
    public interface Listener {
        void found(FileEntry entry);

        /** A folder is about to be searched (for progress). */
        default void entering(Path dir) {
        }

        /** A folder or file that couldn't be read. */
        default void failed(Path path) {
        }

        /** False ends the search early (enough results). */
        default boolean wantsMore() {
            return true;
        }
    }

    /** The name test for {@code pattern}; null when it has nothing to look for. */
    public static Predicate<String> matcher(String pattern) {
        if (pattern == null) {
            return null;
        }
        List<Pattern> parts = new ArrayList<>();
        for (String part : pattern.split(";")) {
            String p = part.strip();
            if (p.isEmpty() || p.chars().allMatch(c -> c == '*')) {
                if (!p.isEmpty()) {
                    parts.add(Pattern.compile(".*", Pattern.DOTALL));
                }
                continue;
            }
            if (p.indexOf('*') < 0 && p.indexOf('?') < 0) {
                p = "*" + p + "*";
            }
            parts.add(Pattern.compile(globToRegex(p), Pattern.CASE_INSENSITIVE | Pattern.UNICODE_CASE | Pattern.DOTALL));
        }
        if (parts.isEmpty()) {
            return null;
        }
        return name -> {
            for (Pattern p : parts) {
                if (p.matcher(name).matches()) {
                    return true;
                }
            }
            return false;
        };
    }

    private static String globToRegex(String glob) {
        StringBuilder re = new StringBuilder();
        StringBuilder literal = new StringBuilder();
        for (int i = 0; i < glob.length(); i++) {
            char c = glob.charAt(i);
            if (c == '*' || c == '?') {
                if (!literal.isEmpty()) {
                    re.append(Pattern.quote(literal.toString()));
                    literal.setLength(0);
                }
                re.append(c == '*' ? ".*" : ".");
            } else {
                literal.append(c);
            }
        }
        if (!literal.isEmpty()) {
            re.append(Pattern.quote(literal.toString()));
        }
        return re.toString();
    }

    /**
     * Searches everything below {@code root} (not {@code root} itself) for names {@code match} accepts.
     * Hidden items, and everything inside hidden folders, are skipped unless {@code includeHidden}.
     * Blocks; stops early when the thread is interrupted.
     *
     * @return true when the whole tree was searched, false when interrupted or the listener had enough
     */
    public static boolean search(Path root, Predicate<String> match, boolean includeHidden, Listener listener)
            throws IOException {
        if (!Files.isDirectory(root)) {
            throw new NotDirectoryException(root.toString());
        }
        // a linked folder is searched through its target, but results keep the path that was asked for
        Path start = Files.isSymbolicLink(root) ? root.toRealPath() : root;
        boolean[] stopped = {false};
        Files.walkFileTree(start, new SimpleFileVisitor<>() {
            @Override
            public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) {
                if (Thread.currentThread().isInterrupted() || !listener.wantsMore()) {
                    stopped[0] = true;
                    return FileVisitResult.TERMINATE;
                }
                if (dir.equals(start)) {
                    listener.entering(root);
                    return FileVisitResult.CONTINUE;
                }
                if (!includeHidden && isHidden(dir, attrs)) {
                    return FileVisitResult.SKIP_SUBTREE;
                }
                check(dir);
                listener.entering(shown(dir));
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFile(Path file, BasicFileAttributes attrs) {
                if (Thread.currentThread().isInterrupted() || !listener.wantsMore()) {
                    stopped[0] = true;
                    return FileVisitResult.TERMINATE;
                }
                if (includeHidden || !isHidden(file, attrs)) {
                    check(file);
                }
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult visitFileFailed(Path file, IOException e) {
                listener.failed(shown(file));
                return FileVisitResult.CONTINUE;
            }

            @Override
            public FileVisitResult postVisitDirectory(Path dir, IOException e) {
                if (e != null) {
                    listener.failed(shown(dir)); // a listing that broke off halfway
                }
                return FileVisitResult.CONTINUE;
            }

            private Path shown(Path p) {
                return start == root ? p : root.resolve(start.relativize(p));
            }

            private void check(Path p) {
                Path name = p.getFileName();
                if (name != null && match.test(name.toString())) {
                    listener.found(FileEntry.read(shown(p)));
                }
            }
        });
        return !stopped[0] && !Thread.currentThread().isInterrupted();
    }

    private static boolean isHidden(Path p, BasicFileAttributes attrs) {
        if (attrs instanceof DosFileAttributes dos) {
            return dos.isHidden();
        }
        Path name = p.getFileName();
        return name != null && name.toString().startsWith(".");
    }
}
