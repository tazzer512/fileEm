package com.fileem.ui;

import com.fileem.ops.FileOperation;
import com.fileem.ops.FileOperation.Decision;
import com.fileem.ops.FileOperation.Kind;
import com.fileem.ops.FileOperation.Resolution;
import com.fileem.util.Bg;
import com.fileem.util.Formats;
import com.fileem.util.FsUtil;
import javafx.animation.PauseTransition;
import javafx.application.Platform;
import javafx.concurrent.Worker;
import javafx.stage.Window;
import javafx.util.Duration;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;

/** Runs a {@link FileOperation} with a row in the shared {@link ProgressWindow} and conflict prompts. */
public final class OperationUi {
    /** Operations started and not yet finished, so quitting can stop them cleanly. */
    private static final Set<FileOperation> RUNNING = ConcurrentHashMap.newKeySet();

    private OperationUi() {
    }

    /** Operations that are running, paused or waiting for an answer. */
    public static List<FileOperation> running() {
        return List.copyOf(RUNNING);
    }

    /**
     * Cancels every running operation and waits up to {@code timeout} for all of them to stop,
     * including their cleanup (temp files removed, displaced items restored). Call off the FX thread.
     * Returns whether all stopped in time.
     */
    public static boolean cancelAllAndWait(java.time.Duration timeout) {
        List<FileOperation> ops = running();
        ops.forEach(FileOperation::cancel);
        long deadline = System.nanoTime() + timeout.toNanos();
        try {
            for (FileOperation op : ops) {
                long left = deadline - System.nanoTime();
                if (left <= 0 || !op.awaitStopped(left, TimeUnit.NANOSECONDS)) {
                    return false;
                }
            }
            return true;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    /** Starts the operation; {@code onDone} gets the folders whose contents changed. */
    static FileOperation run(Window owner, Kind kind, List<Path> sources, Path target, Consumer<Set<Path>> onDone) {
        FileOperation[] self = new FileOperation[1];
        FileOperation op = new FileOperation(kind, sources, target, (s, t) -> askInProgressRow(owner, self[0], s, t));
        self[0] = op;
        start(owner, op, sources, target, onDone);
        return op;
    }

    /** Starts an already built operation (tests pass one with their own conflict handler). */
    static void start(Window owner, FileOperation op, List<Path> sources, Path target, Consumer<Set<Path>> onDone) {
        String title = title(op.kind(), sources, target);
        op.stateProperty().addListener((obs, old, state) -> {
            if (state == Worker.State.SUCCEEDED || state == Worker.State.FAILED || state == Worker.State.CANCELLED) {
                RUNNING.remove(op);
                ProgressWindow.finished(owner, op);
                Set<Path> affected = new HashSet<>();
                if (target != null) {
                    affected.add(target);
                }
                for (Path s : sources) {
                    if (s.getParent() != null) {
                        affected.add(s.getParent());
                    }
                }
                onDone.accept(affected);
                List<String> errors = op.errors();
                if (state == Worker.State.FAILED && op.getException() != null) {
                    Dialogs.error(owner, title, String.valueOf(op.getException().getMessage()));
                } else if (!errors.isEmpty()) {
                    Dialogs.errorList(owner, title,
                            errors.size() == 1 ? "1 item could not be processed." : errors.size() + " items could not be processed.",
                            errors);
                }
            }
        });
        RUNNING.add(op);
        Bg.OPERATIONS.execute(op);
        // Quick operations never show up; slower ones join the owner's progress window.
        PauseTransition delay = new PauseTransition(Duration.millis(350));
        delay.setOnFinished(e -> {
            if (!op.isDone()) {
                ProgressWindow.show(owner, op, title);
            }
        });
        delay.play();
    }

    private static String title(Kind kind, List<Path> sources, Path target) {
        String what = sources.size() == 1 ? "\"" + FsUtil.displayName(sources.get(0)) + "\"" : sources.size() + " items";
        return switch (kind) {
            case COPY -> "Copying " + what + " to " + target;
            case MOVE -> "Moving " + what + " to " + target;
            case DELETE -> "Deleting " + what;
            case TRASH -> "Moving " + what + " to Trash";
            case COMPRESS -> "Compressing " + what;
            case EXTRACT -> "Extracting " + what;
        };
    }

    /** What a conflict prompt shows; built on the operation's thread (it reads file attributes). */
    record Conflict(String question, String source, String existing) {
    }

    /**
     * Asks how to resolve a name conflict, from the operation's own thread. The question appears in
     * the operation's row of the progress window: not a modal dialog, so the rest of the app and other
     * operations stay usable, and several operations can each wait for an answer independently
     * (nested modal dialogs could only be answered last-opened-first). Cancelling the operation
     * interrupts the wait.
     */
    static Decision askInProgressRow(Window owner, FileOperation op, Path source, Path target)
            throws InterruptedException {
        boolean dir = Files.isDirectory(target, LinkOption.NOFOLLOW_LINKS);
        Conflict c = new Conflict("The destination already has " + (dir ? "a folder" : "a file") + " named \""
                + FsUtil.displayName(target) + "\".", describe(source), describe(target));
        String title = title(op.kind(), op.sources(), op.targetDir());
        CompletableFuture<Decision> answer = new CompletableFuture<>();
        Platform.runLater(() -> ProgressWindow.ask(owner, op, title, c, answer));
        try {
            return answer.get();
        } catch (ExecutionException e) {
            return new Decision(Resolution.CANCEL, false);
        }
    }

    private static String describe(Path p) {
        try {
            BasicFileAttributes a = Files.readAttributes(p, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
            String size = a.isDirectory() ? "Folder" : Formats.size(a.size());
            return size + ", modified " + Formats.date(a.lastModifiedTime()) + "\n" + p;
        } catch (IOException e) {
            return p.toString();
        }
    }
}
