package com.fileem.fs;

import com.fileem.util.OsUtil;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.DosFileAttributes;
import java.nio.file.attribute.FileTime;
import java.nio.file.attribute.PosixFileAttributes;
import java.nio.file.attribute.PosixFilePermissions;

/** Immutable snapshot of one directory entry, as shown in the file list. */
public final class FileEntry {
    private static final LinkOption[] NOFOLLOW = {LinkOption.NOFOLLOW_LINKS};
    private static final LinkOption[] FOLLOW = {};

    private final Path path;
    private final String name;
    private final boolean directory;
    private final boolean symlink;
    private final long size;
    private final FileTime modified;
    private final FileTime created;
    private final FileTime accessed;
    private final boolean hidden;
    /** Raw attributes, kept so the (slow, owner lookup) attribute string is only built on demand. */
    private final BasicFileAttributes raw;
    private volatile String attributes;
    private final String extension;
    private final FileTypes.Category category;
    private final String type;

    private FileEntry(Path path, BasicFileAttributes a, boolean symlink, boolean hidden) {
        this.path = path;
        this.name = path.getFileName() != null ? path.getFileName().toString() : path.toString();
        this.directory = a != null && a.isDirectory();
        this.symlink = symlink;
        this.size = a == null || directory ? -1 : a.size();
        this.modified = a == null ? null : a.lastModifiedTime();
        this.created = a == null ? null : a.creationTime();
        this.accessed = a == null ? null : a.lastAccessTime();
        this.hidden = hidden;
        this.raw = a;
        this.extension = FileTypes.extension(name, directory);
        FileTypes.Category cat = FileTypes.category(extension, directory);
        this.category = symlink && !directory && cat == FileTypes.Category.FILE ? FileTypes.Category.LINK : cat;
        this.type = symlink && directory ? "Folder link" : FileTypes.description(extension, category);
    }

    /** Reads the attributes of {@code p}. Never throws: unreadable entries still get a row. */
    public static FileEntry read(Path p) {
        String fileName = p.getFileName() != null ? p.getFileName().toString() : "";
        BasicFileAttributes own;
        try {
            own = readAttributes(p, NOFOLLOW);
        } catch (IOException e) {
            return new FileEntry(p, null, false, fileName.startsWith("."));
        }
        boolean link = own.isSymbolicLink();
        BasicFileAttributes a = own;
        if (link) {
            try {
                a = readAttributes(p, FOLLOW);
            } catch (IOException ignored) {
                // broken link: keep the link's own attributes
            }
        }
        boolean hidden = a instanceof DosFileAttributes dos ? dos.isHidden() : fileName.startsWith(".");
        return new FileEntry(p, a, link, hidden);
    }

    private static BasicFileAttributes readAttributes(Path p, LinkOption[] opts) throws IOException {
        try {
            if (OsUtil.isPosix()) {
                return Files.readAttributes(p, PosixFileAttributes.class, opts);
            }
            if (OsUtil.isWindows()) {
                return Files.readAttributes(p, DosFileAttributes.class, opts);
            }
        } catch (UnsupportedOperationException ignored) {
            // fall through to basic attributes
        }
        return Files.readAttributes(p, BasicFileAttributes.class, opts);
    }

    public Path path() {
        return path;
    }

    public String name() {
        return name;
    }

    public boolean isDirectory() {
        return directory;
    }

    public boolean isSymlink() {
        return symlink;
    }

    /** Size in bytes, -1 for folders. */
    public long size() {
        return size;
    }

    public FileTime modified() {
        return modified;
    }

    public FileTime created() {
        return created;
    }

    public FileTime accessed() {
        return accessed;
    }

    public boolean isHidden() {
        return hidden;
    }

    /** "drwxr-xr-x  owner" on POSIX, "RHSA" style flags on Windows. Computed lazily (owner lookup is slow). */
    public String attributes() {
        String a = attributes;
        if (a == null) {
            a = formatAttributes();
            attributes = a;
        }
        return a;
    }

    private String formatAttributes() {
        if (raw instanceof PosixFileAttributes px) {
            char kind = symlink ? 'l' : directory ? 'd' : '-';
            String owner;
            try {
                owner = px.owner() != null ? px.owner().getName() : "";
            } catch (RuntimeException e) {
                owner = "";
            }
            return kind + PosixFilePermissions.toString(px.permissions()) + "  " + owner;
        }
        if (raw instanceof DosFileAttributes dos) {
            StringBuilder sb = new StringBuilder();
            if (dos.isReadOnly()) sb.append('R');
            if (dos.isHidden()) sb.append('H');
            if (dos.isSystem()) sb.append('S');
            if (dos.isArchive()) sb.append('A');
            if (symlink) sb.append('L');
            return sb.toString();
        }
        return "";
    }

    public String extension() {
        return extension;
    }

    public FileTypes.Category category() {
        return category;
    }

    public String type() {
        return type;
    }
}
