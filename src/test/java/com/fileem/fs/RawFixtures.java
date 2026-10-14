package com.fileem.fs;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

/** Builds small camera raw files for tests: TIFF structures (CR2, NEF, RW2 …), CR3 boxes, RAF. */
public final class RawFixtures {
    private RawFixtures() {
    }

    /** A real baseline JPEG of the given size (decodable by ImageIO and JavaFX). */
    public static byte[] jpeg(int w, int h) {
        try {
            BufferedImage img = new BufferedImage(w, h, BufferedImage.TYPE_INT_RGB);
            var g = img.createGraphics();
            g.setColor(java.awt.Color.RED);
            g.fillRect(0, 0, w / 2, h); // left half red, so a turned image can be told apart
            g.dispose();
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            ImageIO.write(img, "jpg", out);
            return out.toByteArray();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    /** The start of a lossless JPEG (SOF3), like the raw sensor data in CR2/DNG: not viewable. */
    public static byte[] losslessJpeg(int w, int h) {
        ByteBuffer b = ByteBuffer.allocate(64).order(ByteOrder.BIG_ENDIAN);
        b.put(new byte[]{(byte) 0xff, (byte) 0xd8, (byte) 0xff, (byte) 0xc3});
        b.putShort((short) 11).put((byte) 14).putShort((short) h).putShort((short) w);
        b.put(new byte[]{1, 1, 0x11, 0});
        b.put(new byte[]{(byte) 0xff, (byte) 0xd9});
        return b.array();
    }

    /** The same JPEG with an EXIF block (APP1) holding an orientation. */
    public static byte[] withExifOrientation(byte[] jpeg, int orientation) {
        byte[] tiff = tiff(ByteOrder.LITTLE_ENDIAN, new Ifd().shortValue(0x112, orientation));
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(jpeg, 0, 2);
        int len = 2 + 6 + tiff.length;
        out.write(0xff);
        out.write(0xe1);
        out.write(len >> 8);
        out.write(len & 0xff);
        out.writeBytes("Exif\0\0".getBytes(StandardCharsets.ISO_8859_1));
        out.writeBytes(tiff);
        out.write(jpeg, 2, jpeg.length - 2);
        return out.toByteArray();
    }

    /** One TIFF IFD; values that need it are laid out after the IFDs. */
    public static final class Ifd {
        private record Entry(int tag, int type, long count, long value, byte[] data, Ifd ref) {
        }

        private final List<Entry> entries = new ArrayList<>();
        private Ifd next;

        public Ifd shortValue(int tag, int v) {
            entries.add(new Entry(tag, 3, 1, v, null, null));
            return this;
        }

        public Ifd longValue(int tag, long v) {
            entries.add(new Entry(tag, 4, 1, v, null, null));
            return this;
        }

        /** {@code data} somewhere in the file: its offset under {@code offsetTag}, its length under {@code lengthTag}. */
        public Ifd data(int offsetTag, int lengthTag, byte[] data) {
            entries.add(new Entry(offsetTag, 4, 1, 0, data, null));
            entries.add(new Entry(lengthTag, 4, 1, data.length, null, null));
            return this;
        }

        /** An UNDEFINED value (bytes stored at an offset), like RW2's JpgFromRaw. */
        public Ifd undefined(int tag, byte[] data) {
            entries.add(new Entry(tag, 7, data.length, 0, data, null));
            return this;
        }

        public Ifd subIfd(Ifd child) {
            entries.add(new Entry(0x14a, 4, 1, 0, null, child));
            return this;
        }

        /** The IFD after this one (IFD0 → IFD1 …). */
        public Ifd next(Ifd n) {
            next = n;
            return this;
        }

        int size() {
            return 2 + 12 * entries.size() + 4;
        }
    }

    /** A TIFF file: header (magic 42), then {@code first} and every IFD it links to, then the data. */
    public static byte[] tiff(ByteOrder order, Ifd first) {
        return tiff(order, 42, first);
    }

    public static byte[] tiff(ByteOrder order, int magic, Ifd first) {
        List<Ifd> all = new ArrayList<>();
        collect(first, all);
        Map<Ifd, Integer> at = new IdentityHashMap<>();
        int pos = 8;
        for (Ifd ifd : all) {
            at.put(ifd, pos);
            pos += ifd.size();
        }
        Map<byte[], Integer> dataAt = new HashMap<>();
        List<byte[]> blobs = new ArrayList<>();
        for (Ifd ifd : all) {
            for (Ifd.Entry e : ifd.entries) {
                if (e.data != null && !dataAt.containsKey(e.data)) {
                    dataAt.put(e.data, pos);
                    blobs.add(e.data);
                    pos += e.data.length;
                }
            }
        }
        ByteBuffer b = ByteBuffer.allocate(pos).order(order);
        b.put(order == ByteOrder.LITTLE_ENDIAN ? new byte[]{'I', 'I'} : new byte[]{'M', 'M'});
        b.putShort((short) magic).putInt(8);
        for (Ifd ifd : all) {
            b.putShort((short) ifd.entries.size());
            for (Ifd.Entry e : ifd.entries) {
                b.putShort((short) e.tag).putShort((short) e.type).putInt((int) e.count);
                long v = e.data != null ? dataAt.get(e.data) : e.ref != null ? at.get(e.ref) : e.value;
                if (e.type == 3) {
                    b.putShort((short) v).putShort((short) 0);
                } else {
                    b.putInt((int) v);
                }
            }
            b.putInt(ifd.next == null ? 0 : at.get(ifd.next));
        }
        for (byte[] blob : blobs) {
            b.put(blob);
        }
        return b.array();
    }

    /** {@code ifd}, its chain of next IFDs and their sub-IFDs, each once (a loop ends the chain). */
    private static void collect(Ifd ifd, List<Ifd> all) {
        for (Ifd i = ifd; i != null; i = i.next) {
            for (Ifd seen : all) {
                if (seen == i) {
                    return;
                }
            }
            all.add(i);
            for (Ifd.Entry e : i.entries) {
                if (e.ref != null) {
                    collect(e.ref, all);
                }
            }
        }
    }

    /** A CR2-like file: full-size JPEG in IFD0's strip (with orientation), a thumbnail, lossless raw data. */
    public static byte[] cr2(byte[] preview, byte[] thumbnail, int orientation) {
        Ifd raw = new Ifd().shortValue(0x103, 6).data(0x111, 0x117, losslessJpeg(4000, 3000));
        Ifd thumb = new Ifd().data(0x201, 0x202, thumbnail).next(raw);
        Ifd ifd0 = new Ifd().shortValue(0x103, 6).shortValue(0x112, orientation).data(0x111, 0x117, preview).next(thumb);
        return tiff(ByteOrder.LITTLE_ENDIAN, ifd0);
    }

    /** A Fujifilm RAF: its header points to the JPEG. */
    public static byte[] raf(byte[] jpeg) {
        ByteBuffer b = ByteBuffer.allocate(100 + jpeg.length).order(ByteOrder.BIG_ENDIAN);
        b.put("FUJIFILMCCD-RAW 0201FF383501".getBytes(StandardCharsets.ISO_8859_1));
        b.putInt(84, 100).putInt(88, jpeg.length);
        b.position(100);
        b.put(jpeg);
        return b.array();
    }

    /** A Canon CR3: ftyp, moov with Canon's uuid box (CMT1 = TIFF with the orientation), the PRVW box. */
    public static byte[] cr3(byte[] jpeg, int orientation) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.writeBytes(box("ftyp", concat("crx ".getBytes(StandardCharsets.ISO_8859_1), new byte[4],
                "crx isom".getBytes(StandardCharsets.ISO_8859_1))));
        byte[] cmt1 = box("CMT1", tiff(ByteOrder.LITTLE_ENDIAN, new Ifd().shortValue(0x112, orientation)));
        out.writeBytes(box("moov", box("uuid", concat(hex("85c0b687820f11e08111f4ce462b6a48"), cmt1))));
        ByteBuffer prvw = ByteBuffer.allocate(16 + jpeg.length).order(ByteOrder.BIG_ENDIAN);
        prvw.putInt(0).putShort((short) 1).putShort((short) 1620).putShort((short) 1080).putShort((short) 1)
                .putInt(jpeg.length).put(jpeg);
        out.writeBytes(box("uuid", concat(hex("eaf42b5e1c984b88b9fbb7dc406e4d16"), new byte[8], box("PRVW", prvw.array()))));
        out.writeBytes(box("mdat", new byte[32]));
        return out.toByteArray();
    }

    private static byte[] box(String type, byte[] content) {
        ByteBuffer b = ByteBuffer.allocate(8 + content.length).order(ByteOrder.BIG_ENDIAN);
        b.putInt(8 + content.length).put(type.getBytes(StandardCharsets.ISO_8859_1)).put(content);
        return b.array();
    }

    private static byte[] concat(byte[]... parts) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (byte[] p : parts) {
            out.writeBytes(p);
        }
        return out.toByteArray();
    }

    private static byte[] hex(String s) {
        byte[] out = new byte[s.length() / 2];
        for (int i = 0; i < out.length; i++) {
            out[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
        }
        return out;
    }
}
