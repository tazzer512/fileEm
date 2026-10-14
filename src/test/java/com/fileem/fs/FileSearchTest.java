package com.fileem.fs;

import com.fileem.util.OsUtil;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.NotDirectoryException;
import java.nio.file.Path;
import java.nio.file.attribute.PosixFilePermissions;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class FileSearchTest {
    @TempDir
    Path dir;

    /** What a search found and couldn't read, as sorted paths relative to {@code dir}. */
    private record Result(List<String> found, List<String> failed, boolean complete) {
    }

    private Result search(Path root, String pattern, boolean hidden) throws IOException {
        List<String> found = new ArrayList<>();
        List<String> failed = new ArrayList<>();
        boolean complete = FileSearch.search(root, FileSearch.matcher(pattern), hidden, new FileSearch.Listener() {
            @Override
            public void found(FileEntry e) {
                found.add(rel(root, e.path()));
            }

            @Override
            public void failed(Path p) {
                failed.add(rel(root, p));
            }
        });
        return new Result(found.stream().sorted().toList(), failed.stream().sorted().toList(), complete);
    }

    private static String rel(Path root, Path p) {
        return root.relativize(p).toString().replace('\\', '/');
    }

    private void tree(String... paths) throws IOException {
        for (String p : paths) {
            Path f = dir.resolve(p);
            if (p.endsWith("/")) {
                Files.createDirectories(f);
            } else {
                Files.createDirectories(f.getParent());
                Files.writeString(f, p);
            }
        }
    }

    @Test
    void wildcardsAndPlainText() {
        Predicate<String> jpg = FileSearch.matcher("*.jpg");
        assertTrue(jpg.test("a.jpg"));
        assertTrue(jpg.test("PHOTO.JPG"), "case doesn't matter");
        assertFalse(jpg.test("a.jpg.txt"), "the whole name has to match");
        assertFalse(jpg.test("ajpg"));

        Predicate<String> img = FileSearch.matcher("IMG_????.*");
        assertTrue(img.test("img_1234.cr2"));
        assertFalse(img.test("IMG_123.cr2"), "? is exactly one character");

        Predicate<String> plain = FileSearch.matcher("report");
        assertTrue(plain.test("Annual Report 2024.pdf"), "no wildcards: anywhere in the name");
        assertFalse(plain.test("repo.txt"));

        Predicate<String> several = FileSearch.matcher(" *.png ; *.gif ;");
        assertTrue(several.test("x.png"));
        assertTrue(several.test("y.GIF"));
        assertFalse(several.test("z.jpg"));

        Predicate<String> special = FileSearch.matcher("a+b (1)[x].txt");
        assertTrue(special.test("a+b (1)[x].txt"), "regex characters are plain text");
        assertFalse(special.test("aab (1)x.txt"));

        assertTrue(FileSearch.matcher("*").test("anything"));
        assertNull(FileSearch.matcher(""));
        assertNull(FileSearch.matcher("  ;  "));
        assertNull(FileSearch.matcher(null));
    }

    @Test
    void searchesEveryFolderBelowAndFindsFoldersToo() throws IOException {
        tree("top.jpg", "notes.txt", "a/b/c/deep.JPG", "a/b/other.png", "photos.jpg/", "photos.jpg/inside.txt");
        Result r = search(dir, "*.jpg", false);
        assertEquals(List.of("a/b/c/deep.JPG", "photos.jpg", "top.jpg"), r.found());
        assertTrue(r.complete());
        assertEquals(List.of(), r.failed());
        assertEquals(List.of(), search(dir, "*.doc", false).found());
        assertEquals(List.of("b/c/deep.JPG"), search(dir.resolve("a"), "deep", false).found(),
                "only below the folder searched");
    }

    @Test
    void theFolderSearchedIsNotAResultItself() throws IOException {
        tree("match/", "match/match.txt");
        assertEquals(List.of("match.txt"), search(dir.resolve("match"), "match", false).found());
    }

    @Test
    void hiddenItemsAndEverythingInHiddenFoldersOnlyWhenAsked() throws IOException {
        assumeTrue(OsUtil.isPosix());
        tree("a.txt", ".b.txt", ".cache/c.txt", "sub/.d.txt");
        assertEquals(List.of("a.txt"), search(dir, "*.txt", false).found());
        assertEquals(List.of(".b.txt", ".cache/c.txt", "a.txt", "sub/.d.txt"), search(dir, "*.txt", true).found());
        assertEquals(List.of("c.txt"), search(dir.resolve(".cache"), "*.txt", false).found(),
                "searching inside a hidden folder itself works");
    }

    @Test
    void linksAreFoundButNotFollowed() throws IOException {
        assumeTrue(OsUtil.isPosix());
        tree("real/inside.txt", "elsewhere/far.txt");
        Files.createSymbolicLink(dir.resolve("real/link"), dir.resolve("elsewhere"));
        Files.createSymbolicLink(dir.resolve("real/loop"), dir.resolve("real"));
        Result r = search(dir.resolve("real"), "*", false);
        assertEquals(List.of("inside.txt", "link", "loop"), r.found(), "no far.txt, and the loop ends");
        assertTrue(r.complete());
    }

    @Test
    void aLinkedFolderIsSearchedThroughItsTargetUnderItsOwnPath() throws IOException {
        assumeTrue(OsUtil.isPosix());
        tree("target/sub/x.txt");
        Path link = Files.createSymbolicLink(dir.resolve("shortcut"), dir.resolve("target"));
        List<Path> found = new ArrayList<>();
        FileSearch.search(link, FileSearch.matcher("x.txt"), false, new FileSearch.Listener() {
            @Override
            public void found(FileEntry e) {
                found.add(e.path());
            }
        });
        assertEquals(List.of(link.resolve("sub/x.txt")), found);
    }

    @Test
    void unreadableFoldersAreSkippedAndReported() throws IOException {
        assumeTrue(OsUtil.isPosix());
        tree("open/a.txt", "locked/b.txt");
        Path locked = dir.resolve("locked");
        Files.setPosixFilePermissions(locked, PosixFilePermissions.fromString("---------"));
        try {
            assumeTrue(!Files.isReadable(locked), "running as root");
            Result r = search(dir, "*.txt", false);
            assertEquals(List.of("open/a.txt"), r.found());
            assertEquals(List.of("locked"), r.failed());
            assertTrue(r.complete(), "the rest is still searched");
        } finally {
            Files.setPosixFilePermissions(locked, PosixFilePermissions.fromString("rwx------"));
        }
    }

    @Test
    void stopsWhenTheListenerHasEnoughOrTheThreadIsInterrupted() throws IOException {
        for (int i = 0; i < 20; i++) {
            tree("f" + i + "/x" + i + ".txt");
        }
        List<FileEntry> found = new ArrayList<>();
        boolean complete = FileSearch.search(dir, FileSearch.matcher("*.txt"), false, new FileSearch.Listener() {
            @Override
            public void found(FileEntry e) {
                found.add(e);
            }

            @Override
            public boolean wantsMore() {
                return found.size() < 5;
            }
        });
        assertFalse(complete);
        assertEquals(5, found.size());

        Thread.currentThread().interrupt();
        try {
            List<FileEntry> none = new ArrayList<>();
            assertFalse(FileSearch.search(dir, FileSearch.matcher("*"), false, none::add));
            assertEquals(List.of(), none);
        } finally {
            Thread.interrupted(); // clear the flag for the next test
        }
    }

    @Test
    void aMissingFolderOrAFileIsAnError() throws IOException {
        tree("file.txt");
        assertThrows(NotDirectoryException.class,
                () -> FileSearch.search(dir.resolve("gone"), FileSearch.matcher("*"), false, e -> { }));
        assertThrows(NotDirectoryException.class,
                () -> FileSearch.search(dir.resolve("file.txt"), FileSearch.matcher("*"), false, e -> { }));
    }
}
