package com.fileem.util;

import javafx.application.Platform;

import java.util.concurrent.Callable;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Consumer;

/** Background work with results delivered on the FX thread. */
public final class Bg {
    private static final AtomicInteger COUNTER = new AtomicInteger();
    public static final ExecutorService POOL = Executors.newCachedThreadPool(r -> {
        Thread t = new Thread(r, "fileem-bg-" + COUNTER.incrementAndGet());
        t.setDaemon(true);
        return t;
    });

    private Bg() {
    }

    public static <T> void run(Callable<T> work, Consumer<T> onSuccess, Consumer<Exception> onError) {
        POOL.execute(() -> {
            try {
                T result = work.call();
                Platform.runLater(() -> onSuccess.accept(result));
            } catch (Exception e) {
                if (onError != null) {
                    Platform.runLater(() -> onError.accept(e));
                }
            }
        });
    }
}
