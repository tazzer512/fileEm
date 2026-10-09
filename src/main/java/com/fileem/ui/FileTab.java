package com.fileem.ui;

import atlantafx.base.controls.CustomTextField;
import com.fileem.fs.DirWatcher;
import com.fileem.fs.FileEntry;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import com.fileem.util.OsUtil;
import javafx.animation.PauseTransition;
import javafx.beans.InvalidationListener;
import javafx.beans.binding.Bindings;
import javafx.collections.FXCollections;
import javafx.collections.ListChangeListener;
import javafx.collections.ObservableList;
import javafx.collections.transformation.FilteredList;
import javafx.collections.transformation.SortedList;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.control.Label;
import javafx.scene.control.Tab;
import javafx.scene.control.Tooltip;
import javafx.scene.input.KeyCode;
import javafx.scene.input.KeyEvent;
import javafx.scene.layout.BorderPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.util.Duration;

import java.io.IOException;
import java.nio.file.AccessDeniedException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.NoSuchFileException;
import java.nio.file.NotDirectoryException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashSet;
import java.util.List;
import java.util.Objects;
import java.util.Set;

/** One tab of a side: address bar, filter, file list and status line for a single folder. */
public final class FileTab extends BorderPane {
    private static final int MAX_HISTORY = 100;

    private final MainWindow main;
    private final ExplorerSide side;
    private final Tab tab = new Tab();
    private final ObservableList<FileEntry> master = FXCollections.observableArrayList();
    private final FilteredList<FileEntry> filtered = new FilteredList<>(master);
    private final SortedList<FileEntry> sorted = new SortedList<>(filtered);
    private final FileTable table;
    private final AddressBar address;
    private final CustomTextField filter = new CustomTextField();
    private final Label status = new Label();
    private final InvalidationListener hiddenListener = o -> updatePredicate();
    private final List<Path> history = new ArrayList<>();

    private Path currentDir;
    private int historyIndex = -1;
    private int loadGeneration;
    private boolean refreshing;
    private boolean refreshPending;
    private volatile long lastListMillis;
    private long freeSpace = -1;
    private AutoCloseable watch;

    FileTab(MainWindow main, ExplorerSide side, Path initial) {
        this.main = main;
        this.side = side;
        getStyleClass().add("file-tab");

        table = new FileTable(main, this);
        sorted.comparatorProperty().bind(Bindings.createObjectBinding(table::effectiveComparator, table.comparatorProperty()));
        table.setSortPolicy(t -> true);
        table.setItems(sorted);
        table.setOnDragOver(e -> {
            if (Dnd.accept(e, currentDir)) {
                e.consume();
            }
        });
        table.setOnDragDropped(e -> {
            if (Dnd.canDrop(e, currentDir)) {
                main.drop(e, currentDir);
            }
        });

        address = new AddressBar(this::navigateTo, this::focusTable);

        filter.setPromptText("Filter (*, ?)");
        filter.setLeft(Icons.icon("mdi2m-magnify"));
        filter.setPrefColumnCount(12);
        filter.getStyleClass().add("filter-field");
        filter.textProperty().addListener(o -> updatePredicate());
        filter.addEventHandler(KeyEvent.KEY_PRESSED, e -> {
            if (e.getCode() == KeyCode.ESCAPE) {
                filter.clear();
                focusTable();
                e.consume();
            } else if (e.getCode() == KeyCode.ENTER || e.getCode() == KeyCode.DOWN) {
                focusTable();
                e.consume();
            }
        });

        HBox top = new HBox(6, address, filter);
        top.setAlignment(Pos.CENTER_LEFT);
        top.setPadding(new Insets(6, 6, 6, 6));
        HBox.setHgrow(address, Priority.ALWAYS);

        status.getStyleClass().add("status-label");
        status.setMaxWidth(Double.MAX_VALUE);

        setTop(top);
        setCenter(table);
        setBottom(status);

        tab.setContent(this);
        tab.setUserData(this);
        tab.setGraphic(Icons.icon("mdi2f-folder", 14));
        tab.getGraphic().getStyleClass().add("icon-folder");

        main.showHiddenProperty().addListener(hiddenListener);
        filtered.addListener((ListChangeListener<FileEntry>) c -> updateStatus());
        table.getSelectionModel().getSelectedItems().addListener((ListChangeListener<FileEntry>) c -> updateStatus());
        updatePredicate();

        go(initial, null, true);
    }

    Tab tab() {
        return tab;
    }

    ExplorerSide side() {
        return side;
    }

    public Path currentDir() {
        return currentDir;
    }

    FileTable table() {
        return table;
    }

