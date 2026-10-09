package com.fileem.ui;

import com.fileem.Session;
import com.fileem.fs.FileEntry;
import com.fileem.util.OsUtil;
import javafx.scene.Node;
import javafx.scene.Scene;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableRow;
import javafx.scene.control.TreeItem;
import javafx.scene.input.KeyCode;
import javafx.stage.Stage;
import javafx.stage.Window;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.junit.jupiter.api.io.TempDir;
import org.testfx.api.FxRobot;
import org.testfx.api.FxToolkit;
import org.testfx.framework.junit5.ApplicationExtension;
import org.testfx.framework.junit5.Start;
import org.testfx.util.WaitForAsyncUtils;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.Callable;
import java.util.concurrent.TimeUnit;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Drives the real main window (headless on Monocle by default, see pom.xml).
 * Every test works in its own temp folders and its own session file.
 */
@ExtendWith(ApplicationExtension.class)
class MainWindowUiTest {
    private static final int TIMEOUT_S = 15;

    @TempDir
    Path tmp;

    private Session session;
    private MainWindow window;
    private Stage stage;

    @Start
    void start(Stage stage) {
        this.stage = stage;
        Theme.apply(false);
        session = Session.load(tmp.resolve("session.properties"));
        window = new MainWindow(stage, session);
        Scene scene = new Scene(window, 1200, 800);
        scene.getStylesheets().add(Theme.APP_CSS);
        stage.setScene(scene);
        stage.show();
    }

    @AfterEach
    void closeWindows() throws Exception {
        FxToolkit.cleanupStages();
        // Detach this test's window: it stays alive (watchers, volume listeners) and would otherwise
        // keep restyling while reacting to its deleted temp folders, polluting later tests.
        WaitForAsyncUtils.asyncFx(() -> stage.setScene(null)).get();
    }

    // ---- helpers ----

