package com.fileem.ui;

import atlantafx.base.theme.Styles;
import com.fileem.Bookmarks;
import com.fileem.Session;
import com.fileem.fs.FileEntry;
import com.fileem.fs.Shell;
import com.fileem.fs.Trash;
import com.fileem.ops.FileClipboard;
import com.fileem.ops.FileOperation.Kind;
import com.fileem.util.FsUtil;
import com.fileem.util.OsUtil;
import javafx.application.Platform;
import javafx.beans.property.BooleanProperty;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.Node;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonBase;
import javafx.scene.control.Label;
import javafx.scene.control.MenuItem;
import javafx.scene.control.SeparatorMenuItem;
import javafx.scene.control.SplitPane;
import javafx.scene.control.TextInputControl;
import javafx.scene.control.ToggleButton;
import javafx.scene.control.ToolBar;
import javafx.scene.control.Tooltip;
import javafx.scene.input.Clipboard;
import javafx.scene.input.ClipboardContent;
import javafx.scene.input.DragEvent;
import javafx.scene.input.KeyCode;
import javafx.scene.input.KeyEvent;
import javafx.scene.input.TransferMode;
import javafx.scene.layout.BorderPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.Region;
import javafx.stage.Stage;
import org.kordamp.ikonli.javafx.FontIcon;

import java.io.IOException;
import java.nio.file.FileAlreadyExistsException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import java.util.stream.Collectors;

/** Top level layout: toolbar, one or two explorer sides, function key bar. Owns all commands. */
public final class MainWindow extends BorderPane {
    private final Stage stage;
    private final Session session;
    private final BooleanProperty showHidden = new SimpleBooleanProperty();
    private final BooleanProperty dual = new SimpleBooleanProperty();
    private final SplitPane sides = new SplitPane();
    private final ExplorerSide left;
    private final ExplorerSide right;
    private ExplorerSide active;

    private Button backButton;
    private Button forwardButton;
    private Button upButton;
    private Button bookmarkButton;
    private final Bookmarks bookmarks;

    public MainWindow(Stage stage, Session session) {
        this.stage = stage;
        this.session = session;
        this.bookmarks = new Bookmarks(session);
        getStyleClass().add("main-window");
        showHidden.set(session.getBool("showHidden", false));
        showHidden.addListener((o, a, v) -> session.put("showHidden", v));

        left = new ExplorerSide(this, "left");
        right = new ExplorerSide(this, "right");
        active = left;
        left.setActive(true);

        setTop(buildToolbar());
        setCenter(sides);
        setBottom(buildFunctionBar());

        left.restore(session);
        right.restore(session);
        dual.set(session.getBool("dual", false));
        dual.addListener((o, a, v) -> layoutSides());
        sides.getItems().setAll(left);
        layoutSides();

        sceneProperty().addListener((o, old, scene) -> {
            if (scene != null) {
                installScene(scene);
            }
        });
    }

    Session session() {
        return session;
    }

    BooleanProperty showHiddenProperty() {
        return showHidden;
    }

    ExplorerSide left() {
        return left;
    }

    ExplorerSide right() {
        return right;
    }

    Bookmarks bookmarks() {
        return bookmarks;
    }

    boolean isDual() {
        return dual.get();
    }

    ExplorerSide activeSide() {
        return active;
    }

    private ExplorerSide side() {
        return active;
    }

    private ExplorerSide otherSide(ExplorerSide s) {
        return s == left ? right : left;
    }

    private FileTab tab() {
        return active.activeTab();
    }

    // ---- layout ----

    private void layoutSides() {
        active.setActive(dual.get());
        if (dual.get()) {
            if (!sides.getItems().contains(right)) {
                sides.getItems().setAll(left, right);
                sides.setDividerPositions(session.getDouble("split.sides", 0.5));
            }
        } else if (sides.getItems().contains(right)) {
            session.put("split.sides", sides.getDividerPositions()[0]);
            sides.getItems().setAll(left);
            if (active == right) {
                setActiveSide(left);
                left.activeTab().focusTable();
            }
        }
    }

