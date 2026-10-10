package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.Bookmarks;
import com.fileem.fs.Volumes;
import com.fileem.fs.Volumes.Volume;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import com.fileem.util.NaturalOrder;
import com.fileem.util.OsUtil;
import javafx.collections.ListChangeListener;
import javafx.collections.ObservableList;
import javafx.css.PseudoClass;
import javafx.geometry.Pos;
import javafx.scene.control.Button;
import javafx.scene.control.ContextMenu;
import javafx.scene.control.Label;
import javafx.scene.control.TreeCell;
import javafx.scene.control.TreeItem;
import javafx.scene.control.TreeView;
import javafx.scene.input.ClipboardContent;
import javafx.scene.input.ContextMenuEvent;
import javafx.scene.input.Dragboard;
import javafx.scene.input.MouseButton;
import javafx.scene.input.MouseEvent;
import javafx.scene.input.TransferMode;
import javafx.scene.layout.HBox;
import org.kordamp.ikonli.javafx.FontIcon;

import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Lazily loaded folder tree with quick-access folders and volumes. */
final class FolderTree extends TreeView<TreeNode> {
    private static final PseudoClass HEADER = PseudoClass.getPseudoClass("header");

    private final MainWindow main;
    private final ExplorerSide side;
    private final TreeItem<TreeNode> quickHeader;
    private final TreeItem<TreeNode> bookmarksHeader;
    private final TreeItem<TreeNode> volumesHeader;
    private int programmatic;
    private int revealToken;

    FolderTree(MainWindow main, ExplorerSide side) {
        this.main = main;
        this.side = side;
        getStyleClass().add("folder-tree");
        setMinWidth(0);

        quickHeader = new TreeItem<>(TreeNode.header("Quick access", "mdi2s-star"));
        volumesHeader = new TreeItem<>(TreeNode.header(OsUtil.isWindows() ? "This PC" : "Computer", "mdi2l-laptop"));
        bookmarksHeader = new TreeItem<>(new TreeNode(TreeNode.Kind.BOOKMARK_HEADER, null, "Bookmarks",
                "mdi2b-bookmark-multiple", "icon-header", null));
        quickHeader.setExpanded(true);
        bookmarksHeader.setExpanded(true);
        volumesHeader.setExpanded(true);
        TreeItem<TreeNode> root = new TreeItem<>(TreeNode.header("", null));
        root.getChildren().setAll(List.of(quickHeader, volumesHeader));
        setRoot(root);
        setShowRoot(false);

        for (TreeNode n : quickAccess()) {
            quickHeader.getChildren().add(new FolderItem(n));
        }

        setCellFactory(tv -> new FolderCell());
        getSelectionModel().selectedItemProperty().addListener((o, old, item) -> {
            if (programmatic > 0 || item == null || item.getValue().path() == null) {
                return;
            }
            FileTab tab = side.activeTab();
            if (tab != null) {
                tab.navigateTo(item.getValue().path());
            }
        });

        main.bookmarks().items().addListener((ListChangeListener<Bookmarks.Bookmark>) c -> syncBookmarks());
        syncBookmarks();

        Volumes.current().addListener((o, old, vols) -> syncVolumes(vols));
        syncVolumes(Volumes.current().get());
        main.showHiddenProperty().addListener(o -> reloadAll(getRoot()));
    }

    // ---- quick access ----

    private static List<TreeNode> quickAccess() {
        Path home = OsUtil.home();
        Map<String, Path> xdg = OsUtil.isLinux() ? xdgUserDirs(home) : Map.of();
        List<TreeNode> out = new ArrayList<>();
        out.add(new TreeNode(TreeNode.Kind.QUICK, home, "Home", "mdi2h-home", "icon-quick", null));
        addQuick(out, home, xdg.getOrDefault("DESKTOP", home.resolve("Desktop")), "mdi2m-monitor");
        addQuick(out, home, xdg.getOrDefault("DOCUMENTS", home.resolve("Documents")), "mdi2f-file-document-multiple");
        addQuick(out, home, xdg.getOrDefault("DOWNLOAD", home.resolve("Downloads")), "mdi2d-download");
        addQuick(out, home, xdg.getOrDefault("PICTURES", home.resolve("Pictures")), "mdi2i-image-multiple");
        addQuick(out, home, xdg.getOrDefault("MUSIC", home.resolve("Music")), "mdi2m-music-box");
        addQuick(out, home, xdg.getOrDefault("VIDEOS", home.resolve(OsUtil.isMac() ? "Movies" : "Videos")),
                "mdi2f-filmstrip-box-multiple");
        return out;
    }

    private static void addQuick(List<TreeNode> out, Path home, Path dir, String icon) {
        if (!dir.equals(home) && Files.isDirectory(dir)) {
            out.add(new TreeNode(TreeNode.Kind.QUICK, dir, FsUtil.displayName(dir), icon, "icon-quick", null));
        }
    }

