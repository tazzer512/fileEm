package com.fileem.ui;

import com.fileem.fs.RawReader;
import javafx.scene.image.Image;
import javafx.scene.image.PixelFormat;
import javafx.scene.image.WritableImage;

import java.io.ByteArrayInputStream;

/** Camera raw previews as JavaFX images: decoded and turned upright. Runs on a background thread. */
final class RawImages {
    private RawImages() {
    }

    /**
     * Decodes {@code p}'s JPEG, at most {@code maxSide} pixels on its longer side (never enlarged), and
     * applies its orientation. Null when it can't be decoded.
     */
    static Image decode(RawReader.Preview p, int maxSide) {
        if (p == null) {
            return null;
        }
        boolean big = Math.max(p.width(), p.height()) > maxSide;
        Image img = big ? new Image(new ByteArrayInputStream(p.jpeg()), maxSide, maxSide, true, true)
                : new Image(new ByteArrayInputStream(p.jpeg()));
        if (img.isError() || img.getWidth() <= 0) {
            return null;
        }
        return oriented(img, p.orientation());
    }

    /** The image turned/mirrored upright for an EXIF orientation (1 = already upright). */
    static Image oriented(Image img, int orientation) {
        if (orientation < 2 || orientation > 8 || img.getPixelReader() == null) {
            return img;
        }
        int w = (int) img.getWidth(), h = (int) img.getHeight();
        int[] src = new int[w * h];
        img.getPixelReader().getPixels(0, 0, w, h, PixelFormat.getIntArgbInstance(), src, 0, w);
        boolean swap = orientation >= 5;
        int ow = swap ? h : w, oh = swap ? w : h;
        int[] out = new int[w * h];
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                int nx, ny;
                switch (orientation) {
                    case 2 -> { nx = w - 1 - x; ny = y; }          // mirrored
                    case 3 -> { nx = w - 1 - x; ny = h - 1 - y; }  // upside down
                    case 4 -> { nx = x; ny = h - 1 - y; }          // mirrored vertically
                    case 5 -> { nx = y; ny = x; }                  // transposed
                    case 6 -> { nx = h - 1 - y; ny = x; }          // turn 90° clockwise
                    case 7 -> { nx = h - 1 - y; ny = w - 1 - x; }  // transversed
                    default -> { nx = y; ny = w - 1 - x; }         // 8: turn 90° counterclockwise
                }
                out[ny * ow + nx] = src[y * w + x];
            }
        }
        WritableImage rotated = new WritableImage(ow, oh);
        rotated.getPixelWriter().setPixels(0, 0, ow, oh, PixelFormat.getIntArgbInstance(), out, 0, ow);
        return rotated;
    }
}