    void setActiveSide(ExplorerSide s) {
        if (s == active) {
            return;
        }
        active.setActive(false);
        active = s;
        active.setActive(dual.get());
        FileTab t = s.activeTab();
        if (t != null) {
            tabChanged(t);
        }
    }

    /** Updates toolbar and title when the visible tab or its folder changed. */
    void tabChanged(FileTab t) {
        if (t.side() != active || t != active.activeTab()) {
            return;
        }
        backButton.setDisable(!t.canGoBack());
        forwardButton.setDisable(!t.canGoForward());
        upButton.setDisable(!t.canGoUp());
        stage.setTitle(t.currentDir() == null ? "fileEm" : FsUtil.displayName(t.currentDir()) + " — fileEm");
        updateBookmarkButton();
    }

    private void updateBookmarkButton() {
        FileTab t = tab();
        boolean marked = t != null && t.currentDir() != null && bookmarks.contains(t.currentDir());
        ((FontIcon) bookmarkButton.getGraphic()).setIconLiteral(marked ? "mdi2b-bookmark" : "mdi2b-bookmark-outline");
        bookmarkButton.getTooltip().setText(marked ? "Remove bookmark (Ctrl+D)" : "Bookmark this folder (Ctrl+D)");
    }

    private Node buildToolbar() {
        backButton = iconButton("mdi2a-arrow-left", "Back (Alt+Left)", () -> tab().back());
        forwardButton = iconButton("mdi2a-arrow-right", "Forward (Alt+Right)", () -> tab().forward());
        upButton = iconButton("mdi2a-arrow-up", "Up (Backspace)", () -> tab().up());
        Button refresh = iconButton("mdi2r-refresh", "Refresh (Ctrl+R)", this::refreshActive);

        Button newFolder = iconButton("mdi2f-folder-plus", "New folder (F7)", this::newFolder);
        Button cut = iconButton("mdi2c-content-cut", "Cut (Ctrl+X)", () -> clipboardPut(true));
        Button copy = iconButton("mdi2c-content-copy", "Copy (Ctrl+C)", () -> clipboardPut(false));
        Button paste = iconButton("mdi2c-content-paste", "Paste (Ctrl+V)", this::paste);
        Button rename = iconButton("mdi2r-rename-box", "Rename (F2)", this::rename);
        Button delete = iconButton("mdi2d-delete", "Delete (Del)", () -> delete(false));
        Button props = iconButton("mdi2i-information-outline", "Properties (Alt+Enter)", this::properties);

        Button newTab = iconButton("mdi2t-tab-plus", "New tab (Ctrl+T)", () -> side().newTab(currentDirOr(), true));
        bookmarkButton = iconButton("mdi2b-bookmark-outline", "Bookmark this folder (Ctrl+D)",
                () -> toggleBookmark(tab().currentDir()));
        bookmarks.items().addListener((javafx.collections.ListChangeListener<Bookmarks.Bookmark>) c -> updateBookmarkButton());

        ToggleButton hidden = toggle("mdi2e-eye", "Show hidden files (Ctrl+H)");
        hidden.selectedProperty().bindBidirectional(showHidden);
        ToggleButton dualToggle = toggle("mdi2v-view-split-vertical", "Dual pane (F9)");
        dualToggle.selectedProperty().bindBidirectional(dual);
        Button theme = iconButton("mdi2t-theme-light-dark", "Light / dark theme", Theme::toggle);

        Region spacer = new Region();
        HBox.setHgrow(spacer, Priority.ALWAYS);

        ToolBar bar = new ToolBar(backButton, forwardButton, upButton, refresh, sep(),
                newFolder, cut, copy, paste, rename, delete, props, sep(), newTab, bookmarkButton,
                spacer, hidden, dualToggle, theme);
        bar.getStyleClass().add("main-toolbar");
        return bar;
    }

    private static Node sep() {
        return new javafx.scene.control.Separator(javafx.geometry.Orientation.VERTICAL);
    }

    private static Button iconButton(String icon, String tooltip, Runnable action) {
        Button b = new Button(null, Icons.icon(icon, 18));
        b.getStyleClass().addAll(Styles.BUTTON_ICON, Styles.FLAT);
        b.setTooltip(new Tooltip(tooltip));
        b.setOnAction(e -> action.run());
        b.setFocusTraversable(false);
        return b;
    }

