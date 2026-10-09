package com.fileem.ui;

import com.fileem.util.FsUtil;

import java.nio.file.Path;

/** Value of a folder-tree item. */
record TreeNode(Kind kind, Path path, String label, String icon, String iconClass, String extra) {
    enum Kind { HEADER, QUICK, VOLUME, FOLDER, LOADING }

    static TreeNode folder(Path p) {
        return new TreeNode(Kind.FOLDER, p, FsUtil.displayName(p), "mdi2f-folder", "icon-folder", null);
    }

    static TreeNode loading() {
        return new TreeNode(Kind.LOADING, null, "Loading…", null, null, null);
    }

    static TreeNode header(String label, String icon) {
        return new TreeNode(Kind.HEADER, null, label, icon, "icon-header", null);
    }
}
