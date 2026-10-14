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

public class ImageIoReaderTest {
    @TempDir
    Path dir;

    /**
     * A fixture copied into the temp folder. They were made with OpenJPEG (through Pillow, lossless):
     * jai-imageio's own writer produces wrong colours, so it can't make trustworthy test files.
     * red-blue: 120×80, left half red, right half blue (also as a bare .j2k codestream); big: the
     * same at 1000×600; gray: 8-bit grey 200; gray16: 16-bit grey 51400 (= 200 in 8 bits).
     */
    public static Path fixture(Path dir, String name) throws IOException {
        try (InputStream in = ImageIoReaderTest.class.getResourceAsStream("jpeg2000/" + name)) {
            assertNotNull(in, name);
            Path f = dir.resolve(name);
            Files.copy(in, f);
            return f;
        }
    }

    private static int pixel(ImageIoReader.Pixels p, int x, int y) {
        return p.argb()[y * p.width() + x] & 0xffffff;
    }

    @Test
    void knowsTheExtensions() {
        for (String ext : new String[]{"jp2", "j2k", "J2K", "j2c", "jpc", "jpf", "jpx"}) {
            assertTrue(ImageIoReader.isJpeg2000(ext), ext);
            assertEquals(Preview.Kind.IMAGEIO, Preview.kindOf(ext.toLowerCase(), false), ext);
            assertEquals(FileTypes.Category.IMAGE, FileTypes.category(ext.toLowerCase(), false), ext);
        }
        assertFalse(ImageIoReader.isJpeg2000("jpg"));
        assertFalse(ImageIoReader.isJpeg2000(null));
    }