    private static ToggleButton toggle(String icon, String tooltip) {
        ToggleButton b = new ToggleButton(null, Icons.icon(icon, 18));
        b.getStyleClass().addAll(Styles.BUTTON_ICON, Styles.FLAT);
        b.setTooltip(new Tooltip(tooltip));
        b.setFocusTraversable(false);
        return b;
    }

    private Node buildFunctionBar() {
        HBox bar = new HBox(4,
                fkey("F2", "Rename", this::rename),
                fkey("F5", "Copy", () -> transferToOther(false)),
                fkey("F6", "Move", () -> transferToOther(true)),
                fkey("F7", "New folder", this::newFolder),
                fkey("F8", "Delete", () -> delete(false)),
                fkey("F9", "Dual pane", () -> dual.set(!dual.get())),
                fkey("Tab", "Switch side", this::switchSide));
        bar.getStyleClass().add("function-bar");
        bar.setPadding(new Insets(3, 6, 3, 6));
        bar.setAlignment(Pos.CENTER_LEFT);
        return bar;
    }

    private static ButtonBase fkey(String key, String label, Runnable action) {
        Label k = new Label(key);
        k.getStyleClass().add("fkey");
        Button b = new Button(label, k);
        b.getStyleClass().addAll(Styles.FLAT, Styles.SMALL);
        b.setFocusTraversable(false);
        b.setMaxWidth(Double.MAX_VALUE);
        HBox.setHgrow(b, Priority.ALWAYS);
        b.setOnAction(e -> action.run());
        return b;
    }

    // ---- keyboard ----

    private void installScene(Scene scene) {
        scene.addEventFilter(KeyEvent.KEY_PRESSED, this::onKey);
        scene.focusOwnerProperty().addListener((o, old, node) -> {
            for (Node n = node; n != null; n = n.getParent()) {
                if (n instanceof ExplorerSide s) {
                    setActiveSide(s);
                    return;
                }
            }
        });
        Platform.runLater(() -> {
            FileTab t = tab();
            if (t != null) {
                t.focusTable();
            }
        });
    }

    private void onKey(KeyEvent e) {
        Node focus = getScene().getFocusOwner();
        boolean typing = focus instanceof TextInputControl;
        boolean inList = !typing && isInside(focus, FileTable.class);
        boolean inTree = !typing && isInside(focus, FolderTree.class);
        KeyCode c = e.getCode();
        boolean shortcut = e.isShortcutDown();
        boolean alt = e.isAltDown();
        boolean shift = e.isShiftDown();
        boolean mac = OsUtil.isMac();
        Runnable action = null;

        if (shortcut && !alt && c == KeyCode.T) {
            action = () -> side().newTab(currentDirOr(), true);
        } else if (shortcut && !alt && c == KeyCode.W) {
            action = () -> side().closeActiveTab();
        } else if (shortcut && !alt && c == KeyCode.L) {
            action = () -> tab().editAddress();
        } else if (shortcut && !alt && c == KeyCode.F) {
            action = () -> tab().focusFilter();
        } else if ((e.isControlDown() && !alt && c == KeyCode.H) || (shortcut && shift && c == KeyCode.PERIOD)) {
            action = () -> showHidden.set(!showHidden.get());
        } else if (shortcut && shift && !alt && c == KeyCode.B) {
            action = this::manageBookmarks;
        } else if (shortcut && !alt && !shift && c == KeyCode.D) {
            action = () -> toggleBookmark(tab().currentDir());
        } else if (shortcut && !alt && c == KeyCode.R) {
            action = this::refreshActive;
        } else if (c == KeyCode.F9) {
            action = () -> dual.set(!dual.get());
        } else if (!typing) {
            if (c == KeyCode.TAB && !shortcut && !alt && !shift && dual.get() && (inList || inTree)) {
                action = this::switchSide;
            } else if (alt && c == KeyCode.ENTER) {
                action = this::properties;
            } else if (inList && (c == KeyCode.ENTER || mac && shortcut && c == KeyCode.DOWN)) {
                action = this::openSelection;
            } else if (alt && c == KeyCode.LEFT || shortcut && c == KeyCode.OPEN_BRACKET) {
                action = () -> tab().back();
            } else if (alt && c == KeyCode.RIGHT || shortcut && c == KeyCode.CLOSE_BRACKET) {
                action = () -> tab().forward();
            } else if (alt && c == KeyCode.UP || mac && shortcut && c == KeyCode.UP
                    || inList && c == KeyCode.BACK_SPACE && !shortcut) {
                action = () -> tab().up();
            } else if (inList && (c == KeyCode.DELETE || c == KeyCode.F8 || mac && shortcut && c == KeyCode.BACK_SPACE)) {
                action = () -> delete(shift);
            } else if (c == KeyCode.F2 && inList) {
                action = this::rename;
            } else if (c == KeyCode.F5) {
                action = () -> transferToOther(false);
            } else if (c == KeyCode.F6) {
                action = () -> transferToOther(true);
            } else if (c == KeyCode.F7 || shortcut && shift && c == KeyCode.N) {
                action = this::newFolder;
            } else if (inList && shortcut && !alt && c == KeyCode.C) {
                action = () -> clipboardPut(false);
            } else if (inList && shortcut && !alt && c == KeyCode.X) {
                action = () -> clipboardPut(true);
            } else if ((inList || inTree) && shortcut && !alt && c == KeyCode.V) {
                action = this::paste;
            } else if (inList && shortcut && !alt && c == KeyCode.A) {
                action = () -> tab().selectAll();
            }
        }
        if (action != null) {
            e.consume();
            action.run();
        }
    }

