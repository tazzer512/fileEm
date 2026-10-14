package com.fileem.fs;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class Jpeg2000ReaderTest {
    @TempDir
    Path dir;

    /**
     * A fixture copied into the temp folder. They were made with OpenJPEG (through Pillow, lossless):
     * jai-imageio's own writer produces wrong colours, so it can't make trustworthy test files.
     * red-blue: 120×80, left half red, right half blue (also as a bare .j2k codestream); big: the
     * same at 1000×600; gray: 8-bit grey 200; gray16: 16-bit grey 51400 (= 200 in 8 bits).
     */
    public static Path fixture(Path dir, String name) throws IOException {
        try (InputStream in = Jpeg2000ReaderTest.class.getResourceAsStream("jpeg2000/" + name)) {
            assertNotNull(in, name);
            Path f = dir.resolve(name);
            Files.copy(in, f);
            return f;
        }
    }

    private static int pixel(Jpeg2000Reader.Pixels p, int x, int y) {
        return p.argb()[y * p.width() + x] & 0xffffff;
    }

    @Test
    void knowsTheExtensions() {
        for (String ext : new String[]{"jp2", "j2k", "J2K", "j2c", "jpc", "jpf", "jpx"}) {
            assertTrue(Jpeg2000Reader.isJpeg2000(ext), ext);
            assertEquals(Preview.Kind.JPEG2000, Preview.kindOf(ext.toLowerCase(), false), ext);
            assertEquals(FileTypes.Category.IMAGE, FileTypes.category(ext.toLowerCase(), false), ext);
        }
        assertFalse(Jpeg2000Reader.isJpeg2000("jpg"));
        assertFalse(Jpeg2000Reader.isJpeg2000(null));
    }

    @Test
    void readsJp2FilesAndBareCodestreams() throws IOException {
        for (boolean codestream : new boolean[]{false, true}) {
            Path f = fixture(dir, codestream ? "red-blue.j2k" : "red-blue.jp2");
            Jpeg2000Reader.Pixels p = Jpeg2000Reader.read(f, 4096);
            assertNotNull(p, f.toString());
            assertEquals(120, p.width());
            assertEquals(80, p.height());
            assertEquals(0xff0000, pixel(p, 10, 40), "left is red");
            assertEquals(0x0000ff, pixel(p, 110, 40), "right is blue");
        }
    }

    @Test
    void bigImagesDecodeAStoredSmallerResolutionThenShrinkToTheLimit() throws IOException {
        Path f = fixture(dir, "big.jp2");
        assertEquals(5, Jpeg2000Reader.decompositionLevels(f), "OpenJPEG's default: 1000, 500, 250 … px");
        assertEquals(5, Jpeg2000Reader.decompositionLevels(fixture(dir, "red-blue.j2k")), "bare codestream too");
        Jpeg2000Reader.Pixels p = Jpeg2000Reader.read(f, 256);
        assertEquals(256, p.width(), "the 500 px level, shrunk to the limit");
        assertEquals(154, p.height());
        assertEquals(1000, p.fullWidth());
        assertEquals(600, p.fullHeight());
        assertEquals(0xffff0000, p.argb()[77 * 256 + 5], "left is red");
        assertEquals(0xff0000ff, p.argb()[77 * 256 + 250], "right is blue");

        Jpeg2000Reader.Pixels natural = Jpeg2000Reader.read(f, 5000);
        assertEquals(1000, natural.width(), "never enlarged");
        assertEquals(0, Jpeg2000Reader.decompositionLevels(Files.writeString(dir.resolve("x.jp2"), "text")));
    }

    @Test
    void shrinkingAveragesThePixelsUnderEachTargetPixel() {
        int[] src = {
                0xff000000, 0xff0000ff, 0xffffffff, 0xffffffff,
                0xff000000, 0xff0000ff, 0x00ffffff, 0x00ffffff};
        int[] out = Jpeg2000Reader.shrink(src, 4, 2, 2, 1);
        assertEquals(0xff00007f, out[0], "black and blue: half blue");
        assertEquals(0x7fffffff, out[1], "half transparent white");
    }

    @Test
    void greyImagesKeepTheirValuesAt8And16Bits() throws IOException {
        for (String name : new String[]{"gray.jp2", "gray16.jp2"}) {
            Jpeg2000Reader.Pixels p = Jpeg2000Reader.read(fixture(dir, name), 4096);
            assertNotNull(p, name);
            assertEquals(64, p.width());
            assertEquals(0xffc8c8c8, p.argb()[100], name + ": grey 200, opaque, not brightened to 229");
        }
    }

    @Test
    void damagedOrOtherFilesGiveNullNotAnError() throws IOException {
        assertNull(Jpeg2000Reader.read(Files.writeString(dir.resolve("text.jp2"), "not an image"), 100));
        assertNull(Jpeg2000Reader.read(Files.write(dir.resolve("empty.j2k"), new byte[0]), 100));
        byte[] whole = Files.readAllBytes(fixture(dir, "big.jp2"));
        Path cut = Files.write(dir.resolve("cut.jp2"), Arrays.copyOf(whole, 60));
        assertNull(Jpeg2000Reader.read(cut, 100), "header only");
    }
}
