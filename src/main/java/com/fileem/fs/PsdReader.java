package com.fileem.fs;

import java.io.EOFException;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

/**
 * Reads a small preview of a Photoshop file (PSD, and PSB for big documents) without decoding all of
 * it. The preview comes from the merged ("composite") image stored after the layers: only the rows and
 * columns needed for the requested size are read, so a huge document costs a few hundred small reads.
 * Supported: RGB, grayscale/duotone, indexed, CMYK and bitmap, 1/8/16 bits, raw or RLE (PackBits).
 * When the composite can't be used (ZIP compression, Lab, 32-bit, or a blank composite saved without
 * "maximize compatibility"), the JPEG thumbnail Photoshop embeds in the image resources is returned.
 */
public final class PsdReader {
    /** A preview: either ARGB pixels ({@code width} × {@code height}) or the embedded JPEG bytes. */
    public record Preview(int width, int height, int[] argb, byte[] jpeg) {
    }

    private static final int RESOURCE_THUMBNAIL = 1036;
    private static final int MAX_SIDE_PSD = 30_000;
    private static final int MAX_SIDE_PSB = 300_000;

    private PsdReader() {
    }

    /** Whether the file name looks like a Photoshop document. */
    public static boolean isPsd(String extension) {
        return "psd".equals(extension) || "psb".equals(extension);
    }

    /**
     * A preview at most {@code maxSide} pixels wide and high, or null when the file has none this reader
     * can use. Throws only for read errors and files that are not Photoshop documents.
     */
    public static Preview read(Path file, int maxSide) throws IOException {
        try (FileChannel ch = FileChannel.open(file, StandardOpenOption.READ)) {
            return new PsdReader.Parser(ch).read(Math.max(1, maxSide));
        }
    }

    private static final class Parser {
        private final FileChannel ch;
        private long pos;
        private boolean psb;
        private int channels;
        private int height;
        private int width;
        private int depth;
        private int mode;
        private byte[] palette;
        private byte[] jpeg;

        Parser(FileChannel ch) {
            this.ch = ch;
        }

        Preview read(int maxSide) throws IOException {
            if (!"8BPS".equals(new String(bytes(4), StandardCharsets.US_ASCII))) {
                throw new IOException("Not a Photoshop file");
            }
            int version = u16();
            if (version != 1 && version != 2) {
                throw new IOException("Unknown Photoshop version " + version);
            }
            psb = version == 2;
            pos += 6; // reserved
            channels = u16();
            height = (int) u32();
            width = (int) u32();
            depth = u16();
            mode = u16();
            int maxDim = psb ? MAX_SIDE_PSB : MAX_SIDE_PSD;
            if (channels < 1 || channels > 56 || width < 1 || height < 1 || width > maxDim || height > maxDim) {
                throw new IOException("Unsupported Photoshop dimensions");
            }
            long colorData = u32();
            if (mode == 2 && colorData >= 768) {
                palette = bytes(768);
                pos += colorData - 768;
            } else {
                pos += colorData;
            }
            long resourcesEnd = u32() + pos;
            readResources(resourcesEnd);
            pos = resourcesEnd;
            long layers = psb ? u64() : u32();
            pos += layers;

            Preview composite = null;
            try {
                composite = composite(maxSide);
            } catch (IOException | RuntimeException e) {
                // fall back to the embedded thumbnail
            }
            if (composite != null && (jpeg == null || !uniform(composite.argb()))) {
                return composite;
            }
            return jpeg != null ? new Preview(0, 0, null, jpeg) : composite;
        }

        /** Finds the JPEG thumbnail (resource 1036) among the image resources. */
        private void readResources(long end) throws IOException {
            while (pos + 12 <= end) {
                String sig = new String(bytes(4), StandardCharsets.US_ASCII);
                if (!sig.equals("8BIM")) {
                    return;
                }
                int id = u16();
                int nameLen = u8();
                pos += nameLen + ((nameLen + 1) % 2 == 0 ? 0 : 1); // Pascal string padded to even
                long size = u32();
                long next = pos + size + (size % 2);
                if (id == RESOURCE_THUMBNAIL && size > 28) {
                    long format = u32();
                    pos += 24; // width, height, widthbytes, total size, compressed size, bpp + planes
                    if (format == 1 && size - 28 < 16 << 20) {
                        jpeg = bytes((int) (size - 28));
                    }
                }
                pos = next;
            }
        }

