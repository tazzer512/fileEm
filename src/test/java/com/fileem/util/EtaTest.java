package com.fileem.util;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class EtaTest {
    private static final long S = 1_000_000_000L;
    private static final long MB = 1L << 20;

    /** Feeds one sample per second at {@code perSecond} units, from second {@code from} to {@code to}. */
    private static long feed(Eta eta, long done, long perSecond, int from, int to) {
        for (int t = from; t <= to; t++) {
            eta.sample(t * S, done);
            done += perSecond;
        }
        return done - perSecond;
    }

    @Test
    void noEstimateUntilItHasMeasuredForAWhile() {
        Eta eta = new Eta();
        assertEquals(-1, eta.secondsLeft(100 * MB));
        feed(eta, 0, 10 * MB, 0, 1);
        assertEquals(-1, eta.secondsLeft(100 * MB), "one second is too little to judge");
        feed(eta, 20 * MB, 10 * MB, 2, 2);
        assertTrue(eta.rate() > 0);
    }

    @Test
    void aSteadyRateGivesTheRemainingTime() {
        Eta eta = new Eta();
        long done = feed(eta, 0, 10 * MB, 0, 4); // 40 MB after 4 s
        assertEquals(40 * MB, done);
        assertEquals(10.0 * MB, eta.rate(), 1);
        assertEquals(6, eta.secondsLeft(100 * MB));
    }

    @Test
    void theEstimateFollowsAChangeInSpeed() {
        Eta eta = new Eta();
        long done = feed(eta, 0, 100 * MB, 0, 10);       // fast at first (big files)
        done = feed(eta, done + MB, MB, 11, 30);         // then 1 MB/s (small files)
        assertEquals(1.0 * MB, eta.rate(), MB * 0.15, "after a full window only the new speed counts");
        assertEquals(60, eta.secondsLeft(done + 60 * MB), 10);
    }

    @Test
    void aResetForgetsThePauseSoWaitingDoesNotLookSlow() {
        Eta eta = new Eta();
        long done = feed(eta, 0, 10 * MB, 0, 5);
        eta.reset(); // paused from 5 s to 600 s
        assertEquals(-1, eta.secondsLeft(200 * MB), "measures afresh after resuming");
        feed(eta, done + 10 * MB, 10 * MB, 600, 603);
        assertEquals(10.0 * MB, eta.rate(), 1, "the pause doesn't count as slow progress");
    }

    @Test
    void noProgressMeansNoEstimateRatherThanInfinity() {
        Eta eta = new Eta();
        feed(eta, 5 * MB, 0, 0, 5);
        assertEquals(-1, eta.secondsLeft(100 * MB));
        assertEquals("", Eta.format(-1));
    }

    @Test
    void timesAreRoundedToWhatPeopleExpect() {
        assertEquals("Almost done", Eta.format(0));
        assertEquals("Almost done", Eta.format(4));
        assertEquals("About 10 seconds left", Eta.format(7));
        assertEquals("About 45 seconds left", Eta.format(45));
        assertEquals("About 1 minute left", Eta.format(58));
        assertEquals("About 1 minute left", Eta.format(80));
        assertEquals("About 2 minutes left", Eta.format(100));
        assertEquals("About 59 minutes left", Eta.format(59 * 60));
        assertEquals("About 1 hour left", Eta.format(3599));
        assertEquals("About 1 hour left", Eta.format(3600));
        assertEquals("About 3 hours left", Eta.format(3 * 3600 + 10));
        assertEquals("About 2 h 5 min left", Eta.format(2 * 3600 + 5 * 60));
        assertEquals("More than a day left", Eta.format(30 * 3600));
        assertEquals("More than 3 days left", Eta.format(80 * 3600));
    }
}
