package com.fileem.fs;

import com.fileem.util.OsUtil;

import java.awt.Desktop;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.FileAlreadyExistsException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.time.temporal.ChronoUnit;
import java.util.ArrayList;
import java.util.List;

/** Move to the OS recycle bin / Trash. */
public final class Trash {
    private static volatile Boolean gioAvailable;

    private Trash() {
    }

    public static boolean isSupported() {
        return OsUtil.isLinux() || desktopTrash();
    }

    private static boolean desktopTrash() {
        try {
            return Desktop.isDesktopSupported() && Desktop.getDesktop().isSupported(Desktop.Action.MOVE_TO_TRASH);
        } catch (Throwable t) {
            return false;
        }
    }

    public static void moveToTrash(Path p) throws IOException {
        if (OsUtil.isLinux()) {
            if (!gioTrash(p)) {
                freedesktopTrash(p);
            }
            return;
        }
        if (!desktopTrash()) {
            throw new IOException("Trash is not supported on this system.");
        }
        if (!Desktop.getDesktop().moveToTrash(p.toFile())) {
            throw new IOException("Could not move to Trash.");
        }
    }

    /**
     * Trashes several items with a single process where the platform allows it (gio on Linux).
     * Returns false when nothing or not everything was trashed; callers then fall back to
     * {@link #moveToTrash(Path)} per remaining item to get individual errors.
     */
    public static boolean moveAllToTrash(List<Path> paths) {
        if (!OsUtil.isLinux() || Boolean.FALSE.equals(gioAvailable) || paths.isEmpty()) {
            return false;
        }
        List<String> cmd = new ArrayList<>(List.of("gio", "trash", "--"));
        paths.forEach(p -> cmd.add(p.toString()));
        try {
            Process process = new ProcessBuilder(cmd).redirectErrorStream(true)
                    .redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
            gioAvailable = true;
            return process.waitFor() == 0;
        } catch (IOException e) {
            gioAvailable = false;
            return false;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    /** Uses GIO, which knows about per-volume trash folders. Returns false when gio is not installed. */
    private static boolean gioTrash(Path p) throws IOException {
        if (Boolean.FALSE.equals(gioAvailable)) {
            return false;
        }
        Process process;
        try {
            process = new ProcessBuilder("gio", "trash", "--", p.toString()).redirectErrorStream(true).start();
        } catch (IOException e) {
            gioAvailable = false;
            return false;
        }
        gioAvailable = true;
        try {
            String out = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8).trim();
            if (process.waitFor() != 0) {
                throw new IOException(out.isEmpty() ? "gio trash failed" : out);
            }
            return true;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IOException("Interrupted");
        }
    }

    /** Minimal freedesktop.org trash implementation for the home trash. */
    private static void freedesktopTrash(Path p) throws IOException {
        String dataHome = System.getenv("XDG_DATA_HOME");
        Path trash = (dataHome != null && !dataHome.isBlank() ? Path.of(dataHome)
                : OsUtil.home().resolve(".local").resolve("share")).resolve("Trash");
        Path files = trash.resolve("files");
        Path info = trash.resolve("info");
        Files.createDirectories(files);
        Files.createDirectories(info);
        // the volume of the item itself, not of a link's target: links are trashed as links
        Path parent = p.toAbsolutePath().getParent();
        if (parent == null || !Files.getFileStore(parent).equals(Files.getFileStore(files))) {
            throw new IOException("Trash is not available on this volume. Use Shift+Delete to delete permanently.");
        }
        String name = p.getFileName().toString();
        String trashName = name;
        Path infoFile;
        for (int i = 2; ; i++) {
            infoFile = info.resolve(trashName + ".trashinfo");
            try {
                Files.createFile(infoFile);
                break;
            } catch (FileAlreadyExistsException e) {
                trashName = name + "." + i;
            }
        }
        String encoded = p.toAbsolutePath().toUri().getRawPath();
        if (encoded.length() > 1 && encoded.endsWith("/")) {
            encoded = encoded.substring(0, encoded.length() - 1);
        }
        String date = LocalDateTime.now().truncatedTo(ChronoUnit.SECONDS).format(DateTimeFormatter.ISO_LOCAL_DATE_TIME);
        Files.writeString(infoFile, "[Trash Info]\nPath=" + encoded + "\nDeletionDate=" + date + "\n");
        try {
            Files.move(p, files.resolve(trashName));
        } catch (IOException e) {
            Files.deleteIfExists(infoFile);
            throw e;
        }
    }
}