        private Preview composite(int maxSide) throws IOException {
            int compression = u16();
            if (compression != 0 && compression != 1) {
                return null; // ZIP: rare for the composite; use the thumbnail
            }
            if (depth != 1 && depth != 8 && depth != 16) {
                return null;
            }
            int[] use; // channel indices: what each output component reads
            switch (mode) {
                case 3 -> use = channels >= 4 ? new int[]{0, 1, 2, 3} : channels >= 3 ? new int[]{0, 1, 2} : null;
                case 1, 8 -> use = channels >= 2 ? new int[]{0, 1} : new int[]{0};
                case 2 -> use = palette != null ? new int[]{0} : null;
                case 4 -> use = channels >= 4 ? new int[]{0, 1, 2, 3} : null;
                case 0 -> use = depth == 1 ? new int[]{0} : null;
                default -> use = null; // Lab, multichannel
            }
            if (use == null) {
                return null;
            }
            double scale = Math.min(1.0, (double) maxSide / Math.max(width, height));
            int ow = Math.max(1, (int) Math.round(width * scale));
            int oh = Math.max(1, (int) Math.round(height * scale));
            int rowBytes = (int) (((long) width * depth + 7) / 8);
            long dataStart;
            long[] rowOffsets = null; // RLE: start of each (channel, row)
            int[] rowLengths = null;
            if (compression == 1) {
                int rows = channels * height;
                int entry = psb ? 4 : 2;
                ByteBuffer table = buffer(pos, (long) rows * entry);
                rowOffsets = new long[rows];
                rowLengths = new int[rows];
                long offset = pos + (long) rows * entry;
                for (int i = 0; i < rows; i++) {
                    int len = psb ? table.getInt() : Short.toUnsignedInt(table.getShort());
                    if (len < 0) {
                        throw new IOException("Corrupt RLE table");
                    }
                    rowOffsets[i] = offset;
                    rowLengths[i] = len;
                    offset += len;
                }
                dataStart = 0;
            } else {
                dataStart = pos;
            }
            int[] srcCols = new int[ow];
            for (int x = 0; x < ow; x++) {
                srcCols[x] = Math.min(width - 1, (int) ((x + 0.5) * width / ow));
            }
            int[] out = new int[ow * oh];
            int[][] comp = new int[use.length][ow];
            byte[] row = new byte[rowBytes];
            for (int y = 0; y < oh; y++) {
                int sy = Math.min(height - 1, (int) ((y + 0.5) * height / oh));
                for (int c = 0; c < use.length; c++) {
                    int index = use[c] * height + sy;
                    if (compression == 1) {
                        unpackBits(buffer(rowOffsets[index], rowLengths[index]), row);
                    } else {
                        buffer(dataStart + (long) index * rowBytes, rowBytes).get(row);
                    }
                    for (int x = 0; x < ow; x++) {
                        comp[c][x] = sample(row, srcCols[x]);
                    }
                }
                for (int x = 0; x < ow; x++) {
                    out[y * ow + x] = toArgb(comp, x);
                }
            }
            return new Preview(ow, oh, out, null);
        }

        /** The 8-bit value of column {@code x} in a decoded row. */
        private int sample(byte[] row, int x) {
            return switch (depth) {
                case 1 -> (row[x >> 3] >> (7 - (x & 7)) & 1) == 1 ? 0 : 255; // bitmap: 1 is black
                case 16 -> row[x * 2] & 0xff; // big-endian: the high byte
                default -> row[x] & 0xff;
            };
        }

        private int toArgb(int[][] comp, int x) {
            int r, g, b, a = 255;
            switch (mode) {
                case 3 -> {
                    r = comp[0][x];
                    g = comp[1][x];
                    b = comp[2][x];
                    if (comp.length > 3) {
                        a = comp[3][x];
                    }
                }
                case 4 -> { // Photoshop stores CMYK inverted: 255 means no ink
                    int k = comp[3][x];
                    r = comp[0][x] * k / 255;
                    g = comp[1][x] * k / 255;
                    b = comp[2][x] * k / 255;
                }
                case 2 -> {
                    int i = comp[0][x];
                    r = palette[i] & 0xff;
                    g = palette[256 + i] & 0xff;
                    b = palette[512 + i] & 0xff;
                }
                default -> { // grayscale, duotone, bitmap
                    r = g = b = comp[0][x];
                    if (comp.length > 1) {
                        a = comp[1][x];
                    }
                }
            }
            return a << 24 | r << 16 | g << 8 | b;
        }

        /** PackBits: n ≥ 0 copies n + 1 bytes, n < 0 repeats the next byte 1 − n times (−128 is a no-op). */
        private static void unpackBits(ByteBuffer in, byte[] out) throws IOException {
            int o = 0;
            while (o < out.length && in.hasRemaining()) {
                int n = in.get();
                if (n >= 0) {
                    int count = Math.min(n + 1, out.length - o);
                    if (in.remaining() < count) {
                        throw new IOException("Truncated RLE row");
                    }
                    in.get(out, o, count);
                    o += count;
                    in.position(in.position() + (n + 1 - count));
                } else if (n != -128) {
                    if (!in.hasRemaining()) {
                        throw new IOException("Truncated RLE row");
                    }
                    byte v = in.get();
                    int count = Math.min(1 - n, out.length - o);
                    java.util.Arrays.fill(out, o, o + count, v);
                    o += count;
                }
            }
        }

        private static boolean uniform(int[] argb) {
            for (int p : argb) {
                if (p != argb[0]) {
                    return false;
                }
            }
            return true;
        }

        // ---- reading ----

        private ByteBuffer buffer(long at, long length) throws IOException {
            if (length < 0 || length > Integer.MAX_VALUE) {
                throw new IOException("Corrupt length");
            }
            ByteBuffer b = ByteBuffer.allocate((int) length);
            while (b.hasRemaining()) {
                if (ch.read(b, at + b.position()) < 0) {
                    throw new EOFException();
                }
            }
            return b.flip();
        }

        private byte[] bytes(int n) throws IOException {
            byte[] out = new byte[n];
            buffer(pos, n).get(out);
            pos += n;
            return out;
        }

        private int u8() throws IOException {
            return bytes(1)[0] & 0xff;
        }

        private int u16() throws IOException {
            ByteBuffer b = buffer(pos, 2);
            pos += 2;
            return Short.toUnsignedInt(b.getShort());
        }

        private long u32() throws IOException {
            ByteBuffer b = buffer(pos, 4);
            pos += 4;
            return Integer.toUnsignedLong(b.getInt());
        }

        private long u64() throws IOException {
            ByteBuffer b = buffer(pos, 8);
            pos += 8;
            long v = b.getLong();
            if (v < 0) {
                throw new IOException("Corrupt length");
            }
            return v;
        }
    }
}