    /** Reads localized folder names from ~/.config/user-dirs.dirs. */
    private static Map<String, Path> xdgUserDirs(Path home) {
        Map<String, Path> out = new HashMap<>();
        Path file = home.resolve(".config").resolve("user-dirs.dirs");
        Pattern line = Pattern.compile("XDG_(\\w+)_DIR=\"(.*)\"");
        try {
            for (String l : Files.readAllLines(file)) {
                Matcher m = line.matcher(l.trim());
                if (m.matches()) {
                    String value = m.group(2).replace("$HOME", home.toString());
                    out.put(m.group(1), Path.of(value));
                }
            }
        } catch (IOException | RuntimeException ignored) {
            // use the English defaults
        }
        return out;
    }

    // ---- bookmarks ----

    /** Mirrors the shared bookmark list; the section is hidden while there are no bookmarks. */
    private void syncBookmarks() {
        Map<Path, FolderItem> existing = new HashMap<>();
        for (TreeItem<TreeNode> c : bookmarksHeader.getChildren()) {
            existing.put(c.getValue().path(), (FolderItem) c);
        }
        List<TreeItem<TreeNode>> wanted = new ArrayList<>();
        for (Bookmarks.Bookmark b : main.bookmarks().items()) {
            TreeNode node = TreeNode.bookmark(b.path(), b.name());
            FolderItem item = existing.get(b.path());
            if (item == null) {
                item = new FolderItem(node);
            } else if (!node.equals(item.getValue())) {
                item.setValue(node);
            }
            wanted.add(item);
        }
        programmatic++;
        try {
            syncChildren(bookmarksHeader.getChildren(), wanted);
            ObservableList<TreeItem<TreeNode>> top = getRoot().getChildren();
            boolean shown = top.contains(bookmarksHeader);
            if (wanted.isEmpty() && shown) {
                top.remove(bookmarksHeader);
            } else if (!wanted.isEmpty() && !shown) {
                top.add(1, bookmarksHeader);
            }
        } finally {
            programmatic--;
        }
    }

    // ---- volumes ----

    /** Shows {@code volumes} under the Computer header (package-private: tests pass their own). */
    void syncVolumes(List<Volume> volumes) {
        Map<Path, FolderItem> existing = new HashMap<>();
        for (TreeItem<TreeNode> c : volumesHeader.getChildren()) {
            existing.put(c.getValue().path(), (FolderItem) c);
        }
        List<TreeItem<TreeNode>> wanted = new ArrayList<>();
        for (Volume v : volumes) {
            String icon = switch (v.kind()) {
                case REMOVABLE -> "mdi2u-usb-flash-drive";
                case NETWORK -> "mdi2s-server-network";
                default -> "mdi2h-harddisk";
            };
            String extra = v.free() >= 0 && v.total() > 0
                    ? Formats.size(v.free()) + " free of " + Formats.size(v.total()) : null;
            TreeNode node = new TreeNode(TreeNode.Kind.VOLUME, v.root(), v.label(), icon, "icon-volume", extra,
                    v.ejectable());
            FolderItem item = existing.get(v.root());
            if (item == null) {
                item = new FolderItem(node);
            } else if (!node.equals(item.getValue())) {
                item.setValue(node);
            }
            wanted.add(item);
        }
        programmatic++;
        try {
            syncChildren(volumesHeader.getChildren(), wanted);
        } finally {
            programmatic--;
        }
        if (getSelectionModel().getSelectedItem() == null) {
            FileTab tab = side.activeTab();
            if (tab != null && tab.currentDir() != null) {
                reveal(tab.currentDir());
            }
        }
    }

    /** Updates {@code children} to {@code wanted} with minimal changes so expansion and selection survive. */
    private static void syncChildren(ObservableList<TreeItem<TreeNode>> children, List<TreeItem<TreeNode>> wanted) {
        Set<TreeItem<TreeNode>> keep = Collections.newSetFromMap(new IdentityHashMap<>());
        keep.addAll(wanted);
        children.removeIf(c -> !keep.contains(c));
        int j = 0;
        for (TreeItem<TreeNode> c : children) {
            while (j < wanted.size() && wanted.get(j) != c) {
                j++;
            }
            if (j == wanted.size()) {
                children.setAll(wanted);
                return;
            }
        }
        for (int i = 0; i < wanted.size(); i++) {
            if (i >= children.size() || children.get(i) != wanted.get(i)) {
                children.add(i, wanted.get(i));
            }
        }
    }

    // ---- reveal / refresh ----