    boolean canGoBack() {
        return historyIndex > 0;
    }

    boolean canGoForward() {
        return historyIndex >= 0 && historyIndex < history.size() - 1;
    }

    boolean canGoUp() {
        return currentDir != null && currentDir.getParent() != null;
    }

    // ---- navigation ----

    /** Navigates to a folder; a file path opens its folder with the file selected. */
    public void navigateTo(Path p) {
        if (p == null) {
            return;
        }
        if (Files.isRegularFile(p) && p.getParent() != null) {
            go(p.getParent(), p, true);
        } else {
            go(p, null, true);
        }
    }

    void back() {
        if (canGoBack()) {
            go(history.get(historyIndex - 1), null, false, -1);
        }
    }

    void forward() {
        if (canGoForward()) {
            go(history.get(historyIndex + 1), null, false, +1);
        }
    }

    void up() {
        if (canGoUp()) {
            go(currentDir.getParent(), currentDir, true);
        }
    }

    void refresh() {
        refresh(null);
    }

    /**
     * Refresh requested by the directory watcher. Coalesced: while a listing runs, further requests
     * collapse into one follow-up, delayed relative to how long listing takes (big folders under
     * heavy writes would otherwise be re-listed continuously).
     */
    void requestRefresh() {
        if (refreshing) {
            refreshPending = true;
            return;
        }
        refresh();
    }

    /** Reloads the listing, keeping the selection (or selecting {@code select} when given). */
    void refresh(Path select) {
        if (currentDir == null) {
            return;
        }
        Path dir = currentDir;
        List<Path> keep = select != null ? List.of(select) : selectedPaths();
        int gen = ++loadGeneration;
        refreshing = true;
        Bg.run(() -> timedList(dir), entries -> {
            finishRefresh();
            if (gen != loadGeneration) {
                return;
            }
            boolean foldersChanged = !folderPaths(master).equals(folderPaths(entries));
            master.setAll(entries);
            restoreSelection(keep, select != null);
            updateStatus();
            if (foldersChanged) {
                main.treesRefresh(dir);
            }
        }, err -> {
            finishRefresh();
            if (gen != loadGeneration) {
                return;
            }
            Path existing = dir.getParent();
            while (existing != null && !Files.isDirectory(existing)) {
                existing = existing.getParent();
            }
            if (existing != null) {
                go(existing, null, true);
            }
        });
    }

    private void finishRefresh() {
        refreshing = false;
        if (refreshPending) {
            refreshPending = false;
            PauseTransition delay = new PauseTransition(Duration.millis(Math.max(250, 2 * lastListMillis)));
            delay.setOnFinished(e -> requestRefresh());
            delay.play();
        }
    }

    private static Set<Path> folderPaths(List<FileEntry> entries) {
        Set<Path> out = new HashSet<>();
        for (FileEntry e : entries) {
            if (e.isDirectory()) {
                out.add(e.path());
            }
        }
        return out;
    }

    private void go(Path dir, Path select, boolean record) {
        go(dir, select, record, 0);
    }

    private void go(Path dir, Path select, boolean record, int historyStep) {
        Path target = dir.toAbsolutePath().normalize();
        Path previous = currentDir;
        Path selectAfter = select != null ? select
                : previous != null && target.equals(previous.getParent()) ? previous : null;
        int gen = ++loadGeneration;
        status.setText("Loading…");
        Bg.run(() -> timedList(target), entries -> {
            if (gen != loadGeneration) {
                return;
            }
            boolean changed = !target.equals(previous);
            currentDir = target;
            if (record) {
                pushHistory(target);
            } else {
                historyIndex += historyStep;
            }
            if (changed) {
                filter.clear();
                onDirectoryChanged();
            }
            master.setAll(entries);
            if (selectAfter != null) {
                restoreSelection(List.of(selectAfter), true);
            } else {
                table.getSelectionModel().clearSelection();
                if (!sorted.isEmpty()) {
                    table.getFocusModel().focus(0);
                    table.scrollTo(0);
                }
            }
            updateStatus();
            side.tabNavigated(this);
        }, err -> {
            if (gen != loadGeneration) {
                return;
            }
            updateStatus();
            if (previous == null) {
                Path home = OsUtil.home();
                if (!target.equals(home)) {
                    go(home, null, true);
                }
                return;
            }
            Dialogs.error(getScene() != null ? getScene().getWindow() : null, "Open folder", describe(target, err));
        });
    }

    private static String describe(Path dir, Exception e) {
        if (e instanceof AccessDeniedException) {
            return "Access denied:\n" + dir;
        }
        if (e instanceof NoSuchFileException) {
            return "The folder does not exist:\n" + dir;
        }
        if (e instanceof NotDirectoryException) {
            return "Not a folder:\n" + dir;
        }
        return "Cannot open " + dir + ":\n" + e.getMessage();
    }

