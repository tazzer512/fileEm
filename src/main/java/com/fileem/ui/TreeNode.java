package com.fileem.ui;

import com.fileem.util.FsUtil;

import java.nio.file.Path;

/** Value of a folder-tree item. {@code ejectable}: a volume that can be safely removed (USB, SD card). */
record TreeNode(Kind kind, Path path, String label, String icon, String iconClass, String extra, boolean ejectable) {
    enum Kind { HEADER, BOOKMARK_HEADER, QUICK, BOOKMARK, VOLUME, FOLDER, LOADING }

    TreeNode(Kind kind, Path path, String label, String icon, String iconClass, String extra) {
        this(kind, path, label, icon, iconClass, extra, false);
    }

    static TreeNode folder(Path p) {
        return new TreeNode(Kind.FOLDER, p, FsUtil.displayName(p), "mdi2f-folder", "icon-folder", null);
    }

    static TreeNode loading() {
        return new TreeNode(Kind.LOADING, null, "Loading…", null, null, null);
    }

    static TreeNode bookmark(Path p, String name) {
        return new TreeNode(Kind.BOOKMARK, p, name, "mdi2b-bookmark", "icon-bookmark", null);
    }

    boolean isHeader() {
        return kind == Kind.HEADER || kind == Kind.BOOKMARK_HEADER;
    }

    static TreeNode header(String label, String icon) {
        return new TreeNode(Kind.HEADER, null, label, icon, "icon-header", null);
    }
}
