package com.fileem;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;

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
}
