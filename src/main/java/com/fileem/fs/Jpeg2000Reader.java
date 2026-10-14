package com.fileem.fs;

import com.github.jaiimageio.jpeg2000.J2KImageReadParam;

import javax.imageio.ImageIO;
import javax.imageio.ImageReadParam;
import javax.imageio.ImageReader;
import javax.imageio.stream.ImageInputStream;
import java.awt.color.ColorSpace;
import java.awt.image.BufferedImage;
import java.awt.image.ColorModel;
import java.awt.image.Raster;
import java.io.IOException;
import java.io.InputStream;
import java.io.InterruptedIOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Iterator;
import java.util.Locale;
import java.util.Set;

/**
 * JPEG 2000 images ({@code .jp2}, {@code .j2k} and the other variants), which neither JavaFX nor
 * the JDK can decode, through the jai-imageio JPEG 2000 plugin for ImageIO. Big images are decoded
 * at one of the reduced resolutions the format stores, so a thumbnail of a huge scan takes a fraction
 * of a second and never holds the whole picture in memory.
 */
public final class Jpeg2000Reader {
    private static final Set<String> EXTENSIONS = Set.of("jp2", "j2k", "j2c", "jpc", "jpf", "jpx");

    private Jpeg2000Reader() {
    }

    /** Pixels as ARGB ints, row by row, and the image's full size. */
    public record Pixels(int[] argb, int width, int height, int fullWidth, int fullHeight) {
    }

    public static boolean isJpeg2000(String extension) {
        return extension != null && EXTENSIONS.contains(extension.toLowerCase(Locale.ROOT));
    }

    /**
     * The pixels as sRGB ARGB. Grey images are taken as they are: Java treats their colour space as
     * linear and {@code getRGB} would brighten them (200 becomes 229).
     */
    static int[] argb(BufferedImage img) {
        int w = img.getWidth();
        int h = img.getHeight();
        ColorModel cm = img.getColorModel();
        if (cm.getColorSpace().getType() != ColorSpace.TYPE_GRAY) {
            return img.getRGB(0, 0, w, h, null, 0, w);
        }
        Raster r = img.getRaster();
        int bits = cm.getComponentSize(0);
        boolean alpha = cm.hasAlpha() && r.getNumBands() > 1;
        int alphaBits = alpha ? cm.getComponentSize(1) : 8;
        int[] out = new int[w * h];
        int[] grey = new int[w];
        int[] alphas = alpha ? new int[w] : null;
        for (int y = 0; y < h; y++) {
            r.getSamples(0, y, w, 1, 0, grey);
            if (alpha) {
                r.getSamples(0, y, w, 1, 1, alphas);
            }
            for (int x = 0; x < w; x++) {
                int v = to8(grey[x], bits);
                int a = alpha ? to8(alphas[x], alphaBits) : 255;
                out[y * w + x] = a << 24 | v << 16 | v << 8 | v;
            }
        }
        return out;
    }

    /** A sample of {@code bits} bits scaled to 0–255. */
    private static int to8(int v, int bits) {
        if (bits == 8) {
            return v & 0xff;
        }
        long max = (1L << bits) - 1;
        return (int) Math.round(Math.min(Math.max(v, 0), max) * 255.0 / max);
    }

