package com.fileem.util;

import com.fileem.FxTestSupport;
import javafx.application.Platform;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.CyclicBarrier;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTimeoutPreemptively;
import static org.junit.jupiter.api.Assertions.assertTrue;

/** The threading guarantees the UI relies on: parallel, never blocked behind other work, cancellable. */
class BgTest {
    @BeforeAll
    static void fx() {
        FxTestSupport.startToolkit();
    }

    @Test
    void ioWorkRunsOnAVirtualThreadAndItsResultArrivesOnTheFxThread() throws Exception {
        CompletableFuture<Boolean> virtual = new CompletableFuture<>();
        CompletableFuture<Boolean> onFx = new CompletableFuture<>();
        Bg.run(() -> Thread.currentThread().isVirtual(), v -> {
            virtual.complete(v);
            onFx.complete(Platform.isFxApplicationThread());
        }, null);
        assertTrue(virtual.get(10, TimeUnit.SECONDS));
        assertTrue(onFx.get(10, TimeUnit.SECONDS));
    }

    /** A thousand stuck listings (a dead network share, say) must not delay the next folder. */
    @Test
    void blockedIoJobsNeverDelayANewOne() throws Exception {
        CountDownLatch stuck = new CountDownLatch(1);
        List<Future<?>> blocked = new ArrayList<>();
        try {
            for (int i = 0; i < 1000; i++) {
                blocked.add(Bg.IO.submit(() -> {
                    stuck.await();
                    return null;
                }));
            }
            CompletableFuture<String> next = new CompletableFuture<>();
            Bg.run(() -> "listed", next::complete, null);
            assertEquals("listed", next.get(5, TimeUnit.SECONDS));
        } finally {
            stuck.countDown();
            blocked.forEach(f -> f.cancel(true));
        }
    }

    @Test
    void cancellingAJobStopsItAndDropsItsResult() throws Exception {
        CountDownLatch running = new CountDownLatch(1);
        CountDownLatch stopped = new CountDownLatch(1);
        AtomicBoolean calledBack = new AtomicBoolean();
        Future<?> job = Bg.run(() -> {
            running.countDown();
            try {
                while (true) {
                    Bg.checkInterrupted(); // what a long listing does between entries
                    Thread.onSpinWait();
                }
            } finally {
                stopped.countDown();
            }
        }, r -> calledBack.set(true), e -> calledBack.set(true));
        assertTrue(running.await(10, TimeUnit.SECONDS));
        job.cancel(true);
        assertTrue(stopped.await(10, TimeUnit.SECONDS), "the work noticed the cancel and stopped");
        FxTestSupport.drainFx();
        assertFalse(calledBack.get(), "no success or error callback for cancelled work");
    }

    /** Starting an operation never waits for, pauses or stops one that is already running. */
    @Test
    void operationsRunInParallelOnTheirOwnPlatformThreads() {
        int count = 4;
        CyclicBarrier allRunning = new CyclicBarrier(count);
        List<Future<Boolean>> ops = new ArrayList<>();
        for (int i = 0; i < count; i++) {
            ops.add(Bg.OPERATIONS.submit(() -> {
                allRunning.await(10, TimeUnit.SECONDS); // only passes when all four run at once
                return Thread.currentThread().isVirtual();
            }));
        }
        assertTimeoutPreemptively(Duration.ofSeconds(15), () -> {
            for (Future<Boolean> op : ops) {
                assertFalse(op.get(), "operations use platform threads");
            }
        });
    }
}
