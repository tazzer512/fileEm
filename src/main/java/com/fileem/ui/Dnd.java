package com.fileem.ui;

import com.fileem.util.FsUtil;
import javafx.scene.input.DragEvent;
import javafx.scene.input.Dragboard;
import javafx.scene.input.TransferMode;

import java.io.File;
import java.nio.file.Path;
import java.util.List;

/** Drag and drop rules shared by the file list and the folder tree. */
final class Dnd {
    private Dnd() {
    }

    static List<Path> files(Dragboard db) {
        return db.hasFiles() ? db.getFiles().stream().map(File::toPath).toList() : List.of();
    }

    /** Whether the dragged files may be dropped into {@code target}. */
    static boolean canDrop(DragEvent e, Path target) {
        if (target == null || !e.getDragboard().hasFiles()) {
            return false;
        }
        List<Path> files = files(e.getDragboard());
        boolean allInTarget = true;
        for (Path f : files) {
            if (FsUtil.isSameOrInside(target, f)) {
                return false;
            }
            if (!target.equals(f.getParent())) {
                allInTarget = false;
            }
        }
        return !allInTarget;
    }

    /** Accepts the drag over {@code target} when that makes sense; returns whether it was accepted. */
    static boolean accept(DragEvent e, Path target) {
        if (!canDrop(e, target)) {
            return false;
        }
        // A drop is a move (confirmed afterwards, with Copy as the alternative: MainWindow.moveOrCopy).
        // Drags from other applications are accepted as a copy all the same: telling them "moved"
        // could let them delete the originals before our background move has copied them.
        e.acceptTransferModes(e.getGestureSource() != null ? TransferMode.MOVE : TransferMode.COPY);
        return true;
    }
}
