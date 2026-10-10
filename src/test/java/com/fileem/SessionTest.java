package com.fileem;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

class SessionTest {
    @TempDir
    Path dir;

    @Test
    void roundTripsValuesThroughTheFile() {
        Path file = dir.resolve("nested").resolve("session.properties");
        Session s = Session.load(file);
        s.put("text", "a b=c:ü");
        s.put("int", 42);
        s.put("double", 0.25);
        s.put("bool", true);
        s.save();

        assertTrue(Files.isRegularFile(file), "save creates missing parent folders");
        Session loaded = Session.load(file);
        assertEquals("a b=c:ü", loaded.get("text", null));
        assertEquals(42, loaded.getInt("int", 0));
        assertEquals(0.25, loaded.getDouble("double", 0), 1e-9);
        assertTrue(loaded.getBool("bool", false));
    }

    @Test
    void returnsDefaultsForMissingOrMalformedValues() {
        Session s = Session.load(dir.resolve("none.properties"));
        s.put("notANumber", "abc");
        assertEquals(7, s.getInt("missing", 7));
        assertEquals(7, s.getInt("notANumber", 7));
        assertEquals(1.5, s.getDouble("notANumber", 1.5), 1e-9);
        assertFalse(s.getBool("missing", false));
        assertNull(s.get("missing", null));
    }

    @Test
    void removePrefixOnlyRemovesMatchingKeys() {
        Session s = Session.load(dir.resolve("s.properties"));
        s.put("left.tab.0", "/a");
        s.put("left.tab.1", "/b");
        s.put("left.tabs", 2);
        s.removePrefix("left.tab.");
        assertNull(s.get("left.tab.0", null));
        assertNull(s.get("left.tab.1", null));
        assertEquals(2, s.getInt("left.tabs", 0));
    }

    @Test
    void corruptFileStartsEmptyInsteadOfFailing() throws Exception {
        Path file = dir.resolve("bad.properties");
        Files.write(file, new byte[]{'k', '=', '\\', 'u', 'Z', 'Z'});
        Session s = Session.load(file);
        assertEquals("d", s.get("k", "d"));
    }

    @Test
    void saveWritesOnlyWhenSomethingChanged() throws Exception {
        Path file = dir.resolve("session.properties");
        Session s = Session.load(file);
        s.put("a", 1);
        s.save();
        assertTrue(Files.isRegularFile(file));

        Files.delete(file);
        s.put("a", 1); // same value
        s.save();
        assertFalse(Files.exists(file), "unchanged contents are not written again");
        assertNull(s.saveInBackground(), "nothing to write in the background either");

        s.put("a", 2);
        s.save();
        assertEquals(2, Session.load(file).getInt("a", 0));
    }

    @Test
    void backgroundSaveWritesTheFileAndLeavesNoTempFiles() throws Exception {
        Path file = dir.resolve("session.properties");
        Session s = Session.load(file);
        s.put("theme.name", "aurora");
        s.saveInBackground().get(10, TimeUnit.SECONDS);
        assertEquals("aurora", Session.load(file).get("theme.name", null));
        try (Stream<Path> files = Files.list(dir)) {
            assertEquals(List.of(file), files.toList(), "the temp file was renamed into place");
        }
    }

    @Test
    void anOlderBackgroundWriteNeverReplacesANewerSave() throws Exception {
        Path file = dir.resolve("session.properties");
        Session s = Session.load(file);
        s.put("v", "old");
        CompletableFuture<Void> older = s.saveInBackground();
        s.put("v", "new");
        s.save();
        older.get(10, TimeUnit.SECONDS);
        assertEquals("new", Session.load(file).get("v", null));
    }

    @Test
    void aFailedSaveIsRetriedOnTheNextSave() throws Exception {
        Path blocker = Files.writeString(dir.resolve("blocker"), "not a folder");
        Session broken = Session.load(blocker.resolve("session.properties"));
        broken.put("a", 1);
        broken.save(); // parent is a file: fails, reports, does not throw
        Files.delete(blocker);
        broken.save(); // nothing changed, but the last write failed
        assertEquals(1, Session.load(blocker.resolve("session.properties")).getInt("a", 0));
    }
}
