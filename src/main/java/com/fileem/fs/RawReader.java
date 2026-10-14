package com.fileem.fs;

import java.io.EOFException;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.FileChannel;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * Previews of camera raw files. Raw sensor data needs a full raw developer, but cameras embed ready
 * JPEG previews (often full size) in the file; this finds them without reading the rest. TIFF based
 * raws (Canon CR2, Nikon NEF, Sony ARW, Adobe DNG, Pentax PEF, Olympus ORF, Panasonic RW2, Samsung SRW,
 * …): JPEGs referenced from any IFD, sub-IFD or RW2's JpgFromRaw tag. Canon CR3 (ISO media boxes): the
 * PRVW preview. Fujifilm RAF: the JPEG its header points to. Only reads headers and the chosen JPEG;
 * JPEGs a normal decoder can't show (lossless raw data, which also starts like a JPEG) are skipped.
 */
public final class RawReader {
    private static final Set<String> EXTENSIONS = Set.of(
            "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "pef", "orf", "rw2", "raf", "srw",
            "erf", "kdc", "dcr", "3fr", "mef", "mos", "iiq", "rwl", "raw");
    /** Embedded JPEGs bigger than this are not believed (damaged offsets). */
    private static final int MAX_JPEG = 64 << 20;
    private static final int MAX_IFDS = 64;
    private static final int MAX_ENTRIES = 1000;
    private static final byte[] CR3_PREVIEW_UUID = hex("eaf42b5e1c984b88b9fbb7dc406e4d16");
    private static final byte[] CR3_CANON_UUID = hex("85c0b687820f11e08111f4ce462b6a48");

    private RawReader() {
    }

    /** Camera raw extensions this reader tries. */
    public static boolean isRaw(String extension) {
        return extension != null && EXTENSIONS.contains(extension);
    }

    /**
     * An embedded JPEG: its bytes and pixel size, plus the orientation the photo was taken in (EXIF
     * values: 1 = as stored, 3 = upside down, 6 = turn 90° clockwise, 8 = 90° counterclockwise, 2/4/5/7
     * mirrored).
     */
    public record Preview(byte[] jpeg, int width, int height, int orientation) {
    }

    private record Candidate(long offset, int length, int width, int height) {
        long pixels() {
            return (long) width * height;
        }
    }

    /**
     * The smallest embedded JPEG at least {@code minSide} pixels on its longer side, or the largest one
     * when none is that big; null when the file has none this reader can find.
     */
    public static Preview read(Path file, int minSide) throws IOException {
        try (FileChannel ch = FileChannel.open(file, StandardOpenOption.READ)) {
            Source src = new FileSource(ch);
            List<Candidate> found = new ArrayList<>();
            int orientation = 0; // unknown: then the JPEG's own EXIF says
            byte[] head = src.bytes(0, (int) Math.min(16, src.size()));
            if (startsWith(head, "FUJIFILMCCD-RAW")) {
                raf(src, found);
            } else if (head.length >= 12 && startsWith(Arrays.copyOfRange(head, 4, 12), "ftypcrx ")) {
                orientation = cr3(src, found);
            } else {
                Tiff tiff = Tiff.open(src, 0);
                if (tiff == null) {
                    return null;
                }
                orientation = tiff.collect(found);
            }
            Candidate best = choose(found, minSide);
            if (best == null) {
                return null;
            }
            byte[] jpeg = src.bytes(best.offset, best.length);
            if (orientation == 0) {
                orientation = exifOrientation(jpeg);
            }
            return new Preview(jpeg, best.width, best.height, orientation == 0 ? 1 : orientation);
        }
    }

    private static Candidate choose(List<Candidate> found, int minSide) {
        Candidate smallestBigEnough = null;
        Candidate largest = null;
        for (Candidate c : found) {
            if (largest == null || c.pixels() > largest.pixels()) {
                largest = c;
            }
            if (Math.max(c.width, c.height) >= minSide
                    && (smallestBigEnough == null || c.pixels() < smallestBigEnough.pixels())) {
                smallestBigEnough = c;
            }
        }
        return smallestBigEnough != null ? smallestBigEnough : largest;
    }

