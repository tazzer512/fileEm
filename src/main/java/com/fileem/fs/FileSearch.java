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
 * {@code *} (any text) and {@code ?} (one character) and ignore case. Several can be given, separated
 * by spaces or {@code ;} ({@code *.jpg *.png}); {@code !} in front excludes ({@code a*.* !*.mp3}). A
 * pattern without wildcards matches anywhere in the name, like Explorer's search box. Links are not followed; unreadable folders are skipped and counted.
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

    /**
     * The name test for {@code pattern}; null when it has nothing to look for. Terms are separated by
     * spaces or {@code ;}; a name matches when it matches any plain term and no term starting with
     * {@code !} ({@code a*.* !*.mp3}: names starting with "a", except mp3s). With only {@code !} terms,
     * everything else matches. Quotes keep spaces in a term ({@code "my file*"}, {@code !"old copy"}).
     */
    public static Predicate<String> matcher(String pattern) {
        if (pattern == null) {
            return null;
        }
        List<Pattern> include = new ArrayList<>();
        List<Pattern> exclude = new ArrayList<>();
        for (String term : terms(pattern)) {
            boolean not = term.startsWith("!");
            String glob = not ? term.substring(1) : term;
            if (glob.isEmpty()) {
                continue;
            }
            if (glob.indexOf('*') < 0 && glob.indexOf('?') < 0) {
                glob = "*" + glob + "*";
            }
            Pattern p = Pattern.compile(globToRegex(glob), Pattern.CASE_INSENSITIVE | Pattern.UNICODE_CASE | Pattern.DOTALL);
            (not ? exclude : include).add(p);
        }
        if (include.isEmpty() && exclude.isEmpty()) {
            return null;
        }
        return name -> (include.isEmpty() || matchesAny(include, name)) && !matchesAny(exclude, name);
    }

    private static boolean matchesAny(List<Pattern> patterns, String name) {
        for (Pattern p : patterns) {
            if (p.matcher(name).matches()) {
                return true;
            }
        }
        return false;
    }

    /** Splits at spaces and {@code ;} outside quotes; the quotes themselves are dropped. */
    static List<String> terms(String pattern) {
        List<String> out = new ArrayList<>();
        StringBuilder cur = new StringBuilder();
        boolean quoted = false;
        for (int i = 0; i < pattern.length(); i++) {
            char c = pattern.charAt(i);
            if (c == '"') {
                quoted = !quoted;
            } else if (!quoted && (Character.isWhitespace(c) || c == ';')) {
                if (!cur.isEmpty()) {
                    out.add(cur.toString());
                    cur.setLength(0);
                }
            } else {
                cur.append(c);
            }
        }
        if (!cur.isEmpty()) {
            out.add(cur.toString());
        }
        return out;
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
