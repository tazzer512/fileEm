package com.fileem.fs;

import java.io.BufferedInputStream;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Netpbm images: PBM (P1 text, P4 binary bitmap), PGM (P2/P5 grey) and PPM (P3/P6 colour), 8 or 16
 * bits per sample. Our own reader: jai-imageio's reads 16-bit files wrongly. Big images are read
 * with whole-pixel steps (skipped rows are only read past), then shrunk to the limit.
 */
final class NetpbmReader {
    /** Larger sides are refused: a damaged header shouldn't make us allocate gigabytes. */
    private static final int MAX_SIDE = 1 << 18;

    private NetpbmReader() {
    }

    /** Decodes {@code file} with its longer side at most {@code maxSide}; null when it isn't a valid Netpbm file. */
    static ImageIoReader.Pixels read(Path file, int maxSide) throws IOException {
        try (InputStream raw = Files.newInputStream(file);
             InputStream in = new BufferedInputStream(raw, 1 << 16)) {
            return read(in, maxSide);
        } catch (InterruptedIOException e) {
            throw e;
        } catch (IOException | RuntimeException e) {
            return null; // damaged or cut off
        }
    }

    private static ImageIoReader.Pixels read(InputStream in, int maxSide) throws IOException {
        if (in.read() != 'P') {
            return null;
        }
        int kind = in.read() - '0';
        if (kind < 1 || kind > 6) {
            return null;
        }
        int w = number(in);
        int h = number(in);
        boolean bitmap = kind == 1 || kind == 4;
        int maxval = bitmap ? 1 : number(in);
        if (w <= 0 || h <= 0 || w > MAX_SIDE || h > MAX_SIDE || maxval <= 0 || maxval > 65535) {
            return null;
        }
        boolean text = kind <= 3;
        int channels = kind == 3 || kind == 6 ? 3 : 1;
        int bytesPerSample = maxval > 255 ? 2 : 1;
        // after the header comes exactly one whitespace character before binary data (already read by number())

        int limit = Math.max(1, maxSide);
        int step = Math.max(1, Math.max(w, h) / limit); // the result stays at least the limit, then shrinks
        int ow = (w + step - 1) / step;
        int oh = (h + step - 1) / step;
        int[] out = new int[ow * oh];
        int rowBytes = bitmap ? (w + 7) / 8 : w * channels * bytesPerSample;
        byte[] row = text ? null : new byte[rowBytes];
        int[] samples = new int[channels];
        for (int y = 0; y < h; y++) {
            if ((y & 63) == 0 && Thread.currentThread().isInterrupted()) {
                throw new InterruptedIOException();
            }
            boolean keep = y % step == 0;
            if (!text) {
                if (in.readNBytes(row, 0, rowBytes) != rowBytes) {
                    throw new EOFException();
                }
                if (!keep) {
                    continue;
                }
            }
            for (int x = 0; x < w; x++) {
                if (text) {
                    for (int c = 0; c < channels; c++) {
                        samples[c] = kind == 1 ? bit(in) : number(in);
                    }
                } else if (bitmap) {
                    samples[0] = row[x >> 3] >> (7 - (x & 7)) & 1;
                } else {
                    for (int c = 0; c < channels; c++) {
                        int at = (x * channels + c) * bytesPerSample;
                        samples[c] = bytesPerSample == 2 ? (row[at] & 0xff) << 8 | row[at + 1] & 0xff : row[at] & 0xff;
                    }
                }
                if (!keep || x % step != 0) {
                    continue;
                }
                int argb;
                if (bitmap) {
                    argb = samples[0] == 1 ? 0xff000000 : 0xffffffff; // 1 is black
                } else if (channels == 1) {
                    int v = scale(samples[0], maxval);
                    argb = 0xff000000 | v << 16 | v << 8 | v;
                } else {
                    argb = 0xff000000 | scale(samples[0], maxval) << 16 | scale(samples[1], maxval) << 8
                            | scale(samples[2], maxval);
                }
                out[(y / step) * ow + x / step] = argb;
            }
        }
        if (Math.max(ow, oh) > limit) {
            double s = limit / (double) Math.max(ow, oh);
            int nw = Math.max(1, (int) Math.round(ow * s));
            int nh = Math.max(1, (int) Math.round(oh * s));
            return new ImageIoReader.Pixels(ImageIoReader.shrink(out, ow, oh, nw, nh), nw, nh, w, h);
        }
        return new ImageIoReader.Pixels(out, ow, oh, w, h);
    }

    private static int scale(int v, int maxval) {
        return maxval == 255 ? Math.min(v, 255) : (int) Math.round(Math.min(v, maxval) * 255.0 / maxval);
    }

    /** The next decimal number, skipping whitespace and # comments; eats the one character after it. */
    private static int number(InputStream in) throws IOException {
        int c = skipSpace(in);
        if (c < '0' || c > '9') {
            throw new IOException("number expected");
        }
        long n = 0;
        while (c >= '0' && c <= '9') {
            n = n * 10 + (c - '0');
            if (n > Integer.MAX_VALUE) {
                throw new IOException("number too big");
            }
            c = in.read();
        }
        return (int) n;
    }

    /** A P1 pixel: a single 0 or 1 (they need no spaces between them). */
    private static int bit(InputStream in) throws IOException {
        int c = skipSpace(in);
        if (c != '0' && c != '1') {
            throw new IOException("0 or 1 expected");
        }
        return c - '0';
    }

    private static int skipSpace(InputStream in) throws IOException {
        int c = in.read();
        while (true) {
            if (c == '#') {
                while (c != '\n' && c != '\r' && c != -1) {
                    c = in.read();
                }
            } else if (c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f' || c == 0x0b) {
                c = in.read();
            } else if (c == -1) {
                throw new EOFException();
            } else {
                return c;
            }
        }
    }
}