    /** Adds the JPEG at {@code offset} when it is one a normal decoder shows (baseline/progressive). */
    private static void addJpeg(Source src, long offset, long length, List<Candidate> found) throws IOException {
        if (offset <= 0 || length < 4 || length > MAX_JPEG || offset + length > src.size()) {
            return;
        }
        for (Candidate c : found) {
            if (c.offset == offset) {
                return;
            }
        }
        int[] size;
        try {
            size = jpegSize(src, offset, offset + length);
        } catch (java.io.InterruptedIOException e) {
            throw e;
        } catch (IOException e) {
            return; // cut off or damaged: not a usable preview
        }
        if (size != null) {
            found.add(new Candidate(offset, (int) length, size[0], size[1]));
        }
    }

    /**
     * Width and height from a JPEG's frame header, or null when it isn't a JPEG or uses a coding
     * decoders don't support (lossless, hierarchical, arithmetic).
     */
    static int[] jpegSize(Source src, long start, long end) throws IOException {
        byte[] soi = src.bytes(start, 2);
        if ((soi[0] & 0xff) != 0xff || (soi[1] & 0xff) != 0xd8) {
            return null;
        }
        long pos = start + 2;
        for (int i = 0; i < 1000 && pos + 4 <= end; i++) {
            byte[] m = src.bytes(pos, 4);
            if ((m[0] & 0xff) != 0xff) {
                return null;
            }
            int marker = m[1] & 0xff;
            if (marker == 0xff) { // fill byte
                pos++;
                continue;
            }
            if (marker == 0x01 || marker >= 0xd0 && marker <= 0xd7) {
                pos += 2;
                continue;
            }
            int len = ((m[2] & 0xff) << 8) | (m[3] & 0xff);
            if (marker >= 0xc0 && marker <= 0xcf && marker != 0xc4 && marker != 0xc8 && marker != 0xcc) {
                if (marker > 0xc2) {
                    return null;
                }
                byte[] sof = src.bytes(pos + 5, 4);
                int h = ((sof[0] & 0xff) << 8) | (sof[1] & 0xff);
                int w = ((sof[2] & 0xff) << 8) | (sof[3] & 0xff);
                return w > 0 && h > 0 ? new int[]{w, h} : null;
            }
            if (marker == 0xda || marker == 0xd9 || len < 2) {
                return null; // image data or the end before any frame header
            }
            pos += 2 + len;
        }
        return null;
    }

    /** The EXIF orientation inside a JPEG (APP1 "Exif"), or 0 when it has none. */
    static int exifOrientation(byte[] jpeg) {
        try {
            Source src = new BytesSource(jpeg);
            int pos = 2;
            for (int i = 0; i < 64 && pos + 4 <= jpeg.length; i++) {
                if ((jpeg[pos] & 0xff) != 0xff) {
                    return 0;
                }
                int marker = jpeg[pos + 1] & 0xff;
                int len = ((jpeg[pos + 2] & 0xff) << 8) | (jpeg[pos + 3] & 0xff);
                if (marker == 0xe1 && len > 8 && startsWith(Arrays.copyOfRange(jpeg, pos + 4, pos + 10), "Exif\0\0")) {
                    Tiff t = Tiff.open(src, pos + 10);
                    return t == null ? 0 : t.orientation();
                }
                if (marker == 0xda || len < 2) {
                    return 0;
                }
                pos += 2 + len;
            }
        } catch (IOException | RuntimeException e) {
            return 0;
        }
        return 0;
    }

    // ---- Fujifilm RAF ----

    private static void raf(Source src, List<Candidate> found) throws IOException {
        ByteBuffer b = ByteBuffer.wrap(src.bytes(84, 8)).order(ByteOrder.BIG_ENDIAN);
        addJpeg(src, Integer.toUnsignedLong(b.getInt()), Integer.toUnsignedLong(b.getInt()), found);
    }

