package com.fileem.fs;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

class PreviewTest {
    @TempDir
    Path dir;

    @Test
    void kindsByName() {
        assertEquals(Preview.Kind.FOLDER, Preview.kindOf("", true));
        assertEquals(Preview.Kind.IMAGE, Preview.kindOf("png", false));
        assertEquals(Preview.Kind.IMAGE, Preview.kindOf("jpeg", false));
        assertEquals(Preview.Kind.PSD, Preview.kindOf("psd", false));
        assertEquals(Preview.Kind.TEXT, Preview.kindOf("java", false));
        assertEquals(Preview.Kind.TEXT, Preview.kindOf("md", false));
        assertEquals(Preview.Kind.OTHER, Preview.kindOf("zip", false));
        assertEquals(Preview.Kind.OTHER, Preview.kindOf(null, false));
    }

    @Test
    void readsTextAndRefusesBinary() throws IOException {
        Path text = Files.writeString(dir.resolve("a.txt"), "hello\nwörld\n");
        Preview.Text t = Preview.readText(text, 1000);
        assertEquals("hello\nwörld\n", t.content());
        assertFalse(t.truncated());

        Path bin = Files.write(dir.resolve("b.bin"), new byte[]{1, 2, 0, 4, 5});
        assertNull(Preview.readText(bin, 1000), "NUL bytes: binary");
        byte[] controls = new byte[100];
        Arrays.fill(controls, (byte) 3);
        assertNull(Preview.readText(Files.write(dir.resolve("c.bin"), controls), 1000), "control characters: binary");
        assertNull(Preview.readText(dir, 1000), "folders are not text");
    }

    @Test
    void longFilesAreCutAtTheLimitWithoutBreakingACharacter() throws IOException {
        // "é" is two bytes in UTF-8; a limit of 5 bytes falls in the middle of the third one
        Path p = Files.write(dir.resolve("long.txt"), "ééé".getBytes(StandardCharsets.UTF_8));
        Preview.Text t = Preview.readText(p, 5);
        assertTrue(t.truncated());
        assertEquals("éé", t.content());
    }

    @Test
    void nonUtf8TextFallsBackToLatin1AndTheBomIsDropped() throws IOException {
        Path latin = Files.write(dir.resolve("latin.txt"), "café".getBytes(StandardCharsets.ISO_8859_1));
        assertEquals("café", Preview.readText(latin, 100).content());
        byte[] bom = {(byte) 0xef, (byte) 0xbb, (byte) 0xbf, 'h', 'i'};
        assertEquals("hi", Preview.readText(Files.write(dir.resolve("bom.txt"), bom), 100).content());
        assertNotNull(Preview.readText(Files.write(dir.resolve("empty.txt"), new byte[0]), 100), "empty is text");
    }
}
