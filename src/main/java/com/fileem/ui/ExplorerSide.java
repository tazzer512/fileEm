package com.fileem.ui;

import com.fileem.Session;
import com.fileem.util.OsUtil;
import javafx.application.Platform;
import javafx.css.PseudoClass;
import javafx.scene.control.ContextMenu;
import javafx.scene.control.MenuItem;
import javafx.scene.control.SeparatorMenuItem;
import javafx.scene.control.SplitPane;
import javafx.scene.control.Tab;
import javafx.scene.control.TabPane;
import javafx.scene.control.Tooltip;
import javafx.scene.input.MouseEvent;

import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.util.List;

/** One explorer side: a folder tree next to a tab pane of file lists. */
public final class ExplorerSide extends SplitPane {
    private static final PseudoClass ACTIVE = PseudoClass.getPseudoClass("active");
    private static final double DEFAULT_TREE_WIDTH = 240;
    private static final double MIN_LIST_WIDTH = 260;

    private final MainWindow main;
    private final String id;
    private final FolderTree tree;
    private final TabPane tabPane = new TabPane();
    private final Tab plusTab = new Tab();
    private boolean closing;

    ExplorerSide(MainWindow main, String id) {
        this.main = main;
        this.id = id;
        getStyleClass().add("explorer-side");
        tree = new FolderTree(main, this);

        tabPane.setTabClosingPolicy(TabPane.TabClosingPolicy.ALL_TABS);
        tabPane.setTabDragPolicy(TabPane.TabDragPolicy.FIXED);
        plusTab.setClosable(false);
        plusTab.setGraphic(Icons.icon("mdi2p-plus", 14));
        plusTab.setTooltip(new Tooltip("New tab (Ctrl+T)"));
        plusTab.getStyleClass().add("plus-tab");
        tabPane.getTabs().add(plusTab);
        tabPane.getSelectionModel().selectedItemProperty().addListener((o, old, tab) -> {
            if (tab == plusTab) {
                Platform.runLater(() -> {
                    if (closing || old == null || old == plusTab) {
                        selectLastRealTab();
                    } else {
                        FileTab from = (FileTab) old.getUserData();
                        newTab(from != null ? from.currentDir() : OsUtil.home(), true);
                    }
                });
                return;
            }
            FileTab ft = tab == null ? null : (FileTab) tab.getUserData();
            if (ft != null) {
                tree.reveal(ft.currentDir());
                main.tabChanged(ft);
            }
        });

        getItems().addAll(tree, tabPane);
        SplitPane.setResizableWithParent(tree, false);
        // the tree keeps its width when the window resizes, but never at the cost of the file list
        tabPane.setMinWidth(MIN_LIST_WIDTH);
        setTreeWidth(DEFAULT_TREE_WIDTH);
        addEventFilter(MouseEvent.MOUSE_PRESSED, e -> main.setActiveSide(this));
    }

    FolderTree tree() {
        return tree;
    }

    List<FileTab> tabs() {
        return tabPane.getTabs().stream().filter(t -> t != plusTab).map(t -> (FileTab) t.getUserData()).toList();
    }

    public FileTab activeTab() {
        Tab t = tabPane.getSelectionModel().getSelectedItem();
        if (t == null || t == plusTab) {
            List<FileTab> all = tabs();
            return all.isEmpty() ? null : all.get(all.size() - 1);
        }
        return (FileTab) t.getUserData();
    }

    void setActive(boolean active) {
        pseudoClassStateChanged(ACTIVE, active);
    }

    /** Sets the tree width in pixels once this side has been laid out (divider positions are fractions). */
    private void setTreeWidth(double px) {
        if (getWidth() > 0) {
            setDividerPositions(Math.min(0.6, px / getWidth()));
            return;
        }
        widthProperty().addListener(new javafx.beans.value.ChangeListener<>() {
            @Override
            public void changed(javafx.beans.value.ObservableValue<? extends Number> o, Number old, Number w) {
                if (w.doubleValue() > 0) {
                    widthProperty().removeListener(this);
                    setDividerPositions(Math.min(0.6, px / w.doubleValue()));
                }
            }
        });
    }

    // ---- tabs ----

