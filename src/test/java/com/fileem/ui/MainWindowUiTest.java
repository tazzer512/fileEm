package com.fileem.ui;

import com.fileem.Autosave;
import com.fileem.Session;
import com.fileem.fs.FileEntry;
import com.fileem.fs.Shell;
import com.fileem.ops.FileOperation;
import com.fileem.util.OsUtil;
import javafx.concurrent.Worker;
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

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.Callable;
import java.util.concurrent.CountDownLatch;
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
        Theme.apply(Theme.Family.CLASSIC, false);
        session = Session.load(tmp.resolve("session.properties"));
        window = new MainWindow(stage, session);
        Scene scene = new Scene(window, 1200, 800);
        Theme.style(scene.getStylesheets());
        stage.setScene(scene);
        stage.show();
    }

    @AfterEach
    void closeWindows() throws Exception {
        FxToolkit.cleanupStages();
        // Detach this test's window: it stays alive (watchers, volume listeners) and would otherwise
        // keep restyling while reacting to its deleted temp folders, polluting later tests.
        WaitForAsyncUtils.asyncFx(() -> stage.setScene(null)).get();
        WaitForAsyncUtils.asyncFx(MainWindowUiTest::resetMonocleDragView).get();
    }

    /**
     * Works around a Monocle bug: a mouse drag that is not a drag-and-drop (e.g. a selection box)
     * leaves {@code MouseInput.dragView} pointing at that window, and only a real drop clears it.
     * The next test's drag-and-drop then sends a drag-leave to the dead window and fails with an NPE.
     */
    private static void resetMonocleDragView() {
        try {
            Class<?> mouseInput = Class.forName("com.sun.glass.ui.monocle.MouseInput");
            var getInstance = mouseInput.getDeclaredMethod("getInstance");
            getInstance.setAccessible(true);
            var dragView = mouseInput.getDeclaredField("dragView");
            dragView.setAccessible(true);
            dragView.set(getInstance.invoke(null), null);
        } catch (ClassNotFoundException e) {
            // not running on Monocle
        } catch (ReflectiveOperationException e) {
            throw new AssertionError("Monocle changed; revisit resetMonocleDragView", e);
        }
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

    /**
     * Regression: conflict questions used to be nested modal dialogs, so with two operations asking,
     * answering the first did nothing until the second was answered, and the whole app was blocked.
     */
    @Test
    void conflictQuestionsOfConcurrentOperationsAreAnsweredIndependentlyWithoutBlockingTheApp(FxRobot robot)
            throws Exception {
        Path from1 = folderWith("q1from", "a.txt");
        Path to1 = folderWith("q1to", "a.txt");
        Path from2 = folderWith("q2from", "b.txt");
        Path to2 = folderWith("q2to", "b.txt");
        Files.writeString(to2.resolve("b.txt"), "old");
        FileOperation first = fx(() -> window.runOperation(FileOperation.Kind.COPY, List.of(from1.resolve("a.txt")), to1));
        FileOperation second = fx(() -> window.runOperation(FileOperation.Kind.COPY, List.of(from2.resolve("b.txt")), to2));
        waitUntil("both ask", () -> ProgressWindow.of(stage) != null
                && ProgressWindow.of(stage).isAsking(first) && ProgressWindow.of(stage).isAsking(second));
        ProgressWindow w = fx(() -> ProgressWindow.of(stage));
        assertTrue(otherWindows(stage).stream().noneMatch(win -> win instanceof Stage st
                && st.getModality() != javafx.stage.Modality.NONE), "no modal dialog");

        // the main window stays usable while they wait (a modal dialog would swallow this click)
        Node newTabButton = robot.lookup(".main-toolbar .button").queryAll().stream()
                .filter(n -> n instanceof javafx.scene.control.Button b && b.getTooltip() != null
                        && b.getTooltip().getText().startsWith("New tab"))
                .findFirst().orElseThrow();
        robot.clickOn(newTabButton);
        waitUntil("new tab while asking", () -> window.left().tabs().size() == 2);

        robot.clickOn(fx(() -> w.conflictButton(first, "Skip")));
        waitUntil("first finished while the second still asks", () -> first.getState() == Worker.State.SUCCEEDED);
        assertTrue(fx(() -> w.isAsking(second)));

        robot.clickOn(fx(() -> w.conflictButton(second, "Replace")));
        waitUntil("second replaced", () -> second.getState() == Worker.State.SUCCEEDED);
        assertEquals("b.txt", Files.readString(to2.resolve("b.txt")));
        waitUntil("window closed", () -> ProgressWindow.of(stage) == null);
    }

    @Test
    void cancellingAnOperationThatAsksAboutAConflictKeepsTheExistingFile(FxRobot robot) throws Exception {
        Path from = folderWith("qcfrom", "a.txt");
        Path to = folderWith("qcto", "a.txt");
        Files.writeString(to.resolve("a.txt"), "existing");
        FileOperation op = fx(() -> window.runOperation(FileOperation.Kind.COPY, List.of(from.resolve("a.txt")), to));
        waitUntil("asks", () -> ProgressWindow.of(stage) != null && ProgressWindow.of(stage).isAsking(op));
        ProgressWindow w = fx(() -> ProgressWindow.of(stage));
        assertTrue(fx(() -> w.pauseButton(op).isDisabled()), "nothing to pause while waiting for an answer");
        robot.clickOn(fx(() -> w.cancelButton(op)));
        waitUntil("cancelled", op::isCancelled);
        waitUntil("window closed", () -> ProgressWindow.of(stage) == null);
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
        // Files are dragged by their name; elsewhere on a row, dragging draws a selection box.
        Node source = nameText(robot, "drag.txt");
        Node target = rowNode(robot, "into");
        robot.drag(source).dropTo(target);
        answerDrop(robot, "Move");
        waitUntil("moved into folder", () -> Files.exists(dir.resolve("into/drag.txt")));
        waitUntil("original gone", () -> !Files.exists(dir.resolve("drag.txt")));
    }

    /** Answers the Move / Copy / Cancel question that every drop asks. */
    private void answerDrop(FxRobot robot, String button) {
        waitUntil("move or copy question", () -> otherWindows(stage).stream()
                .anyMatch(w -> w instanceof Stage st && "Move or copy".equals(st.getTitle())));
        // only the dialog's buttons: the F-key bar also has "Move" and "Copy"
        Node b = robot.lookup(".dialog-pane .button").queryAll().stream()
                .filter(n -> n instanceof javafx.scene.control.Button x && x.getText().equals(button))
                .findFirst().orElseThrow(() -> new AssertionError("no " + button + " button"));
        robot.clickOn(b);
        WaitForAsyncUtils.waitForFxEvents();
    }

    @Test
    void choosingCopyAfterADropKeepsTheOriginal(FxRobot robot) throws Exception {
        Path dir = folderWith("dndcopy", "keep.txt", "into/");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "keep.txt");
        robot.drag(nameText(robot, "keep.txt")).dropTo(rowNode(robot, "into"));
        answerDrop(robot, "Copy");
        waitUntil("copied", () -> Files.exists(dir.resolve("into/keep.txt")));
        assertTrue(Files.exists(dir.resolve("keep.txt")), "the original stays");
    }

    @Test
    void cancellingTheDropQuestionLeavesEverythingAlone(FxRobot robot) throws Exception {
        Path dir = folderWith("dndcancel", "stay.txt", "into/");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "stay.txt");
        robot.drag(nameText(robot, "stay.txt")).dropTo(rowNode(robot, "into"));
        answerDrop(robot, "Cancel");
        assertTrue(OperationUi.running().isEmpty(), "no operation started");
        assertTrue(Files.exists(dir.resolve("stay.txt")));
        assertFalse(Files.exists(dir.resolve("into/stay.txt")));
    }

    private Node rowNode(FxRobot robot, String name) {
        return robot.lookup(".file-table .table-row-cell").queryAll().stream()
                .filter(n -> n instanceof TableRow<?> r && r.getItem() instanceof FileEntry e && e.name().equals(name))
                .findFirst().orElseThrow(() -> new AssertionError("no row " + name));
    }

    private Node nameText(FxRobot robot, String name) {
        return robot.lookup(".file-table .name-cell .text").queryAll().stream()
                .filter(n -> n instanceof javafx.scene.text.Text t && t.getText().equals(name))
                .findFirst().orElseThrow(() -> new AssertionError("no name " + name));
    }

    private Node cellOf(FxRobot robot, String name, String columnId) {
        return robot.from(rowNode(robot, name)).lookup(".table-cell").queryAll().stream()
                .filter(n -> n instanceof javafx.scene.control.TableCell<?, ?> c
                        && columnId.equals(c.getTableColumn().getId()))
                .findFirst().orElseThrow(() -> new AssertionError("no " + columnId + " cell for " + name));
    }

    private static List<String> selectedNames(FileTab tab) {
        return fx(() -> tab.selectedEntries().stream().map(FileEntry::name).toList());
    }

    @Test
    void draggingFromTheSizeColumnDrawsABoxThatSelectsRows(FxRobot robot) throws Exception {
        Path dir = folderWith("marquee", "a.txt", "b.txt", "c.txt", "d.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "d.txt");
        robot.drag(cellOf(robot, "a.txt", "size"));
        robot.moveTo(cellOf(robot, "c.txt", "type"));
        assertTrue(fx(() -> tab.table().isMarqueeActive()), "box is shown while dragging");
        assertTrue(fx(() -> tab.table().lookup(".marquee").isVisible()));
        assertEquals(List.of("a.txt", "b.txt", "c.txt"), selectedNames(tab));
        robot.release(javafx.scene.input.MouseButton.PRIMARY);
        assertFalse(fx(() -> tab.table().isMarqueeActive()));
        assertFalse(fx(() -> tab.table().lookup(".marquee").isVisible()), "box hidden after release");
        assertEquals(List.of("a.txt", "b.txt", "c.txt"), selectedNames(tab));
        assertTrue(Files.exists(dir.resolve("d.txt")) && Files.exists(dir.resolve("a.txt")), "nothing moved");
    }

    @Test
    void boxFromEmptySpaceSelectsUpwardsAndShiftAddsToTheSelection(FxRobot robot) throws Exception {
        Path dir = folderWith("marquee2", "a.txt", "b.txt", "c.txt", "d.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "a.txt");
        Node table = fx(() -> tab.table());
        javafx.geometry.Bounds rows = fx(() -> rowNode(robot, "d.txt").localToScreen(
                rowNode(robot, "d.txt").getBoundsInLocal()));
        // Press in the empty area below the last row, drag up into c.txt.
        robot.moveTo(rows.getMinX() + 400, rows.getMaxY() + 60);
        robot.press(KeyCode.SHIFT);
        robot.press(javafx.scene.input.MouseButton.PRIMARY);
        robot.moveTo(cellOf(robot, "c.txt", "modified"));
        robot.release(javafx.scene.input.MouseButton.PRIMARY);
        robot.release(KeyCode.SHIFT);
        assertEquals(List.of("a.txt", "c.txt", "d.txt"), selectedNames(tab));
        assertTrue(fx(() -> table.isFocused()), "the list keeps focus");
    }

    // ---- themes ----

    private javafx.scene.control.RadioMenuItem themeItem(String id) {
        return fx(() -> (javafx.scene.control.RadioMenuItem) window.themeMenu().getItems().stream()
                .filter(i -> ("theme-" + id).equals(i.getId())).findFirst().orElseThrow());
    }

    @Test
    void themeMenuSwitchesFamilyAndModeAndTheSessionRemembersThem(FxRobot robot) {
        try {
            robot.interact(() -> themeItem("windows").fire());
            assertEquals(Theme.Family.WINDOWS, fx(Theme::family));
            assertTrue(fx(() -> stage.getScene().getStylesheets().containsAll(Theme.Family.WINDOWS.stylesheets(false))));

            robot.interact(() -> themeItem("dark").fire());
            assertTrue(fx(Theme::isDark));
            assertTrue(fx(() -> stage.getScene().getStylesheets().containsAll(Theme.Family.WINDOWS.stylesheets(true))));
            assertFalse(fx(() -> stage.getScene().getStylesheets().containsAll(Theme.Family.WINDOWS.stylesheets(false))),
                    "the light colors are gone");

            robot.interact(() -> themeItem("aurora").fire());
            assertEquals(Theme.Family.AURORA, fx(Theme::family));
            assertTrue(fx(Theme::isDark), "switching the family keeps dark mode");

            robot.interact(window::saveSession);
            assertEquals("aurora", session.get("theme.name", null));
            assertTrue(session.getBool("theme.dark", false));
        } finally {
            robot.interact(() -> Theme.apply(Theme.Family.CLASSIC, false));
        }
    }

    @Test
    void themeMenuMarksTheCurrentThemeWhenOpened(FxRobot robot) {
        try {
            robot.interact(() -> Theme.apply(Theme.Family.ADWAITA, true));
            robot.interact(() -> window.themeMenu().show());
            waitUntil("menu open", () -> window.themeMenu().isShowing());
            assertTrue(fx(() -> themeItem("adwaita").isSelected()));
            assertFalse(fx(() -> themeItem("classic").isSelected()));
            assertTrue(fx(() -> themeItem("dark").isSelected()));
            assertFalse(fx(() -> themeItem("light").isSelected()));
            robot.interact(() -> window.themeMenu().hide());
        } finally {
            robot.interact(() -> Theme.apply(Theme.Family.CLASSIC, false));
        }
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

    /**
     * Opens a node's context menu the way the OS does (a CONTEXT_MENU_REQUESTED event at the node).
     * Monocle doesn't turn robot right-clicks into menu requests; real desktops do.
     */
    private static void requestContextMenu(FxRobot robot, Node node) {
        robot.interact(() -> {
            javafx.geometry.Bounds b = node.localToScreen(node.getBoundsInLocal());
            node.fireEvent(new javafx.scene.input.ContextMenuEvent(javafx.scene.input.ContextMenuEvent.CONTEXT_MENU_REQUESTED,
                    5, 5, b.getMinX() + 5, b.getMinY() + 5, false, null));
        });
        WaitForAsyncUtils.waitForFxEvents();
    }

    /** Finds a menu item by text prefix, descending into submenus ("New", "Sort by"). */
    private static javafx.scene.control.MenuItem findMenuItem(List<javafx.scene.control.MenuItem> items, String textPrefix) {
        for (javafx.scene.control.MenuItem i : items) {
            if (i.getText() != null && i.getText().startsWith(textPrefix)) {
                return i;
            }
            if (i instanceof javafx.scene.control.Menu m) {
                javafx.scene.control.MenuItem inner = findMenuItem(m.getItems(), textPrefix);
                if (inner != null) {
                    return inner;
                }
            }
        }
        return null;
    }

    private static List<String> menuTexts(List<javafx.scene.control.MenuItem> items) {
        return items.stream().map(javafx.scene.control.MenuItem::getText).filter(java.util.Objects::nonNull).toList();
    }

    private static boolean hasItem(List<javafx.scene.control.MenuItem> items, String textPrefix) {
        return findMenuItem(items, textPrefix) != null;
    }

    /** Exact command name, ignoring a shortcut hint: "Copy" matches "Copy" but not "Copy path". */
    private static boolean hasCommand(List<javafx.scene.control.MenuItem> items, String name) {
        return menuTexts(items).stream().anyMatch(t -> t.equals(name) || t.startsWith(name + "  ("));
    }

    private void fireMenuItem(List<javafx.scene.control.MenuItem> items, String textPrefix) {
        javafx.scene.control.MenuItem item = findMenuItem(items, textPrefix);
        if (item == null) {
            throw new AssertionError("no menu item " + textPrefix + " in " + menuTexts(items));
        }
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
    void autosaveKeepsSettingsWithoutANormalClose(FxRobot robot) throws Exception {
        Path a = folderWith("auto-a");
        Path b = folderWith("auto-b");
        Autosave autosave = new Autosave(session, window::saveSession);
        try {
            open(robot, leftTab(), a);
            robot.push(KeyCode.SHORTCUT, KeyCode.T);
            waitUntil("second tab", () -> window.left().tabs().size() == 2);
            open(robot, leftTab(), b);
            robot.type(KeyCode.F9);
            waitUntil("dual on", window::isDual);
            robot.interact(() -> Theme.apply(Theme.Family.ADWAITA, true));
            fx(autosave::saveInBackground).get(TIMEOUT_S, TimeUnit.SECONDS);

            // A fresh read of the file, as on the next start after a kill: no saveSession, no close.
            Session reloaded = Session.load(tmp.resolve("session.properties"));
            assertEquals("adwaita", reloaded.get("theme.name", null));
            assertTrue(reloaded.getBool("theme.dark", false));
            MainWindow restored = fx(() -> new MainWindow(new Stage(), reloaded));
            waitUntil("tabs, paths and dual pane restored", () -> restored.isDual()
                    && restored.left().tabs().size() == 2
                    && a.equals(restored.left().tabs().get(0).currentDir())
                    && b.equals(restored.left().tabs().get(1).currentDir())
                    && restored.left().activeTab() == restored.left().tabs().get(1));
        } finally {
            robot.interact(() -> Theme.apply(Theme.Family.CLASSIC, false));
        }
    }

    @Test
    void bookmarksSurviveARestart(FxRobot robot) throws Exception {
        Path dir = folderWith("kept");
        robot.interact(() -> window.bookmarks().add(dir, "Kept"));
        Session reloaded = Session.load(tmp.resolve("session.properties")); // saved immediately, no close needed
        MainWindow restored = fx(() -> new MainWindow(new Stage(), reloaded));
        assertEquals(List.of("Kept=" + dir), treeBookmarks(fx(() -> restored.left().tree())));
    }

    // ---- context menus ----

    @Test
    void rightClickInTheTreeShowsAMenuWithoutNavigating(FxRobot robot) throws Exception {
        Path dir = folderWith("stay");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        FolderTree tree = fx(() -> window.left().tree());
        waitUntil("tree reveals folder", () -> tree.getSelectionModel().getSelectedItem() != null
                && dir.equals(tree.getSelectionModel().getSelectedItem().getValue().path()));
        Node other = robot.lookup(".folder-tree .tree-cell").queryAll().stream()
                .filter(n -> n instanceof javafx.scene.control.TreeCell<?> c && c.getItem() instanceof TreeNode t
                        && t.path() != null && !t.path().equals(dir) && c.isVisible())
                .reduce((a, b) -> b).orElseThrow();
        robot.rightClickOn(other);
        WaitForAsyncUtils.waitForFxEvents();
        assertEquals(dir, fx(tab::currentDir), "right-click does not navigate");
        assertEquals(dir, fx(() -> tree.getSelectionModel().getSelectedItem().getValue().path()),
                "right-click does not move the tree selection");

        requestContextMenu(robot, other);
        javafx.scene.control.ContextMenu menu = ((javafx.scene.control.TreeCell<?>) other).getContextMenu();
        waitUntil("tree menu shown", menu::isShowing);
        assertTrue(hasItem(fx(menu::getItems), "Open in new tab"));
        assertTrue(fx(() -> other.getStyleClass().contains("context-target")), "right-clicked folder is outlined");
        robot.interact(menu::hide);
        waitUntil("outline removed", () -> !other.getStyleClass().contains("context-target"));
    }

    /** Regression: menus filled in onShowing never opened, since ContextMenu.show ignores empty menus. */
    @Test
    void fileListContextMenuOpensOnItemsAndOnEmptySpace(FxRobot robot) throws Exception {
        Path dir = folderWith("ctx", "one.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "one.txt");
        javafx.scene.control.ContextMenu menu = fx(() -> tab.table().getContextMenu());

        requestContextMenu(robot, rowNode(robot, "one.txt"));
        waitUntil("item menu shown", menu::isShowing);
        assertTrue(hasItem(fx(menu::getItems), "Rename"), "item actions: " + menuTexts(fx(menu::getItems)));
        robot.interact(menu::hide);

        robot.interact(() -> tab.table().getSelectionModel().clearSelection());
        requestContextMenu(robot, fx(tab::table));
        waitUntil("folder menu shown", menu::isShowing);
        assertTrue(hasItem(fx(menu::getItems), "Sort by"), "folder actions: " + menuTexts(fx(menu::getItems)));
        robot.interact(menu::hide);
    }

    @Test
    void fileAndFolderMenusOfferTheUsualActions(FxRobot robot) throws Exception {
        Path dir = folderWith("menu", "doc.txt", "pack.zip", "sub/");
        FileTab tab = leftTab();
        open(robot, tab, dir);

        select(robot, tab, "doc.txt");
        List<javafx.scene.control.MenuItem> file = fx(() -> window.fileMenuItems(tab));
        for (String expected : List.of("Open", "Open in terminal", "Show in", "Cut", "Copy", "Copy to other side",
                "Move to other side", "Duplicate", "Compress to ZIP", "Rename", "Delete permanently", "Copy path",
                "Properties")) {
            assertTrue(hasItem(file, expected), expected + " in " + menuTexts(file));
        }
        assertFalse(hasItem(file, "Extract here"), "only zips can be extracted");
        assertFalse(hasItem(file, "Open in new tab"), "files don't open in tabs");

        select(robot, tab, "pack.zip");
        assertTrue(hasItem(fx(() -> window.fileMenuItems(tab)), "Extract here"));

        select(robot, tab, "sub");
        List<javafx.scene.control.MenuItem> folder = fx(() -> window.fileMenuItems(tab));
        assertTrue(hasItem(folder, "Open in new tab") && hasItem(folder, "Add to bookmarks"));

        select(robot, tab, "doc.txt", "sub");
        assertTrue(fx(() -> findMenuItem(window.fileMenuItems(tab), "Rename").isDisable()), "rename needs one item");
    }

    @Test
    void emptySpaceMenuOffersNewSortAndHiddenFiles(FxRobot robot) throws Exception {
        Path dir = folderWith("bg", "b.txt");
        Files.writeString(dir.resolve("a.txt"), "much longer content");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        robot.interact(() -> tab.table().getSelectionModel().clearSelection());
        List<javafx.scene.control.MenuItem> items = fx(() -> window.fileMenuItems(tab));
        for (String expected : List.of("Paste", "New", "Folder", "Text file", "Sort by", "Show hidden files",
                "Refresh", "Open in terminal", "Properties")) {
            assertTrue(hasItem(items, expected), expected + " in " + menuTexts(items));
        }

        fireMenuItem(items, "Size");
        waitUntil("sorted by size", () -> "size".equals(tab.table().sortColumnId()));
        assertEquals(List.of("b.txt", "a.txt"), names(tab));
        fireMenuItem(fx(() -> window.fileMenuItems(tab)), "Descending");
        waitUntil("descending", () -> names(tab).equals(List.of("a.txt", "b.txt")));
    }

    @Test
    void newTextFileFromTheMenu(FxRobot robot) throws Exception {
        Path dir = folderWith("newfile");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        fireMenuItem(fx(() -> window.fileMenuItems(tab)), "Text file");
        answerDialog(robot, "notes"); // base name is preselected, ".txt" stays
        waitUntil("file created", () -> Files.isRegularFile(dir.resolve("notes.txt")));
        waitUntil("and selected", () -> tab.selectedPaths().equals(List.of(dir.resolve("notes.txt"))));
    }

    @Test
    void duplicateCompressAndExtractFromTheMenu(FxRobot robot) throws Exception {
        Path dir = folderWith("arch", "a.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "a.txt");
        fireMenuItem(fx(() -> window.fileMenuItems(tab)), "Duplicate");
        waitUntil("duplicated", () -> Files.exists(dir.resolve("a - Copy.txt")));

        select(robot, tab, "a.txt", "a - Copy.txt");
        fireMenuItem(fx(() -> window.fileMenuItems(tab)), "Compress to ZIP");
        waitUntil("archive selected", () -> tab.selectedPaths().equals(List.of(dir.resolve("Archive.zip"))));

        fireMenuItem(fx(() -> window.fileMenuItems(tab)), "Extract here");
        waitUntil("extracted and selected", () -> tab.selectedPaths().equals(List.of(dir.resolve("Archive"))));
        assertEquals("a.txt", Files.readString(dir.resolve("Archive/a.txt")));
    }

    @Test
    void pasteIntoAFolderFromItsMenu(FxRobot robot) throws Exception {
        Path dir = folderWith("pin", "f.txt", "target/");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "f.txt");
        robot.push(KeyCode.SHORTCUT, KeyCode.C);
        select(robot, tab, "target");
        fireMenuItem(fx(() -> window.fileMenuItems(tab)), "Paste into folder");
        waitUntil("pasted into folder", () -> Files.exists(dir.resolve("target/f.txt")));
    }

    @Test
    void treeMenusProtectShortcutsAndVolumes(FxRobot robot) throws Exception {
        Path dir = folderWith("plain");
        List<javafx.scene.control.MenuItem> folder = fx(() -> window.treeMenuItems(TreeNode.folder(dir)));
        for (String expected : List.of("Open", "Open in terminal", "Cut", "Copy", "Paste into folder", "New",
                "Duplicate", "Compress to ZIP", "Rename…", "Delete permanently", "Add to bookmarks", "Properties")) {
            assertTrue(hasItem(folder, expected), expected + " in " + menuTexts(folder));
        }

        TreeNode home = new TreeNode(TreeNode.Kind.QUICK, OsUtil.home(), "Home", "mdi2h-home", "icon-quick", null);
        List<javafx.scene.control.MenuItem> quick = fx(() -> window.treeMenuItems(home));
        assertTrue(hasItem(quick, "Copy") && hasItem(quick, "Paste into folder"));
        for (String forbidden : List.of("Cut", "Rename…", "Move to Trash", "Delete permanently", "Duplicate")) {
            assertFalse(hasCommand(quick, forbidden), forbidden + " must not be offered for Home");
        }

        TreeNode volume = new TreeNode(TreeNode.Kind.VOLUME, tmp.getRoot(), "Disk", "mdi2h-harddisk", "icon-volume", null);
        List<javafx.scene.control.MenuItem> vol = fx(() -> window.treeMenuItems(volume));
        for (String forbidden : List.of("Cut", "Copy", "Rename…", "Delete permanently", "Compress to ZIP")) {
            assertFalse(hasCommand(vol, forbidden), forbidden + " must not be offered for a drive");
        }
    }

    // ---- eject ----

    /** Content texts of the open dialogs. */
    private List<String> dialogTexts() {
        return fx(() -> otherWindows(stage).stream()
                .filter(w -> w.isShowing() && w.getScene() != null
                        && w.getScene().getRoot() instanceof javafx.scene.control.DialogPane)
                .map(w -> ((javafx.scene.control.DialogPane) w.getScene().getRoot()).getContentText())
                .filter(java.util.Objects::nonNull).toList());
    }

    /** Shows a fake removable drive at {@code root} in the left tree. */
    private TreeNode fakeDrive(FxRobot robot, Path root) {
        var vol = new com.fileem.fs.Volumes.Volume(root, "STICK", com.fileem.fs.Volumes.Kind.REMOVABLE, 1, 2, true);
        robot.interact(() -> window.left().tree().syncVolumes(List.of(vol)));
        return new TreeNode(TreeNode.Kind.VOLUME, root, "STICK", "mdi2u-usb-flash-drive", "icon-volume", null, true);
    }

    @Test
    void onlyRemovableDrivesOfferEject(FxRobot robot) throws Exception {
        Path root = Files.createDirectories(tmp.resolve("stick"));
        TreeNode drive = fakeDrive(robot, root);
        assertTrue(hasCommand(fx(() -> window.treeMenuItems(drive)), "Eject"));
        TreeNode disk = new TreeNode(TreeNode.Kind.VOLUME, root, "Disk", "mdi2h-harddisk", "icon-volume", null);
        assertFalse(hasCommand(fx(() -> window.treeMenuItems(disk)), "Eject"));
        assertFalse(hasCommand(fx(() -> window.treeMenuItems(TreeNode.folder(root))), "Eject"));
        // the drive's row in the tree has an eject button (scrolled into view: rows are virtualized)
        robot.interact(() -> {
            FolderTree tree = window.left().tree();
            for (var header : tree.getRoot().getChildren()) {
                for (var item : header.getChildren()) {
                    if (root.equals(item.getValue().path())) {
                        tree.scrollTo(tree.getRow(item));
                    }
                }
            }
        });
        waitUntil("eject button shown", () -> robot.lookup(".eject-button").queryAll().stream()
                .anyMatch(n -> n.isVisible() && n.getScene() == stage.getScene()));
    }

    @Test
    void ejectMovesTabsOffTheDriveFirstThenEjectsAndConfirms(FxRobot robot) throws Exception {
        Path root = Files.createDirectories(tmp.resolve("stick2/photos"));
        Path driveRoot = root.getParent();
        TreeNode drive = fakeDrive(robot, driveRoot);
        FileTab tab = leftTab();
        open(robot, tab, root);
        List<Path> ejected = new java.util.concurrent.CopyOnWriteArrayList<>();
        List<Path> tabDirWhenEjecting = new java.util.concurrent.CopyOnWriteArrayList<>();
        CountDownLatch release = new CountDownLatch(1);
        robot.interact(() -> window.setEjector(r -> {
            tabDirWhenEjecting.add(WaitForAsyncUtils.asyncFx(tab::currentDir).get());
            release.await();
            ejected.addAll(r);
        }));

        fireMenuItem(fx(() -> window.treeMenuItems(drive)), "Eject");
        waitUntil("ejecting", () -> !tabDirWhenEjecting.isEmpty());
        assertFalse(tabDirWhenEjecting.get(0).startsWith(driveRoot), "the tab left the drive before ejecting");
        assertTrue(fx(() -> window.isEjecting(driveRoot)));
        assertTrue(fx(() -> window.treeMenuItems(drive).stream()
                .anyMatch(i -> "Eject".equals(i.getText()) && i.isDisable())), "no second eject meanwhile");

        release.countDown();
        waitUntil("safe to remove notice", () -> dialogTexts().stream().anyMatch(t -> t.contains("can now be safely removed")));
        assertEquals(List.of(driveRoot), ejected);
        assertFalse(fx(() -> window.isEjecting(driveRoot)));
        robot.clickOn("OK");
    }

    @Test
    void ejectIsRefusedWhileAnOperationUsesTheDrive(FxRobot robot) throws Exception {
        var release = new CountDownLatch(1);
        try {
            FileOperation op = blockedCopy("busy", release);
            TreeNode drive = fakeDrive(robot, op.targetDir());
            List<Path> ejected = new java.util.concurrent.CopyOnWriteArrayList<>();
            robot.interact(() -> window.setEjector(ejected::addAll));
            waitUntil("operation running", () -> OperationUi.running().contains(op));

            fireMenuItem(fx(() -> window.treeMenuItems(drive)), "Eject");
            waitUntil("refusal shown", () -> dialogTexts().stream().anyMatch(t -> t.contains("in use by a running file operation")));
            assertTrue(ejected.isEmpty(), "nothing was ejected");
            assertFalse(fx(() -> window.isEjecting(op.targetDir())));
            robot.clickOn("OK");
        } finally {
            release.countDown();
        }
    }

    @Test
    void ejectRemovesTheWholeDriveAndMovesTabsOffEveryPartition(FxRobot robot) throws Exception {
        Path photos = Files.createDirectories(tmp.resolve("photos"));
        Path storage = Files.createDirectories(tmp.resolve("storage/sub"));
        Path storageRoot = storage.getParent();
        TreeNode drive = fakeDrive(robot, photos);
        FileTab tab = leftTab();
        open(robot, tab, storage); // a tab on the other partition of the same drive
        List<List<Path>> ejected = new java.util.concurrent.CopyOnWriteArrayList<>();
        List<Path> tabDirWhenEjecting = new java.util.concurrent.CopyOnWriteArrayList<>();
        robot.interact(() -> window.setEjector(new MainWindow.Ejector() {
            @Override
            public List<Path> drive(Path root) {
                return List.of(photos, storageRoot);
            }

            @Override
            public void eject(List<Path> roots) throws Exception {
                tabDirWhenEjecting.add(WaitForAsyncUtils.asyncFx(tab::currentDir).get());
                ejected.add(roots);
            }
        }));
        fireMenuItem(fx(() -> window.treeMenuItems(drive)), "Eject");
        waitUntil("notice", () -> dialogTexts().stream().anyMatch(t -> t.contains("can now be safely removed")));
        assertEquals(List.of(List.of(photos, storageRoot)), ejected);
        assertFalse(tabDirWhenEjecting.get(0).startsWith(storageRoot), "the tab left the other partition first");
        assertTrue(dialogTexts().stream().anyMatch(t -> t.contains("\"photos\" and \"storage\"")), dialogTexts().toString());
        robot.clickOn("OK");
    }

    @Test
    void aFailedEjectShowsTheReasonAndCanBeRetried(FxRobot robot) throws Exception {
        Path root = Files.createDirectories(tmp.resolve("stick3"));
        TreeNode drive = fakeDrive(robot, root);
        robot.interact(() -> window.setEjector(r -> {
            throw new IOException("The drive is in use.");
        }));
        fireMenuItem(fx(() -> window.treeMenuItems(drive)), "Eject");
        waitUntil("error shown", () -> dialogTexts().stream().anyMatch(t -> t.contains("The drive is in use.")));
        assertFalse(fx(() -> window.isEjecting(root)), "can be tried again");
        robot.clickOn("OK");
    }

    @Test
    void renamingAFolderFromTheTreeMovesOpenTabsAndBookmarks(FxRobot robot) throws Exception {
        Path proj = Files.createDirectories(tmp.resolve("proj/inner"));
        Path projDir = proj.getParent();
        FileTab tab = leftTab();
        open(robot, tab, proj);
        robot.interact(() -> window.bookmarks().add(projDir));

        fireMenuItem(fx(() -> window.treeMenuItems(TreeNode.folder(projDir))), "Rename…");
        answerDialog(robot, "renamed");
        Path renamed = tmp.resolve("renamed");
        waitUntil("renamed on disk", () -> Files.isDirectory(renamed.resolve("inner")));
        waitUntil("tab followed", () -> renamed.resolve("inner").equals(tab.currentDir()));
        assertTrue(fx(() -> window.bookmarks().contains(renamed)), "bookmark followed");
        assertEquals("renamed", fx(() -> window.bookmarks().find(renamed).orElseThrow().name()));
    }

    @Test
    void deletingAFolderFromTheTree(FxRobot robot) throws Exception {
        Path dir = folderWith("doomed", "x.txt");
        fireMenuItem(fx(() -> window.treeMenuItems(TreeNode.folder(dir))), "Delete permanently");
        robot.clickOn("OK");
        waitUntil("deleted", () -> !Files.exists(dir));
    }

    @Test
    void bothFileListsStayVisibleInANarrowDualPaneWindow(FxRobot robot) {
        robot.type(KeyCode.F9);
        waitUntil("dual on", window::isDual);
        robot.interact(() -> stage.setWidth(700));
        WaitForAsyncUtils.waitForFxEvents();
        for (ExplorerSide side : List.of(fx(window::left), fx(window::right))) {
            waitUntil("file list keeps a usable width", () -> {
                javafx.scene.Node list = side.getItems().get(1);
                return list.getLayoutBounds().getWidth() >= 250;
            });
        }
    }

    private FileOperation blockedCopy(String name, CountDownLatch release) throws Exception {
        return blockedCopy(name, release, false);
    }

    /**
     * Starts a copy that waits in its conflict handler until {@code release} opens. With {@code more},
     * a second file follows the conflicting one, so the operation still has a step left to pause at.
     */
    private FileOperation blockedCopy(String name, CountDownLatch release, boolean more) throws Exception {
        Path src = more ? folderWith(name + "-src", name + ".txt", name + "-2.txt") : folderWith(name + "-src", name + ".txt");
        Path dst = folderWith(name + "-dst", name + ".txt");
        List<Path> sources = more ? List.of(src.resolve(name + ".txt"), src.resolve(name + "-2.txt"))
                : List.of(src.resolve(name + ".txt"));
        var op = new FileOperation(FileOperation.Kind.COPY, sources, dst, (s, t) -> {
            release.await();
            return new FileOperation.Decision(FileOperation.Resolution.SKIP, false);
        });
        fx(() -> {
            OperationUi.start(stage, op, sources, dst, dirs -> { });
            return null;
        });
        return op;
    }

    @Test
    void theProgressLineShowsSpeedAndTimeLeftOnceKnown() {
        assertEquals("", ProgressWindow.detailText(-1, 0, 0, true, false, ""), "nothing while counting");
        assertEquals("25%   ·   1.00 KB of 4.00 KB",
                ProgressWindow.detailText(0.25, 1024, 4096, true, false, ""), "no estimate yet");
        assertEquals("25%   ·   1.00 KB of 4.00 KB   ·   2.00 MB/s   ·   About 5 seconds left",
                ProgressWindow.detailText(0.25, 1024, 4096, true, false, "2.00 MB/s   ·   About 5 seconds left"));
        assertEquals("Paused   ·   50%   ·   2 of 4 items",
                ProgressWindow.detailText(0.5, 2, 4, false, true, "About 5 seconds left"), "no guess while paused");
    }

    @Test
    void noTimeLeftIsGuessedForAStalledOrPausedOperation(FxRobot robot) throws Exception {
        var release = new CountDownLatch(1);
        try {
            var op = blockedCopy("eta", release, true); // stuck waiting in its conflict handler: no progress
            waitUntil("row shown", () -> ProgressWindow.of(stage) != null
                    && ProgressWindow.of(stage).operations().contains(op));
            ProgressWindow w = fx(() -> ProgressWindow.of(stage));
            long s = 1_000_000_000L;
            for (int i = 0; i < 15; i++) {
                long now = (100 + i) * s;
                robot.interact(() -> w.tick(now));
            }
            String stalled = fx(() -> w.detailText(op));
            assertFalse(stalled.contains("left") || stalled.contains("/s"), stalled);

            robot.interact(op::pause);
            for (int i = 15; i < 30; i++) {
                long now = (100 + i) * s;
                robot.interact(() -> w.tick(now));
            }
            String paused = fx(() -> w.detailText(op));
            assertTrue(paused.startsWith("Paused") && !paused.contains("left"), paused);
            robot.interact(op::resume);
        } finally {
            release.countDown();
        }
    }

    @Test
    void pauseButtonPausesAndResumesAnOperation(FxRobot robot) throws Exception {
        var release = new CountDownLatch(1);
        try {
            var op = blockedCopy("paused", release, true);
            waitUntil("operation shown", () -> ProgressWindow.of(stage) != null
                    && ProgressWindow.of(stage).operations().contains(op));
            ProgressWindow w = fx(() -> ProgressWindow.of(stage));
            javafx.scene.control.Button pause = fx(() -> w.pauseButton(op));
            assertEquals("Pause", fx(pause::getText));

            robot.clickOn(pause);
            waitUntil("paused", () -> op.isPaused() && "Resume".equals(pause.getText()));
            release.countDown(); // the conflict is answered; the paused operation must now wait
            waitUntil("waiting at the next step", op::isWaitingWhilePaused);
            assertFalse(fx(op::isDone), "a paused operation does not finish");

            robot.clickOn(pause);
            waitUntil("resumed and finished", () -> !op.isPaused() && op.getState() == Worker.State.SUCCEEDED);
            waitUntil("window closed", () -> ProgressWindow.of(stage) == null);
        } finally {
            release.countDown();
        }
    }

    @Test
    void cancellingAPausedOperationStopsIt(FxRobot robot) throws Exception {
        var release = new CountDownLatch(1);
        try {
            var op = blockedCopy("pausedcancel", release, true);
            waitUntil("operation shown", () -> ProgressWindow.of(stage) != null);
            ProgressWindow w = fx(() -> ProgressWindow.of(stage));
            robot.clickOn(fx(() -> w.pauseButton(op)));
            waitUntil("paused", op::isPaused);
            release.countDown();
            waitUntil("waiting while paused", op::isWaitingWhilePaused);
            robot.clickOn(fx(() -> w.cancelButton(op)));
            waitUntil("cancelled", op::isCancelled);
            waitUntil("window closed", () -> ProgressWindow.of(stage) == null);
        } finally {
            release.countDown();
        }
    }

    @Test
    void concurrentOperationsStackInOneProgressWindowAndCancelIndependently(FxRobot robot) throws Exception {
        var release = new CountDownLatch(1);
        try {
            var first = blockedCopy("first", release);
            var second = blockedCopy("second", release);
            waitUntil("both operations shown", () -> {
                ProgressWindow w = ProgressWindow.of(stage);
                return w != null && w.operations().size() == 2;
            });
            ProgressWindow w = fx(() -> ProgressWindow.of(stage));
            assertEquals(List.of(first, second), fx(w::operations));
            assertEquals(1, otherWindows(stage).size(), "one shared window, not one per operation");
            assertEquals("2 operations in progress", fx(() -> w.stage().getTitle()));
            assertEquals(Icons.appIcons(), fx(() -> List.copyOf(w.stage().getIcons())), "has the app icon");

            robot.clickOn(fx(() -> w.cancelButton(first)));
            waitUntil("first cancelled", first::isCancelled);
            waitUntil("first's row removed", () -> w.operations().equals(List.of(second)));
            assertFalse(fx(second::isDone), "cancelling one operation leaves the other running");
            assertTrue(fx(() -> w.stage().isShowing()));
            assertTrue(fx(() -> w.stage().getTitle().startsWith("Copying \"second.txt\"")));

            release.countDown();
            waitUntil("second finished", () -> second.getState() == Worker.State.SUCCEEDED);
            waitUntil("window closed after the last operation", () -> ProgressWindow.of(stage) == null
                    && !w.stage().isShowing());
        } finally {
            release.countDown();
        }
    }

    // ---- threading ----

    @Test
    void aWatcherRefreshDoesNotSwallowANavigationThatIsStillLoading(FxRobot robot) throws Exception {
        Path a = folderWith("nav-a", "one.txt");
        Path b = folderWith("nav-b", "two.txt");
        FileTab tab = leftTab();
        open(robot, tab, a);
        // Both in one FX pulse: the refresh arrives while b is still being listed.
        robot.interact(() -> {
            tab.navigateTo(b);
            tab.refresh();
        });
        waitUntil("navigation completed", () -> b.equals(tab.currentDir()));
        assertEquals(List.of("two.txt"), names(tab));
    }

    @Test
    void aNavigationThatCancelsARefreshKeepsTheWatcherRefreshing(FxRobot robot) throws Exception {
        Path a = folderWith("cancel-a", "one.txt");
        Path b = folderWith("cancel-b", "two.txt");
        FileTab tab = leftTab();
        open(robot, tab, a);
        // the navigation cancels the refresh that is still listing, which then never calls back
        robot.interact(() -> {
            tab.refresh();
            tab.navigateTo(b);
        });
        waitUntil("navigation completed and watching", () -> b.equals(tab.currentDir()) && tab.isWatching());
        Files.writeString(b.resolve("new.txt"), "n");
        waitUntil("the watcher refreshed the list", () -> names(tab).contains("new.txt"));
    }

    @Test
    void aRefreshRestoresALargeSelectionInOneChange(FxRobot robot) throws Exception {
        String[] files = new String[400];
        for (int i = 0; i < files.length; i++) {
            files[i] = "f" + i + ".txt";
        }
        Path dir = folderWith("bigsel", files);
        FileTab tab = leftTab();
        open(robot, tab, dir);
        robot.interact(tab::selectAll);
        int[] changes = {0};
        javafx.collections.ListChangeListener<FileEntry> count = c -> changes[0]++;
        robot.interact(() -> tab.table().getSelectionModel().getSelectedItems().addListener(count));
        try {
            robot.interact(() -> tab.refresh());
            waitUntil("selection restored", () -> changes[0] > 0 && tab.selectedPaths().size() == files.length);
            // clearing plus one bulk select; one change per item made big selections quadratic
            assertTrue(changes[0] <= 3, changes[0] + " selection changes");
        } finally {
            robot.interact(() -> tab.table().getSelectionModel().getSelectedItems().removeListener(count));
        }
    }

    @Test
    void navigatingToAFileOpensItsFolderWithTheFileSelected(FxRobot robot) throws Exception {
        Path dir = folderWith("navfile", "a.txt", "target.txt", "z.txt");
        FileTab tab = leftTab();
        robot.interact(() -> tab.navigateTo(dir.resolve("target.txt")));
        waitUntil("folder shown", () -> dir.equals(tab.currentDir()));
        assertEquals(List.of("target.txt"), selectedNames(tab));
    }

    @Test
    void cancelAllAndWaitStopsEveryRunningOperationAndItsCleanup(FxRobot robot) throws Exception {
        var release = new CountDownLatch(1);
        try {
            var first = blockedCopy("quit1", release, true);
            var second = blockedCopy("quit2", release, true);
            waitUntil("both registered", () -> OperationUi.running().containsAll(List.of(first, second)));
            assertTrue(OperationUi.cancelAllAndWait(java.time.Duration.ofSeconds(10)));
            assertTrue(first.awaitStopped(0, TimeUnit.SECONDS) && second.awaitStopped(0, TimeUnit.SECONDS));
            waitUntil("registry empty and window closed", () -> OperationUi.running().isEmpty()
                    && ProgressWindow.of(stage) == null);
            assertFalse(Files.exists(tmp.resolve("quit1-dst/quit1-2.txt")), "the second file was never copied");
        } finally {
            release.countDown();
        }
    }

    // ---- view modes ----

    private FileGrid grid(FileTab tab) {
        return fx(tab::grid);
    }

    private void setView(FxRobot robot, FileTab tab, ViewMode m) {
        robot.interact(() -> tab.setViewMode(m));
        WaitForAsyncUtils.waitForFxEvents();
    }

    // ---- item size (+ / -) ----

    private static boolean shown(javafx.scene.control.Button b) {
        return fx(() -> b.isVisible() && b.isManaged());
    }

    @Test
    void sizeButtonsShowOnlyInIconViewsAndTiles(FxRobot robot) throws Exception {
        Path dir = folderWith("sizebtn", "a.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        for (ViewMode m : ViewMode.values()) {
            setView(robot, tab, m);
            boolean expected = m == ViewMode.EXTRA_LARGE_ICONS || m == ViewMode.LARGE_ICONS
                    || m == ViewMode.MEDIUM_ICONS || m == ViewMode.SMALL_ICONS || m == ViewMode.TILES;
            assertEquals(expected, shown(tab.biggerButton()), m + " +");
            assertEquals(expected, shown(tab.smallerButton()), m + " -");
        }
    }

    @Test
    void plusAndMinusResizeTheItemsWithinLimits(FxRobot robot) throws Exception {
        Path dir = folderWith("sizes", "a.txt", "b.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        setView(robot, tab, ViewMode.LARGE_ICONS);
        FileGrid g = grid(tab);
        double width = fx(() -> g.cellFor(0).getWidth());
        assertEquals(96, (int) fx(g::iconPx));

        robot.clickOn(tab.biggerButton());
        waitUntil("bigger", () -> g.iconPx() == 120 && g.cellFor(0).getWidth() > width);
        robot.clickOn(tab.smallerButton());
        robot.clickOn(tab.smallerButton());
        waitUntil("smaller", () -> g.iconPx() == 77 && g.cellFor(0).getWidth() < width);

        // limits: Small icons can't shrink below 16 px; Extra large stops before 320 px
        setView(robot, tab, ViewMode.SMALL_ICONS);
        assertTrue(fx(() -> tab.smallerButton().isDisabled()));
        setView(robot, tab, ViewMode.EXTRA_LARGE_ICONS);
        robot.clickOn(tab.biggerButton());
        robot.clickOn(tab.biggerButton());
        waitUntil("largest", () -> g.iconPx() == 300 && tab.biggerButton().isDisabled());
    }

    @Test
    void eachViewKeepsItsOwnSizeAndTheShortcutsWork(FxRobot robot) throws Exception {
        Path dir = folderWith("sizeview", "a.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        setView(robot, tab, ViewMode.TILES);
        robot.push(KeyCode.SHORTCUT, KeyCode.EQUALS);
        waitUntil("tiles bigger", () -> grid(tab).iconPx() == 60);
        setView(robot, tab, ViewMode.MEDIUM_ICONS);
        assertEquals(48, (int) fx(() -> grid(tab).iconPx()), "another view keeps its own size");
        robot.push(KeyCode.SHORTCUT, KeyCode.MINUS);
        waitUntil("medium smaller", () -> grid(tab).iconPx() == 38);
        setView(robot, tab, ViewMode.TILES);
        assertEquals(60, (int) fx(() -> grid(tab).iconPx()), "switching back restores the size");

        setView(robot, tab, ViewMode.DETAILS);
        robot.push(KeyCode.SHORTCUT, KeyCode.EQUALS); // no effect in Details
        setView(robot, tab, ViewMode.TILES);
        assertEquals(60, (int) fx(() -> grid(tab).iconPx()));

        robot.interact(window::saveSession);
        MainWindow restored = fx(() -> new MainWindow(new Stage(), session));
        FileTab again = fx(() -> restored.left().activeTab());
        assertEquals(ViewMode.TILES, fx(again::viewMode));
        assertEquals(60, (int) fx(() -> again.grid().iconPx()), "the size is saved in the session");
        robot.interact(() -> again.setViewMode(ViewMode.MEDIUM_ICONS));
        assertEquals(38, (int) fx(() -> again.grid().iconPx()));
    }

    private List<String> gridNames(FileTab tab) {
        FileGrid g = grid(tab);
        return fx(() -> g.visibleIndices().stream().map(i -> g.cellFor(i).nameText()).toList());
    }

    private javafx.geometry.Point2D itemPoint(FileTab tab, String name) {
        FileGrid g = grid(tab);
        int i = fx(() -> {
            List<FileEntry> items = tab.table().getItems();
            for (int k = 0; k < items.size(); k++) {
                if (items.get(k).name().equals(name)) {
                    return k;
                }
            }
            throw new AssertionError("no item " + name);
        });
        return fx(() -> g.screenCenterOf(i));
    }

    @Test
    void theViewMenuHasExplorersModesAndSwitchesTheFileView(FxRobot robot) throws Exception {
        Path dir = folderWith("views", "sub/", "a.txt", "b.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "a.txt");
        List<String> ids = fx(() -> window.viewMenu().getItems().stream().map(javafx.scene.control.MenuItem::getId).toList());
        assertEquals(List.of("view-xl-icons", "view-large-icons", "view-medium-icons", "view-small-icons",
                "view-list", "view-details", "view-tiles", "view-content"), ids);
        for (ViewMode m : ViewMode.values()) {
            robot.interact(() -> {
                window.viewMenu().fire(); // opens it, which rebuilds the items for the active tab
                window.viewMenu().getItems().stream().filter(i -> ("view-" + m.id()).equals(i.getId()))
                        .findFirst().orElseThrow().fire();
                window.viewMenu().hide();
            });
            WaitForAsyncUtils.waitForFxEvents();
            assertEquals(m, fx(tab::viewMode));
            if (m == ViewMode.DETAILS) {
                assertTrue(fx(() -> tab.getCenter() == tab.table()));
            } else {
                assertTrue(fx(() -> tab.getCenter() == tab.grid()));
                assertEquals(List.of("sub", "a.txt", "b.txt"), gridNames(tab), m + " shows the items in order");
            }
            assertEquals(List.of("a.txt"), selectedNames(tab), "the selection carries over to " + m);
        }
    }

    @Test
    void ctrlShiftDigitsAndCtrlWheelSwitchTheView(FxRobot robot) throws Exception {
        FileTab tab = leftTab();
        open(robot, tab, folderWith("keys-view", "a.txt"));
        robot.push(KeyCode.SHORTCUT, KeyCode.SHIFT, KeyCode.DIGIT1);
        waitUntil("extra large icons", () -> tab.viewMode() == ViewMode.EXTRA_LARGE_ICONS);
        robot.push(KeyCode.SHORTCUT, KeyCode.SHIFT, KeyCode.DIGIT7);
        waitUntil("tiles", () -> tab.viewMode() == ViewMode.TILES);
        robot.push(KeyCode.SHORTCUT, KeyCode.SHIFT, KeyCode.DIGIT6);
        waitUntil("details", () -> tab.viewMode() == ViewMode.DETAILS);

        // Ctrl+wheel up: bigger (toward Extra large icons), down: toward Content
        robot.interact(() -> javafx.event.Event.fireEvent(tab.view(), scroll(tab.view(), 40)));
        assertEquals(ViewMode.LIST, fx(tab::viewMode));
        robot.interact(() -> javafx.event.Event.fireEvent(tab.view(), scroll(tab.view(), -40)));
        assertEquals(ViewMode.DETAILS, fx(tab::viewMode));
    }

    private static javafx.scene.input.ScrollEvent scroll(Node target, double deltaY) {
        boolean mac = OsUtil.isMac();
        return new javafx.scene.input.ScrollEvent(javafx.scene.input.ScrollEvent.SCROLL, 10, 10, 10, 10,
                false, !mac, false, mac, false, false, 0, deltaY, 0, deltaY,
                javafx.scene.input.ScrollEvent.HorizontalTextScrollUnits.NONE, 0,
                javafx.scene.input.ScrollEvent.VerticalTextScrollUnits.NONE, 0, 0, null);
    }

    @Test
    void clickingInTheGridSelectsLikeExplorerAndDoubleClickOpens(FxRobot robot) throws Exception {
        Path dir = folderWith("gridclick", "sub/", "a.txt", "b.txt", "c.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        setView(robot, tab, ViewMode.LARGE_ICONS);
        robot.clickOn(itemPoint(tab, "a.txt"));
        assertEquals(List.of("a.txt"), selectedNames(tab));
        robot.press(KeyCode.CONTROL).clickOn(itemPoint(tab, "c.txt")).release(KeyCode.CONTROL);
        assertEquals(List.of("a.txt", "c.txt"), selectedNames(tab));
        robot.press(KeyCode.SHIFT).clickOn(itemPoint(tab, "b.txt")).release(KeyCode.SHIFT);
        assertEquals(List.of("b.txt", "c.txt"), selectedNames(tab), "shift selects from the last clicked item");
        assertTrue(fx(() -> grid(tab).isFocused()));
        robot.doubleClickOn(itemPoint(tab, "sub"));
        waitUntil("opened sub", () -> dir.resolve("sub").equals(tab.currentDir()));
    }

    @Test
    void arrowKeysFollowTheGridLayout(FxRobot robot) throws Exception {
        String[] files = new String[40];
        for (int i = 0; i < files.length; i++) {
            files[i] = String.format("f%02d.txt", i);
        }
        FileTab tab = leftTab();
        open(robot, tab, folderWith("gridkeys", files));
        setView(robot, tab, ViewMode.MEDIUM_ICONS);
        robot.interact(tab::focusTable);
        select(robot, tab, "f00.txt");
        robot.interact(() -> tab.grid().requestFocus());
        int lanes = fx(() -> grid(tab).lanes());
        assertTrue(lanes > 1, "several icons per row");
        robot.type(KeyCode.RIGHT);
        assertEquals(List.of("f01.txt"), selectedNames(tab));
        robot.type(KeyCode.DOWN);
        assertEquals(List.of(files[1 + lanes]), selectedNames(tab), "Down moves one row");
        robot.press(KeyCode.SHIFT).type(KeyCode.LEFT).release(KeyCode.SHIFT);
        assertEquals(List.of(files[lanes], files[1 + lanes]), selectedNames(tab), "Shift extends");
        robot.type(KeyCode.END);
        assertEquals(List.of("f39.txt"), selectedNames(tab));

        setView(robot, tab, ViewMode.LIST);
        robot.interact(() -> tab.grid().requestFocus());
        select(robot, tab, "f00.txt");
        int rows = fx(() -> grid(tab).lanes());
        robot.type(KeyCode.DOWN);
        assertEquals(List.of("f01.txt"), selectedNames(tab), "List runs down the column");
        robot.type(KeyCode.RIGHT);
        assertEquals(List.of(files[Math.min(39, 1 + rows)]), selectedNames(tab), "Right moves one column");
    }

    @Test
    void aBoxDraggedInTheGridSelectsTheItemsItTouches(FxRobot robot) throws Exception {
        Path dir = folderWith("gridband", "a.txt", "b.txt", "c.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        setView(robot, tab, ViewMode.LARGE_ICONS);
        javafx.geometry.Point2D c = itemPoint(tab, "c.txt");
        javafx.geometry.Point2D b = itemPoint(tab, "b.txt");
        // start in the empty space right of the last icon, drag left into b
        robot.moveTo(c.getX() + 300, c.getY());
        robot.press(javafx.scene.input.MouseButton.PRIMARY);
        robot.moveTo(c.getX() + 100, c.getY() + 5);
        robot.moveTo(b);
        assertTrue(fx(() -> grid(tab).lookup(".marquee").isVisible()), "the box shows while dragging");
        robot.release(javafx.scene.input.MouseButton.PRIMARY);
        assertEquals(List.of("b.txt", "c.txt"), selectedNames(tab));
        assertFalse(fx(() -> grid(tab).lookup(".marquee").isVisible()));
        robot.clickOn(c.getX() + 300, c.getY());
        assertEquals(List.of(), selectedNames(tab), "a click on empty space clears the selection");
    }

    @Test
    void openingAFileInTestsIsRecordedInsteadOfLaunchingARealProgram(FxRobot robot) throws Exception {
        Path dir = folderWith("openrec", "note.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        select(robot, tab, "note.txt");
        Shell.clearLaunched();
        robot.interact(window::openSelection);
        assertEquals(List.of("open " + dir.resolve("note.txt")), Shell.launched());
    }

    @Test
    void dragAFileOntoAFolderInTheGrid(FxRobot robot) throws Exception {
        Path dir = folderWith("griddnd", "into/", "drag.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        setView(robot, tab, ViewMode.TILES);
        Shell.clearLaunched();
        // one press only: a click right before the drag's press would be a double-click, which opens the file
        robot.drag(itemPoint(tab, "drag.txt"), javafx.scene.input.MouseButton.PRIMARY).dropTo(itemPoint(tab, "into"));
        answerDrop(robot, "Move");
        waitUntil("moved into folder", () -> Files.exists(dir.resolve("into/drag.txt"))
                && !Files.exists(dir.resolve("drag.txt")));
        assertEquals(List.of(), Shell.launched(), "dragging never opens the file");
    }

    @Test
    void gridContextMenusTargetTheItemOrTheFolder(FxRobot robot) throws Exception {
        Path dir = folderWith("gridmenu", "a.txt", "b.txt");
        FileTab tab = leftTab();
        open(robot, tab, dir);
        setView(robot, tab, ViewMode.CONTENT);
        robot.rightClickOn(itemPoint(tab, "b.txt"));
        assertEquals(List.of("b.txt"), selectedNames(tab), "right-click selects the item");
        assertTrue(hasItem(fx(() -> window.fileMenuItems(tab)), "Open"));
        robot.interact(() -> tab.table().getSelectionModel().clearSelection());
        FileGrid g = grid(tab);
        requestContextMenu(robot, g);
        waitUntil("menu shown", () -> g.getContextMenu().isShowing());
        assertTrue(hasItem(fx(() -> g.getContextMenu().getItems()), "View"), "the folder menu offers the views");
        robot.interact(() -> g.getContextMenu().hide());
    }

    @Test
    void typeAheadWorksInTheGrid(FxRobot robot) throws Exception {
        FileTab tab = leftTab();
        open(robot, tab, folderWith("gridtype", "apple.txt", "notes.txt", "zebra.txt"));
        setView(robot, tab, ViewMode.SMALL_ICONS);
        robot.interact(tab::focusTable);
        robot.write("no");
        assertEquals(List.of("notes.txt"), selectedNames(tab));
    }

    @Test
    void imagesShowAsThumbnailsInTheBigIconViews(FxRobot robot) throws Exception {
        Path dir = folderWith("thumbs", "plain.txt");
        java.awt.image.BufferedImage img = new java.awt.image.BufferedImage(64, 40, java.awt.image.BufferedImage.TYPE_INT_RGB);
        javax.imageio.ImageIO.write(img, "png", dir.resolve("pic.png").toFile());
        FileTab tab = leftTab();
        open(robot, tab, dir);
        setView(robot, tab, ViewMode.LARGE_ICONS);
        FileGrid g = grid(tab);
        waitUntil("thumbnail loaded", () -> g.cellFor(0).nameText().equals("pic.png") && g.cellFor(0).showsThumbnail());
        assertFalse(fx(() -> g.cellFor(1).showsThumbnail()), "other files keep their type icon");
        setView(robot, tab, ViewMode.SMALL_ICONS);
        assertFalse(fx(() -> grid(tab).cellFor(0).showsThumbnail()), "small views use icons");
    }

    @Test
    void viewModesAreSavedPerTabAndNewTabsUseTheCurrentOne(FxRobot robot) throws Exception {
        Path a = folderWith("va");
        FileTab first = leftTab();
        open(robot, first, a);
        setView(robot, first, ViewMode.TILES);
        robot.push(KeyCode.SHORTCUT, KeyCode.T);
        waitUntil("second tab", () -> window.left().tabs().size() == 2);
        FileTab second = fx(() -> window.left().tabs().get(1));
        assertEquals(ViewMode.TILES, fx(second::viewMode), "new tab inherits the view");
        setView(robot, second, ViewMode.LIST);
        robot.interact(window::saveSession);

        MainWindow restored = fx(() -> new MainWindow(new Stage(), session));
        waitUntil("tabs restored", () -> restored.left().tabs().size() == 2);
        assertEquals(List.of(ViewMode.TILES, ViewMode.LIST),
                fx(() -> restored.left().tabs().stream().map(FileTab::viewMode).toList()));
    }
}
