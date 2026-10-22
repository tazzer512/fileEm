package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.Bookmarks;
import com.fileem.fs.DirWatcher;
import com.fileem.fs.Volumes;
import com.fileem.fs.Volumes.Volume;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import com.fileem.util.NaturalOrder;
import com.fileem.util.OsUtil;
import javafx.beans.property.ObjectProperty;
import javafx.beans.property.SimpleObjectProperty;
import javafx.beans.value.ObservableValue;
import javafx.collections.ListChangeListener;
import javafx.collections.ObservableList;
import javafx.css.PseudoClass;
import javafx.geometry.Bounds;
import javafx.geometry.Orientation;
import javafx.geometry.Pos;
import javafx.scene.Node;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.ContextMenu;
import javafx.scene.control.Label;
import javafx.scene.control.ScrollBar;
import javafx.scene.control.TreeCell;
import javafx.scene.control.TreeItem;
import javafx.scene.control.TreeView;
import javafx.scene.control.skin.VirtualFlow;
import javafx.scene.input.ClipboardContent;
import javafx.scene.input.ContextMenuEvent;
import javafx.scene.input.Dragboard;
import javafx.scene.input.MouseButton;
import javafx.scene.input.MouseEvent;
import javafx.scene.input.TransferMode;
import javafx.scene.layout.HBox;
import javafx.stage.Window;
import org.kordamp.ikonli.javafx.FontIcon;

import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Lazily loaded folder tree with quick-access folders and volumes. */
final class FolderTree extends TreeView<TreeNode> {
    private static final PseudoClass HEADER = PseudoClass.getPseudoClass("header");
    private static final PseudoClass CURRENT = PseudoClass.getPseudoClass("current");
    private static final PseudoClass ON_PATH = PseudoClass.getPseudoClass("on-path");