    private List<FileEntry> timedList(Path dir) throws IOException {
        long start = System.nanoTime();
        List<FileEntry> entries = list(dir);
        lastListMillis = (System.nanoTime() - start) / 1_000_000;
        return entries;
    }

    private static List<FileEntry> list(Path dir) throws IOException {
        List<FileEntry> out = new ArrayList<>();
        try (DirectoryStream<Path> ds = Files.newDirectoryStream(dir)) {
            for (Path p : ds) {
                out.add(FileEntry.read(p));
            }
        }
        return out;
    }

    private void pushHistory(Path dir) {
        if (historyIndex >= 0 && history.get(historyIndex).equals(dir)) {
            return;
        }
        while (history.size() > historyIndex + 1) {
            history.remove(history.size() - 1);
        }
        history.add(dir);
        if (history.size() > MAX_HISTORY) {
            history.remove(0);
        }
        historyIndex = history.size() - 1;
    }

    private void onDirectoryChanged() {
        address.setPath(currentDir);
        String name = FsUtil.displayName(currentDir);
        tab.setText(name);
        tab.setTooltip(new Tooltip(currentDir.toString()));
        closeWatch();
        Path watched = currentDir;
        watch = DirWatcher.watch(watched, () -> {
            if (watched.equals(currentDir)) {
                requestRefresh();
            }
        });
        freeSpace = -1;
        Bg.run(() -> Files.getFileStore(watched).getUsableSpace(), free -> {
            if (watched.equals(currentDir)) {
                freeSpace = free;
                updateStatus();
            }
        }, null);
    }

    private void closeWatch() {
        if (watch != null) {
            try {
                watch.close();
            } catch (Exception ignored) {
                // nothing to do
            }
            watch = null;
        }
    }

    void dispose() {
        loadGeneration++;
        closeWatch();
        main.showHiddenProperty().removeListener(hiddenListener);
    }

    // ---- selection & focus ----

    public List<Path> selectedPaths() {
        return table.getSelectionModel().getSelectedItems().stream()
                .filter(Objects::nonNull).map(FileEntry::path).toList();
    }

    List<FileEntry> selectedEntries() {
        return table.getSelectionModel().getSelectedItems().stream().filter(Objects::nonNull).toList();
    }

    private void restoreSelection(Collection<Path> paths, boolean scroll) {
        Set<Path> wanted = new HashSet<>(paths);
        var sm = table.getSelectionModel();
        sm.clearSelection();
        int first = -1;
        for (int i = 0; i < sorted.size(); i++) {
            if (wanted.contains(sorted.get(i).path())) {
                sm.select(i);
                if (first < 0) {
                    first = i;
                }
            }
        }
        if (first >= 0) {
            table.getFocusModel().focus(first);
            if (scroll) {
                table.ensureVisible(first);
            }
        }
    }

    void selectAll() {
        table.getSelectionModel().selectAll();
    }

    void focusTable() {
        table.requestFocus();
        if (table.getFocusModel().getFocusedIndex() < 0 && !sorted.isEmpty()) {
            table.getFocusModel().focus(0);
        }
    }

    void editAddress() {
        address.edit();
    }

    void focusFilter() {
        filter.requestFocus();
        filter.selectAll();
    }

    // ---- filter & status ----

    private void updatePredicate() {
        boolean showHidden = main.showHiddenProperty().get();
        var matcher = FsUtil.nameMatcher(filter.getText());
        filtered.setPredicate(e -> (showHidden || !e.isHidden()) && matcher.test(e.name()));
    }

    private void updateStatus() {
        int n = filtered.size();
        StringBuilder sb = new StringBuilder(Formats.count(n)).append(n == 1 ? " item" : " items");
        int hiddenCount = master.size() - n;
        if (hiddenCount > 0) {
            sb.append(" (").append(Formats.count(hiddenCount)).append(filter.getText().isBlank() ? " hidden)" : " filtered out)");
        }
        List<FileEntry> sel = selectedEntries();
        if (!sel.isEmpty()) {
            long bytes = sel.stream().filter(e -> !e.isDirectory()).mapToLong(FileEntry::size).sum();
            sb.append("     ").append(Formats.count(sel.size())).append(" selected");
            if (bytes > 0) {
                sb.append("  ·  ").append(Formats.size(bytes));
            }
        }
        if (freeSpace >= 0) {
            sb.append("     ").append(Formats.size(freeSpace)).append(" free");
        }
        status.setText(sb.toString());
    }
}
