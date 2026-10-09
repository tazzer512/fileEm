package com.fileem;

import javafx.application.Platform;
import org.testfx.api.FxToolkit;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

/** Starts the JavaFX toolkit once (headless on Monocle when configured by the build). */
public final class FxTestSupport {
    private FxTestSupport() {
    }

    public static void startToolkit() {
        try {
            FxToolkit.registerPrimaryStage();
        } catch (TimeoutException e) {
            throw new IllegalStateException("JavaFX toolkit did not start", e);
        }
    }

    /** Waits until all FX events queued so far have been processed. */
    public static void drainFx() throws InterruptedException {
        CountDownLatch latch = new CountDownLatch(1);
        Platform.runLater(latch::countDown);
        if (!latch.await(10, TimeUnit.SECONDS)) {
            throw new IllegalStateException("FX thread did not respond");
        }
    }
}
