package com.fileem.ui;

import javafx.scene.control.Control;
import javafx.scene.control.TableView;
import javafx.scene.control.TreeView;
import javafx.scene.control.skin.VirtualFlow;

final class Views {
    private Views() {
    }

    /** Scrolls just enough to make the row visible, instead of TableView.scrollTo's jump-to-top. */
    static void ensureVisible(Control control, int index) {
        if (index < 0) {
            return;
        }
        if (control.lookup(".virtual-flow") instanceof VirtualFlow<?> flow) {
            flow.scrollTo(index);
        } else if (control instanceof TableView<?> t) {
            t.scrollTo(index);
        } else if (control instanceof TreeView<?> t) {
            t.scrollTo(index);
        }
    }
}