    FileTab newTab(Path dir, boolean select) {
        FileTab from = activeTab();
        FileTab ft = new FileTab(main, this, dir);
        if (from != null) {
            ft.setViewMode(from.viewMode()); // new tabs open in the view you're using
        }
        Tab tab = ft.tab();
        tab.setOnCloseRequest(e -> {
            if (tabs().size() <= 1) {
                e.consume();
            } else {
                closing = true;
            }
        });
        tab.setOnClosed(e -> {
            closing = false;
            ft.dispose();
        });
        tab.setContextMenu(tabMenu(ft));
        tabPane.getTabs().add(tabPane.getTabs().size() - 1, tab);
        if (select) {
            tabPane.getSelectionModel().select(tab);
            Platform.runLater(ft::focusTable);
        }
        return ft;
    }

    void closeTab(FileTab ft) {
        if (tabs().size() <= 1) {
            return;
        }
        closing = true;
        try {
            tabPane.getTabs().remove(ft.tab());
        } finally {
            closing = false;
        }
        ft.dispose();
    }

    void closeActiveTab() {
        FileTab t = activeTab();
        if (t != null) {
            closeTab(t);
        }
    }

    private void selectLastRealTab() {
        List<Tab> all = tabPane.getTabs();
        if (all.size() > 1) {
            tabPane.getSelectionModel().select(all.size() - 2);
        }
    }

    void selectTab(FileTab ft) {
        tabPane.getSelectionModel().select(ft.tab());
    }

    private ContextMenu tabMenu(FileTab ft) {
        MenuItem dup = new MenuItem("Duplicate tab");
        dup.setOnAction(e -> newTab(ft.currentDir(), true));
        MenuItem other = new MenuItem("Open in other side");
        other.setOnAction(e -> main.openInOtherSide(ft.currentDir()));
        MenuItem close = new MenuItem("Close tab");
        close.setOnAction(e -> closeTab(ft));
        MenuItem closeOthers = new MenuItem("Close other tabs");
        closeOthers.setOnAction(e -> tabs().stream().filter(t -> t != ft).forEach(this::closeTab));
        ContextMenu menu = new ContextMenu(dup, other, new SeparatorMenuItem(), close, closeOthers);
        menu.setOnShowing(e -> {
            close.setDisable(tabs().size() <= 1);
            closeOthers.setDisable(tabs().size() <= 1);
        });
        return menu;
    }

    /** Called by a tab once a navigation has completed. */
    void tabNavigated(FileTab ft) {
        if (ft == activeTab()) {
            tree.reveal(ft.currentDir());
            main.tabChanged(ft);
        }
    }

    // ---- session ----

    void restore(Session s) {
        int count = s.getInt(id + ".tabs", 0);
        int selected = s.getInt(id + ".selected", 0);
        for (int i = 0; i < count; i++) {
            Path p = savedDir(s.get(id + ".tab." + i, null));
            if (p != null) {
                newTab(p, false).setViewMode(ViewMode.byId(s.get(id + ".tab." + i + ".view", null)));
            }
        }
        if (tabs().isEmpty()) {
            newTab(OsUtil.home(), false);
        }
        tabPane.getSelectionModel().select(Math.max(0, Math.min(selected, tabs().size() - 1)));
        setTreeWidth(s.getDouble(id + ".treeWidth", DEFAULT_TREE_WIDTH));
    }

    void save(Session s) {
        s.removePrefix(id + ".tab.");
        List<FileTab> all = tabs();
        s.put(id + ".tabs", all.size());
        for (int i = 0; i < all.size(); i++) {
            Path dir = all.get(i).currentDir();
            s.put(id + ".tab." + i, dir != null ? dir.toString() : OsUtil.home().toString());
            s.put(id + ".tab." + i + ".view", all.get(i).viewMode().id());
        }
        s.put(id + ".selected", Math.max(0, all.indexOf(activeTab())));
        if (tree.getWidth() > 0) {
            s.put(id + ".treeWidth", Math.round(tree.getWidth()));
        }
    }

    /** The saved folder; the tab walks up to an existing one in the background (no disk access here). */
    private static Path savedDir(String value) {
        if (value == null || value.isBlank()) {
            return null;
        }
        try {
            return Path.of(value);
        } catch (InvalidPathException e) {
            return null;
        }
    }
}
