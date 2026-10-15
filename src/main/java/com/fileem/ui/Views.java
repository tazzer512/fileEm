package com.fileem.ui;

import javafx.scene.control.ContextMenu;
import javafx.scene.control.Control;
import javafx.scene.control.MenuItem;
import javafx.scene.control.TableView;
import javafx.scene.control.TreeView;
import javafx.scene.control.skin.VirtualFlow;
import javafx.scene.input.ContextMenuEvent;

import java.util.List;
import java.util.function.Supplier;

final class Views {
    private Views() {
    }

    /**
     * Attaches a context menu whose items are rebuilt for every request. The items must be filled in
     * before JavaFX shows it ({@code ContextMenu.show} silently does nothing for an empty menu, and
     * {@code onShowing} runs too late), so they are built in an event filter on the request.
     */
    static ContextMenu dynamicContextMenu(Control control, Supplier<List<MenuItem>> items) {
        ContextMenu menu = new ContextMenu();
        control.setContextMenu(menu);
        control.addEventFilter(ContextMenuEvent.CONTEXT_MENU_REQUESTED, e -> menu.getItems().setAll(items.get()));
        return menu;
    }

    /** Scrolls just enough to make the row visible, instead of TableView.scrollTo's jump-to-top. */
    static void ensureVisible(Control control, int index) {
        ensureVisible(control, index, 0);
    }

    /** Like {@link #ensureVisible(Control, int)}, but a scroll puts the item {@code context} rows below the top. */
    static void ensureVisible(Control control, int index, int context) {
        if (index < 0) {
            return;
        }
        if (control.lookup(".virtual-flow") instanceof VirtualFlow<?> flow) {
            var first = flow.getFirstVisibleCell();
            var last = flow.getLastVisibleCell();
            if (first != null && last != null && index > first.getIndex() && index < last.getIndex()) {
                return; // already fully on screen: skip the (row-measuring) scroll
            }
            if (context > 0) {
                flow.scrollToTop(Math.max(0, index - context)); // the item lands below its context
            } else {
                flow.scrollTo(index);
            }
        } else if (control instanceof TableView<?> t) {
            t.scrollTo(Math.max(0, index - context));
        } else if (control instanceof TreeView<?> t) {
            t.scrollTo(Math.max(0, index - context));
        }
    }
}