    private static boolean isInside(Node n, Class<?> type) {
        for (; n != null; n = n.getParent()) {
            if (type.isInstance(n)) {
                return true;
            }
        }
        return false;
    }

    private void switchSide() {
        if (!dual.get()) {
            return;
        }
        ExplorerSide target = otherSide(active);
        setActiveSide(target);
        target.activeTab().focusTable();
    }

    // ---- commands ----

    private Path currentDirOr() {
        FileTab t = tab();
        return t != null && t.currentDir() != null ? t.currentDir() : OsUtil.home();
    }

    private void refreshActive() {
        FileTab t = tab();
        t.refresh();
        side().tree().refreshDir(t.currentDir());
    }

    void openSelection() {
        FileTab t = tab();
        List<FileEntry> sel = t.selectedEntries();
        if (sel.isEmpty()) {
            return;
        }
        if (sel.size() == 1 && sel.get(0).isDirectory()) {
            t.navigateTo(sel.get(0).path());
            return;
        }
        for (FileEntry e : sel) {
            if (!e.isDirectory()) {
                Shell.open(e.path(), msg -> Dialogs.error(stage, "Open", "Cannot open " + e.name() + ":\n" + msg));
            }
        }
    }

    void openInOtherSide(Path dir) {
        if (!dual.get()) {
            dual.set(true);
        }
        ExplorerSide other = otherSide(active);
        FileTab t = other.activeTab();
        t.navigateTo(dir);
        setActiveSide(other);
        t.focusTable();
    }

    private void clipboardPut(boolean cut) {
        List<Path> sel = tab().selectedPaths();
        if (!sel.isEmpty()) {
            FileClipboard.put(sel, cut);
        }
    }

    private void paste() {
        pasteInto(tab().currentDir());
    }

    private void pasteInto(Path target) {
        List<Path> files = FileClipboard.files();
        if (files.isEmpty() || target == null) {
            return;
        }
        boolean move = FileClipboard.isCut();
        runOperation(move ? Kind.MOVE : Kind.COPY, files, target);
        if (move) {
            FileClipboard.clearAfterMove();
        }
    }

    private void copyPaths(List<Path> paths) {
        ClipboardContent c = new ClipboardContent();
        c.putString(paths.stream().map(Path::toString).collect(Collectors.joining(System.lineSeparator())));
        Clipboard.getSystemClipboard().setContent(c);
    }