    /** Expands the tree down to {@code dir} and selects it, without navigating. */
    void reveal(Path dir) {
        if (dir == null) {
            return;
        }
        TreeItem<TreeNode> selected = getSelectionModel().getSelectedItem();
        if (selected != null && dir.equals(selected.getValue().path())) {
            return;
        }
        int token = ++revealToken;
        FolderItem best = null;
        int bestDepth = -1;
        for (TreeItem<TreeNode> v : volumesHeader.getChildren()) {
            Path r = v.getValue().path();
            if (dir.startsWith(r) && r.getNameCount() > bestDepth) {
                best = (FolderItem) v;
                bestDepth = r.getNameCount();
            }
        }
        if (best == null) {
            programmatic++;
            try {
                getSelectionModel().clearSelection();
            } finally {
                programmatic--;
            }
            return;
        }
        List<String> names = new ArrayList<>();
        if (!dir.equals(best.path())) {
            for (Path n : best.path().relativize(dir)) {
                names.add(n.toString());
            }
        }
        step(best, names, 0, token);
    }

    private void step(FolderItem item, List<String> names, int i, int token) {
        if (token != revealToken) {
            return;
        }
        if (i == names.size()) {
            select(item);
            return;
        }
        item.load(() -> {
            if (token != revealToken) {
                return;
            }
            Path next = item.path().resolve(names.get(i));
            for (TreeItem<TreeNode> c : item.getChildren()) {
                if (c instanceof FolderItem f && f.path().equals(next)) {
                    item.setExpanded(true);
                    step(f, names, i + 1, token);
                    return;
                }
            }
            select(item);
        });
    }

    private void select(TreeItem<TreeNode> item) {
        programmatic++;
        try {
            getSelectionModel().select(item);
        } finally {
            programmatic--;
        }
        Views.ensureVisible(this, getRow(item));
    }

    /** Reloads the children of every loaded tree item showing {@code dir}. */
    void refreshDir(Path dir) {
        forEachLoaded(getRoot(), item -> {
            if (dir.equals(item.path())) {
                item.reload();
            }
        });
    }

    private void reloadAll(TreeItem<TreeNode> root) {
        forEachLoaded(root, FolderItem::reload);
    }

    private void forEachLoaded(TreeItem<TreeNode> item, java.util.function.Consumer<FolderItem> action) {
        if (item instanceof FolderItem f) {
            if (!f.loaded) {
                return;
            }
            action.accept(f);
        }
        for (TreeItem<TreeNode> c : List.copyOf(item.getChildren())) {
            forEachLoaded(c, action);
        }
    }

    private static List<Path> listSubfolders(Path dir, boolean showHidden) throws IOException {
        List<Path> out = new ArrayList<>();
        try (DirectoryStream<Path> ds = Files.newDirectoryStream(dir)) {
            for (Path p : ds) {
                if (Files.isDirectory(p) && (showHidden || !FsUtil.isHidden(p))) {
                    out.add(p);
                }
            }
        }
        out.sort(Comparator.comparing(FsUtil::displayName, NaturalOrder.INSTANCE));
        return out;
    }

    // ---- items & cells ----

    private final class FolderItem extends TreeItem<TreeNode> {
        private boolean loaded;
        private boolean loading;
        private final List<Runnable> waiting = new ArrayList<>();

        FolderItem(TreeNode node) {
            super(node);
            getChildren().add(new TreeItem<>(TreeNode.loading()));
            expandedProperty().addListener((o, was, expanded) -> {
                if (expanded) {
                    load(null);
                }
            });
        }

        Path path() {
            return getValue().path();
        }

        void load(Runnable then) {
            if (loaded) {
                if (then != null) {
                    then.run();
                }
                return;
            }
            if (then != null) {
                waiting.add(then);
            }
            if (!loading) {
                fetch();
            }
        }

        void reload() {
            if (loaded && !loading) {
                fetch();
            }
        }

        private void fetch() {
            loading = true;
            Path dir = path();
            boolean hidden = main.showHiddenProperty().get();
            Bg.run(() -> listSubfolders(dir, hidden), this::apply, err -> apply(List.of()));
        }

        private void apply(List<Path> dirs) {
            loading = false;
            loaded = true;
            Map<Path, TreeItem<TreeNode>> existing = new HashMap<>();
            for (TreeItem<TreeNode> c : getChildren()) {
                if (c.getValue().path() != null) {
                    existing.put(c.getValue().path(), c);
                }
            }
            List<TreeItem<TreeNode>> wanted = new ArrayList<>(dirs.size());
            for (Path p : dirs) {
                TreeItem<TreeNode> item = existing.get(p);
                wanted.add(item != null ? item : new FolderItem(TreeNode.folder(p)));
            }
            programmatic++;
            try {
                syncChildren(getChildren(), wanted);
            } finally {
                programmatic--;
            }
            List<Runnable> callbacks = List.copyOf(waiting);
            waiting.clear();
            callbacks.forEach(Runnable::run);
        }
    }

