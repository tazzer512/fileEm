package com.fileem.util;

import javafx.application.Platform;

import java.util.concurrent.Callable;
import java.util.concurrent.CancellationException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Consumer;

/**
 * Background work with results delivered on the FX thread. Two executors, by kind of work:
 * <ul>
 *   <li>{@link #IO}: one <b>virtual thread</b> per task, for the many short blocking jobs (folder
 *   listings, tree loads, free space, launching programs). Creating one costs almost nothing, so a
 *   slow network folder never makes other folders wait, and nothing needs sizing.</li>
 *   <li>{@link #OPERATIONS}: one <b>platform thread</b> per copy/move/delete. These are few and run
 *   for minutes in blocking file I/O, which in JDK 21 pins a virtual thread to its carrier anyway,
 *   so a dedicated, named thread is the honest choice (and shows up clearly in a thread dump).</li>
 * </ul>
 * Every operation and listing gets its own thread: starting one never stops or delays another.
 */
public final class Bg {
    private static final AtomicInteger OPS = new AtomicInteger();

    public static final ExecutorService IO = Executors.newThreadPerTaskExecutor(
            Thread.ofVirtual().name("fileem-io-", 1).factory());

    public static final ExecutorService OPERATIONS = Executors.newCachedThreadPool(r -> {
        Thread t = new Thread(r, "fileem-op-" + OPS.incrementAndGet());
        t.setDaemon(true);
        return t;
    });

    private Bg() {
    }

    /**
     * Runs {@code work} on a virtual thread; {@code onSuccess} or {@code onError} follow on the FX
     * thread. Cancelling the returned future interrupts the work and drops its result (no callback).
     */
    public static <T> Future<?> run(Callable<T> work, Consumer<T> onSuccess, Consumer<Exception> onError) {
        return IO.submit(() -> {
            try {
                T result = work.call();
                if (!Thread.currentThread().isInterrupted()) {
                    Platform.runLater(() -> onSuccess.accept(result));
                }
            } catch (CancellationException e) {
                // cancelled: nobody wants the result
            } catch (Exception e) {
                if (onError != null && !Thread.currentThread().isInterrupted()) {
                    Platform.runLater(() -> onError.accept(e));
                }
            }
            return null;
        });
    }

    /** Throws {@link CancellationException} when the current job was cancelled (call it in long loops). */
    public static void checkInterrupted() {
        if (Thread.currentThread().isInterrupted()) {
            throw new CancellationException();
        }
    }
}