    // ---- Canon CR3 ----

    /** Finds the PRVW preview; returns the orientation from the CMT1 (TIFF IFD0) box, or 0. */
    private static int cr3(Source src, List<Candidate> found) throws IOException {
        int[] orientation = {0};
        boxes(src, 0, src.size(), 0, (type, start, contentStart, end, depth) -> {
            if (type.equals("moov")) {
                return true; // look inside for Canon's uuid box
            }
            if (type.equals("uuid") && end - contentStart >= 16) {
                byte[] uuid = src.bytes(contentStart, 16);
                if (Arrays.equals(uuid, CR3_CANON_UUID)) {
                    boxes(src, contentStart + 16, end, depth + 1, (t, s, cs, e, d) -> {
                        if (t.equals("CMT1")) {
                            Tiff tiff = Tiff.open(src, cs);
                            if (tiff != null) {
                                orientation[0] = tiff.orientation();
                            }
                        }
                        return false;
                    });
                } else if (Arrays.equals(uuid, CR3_PREVIEW_UUID)) {
                    // 8 bytes, then a PRVW box: a few size fields, then the JPEG
                    boxes(src, contentStart + 24, end, depth + 1, (t, s, cs, e, d) -> {
                        if (t.equals("PRVW")) {
                            long jpeg = find(src, cs, Math.min(e, cs + 64), new byte[]{(byte) 0xff, (byte) 0xd8, (byte) 0xff});
                            if (jpeg >= 0) {
                                addJpeg(src, jpeg, e - jpeg, found);
                            }
                        }
                        return false;
                    });
                }
            }
            return false;
        });
        return orientation[0];
    }

    private interface BoxVisitor {
        /** Returns true to look inside the box. */
        boolean visit(String type, long start, long contentStart, long end, int depth) throws IOException;
    }

    private static void boxes(Source src, long from, long to, int depth, BoxVisitor visitor) throws IOException {
        long pos = from;
        for (int i = 0; i < 10_000 && pos + 8 <= to && depth < 6; i++) {
            ByteBuffer h = ByteBuffer.wrap(src.bytes(pos, 8)).order(ByteOrder.BIG_ENDIAN);
            long size = Integer.toUnsignedLong(h.getInt());
            byte[] t = new byte[4];
            h.get(t);
            String type = new String(t, java.nio.charset.StandardCharsets.ISO_8859_1);
            long content = pos + 8;
            if (size == 1) {
                if (pos + 16 > to) {
                    return;
                }
                size = ByteBuffer.wrap(src.bytes(pos + 8, 8)).getLong();
                content = pos + 16;
            } else if (size == 0) {
                size = to - pos;
            }
            if (size < content - pos || pos + size > to) {
                return;
            }
            if (visitor.visit(type, pos, content, pos + size, depth)) {
                boxes(src, content, pos + size, depth + 1, visitor);
            }
            pos += size;
        }
    }

    private static long find(Source src, long from, long to, byte[] pattern) throws IOException {
        if (to - from < pattern.length) {
            return -1;
        }
        byte[] data = src.bytes(from, (int) (to - from));
        outer:
        for (int i = 0; i + pattern.length <= data.length; i++) {
            for (int j = 0; j < pattern.length; j++) {
                if (data[i + j] != pattern[j]) {
                    continue outer;
                }
            }
            return from + i;
        }
        return -1;
    }

    // ---- TIFF ----

    /** A TIFF structure at {@code base} (offsets in it are relative to base). */
    private static final class Tiff {
        private final Source src;
        private final long base;
        private final ByteOrder order;
        private final long firstIfd;

        private Tiff(Source src, long base, ByteOrder order, long firstIfd) {
            this.src = src;
            this.base = base;
            this.order = order;
            this.firstIfd = firstIfd;
        }