    /**
     * Decodes {@code file} with its longer side at most {@code maxSide} pixels (never enlarged). A
     * JPEG 2000 file holds the picture at several resolutions (each half the next), and decoding a
     * smaller one skips most of the work: the smallest one at least {@code maxSide} is decoded, then
     * scaled down. Null when it isn't an image ImageIO can read or can't be decoded.
     */
    public static Pixels read(Path file, int maxSide) throws IOException {
        int levels = decompositionLevels(file);
        try (ImageInputStream in = ImageIO.createImageInputStream(file.toFile())) {
            if (in == null) {
                return null;
            }
            Iterator<ImageReader> readers = ImageIO.getImageReaders(in);
            if (!readers.hasNext()) {
                return null; // not JPEG 2000 (nor anything else ImageIO knows)
            }
            ImageReader reader = readers.next();
            try {
                reader.setInput(in, true, true);
                int w = reader.getWidth(0);
                int h = reader.getHeight(0);
                if (w <= 0 || h <= 0) {
                    return null;
                }
                int longer = Math.max(w, h);
                int limit = Math.max(1, maxSide);
                ImageReadParam param = reader.getDefaultReadParam();
                if (param instanceof J2KImageReadParam j2k && levels > 0) {
                    int skip = 0; // halvings: as many as keep the longer side at least the limit
                    while (skip < levels && ceilDiv(longer, 1 << (skip + 1)) >= limit) {
                        skip++;
                    }
                    j2k.setResolution(levels - skip);
                } else {
                    int step = Math.max(1, ceilDiv(longer, limit)); // no levels known: whole-pixel steps
                    if (step > 1) {
                        param.setSourceSubsampling(step, step, 0, 0);
                    }
                }
                BufferedImage img = reader.read(0, param);
                if (Thread.currentThread().isInterrupted()) {
                    throw new InterruptedIOException();
                }
                int iw = img.getWidth();
                int ih = img.getHeight();
                int[] px = argb(img);
                if (Math.max(iw, ih) > limit) {
                    double scale = limit / (double) Math.max(iw, ih);
                    int nw = Math.max(1, (int) Math.round(iw * scale));
                    int nh = Math.max(1, (int) Math.round(ih * scale));
                    return new Pixels(shrink(px, iw, ih, nw, nh), nw, nh, w, h);
                }
                return new Pixels(px, iw, ih, w, h);
            } catch (InterruptedIOException e) {
                throw e;
            } catch (IOException | RuntimeException e) {
                return null; // damaged or unsupported variant: the decoder throws all kinds of things
            } finally {
                reader.dispose();
            }
        }
    }

    private static int ceilDiv(int a, int b) {
        return (a + b - 1) / b;
    }

    /**
     * The number of wavelet decomposition levels (resolutions minus one) from the codestream's COD
     * marker, or 0 when it can't be found. The main header sits at the start: SOC (FF4F) and SIZ
     * (FF51), then COD (FF52) whose 10th byte is the level count.
     */
    static int decompositionLevels(Path file) {
        byte[] head = new byte[64 * 1024];
        int n;
        try (InputStream in = Files.newInputStream(file)) {
            n = in.readNBytes(head, 0, head.length);
        } catch (IOException e) {
            return 0;
        }
        for (int i = 0; i + 3 < n; i++) {
            if ((head[i] & 0xff) == 0xff && (head[i + 1] & 0xff) == 0x4f
                    && (head[i + 2] & 0xff) == 0xff && (head[i + 3] & 0xff) == 0x51) {
                for (int j = i + 4; j + 9 < n; j++) {
                    if ((head[j] & 0xff) == 0xff && (head[j + 1] & 0xff) == 0x52) {
                        int levels = head[j + 9] & 0xff;
                        return levels <= 32 ? levels : 0;
                    }
                }
                return 0;
            }
        }
        return 0;
    }

    /** Scales ARGB pixels down by averaging the source pixels under each target pixel. */
    static int[] shrink(int[] src, int w, int h, int nw, int nh) {
        int[] out = new int[nw * nh];
        for (int y = 0; y < nh; y++) {
            int y0 = (int) ((long) y * h / nh);
            int y1 = Math.max(y0 + 1, (int) ((long) (y + 1) * h / nh));
            for (int x = 0; x < nw; x++) {
                int x0 = (int) ((long) x * w / nw);
                int x1 = Math.max(x0 + 1, (int) ((long) (x + 1) * w / nw));
                long a = 0, r = 0, g = 0, b = 0;
                for (int yy = y0; yy < y1; yy++) {
                    for (int xx = x0; xx < x1; xx++) {
                        int p = src[yy * w + xx];
                        a += p >>> 24;
                        r += p >> 16 & 0xff;
                        g += p >> 8 & 0xff;
                        b += p & 0xff;
                    }
                }
                long count = (long) (y1 - y0) * (x1 - x0);
                out[y * nw + x] = (int) (a / count) << 24 | (int) (r / count) << 16 | (int) (g / count) << 8 | (int) (b / count);
            }
        }
        return out;
    }
}
