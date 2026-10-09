package com.fileem.fs;

import com.fileem.FxTestSupport;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class DirWatcherTest {
    @TempDir
    Path dir;

    @BeforeAll
    static void fx() {
        FxTestSupport.startToolkit();
    }

    @Test
    void notifiesOnChangesAndStopsAfterClose() throws Exception {
        CountDownLatch changed = new CountDownLatch(1);
        AtomicInteger calls = new AtomicInteger();
        AutoCloseable handle = DirWatcher.watch(dir, () -> {
            calls.incrementAndGet();
            changed.countDown();
        });
        Files.writeString(dir.resolve("new.txt"), "x");
        // macOS uses a polling watch service (up to ~10 s)
        assertTrue(changed.await(15, TimeUnit.SECONDS), "change was reported");

        handle.close();
        Thread.sleep(600);
        FxTestSupport.drainFx();
        int before = calls.get();
        Files.writeString(dir.resolve("after-close.txt"), "x");
        Thread.sleep(1500);
        FxTestSupport.drainFx();
        assertEquals(before, calls.get(), "no notifications after close");
    }

    @Test
    void burstsOfChangesAreDebounced() throws Exception {
        AtomicInteger calls = new AtomicInteger();
        CountDownLatch first = new CountDownLatch(1);
        try (AutoCloseable ignored = DirWatcher.watch(dir, () -> {
            calls.incrementAndGet();
            first.countDown();
        })) {
            for (int i = 0; i < 200; i++) {
                Files.writeString(dir.resolve("f" + i), "x");
            }
            assertTrue(first.await(15, TimeUnit.SECONDS));
            Thread.sleep(1200);
            FxTestSupport.drainFx();
            assertTrue(calls.get() <= 5, "200 writes caused " + calls.get() + " notifications");
        }
    }
}
