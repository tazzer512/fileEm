package com.fileem;

import com.fileem.Bookmarks.Bookmark;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class BookmarksTest {
    @TempDir
    Path dir;

    private Path sessionFile() {
        return dir.resolve("session.properties");
    }

    private Bookmarks fresh() {
        return new Bookmarks(Session.load(sessionFile()));
    }

    private static List<String> names(Bookmarks b) {
        return b.items().stream().map(Bookmark::name).toList();
    }

    @Test
    void addUsesTheFolderNameAndIgnoresDuplicates() {
        Bookmarks b = fresh();
        assertTrue(b.add(dir.resolve("Projects")));
        assertFalse(b.add(dir.resolve("Projects")), "same folder twice");
        assertFalse(b.add(dir.resolve("x/../Projects")), "same folder, different spelling");
        assertEquals(List.of("Projects"), names(b));
        assertTrue(b.contains(dir.resolve("./Projects")));
    }

    @Test
    void customNamesAreTrimmedAndBlankFallsBackToTheFolderName() {
        Bookmarks b = fresh();
        b.add(dir.resolve("a"), "  Work  ");
        b.add(dir.resolve("b"), "   ");
        assertEquals(List.of("Work", "b"), names(b));
        b.rename(dir.resolve("a"), "Renamed");
        b.rename(dir.resolve("b"), "");
        assertEquals(List.of("Renamed", "b"), names(b));
        b.rename(dir.resolve("unknown"), "ignored");
        assertEquals(2, b.items().size());
    }

    @Test
    void removeAndMove() {
        Bookmarks b = fresh();
        for (String n : List.of("a", "b", "c", "d")) {
            b.add(dir.resolve(n));
        }
        b.move(dir.resolve("c"), -1);
        assertEquals(List.of("a", "c", "b", "d"), names(b));
        b.move(dir.resolve("a"), 10);
        assertEquals(List.of("c", "b", "d", "a"), names(b), "moves are clamped to the end");
        b.move(dir.resolve("b"), -10);
        assertEquals(List.of("b", "c", "d", "a"), names(b), "moves are clamped to the start");
        b.remove(dir.resolve("d"));
        b.remove(dir.resolve("not-there"));
        assertEquals(List.of("b", "c", "a"), names(b));
    }

    @Test
    void everyChangeIsSavedImmediatelyAndRestoredInOrder() throws Exception {
        Bookmarks b = fresh();
        b.add(dir.resolve("one"), "First");
        b.add(dir.resolve("two"));
        b.move(dir.resolve("two"), -1);
        assertTrue(Files.isRegularFile(sessionFile()), "written without an explicit save");

        Bookmarks restored = fresh();
        assertEquals(List.of("two", "First"), names(restored));
        assertEquals(dir.resolve("two"), restored.items().get(0).path());

        restored.remove(dir.resolve("two"));
        assertEquals(List.of("First"), names(fresh()));
    }

    @Test
    void brokenEntriesInTheSessionAreSkipped() {
        Session s = Session.load(sessionFile());
        s.put("bookmarks.count", 4);
        s.put("bookmarks.0.path", dir.resolve("ok").toString());
        s.put("bookmarks.1.name", "no path");
        s.put("bookmarks.2.path", dir.resolve("ok").toString()); // duplicate
        s.put("bookmarks.3.path", "\0invalid");
        assertEquals(List.of("ok"), names(new Bookmarks(s)));
    }

    @Test
    void theListIsReadOnlyFromOutside() {
        Bookmarks b = fresh();
        assertThrows(UnsupportedOperationException.class,
                () -> b.items().add(new Bookmark("x", dir)));
    }
}