        /** Reads the header: II/MM, then 42 (or ORF's and RW2's own magic numbers). */
        static Tiff open(Source src, long base) throws IOException {
            if (src.size() < base + 8) {
                return null;
            }
            byte[] h = src.bytes(base, 8);
            ByteOrder order;
            if (h[0] == 'I' && h[1] == 'I') {
                order = ByteOrder.LITTLE_ENDIAN;
            } else if (h[0] == 'M' && h[1] == 'M') {
                order = ByteOrder.BIG_ENDIAN;
            } else {
                return null;
            }
            ByteBuffer b = ByteBuffer.wrap(h).order(order);
            int magic = b.getShort(2) & 0xffff;
            if (magic != 42 && magic != 0x4f52 && magic != 0x5352 && magic != 0x55) {
                return null;
            }
            return new Tiff(src, base, order, Integer.toUnsignedLong(b.getInt(4)));
        }

        int orientation() throws IOException {
            List<Entry> ifd0 = ifd(firstIfd);
            for (Entry e : ifd0) {
                if (e.tag == 0x112) {
                    return (int) e.first();
                }
            }
            return 0;
        }

        /** Adds every embedded JPEG; returns IFD0's orientation (0: none). */
        int collect(List<Candidate> found) throws IOException {
            int orientation = 0;
            Set<Long> seen = new HashSet<>();
            List<Long> todo = new ArrayList<>();
            todo.add(firstIfd);
            boolean first = true;
            while (!todo.isEmpty() && seen.size() < MAX_IFDS) {
                long at = todo.remove(0);
                if (at <= 0 || !seen.add(at)) {
                    continue;
                }
                List<Entry> entries = ifd(at);
                long jpegOffset = -1, jpegLength = -1, strip = -1, stripLength = -1;
                int stripCount = 0;
                for (Entry e : entries) {
                    switch (e.tag) {
                        case 0x112 -> {
                            if (first) {
                                orientation = (int) e.first();
                            }
                        }
                        case 0x201 -> jpegOffset = e.first();
                        case 0x202 -> jpegLength = e.first();
                        case 0x111 -> {
                            strip = e.first();
                            stripCount = (int) e.count;
                        }
                        case 0x117 -> stripLength = e.first();
                        case 0x14a -> {
                            for (long sub : e.values(16)) {
                                todo.add(sub);
                            }
                        }
                        case 0x2e -> { // Panasonic RW2: JpgFromRaw, stored inline as undefined bytes
                            if (e.type == 7) {
                                addJpeg(src, base + e.valueOffset(), e.count(), found);
                            }
                        }
                        default -> {
                        }
                    }
                }
                if (jpegOffset > 0 && jpegLength > 0) {
                    addJpeg(src, base + jpegOffset, jpegLength, found);
                }
                if (stripCount == 1 && strip > 0 && stripLength > 0) {
                    addJpeg(src, base + strip, stripLength, found); // checked to be a viewable JPEG
                }
                if (first) {
                    first = false;
                }
                long next = nextIfd(at, entries.size());
                if (next > 0) {
                    todo.add(next);
                }
            }
            return orientation;
        }

        private long nextIfd(long at, int count) throws IOException {
            long pos = base + at + 2 + 12L * count;
            if (pos + 4 > src.size()) {
                return 0;
            }
            return Integer.toUnsignedLong(ByteBuffer.wrap(src.bytes(pos, 4)).order(order).getInt());
        }

        private List<Entry> ifd(long at) throws IOException {
            long pos = base + at;
            if (pos + 2 > src.size()) {
                return List.of();
            }
            int count = ByteBuffer.wrap(src.bytes(pos, 2)).order(order).getShort() & 0xffff;
            if (count > MAX_ENTRIES || pos + 2 + 12L * count > src.size()) {
                return List.of();
            }
            ByteBuffer b = ByteBuffer.wrap(src.bytes(pos + 2, 12 * count)).order(order);
            List<Entry> entries = new ArrayList<>(count);
            for (int i = 0; i < count; i++) {
                int tag = b.getShort() & 0xffff;
                int type = b.getShort() & 0xffff;
                long n = Integer.toUnsignedLong(b.getInt());
                byte[] value = new byte[4];
                b.get(value);
                entries.add(new Entry(this, tag, type, n, value));
            }
            return entries;
        }
    }

