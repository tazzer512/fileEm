package com.fileem.fs;

import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.CharBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CharsetDecoder;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.util.Set;

/**
 * What the preview window can show for a file, and the reading it needs. No JavaFX here; everything
 * that touches the disk is meant for a background thread.
 */
public final class Preview {
    public enum Kind { IMAGE, PSD, RAW, IMAGEIO, TEXT, FOLDER, OTHER }

    /** Formats JavaFX decodes. */
    private static final Set<String> IMAGES = Set.of("png", "jpg", "jpeg", "gif", "bmp");
    /** Extensions that are text, so no need to look inside first. */
    private static final Set<String> TEXT = Set.of(
            "txt", "text", "md", "markdown", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "ini",
            "cfg", "conf", "properties", "env", "html", "htm", "css", "js", "mjs", "ts", "tsx", "jsx", "java",
            "kt", "kts", "groovy", "gradle", "scala", "py", "rb", "go", "rs", "c", "h", "cpp", "hpp", "cc", "cs",
            "swift", "m", "php", "pl", "lua", "sh", "bash", "zsh", "fish", "bat", "cmd", "ps1", "sql", "r",
            "tex", "srt", "vtt", "diff", "patch", "gitignore", "dockerfile", "makefile", "svg");
    /** Text is shown up to this much; the window says when a file is longer. */
    public static final int MAX_TEXT_BYTES = 256 * 1024;

    private Preview() {
    }

    /**
     * The kind judged from the name alone. Unknown files are {@link Kind#OTHER}; {@link #readText} may
     * still find text in them.
     */
    public static Kind kindOf(String extension, boolean directory) {
        if (directory) {
            return Kind.FOLDER;
        }
        String ext = extension == null ? "" : extension;
        if (IMAGES.contains(ext)) {
            return Kind.IMAGE;
        }
        if (PsdReader.isPsd(ext)) {
            return Kind.PSD;
        }
        if (RawReader.isRaw(ext)) {
            return Kind.RAW;
        }
        if (ImageIoReader.handles(ext)) {
            return Kind.IMAGEIO;
        }
        return TEXT.contains(ext) ? Kind.TEXT : Kind.OTHER;
    }

    /** Text read for the preview: the content and whether the file is longer than what was read. */
    public record Text(String content, boolean truncated) {
    }

    /**
     * Reads the start of a file as text (UTF-8, else Latin-1), or returns null when it looks binary
     * (NUL bytes or many control characters). Reads at most {@code maxBytes}. Regular files only.
     */
    public static Text readText(Path file, int maxBytes) throws IOException {
        if (!Files.isRegularFile(file, LinkOption.NOFOLLOW_LINKS) && !Files.isRegularFile(file)) {
            return null;
        }
        byte[] data;
        boolean truncated;
        try (InputStream in = Files.newInputStream(file)) {
            data = in.readNBytes(maxBytes);
            truncated = in.read() >= 0;
        }
        if (looksBinary(data)) {
            return null;
        }
        return new Text(decode(data, truncated), truncated);
    }

    /** NUL bytes, or more than 5% control characters (other than tab, newline, CR, form feed, escape). */
    static boolean looksBinary(byte[] data) {
        int n = Math.min(data.length, 8192);
        int control = 0;
        for (int i = 0; i < n; i++) {
            int b = data[i] & 0xff;
            if (b == 0) {
                return true;
            }
            if (b < 32 && b != '\t' && b != '\n' && b != '\r' && b != '\f' && b != 27) {
                control++;
            }
        }
        return n > 0 && control * 20 > n;
    }

    /** UTF-8 (tolerating a character cut off at the end of a truncated read), else Latin-1. */
    static String decode(byte[] data, boolean truncated) {
        int start = data.length >= 3 && (data[0] & 0xff) == 0xef && (data[1] & 0xff) == 0xbb && (data[2] & 0xff) == 0xbf
                ? 3 : 0; // byte order mark
        int end = data.length;
        if (truncated) {
            // don't let a multi-byte character split by the read limit spoil the whole decode
            int back = 0;
            while (back < 3 && end - back - 1 >= start && (data[end - back - 1] & 0xc0) == 0x80) {
                back++;
            }
            if (end - back - 1 >= start && (data[end - back - 1] & 0xc0) == 0xc0) {
                end = end - back - 1;
            }
        }
        CharsetDecoder utf8 = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT);
        try {
            CharBuffer chars = utf8.decode(ByteBuffer.wrap(data, start, end - start));
            return chars.toString();
        } catch (CharacterCodingException e) {
            return new String(data, start, data.length - start, StandardCharsets.ISO_8859_1);
        }
    }
}