    private void delete(boolean permanent) {
        List<Path> sel = tab().selectedPaths();
        if (sel.isEmpty()) {
            return;
        }
        boolean trash = !permanent && Trash.isSupported();
        String what = sel.size() == 1 ? "\"" + FsUtil.displayName(sel.get(0)) + "\"" : sel.size() + " items";
        String question = trash ? "Move " + what + " to the Trash?"
                : "Permanently delete " + what + "?\n\nThis cannot be undone.";
        if (Dialogs.confirm(stage, trash ? "Move to Trash" : "Delete permanently", question)) {
            runOperation(trash ? Kind.TRASH : Kind.DELETE, sel, null);
        }
    }

    private void rename() {
        FileTab t = tab();
        List<Path> sel = t.selectedPaths();
        if (sel.size() != 1) {
            return;
        }
        Path src = sel.get(0);
        String oldName = FsUtil.displayName(src);
        Dialogs.askName(stage, "Rename", "New name:", oldName, name -> {
            Path target = src.resolveSibling(name);
            try {
                if (!name.equals(oldName) && Files.exists(target, LinkOption.NOFOLLOW_LINKS)
                        && !Files.isSameFile(src, target)) {
                    return "An item named \"" + name + "\" already exists.";
                }
            } catch (IOException ignored) {
                // let the move report it
            }
            return null;
        }).filter(name -> !name.equals(oldName)).ifPresent(name -> {
            Path target = src.resolveSibling(name);
            try {
                if (Files.exists(target, LinkOption.NOFOLLOW_LINKS) && Files.isSameFile(src, target)) {
                    // case-only rename on a case-insensitive file system
                    Path tmp = FsUtil.uniqueName(src.resolveSibling(name + ".renaming"), false);
                    Files.move(src, tmp);
                    Files.move(tmp, target);
                } else {
                    Files.move(src, target);
                }
                t.refresh(target);
                treesRefresh(t.currentDir());
            } catch (FileAlreadyExistsException e) {
                Dialogs.error(stage, "Rename", "An item named \"" + name + "\" already exists.");
            } catch (IOException e) {
                Dialogs.error(stage, "Rename", "Cannot rename \"" + oldName + "\":\n" + e.getMessage());
            }
        });
    }

    private void newFolder() {
        FileTab t = tab();
        Path dir = t.currentDir();
        if (dir == null) {
            return;
        }
        String initial = FsUtil.displayName(FsUtil.uniqueName(dir.resolve("New folder"), false));
        Dialogs.askName(stage, "New folder", "Folder name:", initial,
                name -> Files.exists(dir.resolve(name), LinkOption.NOFOLLOW_LINKS)
                        ? "An item named \"" + name + "\" already exists." : null)
                .ifPresent(name -> {
                    Path created = dir.resolve(name);
                    try {
                        Files.createDirectory(created);
                        t.refresh(created);
                        treesRefresh(dir);
                    } catch (IOException e) {
                        Dialogs.error(stage, "New folder", "Cannot create folder:\n" + e.getMessage());
                    }
                });
    }

    private void properties() {
        FileTab t = tab();
        List<Path> sel = t.selectedPaths();
        if (sel.isEmpty() && t.currentDir() != null) {
            sel = List.of(t.currentDir());
        }
        if (!sel.isEmpty()) {
            PropertiesDialog.show(stage, sel);
        }
    }

    /** F5 / F6: copy or move the selection to the other side's folder (or a typed folder). */
    private void transferToOther(boolean move) {
        FileTab t = tab();
        List<Path> sel = t.selectedPaths();
        if (sel.isEmpty()) {
            return;
        }
        Path initial = dual.get() ? otherSide(active).activeTab().currentDir() : t.currentDir();
        String what = sel.size() == 1 ? "\"" + FsUtil.displayName(sel.get(0)) + "\"" : sel.size() + " items";
        String verb = move ? "Move" : "Copy";
        Dialogs.askTarget(stage, verb, verb + " " + what + " to:", initial).ifPresent(target -> {
            if (!Files.isDirectory(target)) {
                Dialogs.error(stage, verb, "The target folder does not exist:\n" + target);
                return;
            }
            runOperation(move ? Kind.MOVE : Kind.COPY, sel, target);
        });
    }