    private final class FolderCell extends TreeCell<TreeNode> {
        private final FontIcon icon = Icons.icon("mdi2f-folder");
        private final Label name = new Label();
        private final Label extra = new Label();
        private final Button eject = new Button(null, Icons.icon("mdi2e-eject", 14));
        private final HBox box = new HBox(6, icon, name, eject, extra); // eject before the free space, which gets cut off first
        private final javafx.scene.control.Tooltip tooltip = new javafx.scene.control.Tooltip();

        FolderCell() {
            box.setAlignment(Pos.CENTER_LEFT);
            extra.getStyleClass().add("tree-extra");
            name.setMinWidth(0);
            eject.getStyleClass().addAll(Styles.BUTTON_ICON, Styles.FLAT, "eject-button");
            eject.setFocusTraversable(false);
            eject.setTooltip(new javafx.scene.control.Tooltip("Eject (safely remove)"));
            // a press must not reach the cell: selecting the drive would navigate onto it
            eject.addEventHandler(MouseEvent.MOUSE_PRESSED, MouseEvent::consume);
            eject.setOnAction(e -> {
                TreeNode n = getItem();
                if (n != null && n.ejectable()) {
                    main.eject(n.path(), n.label());
                }
            });

            ContextMenu menu = Views.dynamicContextMenu(this, () -> {
                TreeNode n = getItem();
                return n != null && n.path() != null ? main.treeMenuItems(n)
                        : n != null && n.kind() == TreeNode.Kind.BOOKMARK_HEADER ? main.bookmarksHeaderMenuItems()
                        : List.of();
            });
            // Right-click must not select (selecting navigates the tab); outline the item instead.
            addEventFilter(MouseEvent.MOUSE_PRESSED, e -> {
                if (e.getButton() == MouseButton.SECONDARY) {
                    e.consume();
                }
            });
            addEventFilter(ContextMenuEvent.CONTEXT_MENU_REQUESTED, e -> {
                if (getItem() != null && !getStyleClass().contains("context-target")) {
                    getStyleClass().add("context-target");
                }
            });
            menu.setOnHidden(e -> getStyleClass().remove("context-target"));

            setOnDragDetected(e -> {
                TreeNode n = getItem();
                if (n == null || n.path() == null || n.kind() == TreeNode.Kind.VOLUME) {
                    return;
                }
                Dragboard db = startDragAndDrop(TransferMode.COPY_OR_MOVE);
                ClipboardContent content = new ClipboardContent();
                content.putFiles(List.of(n.path().toFile()));
                db.setContent(content);
                db.setDragView(snapshot(null, null));
                e.consume();
            });
            setOnDragOver(e -> {
                TreeNode n = getItem();
                if (n != null && Dnd.accept(e, n.path())) {
                    e.consume();
                }
            });
            setOnDragEntered(e -> {
                TreeNode n = getItem();
                if (n != null && Dnd.canDrop(e, n.path())) {
                    getStyleClass().add("drop-target");
                }
            });
            setOnDragExited(e -> getStyleClass().remove("drop-target"));
            setOnDragDropped(e -> {
                TreeNode n = getItem();
                if (n != null && Dnd.canDrop(e, n.path())) {
                    main.drop(e, n.path());
                }
            });
        }

        @Override
        protected void updateItem(TreeNode n, boolean empty) {
            super.updateItem(n, empty);
            getStyleClass().removeAll("drop-target", "context-target");
            setText(null);
            if (empty || n == null) {
                setGraphic(null);
                pseudoClassStateChanged(HEADER, false);
                return;
            }
            pseudoClassStateChanged(HEADER, n.isHeader());
            icon.setVisible(n.icon() != null);
            icon.setManaged(n.icon() != null);
            if (n.icon() != null) {
                Icons.apply(icon, n.icon(), n.iconClass());
            }
            name.setText(n.label());
            boolean ejecting = n.ejectable() && main.isEjecting(n.path());
            String extraText = ejecting ? "Ejecting…" : n.extra();
            extra.setText(extraText == null ? "" : extraText);
            extra.setVisible(extraText != null);
            extra.setManaged(extraText != null);
            eject.setVisible(n.ejectable());
            eject.setManaged(n.ejectable());
            eject.setDisable(ejecting);
            if (getGraphic() != box) {
                // setGraphic makes the skin re-run CSS on the disclosure arrow, so only when needed
                setGraphic(box);
            }
            if (n.kind() == TreeNode.Kind.BOOKMARK) {
                tooltip.setText(n.path().toString());
                setTooltip(tooltip);
            } else if (getTooltip() != null) {
                setTooltip(null);
            }
        }
    }
}
