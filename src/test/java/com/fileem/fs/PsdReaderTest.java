package com.fileem.fs;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class PsdReaderTest {
    @TempDir
    Path dir;

    // ---- a minimal PSD writer ----

    /** Channel planes, {@code planes[c][y * rowBytes + x]}. */
    private Path psd(String name, int version, int mode, int depth, int w, int h, byte[][] planes, int compression,
                     byte[] colorData, byte[] resources) throws IOException {
        int rowBytes = (w * depth + 7) / 8;
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream out = new DataOutputStream(bytes);
        out.writeBytes("8BPS");
        out.writeShort(version);
        out.write(new byte[6]);
        out.writeShort(planes.length);
        out.writeInt(h);
        out.writeInt(w);
        out.writeShort(depth);
        out.writeShort(mode);
        out.writeInt(colorData.length);
        out.write(colorData);
        out.writeInt(resources.length);
        out.write(resources);
        if (version == 2) {
            out.writeLong(0); // no layers
        } else {
            out.writeInt(0);
        }
        out.writeShort(compression);
        if (compression == 1) {
            byte[][][] rows = new byte[planes.length][h][];
            for (int c = 0; c < planes.length; c++) {
                for (int y = 0; y < h; y++) {
                    rows[c][y] = packBits(Arrays.copyOfRange(planes[c], y * rowBytes, (y + 1) * rowBytes));
                    if (version == 2) {
                        out.writeInt(rows[c][y].length);
                    } else {
                        out.writeShort(rows[c][y].length);
                    }
                }
            }
            for (byte[][] channel : rows) {
                for (byte[] row : channel) {
                    out.write(row);
                }
            }
        } else if (compression == 0) {
            for (byte[] plane : planes) {
                out.write(plane);
            }
        } else {
            out.write(new byte[]{1, 2, 3, 4}); // pretend ZIP data
        }
        return Files.write(dir.resolve(name), bytes.toByteArray());
    }

    private Path psd(String name, int mode, int depth, int w, int h, byte[][] planes, int compression) throws IOException {
        return psd(name, 1, mode, depth, w, h, planes, compression, new byte[0], new byte[0]);
    }

    /** PackBits with both runs (3+ equal bytes) and literals, to exercise both decoder branches. */
    private static byte[] packBits(byte[] row) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        int i = 0;
        while (i < row.length) {
            int run = 1;
            while (i + run < row.length && row[i + run] == row[i] && run < 128) {
                run++;
            }
            if (run >= 3) {
                out.write(1 - run);
                out.write(row[i]);
                i += run;
            } else {
                int start = i;
                while (i < row.length && i - start < 128
                        && !(i + 2 < row.length && row[i] == row[i + 1] && row[i] == row[i + 2])) {
                    i++;
                }
                out.write(i - start - 1);
                out.write(row, start, i - start);
            }
        }
        return out.toByteArray();
    }

    /** One plane of {@code w}×{@code h} with value {@code f(x, y)}. */
    private static byte[] plane(int w, int h, java.util.function.IntBinaryOperator f) {
        byte[] p = new byte[w * h];
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                p[y * w + x] = (byte) f.applyAsInt(x, y);
            }
        }
        return p;
    }

    private static byte[] jpegThumbnailResource() throws IOException {
        java.awt.image.BufferedImage img = new java.awt.image.BufferedImage(16, 8, java.awt.image.BufferedImage.TYPE_INT_RGB);
        ByteArrayOutputStream jpg = new ByteArrayOutputStream();
        javax.imageio.ImageIO.write(img, "jpg", jpg);
        byte[] data = jpg.toByteArray();
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream out = new DataOutputStream(bytes);
        out.writeBytes("8BIM");
        out.writeShort(1036);
        out.writeShort(0); // empty name, padded to even
        out.writeInt(28 + data.length);
        out.writeInt(1); // kJpegRGB
        out.writeInt(16);
        out.writeInt(8);
        out.writeInt(48);
        out.writeInt(16 * 8 * 3);
        out.writeInt(data.length);
        out.writeShort(24);
        out.writeShort(1);
        out.write(data);
        if (data.length % 2 == 1) {
            out.write(0);
        }
        return bytes.toByteArray();
    }

    private static int rgb(int r, int g, int b) {
        return 0xff000000 | r << 16 | g << 8 | b;
    }

    // ---- tests ----

    @Test
    void rawRgbAtFullSize() throws IOException {
        int w = 4, h = 3;
        Path p = psd("raw.psd", 3, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> x * 60), plane(w, h, (x, y) -> y * 100), plane(w, h, (x, y) -> 7)}, 0);
        PsdReader.Preview pv = PsdReader.read(p, 256);
        assertEquals(4, pv.width());
        assertEquals(3, pv.height());
        assertEquals(rgb(0, 0, 7), pv.argb()[0]);
        assertEquals(rgb(180, 200, 7), pv.argb()[2 * 4 + 3]);
    }

    @Test
    void rleRgbIsScaledDownReadingOnlyTheRowsItNeeds() throws IOException {
        int w = 200, h = 100;
        Path p = psd("rle.psd", 3, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> x < 100 ? 255 : 0), plane(w, h, (x, y) -> 0), plane(w, h, (x, y) -> x < 100 ? 0 : 255)}, 1);
        PsdReader.Preview pv = PsdReader.read(p, 50);
        assertEquals(50, pv.width());
        assertEquals(25, pv.height());
        assertEquals(rgb(255, 0, 0), pv.argb()[10 * 50 + 5], "left half red");
        assertEquals(rgb(0, 0, 255), pv.argb()[10 * 50 + 45], "right half blue");
    }

    @Test
    void aFourthRgbChannelIsTransparency() throws IOException {
        int w = 2, h = 1;
        Path p = psd("alpha.psd", 3, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> 255), plane(w, h, (x, y) -> 255), plane(w, h, (x, y) -> 255),
                plane(w, h, (x, y) -> x == 0 ? 0 : 128)}, 1);
        int[] px = PsdReader.read(p, 10).argb();
        assertEquals(0x00ffffff, px[0]);
        assertEquals(0x80ffffff, px[1]);
    }

    @Test
    void sixteenBitGrayscaleUsesTheHighByte() throws IOException {
        int w = 2, h = 1;
        byte[] plane = {(byte) 0x12, (byte) 0x34, (byte) 0xfe, (byte) 0xdc}; // big-endian 0x1234, 0xfedc
        int[] px = PsdReader.read(psd("gray16.psd", 1, 16, w, h, new byte[][]{plane}, 0), 10).argb();
        assertEquals(rgb(0x12, 0x12, 0x12), px[0]);
        assertEquals(rgb(0xfe, 0xfe, 0xfe), px[1]);
    }

    @Test
    void cmykIsStoredInvertedAndConvertedToRgb() throws IOException {
        int w = 2, h = 1;
        Path p = psd("cmyk.psd", 4, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> 255), plane(w, h, (x, y) -> 255), plane(w, h, (x, y) -> 255),
                plane(w, h, (x, y) -> x == 0 ? 255 : 0)}, 1);
        int[] px = PsdReader.read(p, 10).argb();
        assertEquals(rgb(255, 255, 255), px[0], "no ink: white");
        assertEquals(rgb(0, 0, 0), px[1], "full black ink");
    }

    @Test
    void indexedColorsComeFromThePalette() throws IOException {
        byte[] palette = new byte[768];
        palette[3] = (byte) 10;          // index 3: red 10
        palette[256 + 3] = (byte) 20;    // green 20
        palette[512 + 3] = (byte) 30;    // blue 30
        Path p = psd("indexed.psd", 1, 2, 8, 1, 1, new byte[][]{{3}}, 0, palette, new byte[0]);
        assertEquals(rgb(10, 20, 30), PsdReader.read(p, 10).argb()[0]);
    }

    @Test
    void bigDocumentsInPsbFormatAreReadToo() throws IOException {
        int w = 3, h = 2;
        Path p = psd("big.psb", 2, 3, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> 1), plane(w, h, (x, y) -> 2), plane(w, h, (x, y) -> 3)}, 1, new byte[0], new byte[0]);
        assertEquals(rgb(1, 2, 3), PsdReader.read(p, 10).argb()[5]);
    }

    @Test
    void withoutAUsableCompositeTheEmbeddedJpegIsUsed() throws IOException {
        int w = 2, h = 2;
        byte[] thumb = jpegThumbnailResource();
        Path zip = psd("zip.psd", 1, 3, 8, w, h, new byte[][]{new byte[4], new byte[4], new byte[4]}, 2, new byte[0], thumb);
        PsdReader.Preview pv = PsdReader.read(zip, 64);
        assertNull(pv.argb());
        assertNotNull(pv.jpeg());
        assertTrue(pv.jpeg()[0] == (byte) 0xff && pv.jpeg()[1] == (byte) 0xd8, "a JPEG");

        // a blank composite (saved without "maximize compatibility") also prefers the thumbnail
        Path blank = psd("blank.psd", 1, 3, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> 255), plane(w, h, (x, y) -> 255), plane(w, h, (x, y) -> 255)}, 0, new byte[0], thumb);
        assertNotNull(PsdReader.read(blank, 64).jpeg());
        // ... but a genuinely plain image without a thumbnail is still shown
        Path plain = psd("plain.psd", 3, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> 9), plane(w, h, (x, y) -> 9), plane(w, h, (x, y) -> 9)}, 0);
        assertEquals(rgb(9, 9, 9), PsdReader.read(plain, 64).argb()[0]);
    }

    @Test
    void damagedOrUnsupportedFilesGiveNoPreviewInsteadOfGarbage() throws IOException {
        int w = 10, h = 10;
        Path good = psd("cut.psd", 3, 8, w, h, new byte[][]{
                plane(w, h, (x, y) -> x), plane(w, h, (x, y) -> y), plane(w, h, (x, y) -> 1)}, 1);
        byte[] all = Files.readAllBytes(good);
        Path cut = Files.write(dir.resolve("truncated.psd"), Arrays.copyOf(all, all.length - 40));
        assertNull(PsdReader.read(cut, 10), "truncated image data");

        Path lab = psd("lab.psd", 9, 8, 1, 1, new byte[][]{{1}, {2}, {3}}, 0);
        assertNull(PsdReader.read(lab, 10), "Lab without a thumbnail");

        assertThrows(IOException.class, () -> PsdReader.read(Files.writeString(dir.resolve("fake.psd"), "not a psd"), 10));
        Path huge = psd("huge.psd", 3, 8, 1, 1, new byte[][]{{1}, {1}, {1}}, 0);
        byte[] h2 = Files.readAllBytes(huge);
        h2[14] = 0x7f; // height 2 billion
        assertThrows(IOException.class, () -> PsdReader.read(Files.write(dir.resolve("huge2.psd"), h2), 10));
    }

    @Test
    void psdAndPsbAreRecognised() {
        assertTrue(PsdReader.isPsd("psd"));
        assertTrue(PsdReader.isPsd("psb"));
        assertArrayEquals(new boolean[]{false, false}, new boolean[]{PsdReader.isPsd("png"), PsdReader.isPsd(null)});
    }
}
