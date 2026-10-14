package com.fileem.fs;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

import static com.fileem.fs.RawFixtures.Ifd;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

class RawReaderTest {
    @TempDir
    Path dir;

    private Path write(String name, byte[] data) throws IOException {
        return Files.write(dir.resolve(name), data);
    }

    private static void assertDecodes(RawReader.Preview p, int w, int h) throws IOException {
        assertNotNull(p);
        assertEquals(w, p.width());
        assertEquals(h, p.height());
        BufferedImage img = ImageIO.read(new ByteArrayInputStream(p.jpeg()));
        assertNotNull(img, "the bytes are the whole JPEG");
        assertEquals(w, img.getWidth());
        assertEquals(h, img.getHeight());
    }

    @Test
    void knowsRawExtensions() {
        for (String ext : new String[]{"cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2", "pef"}) {
            assertTrue(RawReader.isRaw(ext), ext);
            assertEquals(Preview.Kind.RAW, Preview.kindOf(ext, false), ext);
            assertEquals(FileTypes.Category.IMAGE, FileTypes.category(ext, false), ext);
        }
        assertFalse(RawReader.isRaw("jpg"));
        assertFalse(RawReader.isRaw(null));
    }

    /**
     * CR2 keeps a full-size JPEG in IFD0's strip, a small thumbnail in IFD1 and the sensor data as a
     * lossless JPEG; the biggest viewable one is the preview, the smallest big-enough one the thumbnail.
     */
    @Test
    void cr2PicksTheRightEmbeddedJpegAndSkipsTheRawData() throws IOException {
        Path f = write("IMG_1.CR2", RawFixtures.cr2(RawFixtures.jpeg(640, 480), RawFixtures.jpeg(160, 120), 8));
        RawReader.Preview big = RawReader.read(f, Integer.MAX_VALUE);
        assertDecodes(big, 640, 480);
        assertEquals(8, big.orientation());
        assertDecodes(RawReader.read(f, 100), 160, 120);
        assertDecodes(RawReader.read(f, 161), 640, 480);
    }

    @Test
    void bigEndianSubIfdsAsInNikonFiles() throws IOException {
        byte[] jpeg = RawFixtures.jpeg(300, 200);
        Ifd sub = new Ifd().data(0x201, 0x202, jpeg);
        Ifd ifd0 = new Ifd().shortValue(0x112, 6).subIfd(sub);
        RawReader.Preview p = RawReader.read(write("a.nef", RawFixtures.tiff(ByteOrder.BIG_ENDIAN, ifd0)), 0);
        assertDecodes(p, 300, 200);
        assertEquals(6, p.orientation());
    }

    @Test
    void panasonicRw2KeepsItsJpegInATagOfItsOwn() throws IOException {
        Ifd ifd0 = new Ifd().undefined(0x2e, RawFixtures.jpeg(120, 80));
        RawReader.Preview p = RawReader.read(write("a.rw2", RawFixtures.tiff(ByteOrder.LITTLE_ENDIAN, 0x55, ifd0)), 0);
        assertDecodes(p, 120, 80);
        assertEquals(1, p.orientation(), "no orientation: as stored");
    }

    @Test
    void fujifilmRafTakesTheOrientationFromTheJpeg() throws IOException {
        byte[] jpeg = RawFixtures.withExifOrientation(RawFixtures.jpeg(90, 60), 3);
        RawReader.Preview p = RawReader.read(write("a.raf", RawFixtures.raf(jpeg)), 0);
        assertDecodes(p, 90, 60);
        assertEquals(3, p.orientation());
        assertArrayEquals(jpeg, p.jpeg());
    }

    @Test
    void canonCr3PreviewBoxAndOrientation() throws IOException {
        RawReader.Preview p = RawReader.read(write("a.cr3", RawFixtures.cr3(RawFixtures.jpeg(162, 108), 6)), 0);
        assertDecodes(p, 162, 108);
        assertEquals(6, p.orientation());
    }

    @Test
    void filesWithoutAPreviewOrDamagedOnesGiveNullNotAnError() throws IOException {
        assertNull(RawReader.read(write("text.cr2", "not a raw file at all".getBytes()), 0));
        assertNull(RawReader.read(write("empty.nef", new byte[0]), 0));
        // macOS resource forks (._IMG.CR2) are not raws
        assertNull(RawReader.read(write("._IMG.CR2", new byte[]{0, 5, 22, 7, 0, 2, 0, 0, 'M', 'a', 'c'}), 0));
        // only raw sensor data (like a merged HDR DNG): nothing to show
        assertNull(RawReader.read(write("hdr.dng", RawFixtures.tiff(ByteOrder.LITTLE_ENDIAN,
                new Ifd().shortValue(0x103, 7).data(0x111, 0x117, RawFixtures.losslessJpeg(3000, 2000)))), 0));
        // a JPEG pointer past the end of the file
        assertNull(RawReader.read(write("bad.cr2", RawFixtures.tiff(ByteOrder.LITTLE_ENDIAN,
                new Ifd().longValue(0x201, 1_000_000).longValue(0x202, 5000))), 0));
        // cut off halfway through the JPEG
        byte[] whole = RawFixtures.cr2(RawFixtures.jpeg(640, 480), RawFixtures.jpeg(160, 120), 1);
        RawReader.read(write("cut.cr2", Arrays.copyOf(whole, whole.length / 3)), 0); // no exception
        // an IFD that links to itself ends
        Ifd loop = new Ifd().data(0x201, 0x202, RawFixtures.jpeg(50, 40));
        loop.next(loop);
        assertDecodes(RawReader.read(write("loop.cr2", RawFixtures.tiff(ByteOrder.LITTLE_ENDIAN, loop)), 0), 50, 40);
    }

    @Test
    void exifOrientationOfPlainJpegs() {
        byte[] jpeg = RawFixtures.jpeg(20, 10);
        assertEquals(0, RawReader.exifOrientation(jpeg), "no EXIF");
        assertEquals(8, RawReader.exifOrientation(RawFixtures.withExifOrientation(jpeg, 8)));
        assertEquals(0, RawReader.exifOrientation(new byte[]{(byte) 0xff, (byte) 0xd8, (byte) 0xff, (byte) 0xe1, 0, 4}),
                "cut off: no orientation, no exception");
    }
}