    void drop(DragEvent e, Path target) {
        List<Path> files = Dnd.files(e.getDragboard());
        TransferMode mode = e.getTransferMode();
        e.setDropCompleted(true);
        e.consume();
        Platform.runLater(() -> runOperation(mode == TransferMode.MOVE ? Kind.MOVE : Kind.COPY, files, target));
    }

    void runOperation(Kind kind, List<Path> sources, Path target) {
        if (sources.isEmpty()) {
            return;
        }
        OperationUi.run(stage, kind, sources, target, this::refreshDirs);
    }

    /** Refreshes every tab and tree item that shows one of {@code dirs}. */
    void refreshDirs(Set<Path> dirs) {
        for (ExplorerSide s : List.of(left, right)) {
            for (FileTab t : s.tabs()) {
                if (t.currentDir() != null && dirs.contains(t.currentDir())) {
                    t.requestRefresh();
                }
            }
            dirs.forEach(s.tree()::refreshDir);
        }
    }

    void treesRefresh(Path dir) {
        left.tree().refreshDir(dir);
        right.tree().refreshDir(dir);
    }

    // ---- context menus ----

    List<MenuItem> fileMenuItems(FileTab t) {
        List<Path> sel = t.selectedPaths();
        List<FileEntry> entries = t.selectedEntries();
        List<MenuItem> items = new ArrayList<>();
        boolean canPaste = FileClipboard.hasFiles();
        if (!sel.isEmpty()) {
            items.add(item("Open", "mdi2o-open-in-app", this::openSelection));
            if (entries.size() == 1 && entries.get(0).isDirectory()) {
                Path dir = entries.get(0).path();
                items.add(item("Open in new tab", "mdi2t-tab-plus", () -> t.side().newTab(dir, true)));
                items.add(item("Open in other side", "mdi2v-view-split-vertical", () -> openInOtherSide(dir)));
                items.add(bookmarkToggleItem(dir));
            }
            items.add(new SeparatorMenuItem());
            items.add(item("Cut", "mdi2c-content-cut", () -> clipboardPut(true)));
            items.add(item("Copy", "mdi2c-content-copy", () -> clipboardPut(false)));
            if (canPaste) {
                items.add(item("Paste", "mdi2c-content-paste", this::paste));
            }
            items.add(new SeparatorMenuItem());
            items.add(item("Copy to…  (F5)", "mdi2f-file-move", () -> transferToOther(false)));
            items.add(item("Move to…  (F6)", "mdi2f-file-move", () -> transferToOther(true)));
            items.add(new SeparatorMenuItem());
            MenuItem rename = item("Rename  (F2)", "mdi2r-rename-box", this::rename);
            rename.setDisable(sel.size() != 1);
            items.add(rename);
            if (Trash.isSupported()) {
                items.add(item("Move to Trash  (Del)", "mdi2d-delete", () -> delete(false)));
            }
            items.add(item("Delete permanently  (Shift+Del)", "mdi2d-delete-forever", () -> delete(true)));
            items.add(new SeparatorMenuItem());
            items.add(item("Copy path", "mdi2c-clipboard-text-outline", () -> copyPaths(sel)));
            items.add(item("Properties", "mdi2i-information-outline", this::properties));
        } else {
            MenuItem paste = item("Paste", "mdi2c-content-paste", this::paste);
            paste.setDisable(!canPaste);
            items.add(paste);
            items.add(new SeparatorMenuItem());
            items.add(item("New folder  (F7)", "mdi2f-folder-plus", this::newFolder));
            items.add(item("Refresh", "mdi2r-refresh", this::refreshActive));
            items.add(new SeparatorMenuItem());
            if (t.currentDir() != null) {
                items.add(bookmarkToggleItem(t.currentDir()));
                items.add(item("Copy path", "mdi2c-clipboard-text-outline", () -> copyPaths(List.of(t.currentDir()))));
            }
            items.add(item("Properties", "mdi2i-information-outline", this::properties));
        }
        return items;
    }