    /** An IFD entry; values that don't fit in its 4 bytes live at an offset. */
    private record Entry(Tiff tiff, int tag, int type, long count, byte[] raw) {
        long valueOffset() {
            return Integer.toUnsignedLong(ByteBuffer.wrap(raw).order(tiff.order).getInt());
        }

        int typeSize() {
            return switch (type) {
                case 3, 8 -> 2;
                case 4, 9, 13 -> 4;
                default -> 1;
            };
        }

        /** The first value as a number (SHORT or LONG). */
        long first() {
            ByteBuffer b = ByteBuffer.wrap(raw).order(tiff.order);
            if (count == 1 || typeSize() * count <= 4) {
                return typeSize() == 2 ? b.getShort() & 0xffff : typeSize() == 4 ? Integer.toUnsignedLong(b.getInt()) : raw[0] & 0xff;
            }
            try {
                ByteBuffer v = ByteBuffer.wrap(tiff.src.bytes(tiff.base + valueOffset(), typeSize())).order(tiff.order);
                return typeSize() == 2 ? v.getShort() & 0xffff : Integer.toUnsignedLong(v.getInt());
            } catch (IOException e) {
                return -1;
            }
        }

        /** Up to {@code max} LONG values (sub-IFD offsets). */
        long[] values(int max) throws IOException {
            int n = (int) Math.min(count, max);
            if (typeSize() != 4 || n <= 0) {
                return new long[0];
            }
            ByteBuffer b = n == 1 ? ByteBuffer.wrap(raw).order(tiff.order)
                    : ByteBuffer.wrap(tiff.src.bytes(tiff.base + valueOffset(), 4 * n)).order(tiff.order);
            long[] out = new long[n];
            for (int i = 0; i < n; i++) {
                out[i] = Integer.toUnsignedLong(b.getInt());
            }
            return out;
        }
    }

    // ---- reading ----

    /** Random access to the file (or a byte array); reads past the end throw. */
    interface Source {
        long size() throws IOException;

        byte[] bytes(long pos, int length) throws IOException;
    }

    private static final class FileSource implements Source {
        private final FileChannel ch;
        private final long size;

        FileSource(FileChannel ch) throws IOException {
            this.ch = ch;
            this.size = ch.size();
        }

        @Override
        public long size() {
            return size;
        }

        @Override
        public byte[] bytes(long pos, int length) throws IOException {
            if (pos < 0 || length < 0 || pos + length > size) {
                throw new EOFException("Past the end of the file");
            }
            ByteBuffer b = ByteBuffer.allocate(length);
            while (b.hasRemaining()) {
                if (Thread.currentThread().isInterrupted()) {
                    throw new java.io.InterruptedIOException();
                }
                if (ch.read(b, pos + b.position()) < 0) {
                    throw new EOFException("Past the end of the file");
                }
            }
            return b.array();
        }
    }

    private record BytesSource(byte[] data) implements Source {
        @Override
        public long size() {
            return data.length;
        }

        @Override
        public byte[] bytes(long pos, int length) throws IOException {
            if (pos < 0 || length < 0 || pos + length > data.length) {
                throw new EOFException("Past the end of the data");
            }
            return Arrays.copyOfRange(data, (int) pos, (int) pos + length);
        }
    }

    private static boolean startsWith(byte[] data, String prefix) {
        if (data.length < prefix.length()) {
            return false;
        }
        for (int i = 0; i < prefix.length(); i++) {
            if (data[i] != (byte) prefix.charAt(i)) {
                return false;
            }
        }
        return true;
    }

    private static byte[] hex(String s) {
        byte[] out = new byte[s.length() / 2];
        for (int i = 0; i < out.length; i++) {
            out[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
        }
        return out;
    }
}
