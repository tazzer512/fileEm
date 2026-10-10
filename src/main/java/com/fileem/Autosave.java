package com.fileem;

import javafx.animation.KeyFrame;
import javafx.animation.Timeline;
import javafx.application.Platform;
import javafx.util.Duration;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.FutureTask;
import java.util.concurrent.TimeUnit;

/**
 * Keeps the session file up to date while the app runs, so settings survive a crash, a kill or a
 * logout, not only a normal close. Every few seconds it collects the current state (on the FX thread)
 * and writes it in the background if anything changed. A JVM shutdown hook does a last save.
 */
public final class Autosave {
    static final Duration INTERVAL = Duration.seconds(2);

    private final Session session;
    private final Runnable collect;
    private final Timeline timeline;
    private Thread hook;

    /** {@code collect} copies the app state into the session; it runs on the FX thread. */
    public Autosave(Session session, Runnable collect) {
        this.session = session;
        this.collect = collect;
        timeline = new Timeline(new KeyFrame(INTERVAL, e -> saveInBackground()));
        timeline.setCycleCount(Timeline.INDEFINITE);
    }

    public void start() {
        timeline.play();
        hook = new Thread(this::saveOnShutdown, "fileem-session-shutdown");
        Runtime.getRuntime().addShutdownHook(hook);
    }

    public void stop() {
        timeline.stop();
        if (hook != null) {
            try {
                Runtime.getRuntime().removeShutdownHook(hook);
            } catch (IllegalStateException e) {
                // already shutting down
            }
            hook = null;
        }
    }

    /** Collects the state and writes it in the background if it changed; returns the write, or null. */
    public CompletableFuture<Void> saveInBackground() {
        collect.run();
        return session.saveInBackground();
    }

    /** Collects the state and writes it now (on close). */
    public void saveNow() {
        collect.run();
        session.save();
    }

    /** Runs on the shutdown hook thread: collects on the FX thread if it still answers, then writes. */
    private void saveOnShutdown() {
        FutureTask<Void> task = new FutureTask<>(collect, null);
        try {
            Platform.runLater(task);
            task.get(1, TimeUnit.SECONDS);
        } catch (Exception e) {
            // FX is gone or stuck: write what the last collection gathered
        }
        session.save();
    }
}
