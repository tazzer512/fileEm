package com.fileem.ops;

import com.fileem.util.OsUtil;
import javafx.scene.input.Clipboard;
import javafx.scene.input.ClipboardContent;
import javafx.scene.input.DataFormat;

import java.io.File;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.List;
import java.util.stream.Collectors;

/** Cut / copy / paste of files through the OS clipboard. */
public final class FileClipboard {
    private static final DataFormat GNOME_FILES = gnomeFormat();
    private static List<File> cutFiles = List.of();

    private FileClipboard() {
    }

    private static DataFormat gnomeFormat() {
        if (!OsUtil.isLinux()) {
            return null;
        }
        DataFormat existing = DataFormat.lookupMimeType("x-special/gnome-copied-files");
        return existing != null ? existing : new DataFormat("x-special/gnome-copied-files");
    }

    public static void put(List<Path> paths, boolean cut) {
        List<File> files = paths.stream().map(Path::toFile).toList();
        ClipboardContent content = new ClipboardContent();
        content.putFiles(files);
        content.putString(paths.stream().map(Path::toString).collect(Collectors.joining(System.lineSeparator())));
        if (GNOME_FILES != null) {
            String uris = paths.stream().map(p -> p.toUri().toString()).collect(Collectors.joining("\n"));
            content.put(GNOME_FILES, ByteBuffer.wrap(((cut ? "cut\n" : "copy\n") + uris).getBytes(StandardCharsets.UTF_8)));
        }
        Clipboard.getSystemClipboard().setContent(content);
        cutFiles = cut ? files : List.of();
    }

    public static boolean hasFiles() {
        return Clipboard.getSystemClipboard().hasFiles();
    }

    public static List<Path> files() {
        Clipboard cb = Clipboard.getSystemClipboard();
        return cb.hasFiles() ? cb.getFiles().stream().map(File::toPath).toList() : List.of();
    }

    /** True when the clipboard still holds exactly the files we cut. */
    public static boolean isCut() {
        Clipboard cb = Clipboard.getSystemClipboard();
        return !cutFiles.isEmpty() && cb.hasFiles() && cutFiles.equals(cb.getFiles());
    }

    public static void clearAfterMove() {
        cutFiles = List.of();
        Clipboard.getSystemClipboard().clear();
    }
}