    @Test
    void readsJp2FilesAndBareCodestreams() throws IOException {
        for (boolean codestream : new boolean[]{false, true}) {
            Path f = fixture(dir, codestream ? "red-blue.j2k" : "red-blue.jp2");
            ImageIoReader.Pixels p = ImageIoReader.read(f, 4096);
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
        assertEquals(5, ImageIoReader.decompositionLevels(f), "OpenJPEG's default: 1000, 500, 250 … px");
        assertEquals(5, ImageIoReader.decompositionLevels(fixture(dir, "red-blue.j2k")), "bare codestream too");
        ImageIoReader.Pixels p = ImageIoReader.read(f, 256);
        assertEquals(256, p.width(), "the 500 px level, shrunk to the limit");
        assertEquals(154, p.height());
        assertEquals(1000, p.fullWidth());
        assertEquals(600, p.fullHeight());
        assertEquals(0xffff0000, p.argb()[77 * 256 + 5], "left is red");
        assertEquals(0xff0000ff, p.argb()[77 * 256 + 250], "right is blue");

        ImageIoReader.Pixels natural = ImageIoReader.read(f, 5000);
        assertEquals(1000, natural.width(), "never enlarged");
        assertEquals(0, ImageIoReader.decompositionLevels(Files.writeString(dir.resolve("x.jp2"), "text")));
    }

    @Test
    void shrinkingAveragesThePixelsUnderEachTargetPixel() {
        int[] src = {
                0xff000000, 0xff0000ff, 0xffffffff, 0xffffffff,
                0xff000000, 0xff0000ff, 0x00ffffff, 0x00ffffff};
        int[] out = ImageIoReader.shrink(src, 4, 2, 2, 1);
        assertEquals(0xff00007f, out[0], "black and blue: half blue");
        assertEquals(0x7fffffff, out[1], "half transparent white");
    }

    @Test
    void greyImagesKeepTheirValuesAt8And16Bits() throws IOException {
        for (String name : new String[]{"gray.jp2", "gray16.jp2"}) {
            ImageIoReader.Pixels p = ImageIoReader.read(fixture(dir, name), 4096);
            assertNotNull(p, name);
            assertEquals(64, p.width());
            assertEquals(0xffc8c8c8, p.argb()[100], name + ": grey 200, opaque, not brightened to 229");
        }
    }

    /** A Netpbm file: the text header, then the given pixel bytes. */
    private Path netpbm(String name, String header, byte[] pixels) throws IOException {
        byte[] head = header.getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        byte[] all = Arrays.copyOf(head, head.length + pixels.length);
        System.arraycopy(pixels, 0, all, head.length, pixels.length);
        return Files.write(dir.resolve(name), all);
    }

    @Test
    void netpbmImagesInTheirBinaryTextGreyAndBitmapForms() throws IOException {
        for (String ext : new String[]{"ppm", "PGM", "pbm", "pnm"}) {
            assertTrue(ImageIoReader.isNetpbm(ext), ext);
            assertTrue(ImageIoReader.handles(ext), ext);
            assertEquals(Preview.Kind.IMAGEIO, Preview.kindOf(ext.toLowerCase(), false), ext);
            assertEquals(FileTypes.Category.IMAGE, FileTypes.category(ext.toLowerCase(), false), ext);
        }
        assertFalse(ImageIoReader.isNetpbm("jp2"));

        // binary PPM (P6): 2×1, red then blue
        assertTwoPixels(ImageIoReader.read(netpbm("a.ppm", "P6\n2 1\n255\n", new byte[]{(byte) 255, 0, 0, 0, 0, (byte) 255}), 100),
                0xffff0000, 0xff0000ff);
        // text PPM (P3) with a comment
        assertTwoPixels(ImageIoReader.read(Files.writeString(dir.resolve("t.ppm"), "P3\n# made by hand\n2 1\n255\n255 0 0  0 0 255\n"), 100),
                0xffff0000, 0xff0000ff);
        // grey PGM (P5), 8 and 16 bits: values kept, not brightened
        assertTwoPixels(ImageIoReader.read(netpbm("g.pgm", "P5\n2 1\n255\n", new byte[]{(byte) 200, 0}), 100),
                0xffc8c8c8, 0xff000000);
        assertTwoPixels(ImageIoReader.read(netpbm("g16.pgm", "P5\n2 1\n65535\n", new byte[]{(byte) 0xc8, (byte) 0xc8, 0, 0}), 100),
                0xffc8c8c8, 0xff000000);
        // bitmap PBM (P4): 1 is black
        assertTwoPixels(ImageIoReader.read(netpbm("b.pbm", "P4\n2 1\n", new byte[]{(byte) 0b0100_0000}), 100),
                0xffffffff, 0xff000000);
    }

    @Test
    void netpbmTextBitmapsAndGreyAndDamagedFiles() throws IOException {
        // P1 pixels need no spaces between them; P2 is text grey with a maxval of 15
        assertTwoPixels(ImageIoReader.read(Files.writeString(dir.resolve("t.pbm"), "P1 2 1\n10"), 100),
                0xff000000, 0xffffffff);
        assertTwoPixels(ImageIoReader.read(Files.writeString(dir.resolve("t.pgm"), "P2 2 1 15\n15 0"), 100),
                0xffffffff, 0xff000000);

        assertNull(ImageIoReader.read(netpbm("cut.ppm", "P6\n4 4\n255\n", new byte[10]), 100), "cut off");
        assertNull(ImageIoReader.read(Files.writeString(dir.resolve("bad.ppm"), "P9\n1 1\n255\n"), 100), "no such type");
        assertNull(ImageIoReader.read(Files.writeString(dir.resolve("zero.pgm"), "P5\n1 1\n0\n "), 100), "maxval 0");
        assertNull(ImageIoReader.read(Files.writeString(dir.resolve("huge.ppm"), "P6\n2000000000 2000000000\n255\n"), 100),
                "a damaged header can't make it allocate gigabytes");
        assertNull(ImageIoReader.read(Files.writeString(dir.resolve("text.pnm"), "hello"), 100));
        assertNull(ImageIoReader.read(Files.write(dir.resolve("empty.pbm"), new byte[0]), 100));
    }

    @Test
    void bigNetpbmImagesAreDecodedSmaller() throws IOException {
        int w = 900, h = 300;
        byte[] px = new byte[w * h * 3];
        for (int i = 0; i < w * h; i++) {
            px[i * 3] = (byte) (i % w < w / 2 ? 255 : 0);
            px[i * 3 + 2] = (byte) (i % w < w / 2 ? 0 : 255);
        }
        ImageIoReader.Pixels p = ImageIoReader.read(netpbm("big.ppm", "P6\n" + w + " " + h + "\n255\n", px), 128);
        assertNotNull(p);
        assertEquals(128, p.width());
        assertEquals(43, p.height());
        assertEquals(900, p.fullWidth());
        assertEquals(0xffff0000, p.argb()[20 * 128 + 3], "left is red");
        assertEquals(0xff0000ff, p.argb()[20 * 128 + 124], "right is blue");
    }

    /** {@code p} is 2×1 with these two pixels. */
    private static void assertTwoPixels(ImageIoReader.Pixels p, int left, int right) {
        assertNotNull(p);
        assertEquals(2, p.width());
        assertEquals(1, p.height());
        assertEquals(Integer.toHexString(left), Integer.toHexString(p.argb()[0]), "left");
        assertEquals(Integer.toHexString(right), Integer.toHexString(p.argb()[1]), "right");
    }

    @Test
    void damagedOrOtherFilesGiveNullNotAnError() throws IOException {
        assertNull(ImageIoReader.read(Files.writeString(dir.resolve("text.jp2"), "not an image"), 100));
        assertNull(ImageIoReader.read(Files.write(dir.resolve("empty.j2k"), new byte[0]), 100));
        byte[] whole = Files.readAllBytes(fixture(dir, "big.jp2"));
        Path cut = Files.write(dir.resolve("cut.jp2"), Arrays.copyOf(whole, 60));
        assertNull(ImageIoReader.read(cut, 100), "header only");
    }
}
