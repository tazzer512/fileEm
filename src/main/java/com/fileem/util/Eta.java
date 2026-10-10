package com.fileem.util;

import java.util.ArrayDeque;
import java.util.Iterator;

/**
 * Estimates the time left of a running job from samples of its progress. The rate is measured over
 * a sliding window of the last {@link #WINDOW_NANOS}, so it follows changes in speed (small files vs.
 * big ones, a slower drive) without jumping around on every sample. Waits (pause, a question to the
 * user) must not count as slow progress: call {@link #reset()} when the job stops and resumes.
 */
public final class Eta {
    static final long WINDOW_NANOS = 10_000_000_000L;
    /** No estimate before this much measuring: the first moments are dominated by startup and caches. */
    static final long MIN_MEASURE_NANOS = 2_000_000_000L;

    /** {time in nanos, units done}, oldest first. */
    private final ArrayDeque<long[]> samples = new ArrayDeque<>();
    private long measuringSince = -1;

    /** Forgets all samples; the next estimate needs a fresh {@link #MIN_MEASURE_NANOS} of progress. */
    public void reset() {
        samples.clear();
        measuringSince = -1;
    }

    /** Records that {@code done} units were finished at {@code nowNanos}. */
    public void sample(long nowNanos, long done) {
        if (measuringSince < 0) {
            measuringSince = nowNanos;
        }
        samples.addLast(new long[]{nowNanos, done});
        // keep one sample at or beyond the window edge, so the window is always fully spanned
        while (samples.size() > 2) {
            Iterator<long[]> it = samples.iterator();
            it.next();
            if (nowNanos - it.next()[0] >= WINDOW_NANOS) {
                samples.removeFirst();
            } else {
                break;
            }
        }
    }

    /** Units per second over the window, or -1 while unknown. */
    public double rate() {
        if (samples.size() < 2) {
            return -1;
        }
        long[] first = samples.peekFirst();
        long[] last = samples.peekLast();
        long elapsed = last[0] - first[0];
        if (last[0] - measuringSince < MIN_MEASURE_NANOS || elapsed <= 0) {
            return -1;
        }
        return (last[1] - first[1]) * 1e9 / elapsed;
    }

    /** Seconds until {@code total} is reached at the current rate, or -1 while unknown. */
    public long secondsLeft(long total) {
        double rate = rate();
        if (rate <= 0 || samples.isEmpty()) {
            return -1;
        }
        long left = Math.max(0, total - samples.peekLast()[1]);
        return (long) Math.ceil(left / rate);
    }

    /** "About 2 minutes left"; coarse on purpose, an estimate shouldn't pretend to be exact. */
    public static String format(long seconds) {
        if (seconds < 0) {
            return "";
        }
        if (seconds < 5) {
            return "Almost done";
        }
        if (seconds < 60) {
            long s = (seconds + 4) / 5 * 5; // up to the next 5 s
            return s >= 60 ? "About 1 minute left" : "About " + s + " seconds left";
        }
        if (seconds < 3600) {
            long m = Math.round(seconds / 60.0);
            if (m >= 60) {
                return "About 1 hour left";
            }
            return m == 1 ? "About 1 minute left" : "About " + m + " minutes left";
        }
        long totalMinutes = Math.round(seconds / 60.0);
        long h = totalMinutes / 60, m = totalMinutes % 60;
        if (h >= 24) {
            return "More than " + (h / 24 == 1 ? "a day" : h / 24 + " days") + " left";
        }
        if (m == 0) {
            return h == 1 ? "About 1 hour left" : "About " + h + " hours left";
        }
        return "About " + h + " h " + m + " min left";
    }
}