    /** Context menu for a folder, bookmark or volume in the tree. */
    List<MenuItem> treeMenuItems(TreeNode node) {
        Path dir = node.path();
        List<MenuItem> items = new ArrayList<>();
        items.add(item("Open in new tab", "mdi2t-tab-plus", () -> side().newTab(dir, true)));
        items.add(item("Open in other side", "mdi2v-view-split-vertical", () -> openInOtherSide(dir)));
        items.add(new SeparatorMenuItem());
        if (node.kind() == TreeNode.Kind.BOOKMARK) {
            int index = bookmarkIndex(dir);
            items.add(item("Rename bookmark…", "mdi2p-pencil", () -> renameBookmark(dir)));
            items.add(item("Remove bookmark", "mdi2b-bookmark-remove", () -> bookmarks.remove(dir)));
            MenuItem up = item("Move up", "mdi2a-arrow-up", () -> bookmarks.move(dir, -1));
            up.setDisable(index <= 0);
            MenuItem down = item("Move down", "mdi2a-arrow-down", () -> bookmarks.move(dir, 1));
            down.setDisable(index < 0 || index >= bookmarks.items().size() - 1);
            items.add(up);
            items.add(down);
            items.add(item("Manage bookmarks…  (Ctrl+Shift+B)", "mdi2b-bookmark-multiple", this::manageBookmarks));
        } else {
            items.add(bookmarkToggleItem(dir));
        }
        items.add(new SeparatorMenuItem());
        MenuItem paste = item("Paste into folder", "mdi2c-content-paste", () -> pasteInto(dir));
        paste.setDisable(!FileClipboard.hasFiles());
        items.add(paste);
        items.add(item("Copy path", "mdi2c-clipboard-text-outline", () -> copyPaths(List.of(dir))));
        items.add(item("Properties", "mdi2i-information-outline", () -> PropertiesDialog.show(stage, List.of(dir))));
        return items;
    }

    List<MenuItem> bookmarksHeaderMenuItems() {
        return List.of(item("Manage bookmarks…  (Ctrl+Shift+B)", "mdi2b-bookmark-multiple", this::manageBookmarks));
    }

    private MenuItem bookmarkToggleItem(Path dir) {
        return bookmarks.contains(dir)
                ? item("Remove from bookmarks", "mdi2b-bookmark-remove", () -> bookmarks.remove(dir))
                : item("Add to bookmarks  (Ctrl+D)", "mdi2b-bookmark-plus", () -> bookmarks.add(dir));
    }

    // ---- bookmarks ----

    private int bookmarkIndex(Path dir) {
        List<Bookmarks.Bookmark> all = bookmarks.items();
        for (int i = 0; i < all.size(); i++) {
            if (all.get(i).path().equals(dir)) {
                return i;
            }
        }
        return -1;
    }

    /** Ctrl+D: bookmarks the folder, or removes its bookmark when it already has one. */
    void toggleBookmark(Path dir) {
        if (dir == null) {
            return;
        }
        if (bookmarks.contains(dir)) {
            bookmarks.remove(dir);
        } else {
            bookmarks.add(dir);
        }
    }

    void renameBookmark(Path dir) {
        bookmarks.find(dir).ifPresent(b -> Dialogs.askText(stage, "Rename bookmark", "Name:", b.name())
                .ifPresent(name -> bookmarks.rename(dir, name)));
    }

    void openBookmark(Path dir) {
        FileTab t = tab();
        t.navigateTo(dir);
        t.focusTable();
    }

    void manageBookmarks() {
        BookmarksDialog.show(stage, this);
    }

    private static MenuItem item(String text, String icon, Runnable action) {
        FontIcon graphic = Icons.icon(icon);
        MenuItem mi = new MenuItem(text, graphic);
        mi.setOnAction(e -> action.run());
        return mi;
    }

    // ---- session ----

    public void saveSession() {
        session.put("dual", dual.get());
        if (dual.get() && sides.getDividerPositions().length > 0) {
            session.put("split.sides", sides.getDividerPositions()[0]);
        }
        left.save(session);
        right.save(session);
    }
}
