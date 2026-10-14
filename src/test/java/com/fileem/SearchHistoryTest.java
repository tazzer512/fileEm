package com.fileem;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.stream.IntStream;

import static org.junit.jupiter.api.Assertions.assertEquals;

class SearchHistoryTest {
    @TempDir
    Path dir;

    @Test
    void newestFirstAndAPatternUsedAgainMovesUp() {
        SearchHistory h = new SearchHistory(Session.load(dir.resolve("s.properties")));
        assertEquals(List.of(), h.items());
        h.add("*.jpg");
        h.add("  a*.* !*.mp3 ");
        h.add("");
        h.add("   ");
        h.add(null);
        assertEquals(List.of("a*.* !*.mp3", "*.jpg"), h.items(), "trimmed, blanks ignored");
        h.add("*.jpg");
        assertEquals(List.of("*.jpg", "a*.* !*.mp3"), h.items(), "no duplicates");
        h.add("*.JPG");
        assertEquals(List.of("*.JPG", "*.jpg", "a*.* !*.mp3"), h.items(), "as typed: case is kept apart");
    }

    @Test
    void keepsTheLast15() {
        SearchHistory h = new SearchHistory(Session.load(dir.resolve("s.properties")));
        IntStream.rangeClosed(1, 20).forEach(i -> h.add("p" + i));
        assertEquals(15, h.items().size());
        assertEquals("p20", h.items().get(0));
        assertEquals("p6", h.items().get(14));
    }

    @Test
    void survivesARestartThroughTheSession() throws Exception {
        Path file = dir.resolve("s.properties");
        Session session = Session.load(file);
        SearchHistory h = new SearchHistory(session);
        IntStream.rangeClosed(1, 17).forEach(i -> h.add("p" + i));
        h.add("p3 \"with space\"");
        session.save();
        assertEquals(h.items(), new SearchHistory(Session.load(file)).items());

        // a hand-edited or damaged file: duplicates and blanks are dropped, the rest kept in order
        Files.writeString(file, "search.history.0=b\nsearch.history.1=\nsearch.history.2=b\nsearch.history.3=a\n");
        assertEquals(List.of("b", "a"), new SearchHistory(Session.load(file)).items());
    }
}