    private static <T> T fx(Callable<T> read) {
        WaitForAsyncUtils.waitForFxEvents();
        try {
            return WaitForAsyncUtils.asyncFx(read).get(TIMEOUT_S, TimeUnit.SECONDS);
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    private static void waitUntil(String what, Callable<Boolean> condition) {
        try {
            WaitForAsyncUtils.waitFor(TIMEOUT_S, TimeUnit.SECONDS,
                    () -> WaitForAsyncUtils.asyncFx(condition).get(TIMEOUT_S, TimeUnit.SECONDS));
        } catch (Exception e) {
            throw new AssertionError("Timed out waiting until " + what, e);
        }
    }

    private FileTab leftTab() {
        return fx(() -> window.left().activeTab());
    }

    private FileTab rightTab() {
        return fx(() -> window.right().activeTab());
    }

    private void open(FxRobot robot, FileTab tab, Path dir) {
        robot.interact(() -> tab.navigateTo(dir));
        waitUntil("tab shows " + dir, () -> dir.equals(tab.currentDir()));
        robot.interact(tab::focusTable);
    }

    private static List<String> names(FileTab tab) {
        return fx(() -> tab.table().getItems().stream().map(FileEntry::name).toList());
    }

    private void select(FxRobot robot, FileTab tab, String... names) {
        robot.interact(() -> {
            var sm = tab.table().getSelectionModel();
            sm.clearSelection();
            List<FileEntry> items = tab.table().getItems();
            for (String n : names) {
                for (int i = 0; i < items.size(); i++) {
                    if (items.get(i).name().equals(n)) {
                        sm.select(i);
                        tab.table().getFocusModel().focus(i);
                    }
                }
            }
            tab.table().requestFocus();
        });
    }

    private Path folderWith(String name, String... files) throws Exception {
        Path dir = Files.createDirectories(tmp.resolve(name));
        for (String f : files) {
            if (f.endsWith("/")) {
                Files.createDirectories(dir.resolve(f));
            } else {
                Files.writeString(dir.resolve(f), f);
            }
        }
        return dir;
    }

    /**
     * Types into the open dialog's text field, replacing its current selection, then presses OK.
     * (Headless Monocle sends robot key events to the main window, not to a dialog.)
     */
    private void answerDialog(FxRobot robot, String replacement) {
        WaitForAsyncUtils.waitForFxEvents();
        javafx.scene.control.TextField field = robot.lookup(".dialog-pane .text-field").query();
        robot.interact(() -> field.replaceSelection(replacement));
        robot.clickOn("OK");
        WaitForAsyncUtils.waitForFxEvents();
    }

    private static List<Window> otherWindows(Stage main) {
        return fx(() -> Window.getWindows().stream().filter(w -> w != main && w.isShowing()).toList());
    }

    // ---- tests ----

    @Test
    void startsWithTreeHeadersAndAHomeTab(FxRobot robot) {
        FileTab tab = leftTab();
        waitUntil("home is listed", () -> OsUtil.home().equals(tab.currentDir()));
        List<String> headers = fx(() -> window.left().tree().getRoot().getChildren().stream()
                .map(i -> i.getValue().label()).toList());
        assertEquals(List.of("Quick access", OsUtil.isWindows() ? "This PC" : "Computer"), headers);
        assertFalse(fx(window::isDual));
    }

    @Test
    void listsFoldersFirstInNaturalOrder(FxRobot robot) throws Exception {
        Path dir = folderWith("list", "b10.txt", "b2.txt", "A.txt", "zdir/", "adir/");
        open(robot, leftTab(), dir);
        assertEquals(List.of("adir", "zdir", "A.txt", "b2.txt", "b10.txt"), names(leftTab()));
    }

    @Test
    void sortingByAnyColumnKeepsFoldersOnTop(FxRobot robot) throws Exception {
        Path dir = folderWith("sort", "dir/");
        Files.writeString(dir.resolve("big"), "x".repeat(300));
        Files.writeString(dir.resolve("small"), "x");
        Files.writeString(dir.resolve("mid"), "x".repeat(20));
        FileTab tab = leftTab();
        open(robot, tab, dir);
        TableColumn<FileEntry, ?> size = fx(() -> tab.table().getColumns().stream()
                .filter(c -> "size".equals(c.getId())).findFirst().orElseThrow());

        robot.interact(() -> {
            size.setSortType(TableColumn.SortType.ASCENDING);
            tab.table().getSortOrder().setAll(List.of(size));
        });
        assertEquals(List.of("dir", "small", "mid", "big"), names(tab));

        robot.interact(() -> size.setSortType(TableColumn.SortType.DESCENDING));
        assertEquals(List.of("dir", "big", "mid", "small"), names(tab));
        assertEquals("size", session.get("sort.column", ""), "sort is remembered");
    }

    @Test
    void hiddenFilesToggleWithCtrlH(FxRobot robot) throws Exception {
        assumeTrue(!OsUtil.isWindows());
        Path dir = folderWith("hidden", ".dot", "visible");
        open(robot, leftTab(), dir);
        assertEquals(List.of("visible"), names(leftTab()));
        robot.push(KeyCode.CONTROL, KeyCode.H);
        waitUntil("hidden file shows", () -> leftTab().table().getItems().size() == 2);
        robot.push(KeyCode.CONTROL, KeyCode.H);
        waitUntil("hidden file hides again", () -> leftTab().table().getItems().size() == 1);
    }

    @Test
    void filterBoxNarrowsTheListAndEscapeClearsIt(FxRobot robot) throws Exception {
        Path dir = folderWith("filter", "apple.txt", "banana.txt", "berry.md");
        open(robot, leftTab(), dir);
        robot.push(KeyCode.SHORTCUT, KeyCode.F);
        robot.write("b*.txt");
        waitUntil("filter applied", () -> leftTab().table().getItems().size() == 1);
        assertEquals(List.of("banana.txt"), names(leftTab()));
        robot.type(KeyCode.ESCAPE);
        waitUntil("filter cleared", () -> leftTab().table().getItems().size() == 3);
    }

    @Test
    void enterOpensAFolderAndBackspaceReturnsWithItSelected(FxRobot robot) throws Exception {
        Path dir = folderWith("nav", "inner/", "file.txt");
        Files.writeString(dir.resolve("inner/child.txt"), "c");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "inner");
        robot.type(KeyCode.ENTER);
        waitUntil("opened inner", () -> dir.resolve("inner").equals(tab.currentDir()));
        assertEquals(List.of("child.txt"), names(tab));

        robot.type(KeyCode.BACK_SPACE);
        waitUntil("back up", () -> dir.equals(tab.currentDir()));
        assertEquals(List.of(dir.resolve("inner")), fx(tab::selectedPaths), "the folder we came from is selected");

        robot.push(KeyCode.ALT, KeyCode.LEFT);
        waitUntil("history back", () -> dir.resolve("inner").equals(tab.currentDir()));
        robot.push(KeyCode.ALT, KeyCode.RIGHT);
        waitUntil("history forward", () -> dir.equals(tab.currentDir()));
    }

    @Test
    void f7CreatesAFolderAndSelectsIt(FxRobot robot) throws Exception {
        Path dir = folderWith("mk");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        robot.type(KeyCode.F7);
        answerDialog(robot, "Fresh");
        waitUntil("folder created", () -> Files.isDirectory(dir.resolve("Fresh")));
        waitUntil("folder selected", () -> tab.selectedPaths().equals(List.of(dir.resolve("Fresh"))));
    }

    @Test
    void f2RenamesKeepingTheExtension(FxRobot robot) throws Exception {
        Path dir = folderWith("ren", "old.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "old.txt");
        robot.type(KeyCode.F2);
        answerDialog(robot, "new"); // only the base name is preselected, so ".txt" stays
        waitUntil("renamed", () -> Files.exists(dir.resolve("new.txt")) && !Files.exists(dir.resolve("old.txt")));
        waitUntil("list updated", () -> names(tab).equals(List.of("new.txt")));
    }

    @Test
    void renameRefusesAnExistingName(FxRobot robot) throws Exception {
        Path dir = folderWith("ren2", "a.txt", "b.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "a.txt");
        robot.type(KeyCode.F2);
        answerDialog(robot, "b");
        assertFalse(otherWindows(stage).isEmpty(), "dialog stays open with an error");
        robot.clickOn("Cancel");
        assertEquals("a.txt", Files.readString(dir.resolve("a.txt")));
        assertEquals("b.txt", Files.readString(dir.resolve("b.txt")));
    }

    @Test
    void shiftDeleteDeletesPermanentlyAfterConfirmation(FxRobot robot) throws Exception {
        Path dir = folderWith("del", "gone.txt", "stays.txt", "sub/");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "gone.txt", "sub");

        robot.push(KeyCode.SHIFT, KeyCode.DELETE);
        robot.type(KeyCode.ESCAPE); // cancel the confirmation
        assertTrue(Files.exists(dir.resolve("gone.txt")));

        select(robot, tab, "gone.txt", "sub");
        robot.push(KeyCode.SHIFT, KeyCode.DELETE);
        robot.type(KeyCode.ENTER);
        waitUntil("deleted", () -> !Files.exists(dir.resolve("gone.txt")) && !Files.exists(dir.resolve("sub")));
        waitUntil("list updated", () -> names(tab).equals(List.of("stays.txt")));
    }

    @Test
    void f9OpensTheSecondSideAndTabSwitches(FxRobot robot) {
        robot.type(KeyCode.F9);
        waitUntil("dual on", window::isDual);
        assertEquals(fx(window::left), fx(window::activeSide));
        robot.type(KeyCode.TAB);
        waitUntil("right side active", () -> window.activeSide() == window.right());
        robot.type(KeyCode.TAB);
        waitUntil("left side active", () -> window.activeSide() == window.left());
        robot.type(KeyCode.F9);
        waitUntil("dual off", () -> !window.isDual());
    }

    @Test
    void f5CopiesAndF6MovesToTheOtherSide(FxRobot robot) throws Exception {
        Path from = folderWith("from", "copy.txt", "move.txt");
        Path to = folderWith("to");
        robot.type(KeyCode.F9);
        waitUntil("dual on", window::isDual);
        open(robot, rightTab(), to);
        FileTab left = leftTab();
        open(robot, left, from);

        select(robot, left, "copy.txt");
        robot.type(KeyCode.F5);
        robot.type(KeyCode.ENTER); // target is prefilled with the other side's folder
        waitUntil("copied", () -> Files.exists(to.resolve("copy.txt")));
        assertTrue(Files.exists(from.resolve("copy.txt")));

        select(robot, left, "move.txt");
        robot.type(KeyCode.F6);
        robot.type(KeyCode.ENTER);
        waitUntil("moved", () -> Files.exists(to.resolve("move.txt")) && !Files.exists(from.resolve("move.txt")));
        waitUntil("both sides refreshed", () -> names(rightTab()).equals(List.of("copy.txt", "move.txt"))
                && names(leftTab()).equals(List.of("copy.txt")));
    }

    @Test
    void conflictDialogKeepBoth(FxRobot robot) throws Exception {
        Path from = folderWith("cfrom", "a.txt");
        Path to = folderWith("cto", "a.txt");
        Files.writeString(to.resolve("a.txt"), "existing");
        robot.type(KeyCode.F9);
        waitUntil("dual on", window::isDual);
        open(robot, rightTab(), to);
        FileTab left = leftTab();
        open(robot, left, from);
        select(robot, left, "a.txt");
        robot.type(KeyCode.F5);
        robot.type(KeyCode.ENTER);
        WaitForAsyncUtils.waitForFxEvents();
        robot.clickOn("Keep both");
        waitUntil("kept both", () -> Files.exists(to.resolve("a (2).txt")));
        assertEquals("existing", Files.readString(to.resolve("a.txt")));
    }

    @Test
    void copyAndPasteThroughTheClipboard(FxRobot robot) throws Exception {
        Path from = folderWith("clipfrom", "c.txt");
        Path to = folderWith("clipto");
        FileTab tab = leftTab();
        open(robot, tab, from);
        select(robot, tab, "c.txt");
        robot.push(KeyCode.SHORTCUT, KeyCode.C);
        open(robot, tab, to);
        robot.push(KeyCode.SHORTCUT, KeyCode.V);
        waitUntil("pasted", () -> Files.exists(to.resolve("c.txt")));
        assertTrue(Files.exists(from.resolve("c.txt")));
    }

    @Test
    void cutAndPasteMoves(FxRobot robot) throws Exception {
        Path from = folderWith("cutfrom", "m.txt");
        Path to = folderWith("cutto");
        FileTab tab = leftTab();
        open(robot, tab, from);
        select(robot, tab, "m.txt");
        robot.push(KeyCode.SHORTCUT, KeyCode.X);
        open(robot, tab, to);
        robot.push(KeyCode.SHORTCUT, KeyCode.V);
        waitUntil("moved", () -> Files.exists(to.resolve("m.txt")) && !Files.exists(from.resolve("m.txt")));
    }

    @Test
    void ctrlTOpensAndCtrlWClosesTabsButNeverTheLastOne(FxRobot robot) {
        assertEquals(1, (int) fx(() -> window.left().tabs().size()));
        robot.push(KeyCode.SHORTCUT, KeyCode.T);
        waitUntil("two tabs", () -> window.left().tabs().size() == 2);
        robot.push(KeyCode.SHORTCUT, KeyCode.W);
        waitUntil("one tab", () -> window.left().tabs().size() == 1);
        robot.push(KeyCode.SHORTCUT, KeyCode.W);
        assertEquals(1, (int) fx(() -> window.left().tabs().size()));
    }

    @Test
    void typedAddressNavigatesAndAFilePathSelectsTheFile(FxRobot robot) throws Exception {
        Path dir = folderWith("addr", "target.txt");
        FileTab tab = leftTab();
        robot.push(KeyCode.SHORTCUT, KeyCode.L);
        robot.push(KeyCode.SHORTCUT, KeyCode.A);
        robot.write(dir.resolve("target.txt").toString());
        robot.type(KeyCode.ENTER);
        waitUntil("navigated", () -> dir.equals(tab.currentDir()));
        waitUntil("file selected", () -> tab.selectedPaths().equals(List.of(dir.resolve("target.txt"))));
    }

    @Test
    void externalChangesAppearAutomatically(FxRobot robot) throws Exception {
        Path dir = folderWith("watch", "first.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        Files.writeString(dir.resolve("second.txt"), "x");
        Files.delete(dir.resolve("first.txt"));
        waitUntil("listing follows the disk", () -> names(tab).equals(List.of("second.txt")));
    }

    @Test
    void treeFollowsTheTabAndClicksInTheTreeNavigate(FxRobot robot) throws Exception {
        Path deep = Files.createDirectories(tmp.resolve("t1/t2"));
        FileTab tab = leftTab();
        FolderTree tree = fx(() -> window.left().tree());
        open(robot, tab, deep);
        waitUntil("tree reveals folder", () -> {
            TreeItem<TreeNode> sel = tree.getSelectionModel().getSelectedItem();
            return sel != null && deep.equals(sel.getValue().path());
        });

        robot.interact(() -> {
            TreeItem<TreeNode> quick = tree.getRoot().getChildren().get(0);
            tree.getSelectionModel().select(quick.getChildren().get(0)); // "Home"
        });
        waitUntil("tab follows the tree", () -> OsUtil.home().equals(tab.currentDir()));
    }

    @Test
    void altEnterShowsProperties(FxRobot robot) throws Exception {
        Path dir = folderWith("props", "info.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "info.txt");
        robot.push(KeyCode.ALT, KeyCode.ENTER);
        waitUntil("properties window", () -> Window.getWindows().stream()
                .anyMatch(w -> w instanceof Stage s && s.getTitle() != null && s.getTitle().startsWith("info.txt")));
    }

    @Test
    void dragAFileOntoAFolderRow(FxRobot robot) throws Exception {
        Path dir = folderWith("dnd", "drag.txt", "into/");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "drag.txt");
        Node source = rowNode(robot, "drag.txt");
        Node target = rowNode(robot, "into");
        robot.drag(source).dropTo(target);
        waitUntil("dropped into folder", () -> Files.exists(dir.resolve("into/drag.txt")));
    }

    private Node rowNode(FxRobot robot, String name) {
        return robot.lookup(".file-table .table-row-cell").queryAll().stream()
                .filter(n -> n instanceof TableRow<?> r && r.getItem() instanceof FileEntry e && e.name().equals(name))
                .findFirst().orElseThrow(() -> new AssertionError("no row " + name));
    }

    @Test
    void sessionRestoresTabsAndDualMode(FxRobot robot) throws Exception {
        Path a = folderWith("sa");
        Path b = folderWith("sb");
        open(robot, leftTab(), a);
        robot.push(KeyCode.SHORTCUT, KeyCode.T);
        waitUntil("second tab", () -> window.left().tabs().size() == 2);
        open(robot, leftTab(), b);
        robot.type(KeyCode.F9);
        waitUntil("dual on", window::isDual);
        robot.interact(window::saveSession);

        MainWindow restored = fx(() -> new MainWindow(new Stage(), session));
        waitUntil("tabs restored", () -> restored.left().tabs().size() == 2
                && a.equals(restored.left().tabs().get(0).currentDir())
                && b.equals(restored.left().tabs().get(1).currentDir()));
        assertTrue(fx(restored::isDual));
        assertEquals(b, fx(() -> restored.left().activeTab().currentDir()), "selected tab restored");
    }

    // ---- bookmarks ----

    private static List<String> treeBookmarks(FolderTree tree) {
        return fx(() -> tree.getRoot().getChildren().stream()
                .filter(i -> i.getValue().kind() == TreeNode.Kind.BOOKMARK_HEADER)
                .flatMap(h -> h.getChildren().stream())
                .map(i -> i.getValue().label() + "=" + i.getValue().path())
                .toList());
    }

    private static TreeNode bookmarkNode(FolderTree tree, Path path) {
        return fx(() -> tree.getRoot().getChildren().stream()
                .filter(i -> i.getValue().kind() == TreeNode.Kind.BOOKMARK_HEADER)
                .flatMap(h -> h.getChildren().stream())
                .map(TreeItem::getValue)
                .filter(n -> n.path().equals(path))
                .findFirst().orElseThrow());
    }

    private void fireMenuItem(List<javafx.scene.control.MenuItem> items, String textPrefix) {
        javafx.scene.control.MenuItem item = items.stream()
                .filter(i -> i.getText() != null && i.getText().startsWith(textPrefix))
                .findFirst().orElseThrow(() -> new AssertionError("no menu item " + textPrefix));
        assertFalse(item.isDisable(), textPrefix + " is enabled");
        WaitForAsyncUtils.asyncFx(item::fire); // may open a dialog, so don't wait for it
        WaitForAsyncUtils.waitForFxEvents();
    }

    @Test
    void ctrlDTogglesABookmarkShownInBothTrees(FxRobot robot) throws Exception {
        Path dir = folderWith("Favourite");
        open(robot, leftTab(), dir);
        assertTrue(treeBookmarks(fx(() -> window.left().tree())).isEmpty(), "no section without bookmarks");

        robot.push(KeyCode.SHORTCUT, KeyCode.D);
        waitUntil("bookmarked", () -> window.bookmarks().contains(dir));
        assertEquals(List.of("Favourite=" + dir), treeBookmarks(fx(() -> window.left().tree())));
        assertEquals(List.of("Favourite=" + dir), treeBookmarks(fx(() -> window.right().tree())));

        robot.push(KeyCode.SHORTCUT, KeyCode.D);
        waitUntil("bookmark removed", () -> !window.bookmarks().contains(dir));
        assertTrue(treeBookmarks(fx(() -> window.left().tree())).isEmpty());
    }

    @Test
    void selectingABookmarkNavigatesTheActiveTab(FxRobot robot) throws Exception {
        Path dir = folderWith("target", "inside.txt");
        robot.interact(() -> window.bookmarks().add(dir));
        FolderTree tree = fx(() -> window.left().tree());
        robot.interact(() -> tree.getRoot().getChildren().stream()
                .filter(i -> i.getValue().kind() == TreeNode.Kind.BOOKMARK_HEADER)
                .findFirst().ifPresent(h -> tree.getSelectionModel().select(h.getChildren().get(0))));
        waitUntil("navigated to bookmark", () -> dir.equals(leftTab().currentDir()));
        assertEquals(List.of("inside.txt"), names(leftTab()));
    }

    @Test
    void bookmarkContextMenuRenamesMovesAndRemoves(FxRobot robot) throws Exception {
        Path a = folderWith("alpha");
        Path b = folderWith("beta");
        robot.interact(() -> {
            window.bookmarks().add(a);
            window.bookmarks().add(b);
        });
        FolderTree tree = fx(() -> window.left().tree());

        fireMenuItem(fx(() -> window.treeMenuItems(bookmarkNode(tree, b))), "Move up");
        waitUntil("beta first", () -> window.bookmarks().items().get(0).path().equals(b));

        fireMenuItem(fx(() -> window.treeMenuItems(bookmarkNode(tree, a))), "Rename bookmark");
        answerDialog(robot, "My Alpha");
        waitUntil("renamed", () -> window.bookmarks().find(a).orElseThrow().name().equals("My Alpha"));
        assertEquals(List.of("beta=" + b, "My Alpha=" + a), treeBookmarks(tree));

        fireMenuItem(fx(() -> window.treeMenuItems(bookmarkNode(tree, b))), "Remove bookmark");
        waitUntil("removed", () -> window.bookmarks().items().size() == 1);
        assertEquals(List.of("My Alpha=" + a), treeBookmarks(tree));
        assertTrue(Files.isDirectory(b), "removing a bookmark never touches the folder");
    }

    @Test
    void folderContextMenusOfferAddToBookmarks(FxRobot robot) throws Exception {
        Path dir = folderWith("menus", "sub/");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "sub");
        fireMenuItem(fx(() -> window.fileMenuItems(tab)), "Add to bookmarks");
        waitUntil("selected folder bookmarked", () -> window.bookmarks().contains(dir.resolve("sub")));
        assertTrue(fx(() -> window.fileMenuItems(tab)).stream()
                .anyMatch(i -> "Remove from bookmarks".equals(i.getText())), "menu now offers removal");
    }

    @Test
    void manageDialogListsAndRemovesBookmarks(FxRobot robot) throws Exception {
        Path a = folderWith("m1");
        Path b = folderWith("m2");
        robot.interact(() -> {
            window.bookmarks().add(a);
            window.bookmarks().add(b);
        });
        robot.push(KeyCode.SHORTCUT, KeyCode.SHIFT, KeyCode.B);
        WaitForAsyncUtils.waitForFxEvents();
        javafx.scene.control.ListView<?> list = robot.lookup(".bookmark-list").queryAs(javafx.scene.control.ListView.class);
        assertEquals(2, (int) fx(() -> list.getItems().size()));
        robot.clickOn("Move down");
        waitUntil("m1 moved down", () -> window.bookmarks().items().get(1).path().equals(a));
        robot.clickOn("Remove");
        waitUntil("one left", () -> window.bookmarks().items().size() == 1);
        assertEquals(1, (int) fx(() -> list.getItems().size()), "dialog list is live");
    }

    @Test
    void bookmarksSurviveARestart(FxRobot robot) throws Exception {
        Path dir = folderWith("kept");
        robot.interact(() -> window.bookmarks().add(dir, "Kept"));
        Session reloaded = Session.load(tmp.resolve("session.properties")); // saved immediately, no close needed
        MainWindow restored = fx(() -> new MainWindow(new Stage(), reloaded));
        assertEquals(List.of("Kept=" + dir), treeBookmarks(fx(() -> restored.left().tree())));
    }
}