    private final MainWindow main;
    private final ExplorerSide side;
    private final TreeItem<TreeNode> quickHeader;
    private final TreeItem<TreeNode> bookmarksHeader;
    private final TreeItem<TreeNode> volumesHeader;
    private int programmatic;
    private int revealToken;
    /** The item showing the active tab's folder (boxed and bold), or null when the tree doesn't show it. */
    private final ObjectProperty<TreeItem<TreeNode>> current = new SimpleObjectProperty<>(this, "current");
    /** Items {@link #reveal} expanded (not the user): collapsed again once the path leaves them. */
    private final Set<TreeItem<TreeNode>> autoExpanded = Collections.newSetFromMap(new IdentityHashMap<>());
    private int autoExpanding;
    /** The item the user just picked in the tree, until the navigation it started is revealed. */
    private TreeItem<TreeNode> clicked;
    private final ObservableValue<Boolean> showing;

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
                clicked = item; // already in view: the reveal after this navigation shouldn't move it
                tab.navigateTo(item.getValue().path());
            }
        });

        main.bookmarks().items().addListener((ListChangeListener<Bookmarks.Bookmark>) c -> syncBookmarks());
        syncBookmarks();

        Volumes.current().addListener((o, old, vols) -> syncVolumes(vols));
        syncVolumes(Volumes.current().get());
        main.showHiddenProperty().addListener(o -> reloadAll(getRoot()));
        // a reveal while hidden doesn't scroll (see shown()): catch up when the window appears
        showing = sceneProperty().flatMap(Scene::windowProperty).flatMap(Window::showingProperty).orElse(false);
        showing.addListener((o, was, now) -> {
            if (now && current.get() != null) {
                scrollToItem(current.get());
            }
        });
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
        for (TreeItem<TreeNode> c : children) {
            if (!keep.contains(c)) {
                stopWatching(c);
            }
        }
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
        TreeItem<TreeNode> fromClick = clicked;
        clicked = null;
        TreeItem<TreeNode> selected = getSelectionModel().getSelectedItem();
        if (selected != null && dir.equals(selected.getValue().path())) {
            markCurrent(selected);
            if (selected != fromClick) {
                scrollToItem(selected); // the same folder again (or another tab on it): show it
            }
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
            markCurrent(null);
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
                    if (!item.isExpanded()) {
                        autoExpanding++;
                        try {
                            item.setExpanded(true);
                        } finally {
                            autoExpanding--;
                        }
                        autoExpanded.add(item);
                    }
                    step(f, names, i + 1, token);
                    return;
                }
            }
            select(item);
        });
    }

    private void select(TreeItem<TreeNode> item) {
        markCurrent(item);
        programmatic++;
        try {
            getSelectionModel().select(item);
        } finally {
            programmatic--;
        }
        scrollToItem(item);
    }

    /**
     * Scrolls {@code item} into view: a few rows below the top so its parents show, and sideways
     * when a deep folder's name lies outside the tree's width. Only for a tree on screen.
     */
    private void scrollToItem(TreeItem<TreeNode> item) {
        if (!shown()) {
            return;
        }
        Views.ensureVisible(this, getRow(item), 3);
        applyCss();
        layout(); // places the cells at the new position
        scrollSideways(item);
        // rows just expanded and the horizontal range (it follows the widest row on screen; a value
        // past the old range is clamped) may only settle in later layout passes: check after a few
        Scene scene = getScene();
        int[] passes = {0};
        Runnable[] again = new Runnable[1];
        again[0] = () -> {
            boolean still = current.get() == item && shown();
            if (++passes[0] == 3 || !still) {
                scene.removePostLayoutPulseListener(again[0]);
            }
            if (still) {
                Views.ensureVisible(this, getRow(item), 3); // a no-op when it is in view
                applyCss();
                layout();
                scrollSideways(item);
            }
        };
        scene.addPostLayoutPulseListener(again[0]);
    }

    /** Scrolls sideways when the start of {@code item}'s name lies outside the tree's width. */
    private void scrollSideways(TreeItem<TreeNode> item) {
        if (!(lookup(".virtual-flow") instanceof VirtualFlow<?> flow)) {
            return;
        }
        ScrollBar hbar = null;
        for (Node n : flow.lookupAll(".scroll-bar")) {
            if (n instanceof ScrollBar b && b.getOrientation() == Orientation.HORIZONTAL && b.getParent() == flow) {
                hbar = b;
            }
        }
        if (hbar == null || !hbar.isVisible()) {
            return;
        }
        for (Node n : flow.lookupAll(".tree-cell")) {
            if (n instanceof TreeCell<?> cell && cell.getTreeItem() == item && cell.isVisible() && cell.getGraphic() != null) {
                Node g = cell.getGraphic();
                Bounds b = flow.sceneToLocal(g.localToScene(g.getBoundsInLocal()));
                double view = flow.getWidth() - verticalBarWidth(flow);
                double slack = 24; // keeps the disclosure arrow in view
                if (b.getMinX() < slack || b.getMinX() + Math.min(b.getWidth(), 120) > view) { // icon + name start
                    double x = hbar.getValue() + b.getMinX() - slack;
                    hbar.setValue(Math.max(hbar.getMin(), Math.min(hbar.getMax(), x)));
                }
                return;
            }
        }
    }

    private static double verticalBarWidth(VirtualFlow<?> flow) {
        for (Node n : flow.lookupAll(".scroll-bar")) {
            if (n instanceof ScrollBar b && b.getOrientation() == Orientation.VERTICAL && b.getParent() == flow
                    && b.isVisible()) {
                return b.getWidth();
            }
        }
        return 0;
    }

    /**
     * Makes {@code item} the current folder and collapses the branches {@link #reveal} opened for
     * earlier folders that aren't on its path (ones the user expanded stay open).
     */
    private void markCurrent(TreeItem<TreeNode> item) {
        current.set(item);
        if (!shown()) {
            return; // folds up at the next change while shown
        }
        programmatic++;
        autoExpanding++;
        try {
            for (Iterator<TreeItem<TreeNode>> it = autoExpanded.iterator(); it.hasNext(); ) {
                TreeItem<TreeNode> t = it.next();
                if (!isOnPath(t, item)) {
                    it.remove();
                    t.setExpanded(false);
                }
            }
        } finally {
            autoExpanding--;
            programmatic--;
        }
    }

    /**
     * Whether the tree is on screen. Scrolling and folding re-measure cells; in a hidden or detached
     * tree that is wasted work, and styling cells outside a shown scene logs CSS lookup warnings.
     */
    private boolean shown() {
        return showing.getValue();
    }

    /** True when {@code t} is {@code item} or one of its parents. */
    private static boolean isOnPath(TreeItem<TreeNode> t, TreeItem<TreeNode> item) {
        for (TreeItem<TreeNode> i = item; i != null; i = i.getParent()) {
            if (i == t) {
                return true;
            }
        }
        return false;
    }

    /** The item marked as the active tab's folder (package-private for tests). */
    TreeItem<TreeNode> currentItem() {
        return current.get();
    }

    /** Reloads the children of every loaded tree item showing {@code dir}. */
    void refreshDir(Path dir) {
        forEachLoaded(getRoot(), item -> {
            if (dir.equals(item.path())) {
                item.reload();
            }
        });
    }

    /** The folders watched for changes (test hook). */
    Set<Path> watchedPaths() {
        Set<Path> out = new HashSet<>();
        forEachLoaded(getRoot(), item -> {
            if (item.watched) {
                out.add(item.path());
            }
        });
        return out;
    }

    /** Starts or stops the folders' watches (after an eject started or ended). */
    void updateWatches() {
        forEachLoaded(getRoot(), FolderItem::updateWatch);
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

    private static void closeQuietly(AutoCloseable c) {
        try {
            c.close();
        } catch (Exception ignored) {
            // nothing to do
        }
    }

    /** Stops the watches of {@code item} and everything below it (it leaves the tree). */
    private static void stopWatching(TreeItem<TreeNode> item) {
        if (item instanceof FolderItem f) {
            f.unwatch();
        }
        for (TreeItem<TreeNode> c : item.getChildren()) {
            stopWatching(c);
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
        /** A reload asked for while loading: the running one may have read the folder too early. */
        private boolean reloadPending;
        /** Watches the folder while it is expanded, so other programs' changes show up. */
        private AutoCloseable watch;
        private boolean watched;
        private final List<Runnable> waiting = new ArrayList<>();

        FolderItem(TreeNode node) {
            super(node);
            getChildren().add(new TreeItem<>(TreeNode.loading()));
            expandedProperty().addListener((o, was, expanded) -> {
                if (autoExpanding == 0) {
                    autoExpanded.remove(this); // the user's choice now: leave it as it is
                }
                if (expanded) {
                    load(null);
                }
                updateWatch();
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
            if (loading) {
                reloadPending = loaded;
            } else if (loaded) {
                fetch();
            }
        }

        /** Watches the folder while it is loaded and expanded, unless its drive is being ejected. */
        void updateWatch() {
            boolean want = loaded && isExpanded() && !main.onEjectingDrive(path());
            if (want == watched) {
                return;
            }
            if (!want) {
                unwatch();
                return;
            }
            watched = true;
            Path dir = path();
            Bg.run(() -> DirWatcher.watch(dir, this::reload), handle -> {
                if (watched && watch == null) {
                    watch = handle;
                    reload(); // what changed while it was being registered
                } else {
                    closeQuietly(handle); // collapsed or removed meanwhile
                }
            }, null);
        }

        void unwatch() {
            watched = false;
            if (watch != null) {
                closeQuietly(watch);
                watch = null;
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
            if (reloadPending) {
                reloadPending = false;
                fetch();
            }
            updateWatch();
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
            name.getStyleClass().add("tree-name");
            extra.getStyleClass().add("tree-extra");
            // not for VirtualFlow's detached measuring cell: restyling it out of the scene logs lookup warnings
            current.addListener(o -> {
                if (getScene() != null) {
                    updateMarks();
                }
            });
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

        /** Boxes the current folder and makes the folders on its path bold. */
        private void updateMarks() {
            TreeItem<TreeNode> item = isEmpty() ? null : getTreeItem();
            TreeItem<TreeNode> cur = current.get();
            boolean folder = item != null && item.getValue() != null && item.getValue().path() != null;
            pseudoClassStateChanged(CURRENT, folder && item == cur);
            pseudoClassStateChanged(ON_PATH, folder && cur != null && isOnPath(item, cur));
        }

        @Override
        protected void updateItem(TreeNode n, boolean empty) {
            super.updateItem(n, empty);
            getStyleClass().removeAll("drop-target", "context-target");
            setText(null);
            updateMarks();
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
