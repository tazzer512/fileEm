//! Estimates the time left of a running job from samples of its progress. The rate is measured
//! over a sliding window of the last 10 s, so it follows changes in speed (small files vs. big
//! ones, a slower drive) without jumping around on every sample. Waits (pause, a question to the
//! user) must not count as slow progress: call [`Eta::reset`] when the job stops and resumes.

use std::collections::VecDeque;
use std::time::Duration;

const WINDOW: Duration = Duration::from_secs(10);
/// No estimate before this much measuring: the first moments are dominated by startup and caches.
const MIN_MEASURE: Duration = Duration::from_secs(2);

#[derive(Debug, Default, Clone)]
pub struct Eta {
    /// (time since an arbitrary start, units done), oldest first.
    samples: VecDeque<(Duration, u64)>,
    measuring_since: Option<Duration>,
}

impl Eta {
    pub fn new() -> Eta {
        Eta::default()
    }

    /// Forgets all samples; the next estimate needs a fresh 2 s of progress.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.measuring_since = None;
    }

    /// Records that `done` units were finished at `now` (any monotonic clock, e.g. time since start).
    pub fn sample(&mut self, now: Duration, done: u64) {
        self.measuring_since.get_or_insert(now);
        self.samples.push_back((now, done));
        // keep one sample at or beyond the window edge, so the window is always fully spanned
        while self.samples.len() > 2 && now.saturating_sub(self.samples[1].0) >= WINDOW {
            self.samples.pop_front();
        }
    }

    /// Units per second over the window, or `None` while unknown.
    pub fn rate(&self) -> Option<f64> {
        let (&(t0, d0), &(t1, d1)) = (self.samples.front()?, self.samples.back()?);
        let elapsed = t1.saturating_sub(t0);
        if self.samples.len() < 2 || t1.saturating_sub(self.measuring_since?) < MIN_MEASURE || elapsed.is_zero() {
            return None;
        }
        Some(d1.saturating_sub(d0) as f64 / elapsed.as_secs_f64())
    }

    /// Seconds until `total` is reached at the current rate, or `None` while unknown.
    pub fn seconds_left(&self, total: u64) -> Option<u64> {
        let rate = self.rate().filter(|&r| r > 0.0)?;
        let left = total.saturating_sub(self.samples.back()?.1);
        Some((left as f64 / rate).ceil() as u64)
    }

    /// "About 2 minutes left"; coarse on purpose, an estimate shouldn't pretend to be exact.
    pub fn format(seconds: Option<u64>) -> String {
        let Some(seconds) = seconds else {
            return String::new();
        };
        let round = |x: f64| (x + 0.5).floor() as u64; // Java's Math.round
        if seconds < 5 {
            return "Almost done".into();
        }
        if seconds < 60 {
            let s = seconds.div_ceil(5) * 5; // up to the next 5 s
            return if s >= 60 { "About 1 minute left".into() } else { format!("About {s} seconds left") };
        }
        if seconds < 3600 {
            let m = round(seconds as f64 / 60.0);
            return match m {
                60.. => "About 1 hour left".into(),
                1 => "About 1 minute left".into(),
                m => format!("About {m} minutes left"),
            };
        }
        let total_minutes = round(seconds as f64 / 60.0);
        let (h, m) = (total_minutes / 60, total_minutes % 60);
        if h >= 24 {
            let days = h / 24;
            return if days == 1 { "More than a day left".into() } else { format!("More than {days} days left") };
        }
        match (h, m) {
            (1, 0) => "About 1 hour left".into(),
            (h, 0) => format!("About {h} hours left"),
            (h, m) => format!("About {h} h {m} min left"),
        }
    }
}

/// "12.3 MB/s" for a rate in bytes per second.
pub fn speed(bytes_per_second: f64) -> String {
    format!("{}/s", super::formats::size(Some(bytes_per_second.max(0.0) as u64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    fn s(t: u64) -> Duration {
        Duration::from_secs(t)
    }

    /// Feeds one sample per second at `per_second` units, from second `from` to `to`.
    fn feed(eta: &mut Eta, mut done: u64, per_second: u64, from: u64, to: u64) -> u64 {
        for t in from..=to {
            eta.sample(s(t), done);
            done += per_second;
        }
        done - per_second
    }

    #[test]
    fn no_estimate_until_it_has_measured_for_a_while() {
        let mut eta = Eta::new();
        assert_eq!(eta.seconds_left(100 * MB), None);
        feed(&mut eta, 0, 10 * MB, 0, 1);
        assert_eq!(eta.seconds_left(100 * MB), None, "one second is too little to judge");
        feed(&mut eta, 20 * MB, 10 * MB, 2, 2);
        assert!(eta.rate().unwrap() > 0.0);
    }

    #[test]
    fn a_steady_rate_gives_the_remaining_time() {
        let mut eta = Eta::new();
        let done = feed(&mut eta, 0, 10 * MB, 0, 4); // 40 MB after 4 s
        assert_eq!(done, 40 * MB);
        assert!((eta.rate().unwrap() - 10.0 * MB as f64).abs() <= 1.0);
        assert_eq!(eta.seconds_left(100 * MB), Some(6));
    }

    #[test]
    fn the_estimate_follows_a_change_in_speed() {
        let mut eta = Eta::new();
        let done = feed(&mut eta, 0, 100 * MB, 0, 10); // fast at first (big files)
        let done = feed(&mut eta, done + MB, MB, 11, 30); // then 1 MB/s (small files)
        let rate = eta.rate().unwrap();
        assert!((rate - MB as f64).abs() <= MB as f64 * 0.15, "after a full window only the new speed counts: {rate}");
        let left = eta.seconds_left(done + 60 * MB).unwrap() as i64;
        assert!((left - 60).abs() <= 10, "{left}");
    }

    #[test]
    fn a_reset_forgets_the_pause_so_waiting_does_not_look_slow() {
        let mut eta = Eta::new();
        let done = feed(&mut eta, 0, 10 * MB, 0, 5);
        eta.reset(); // paused from 5 s to 600 s
        assert_eq!(eta.seconds_left(200 * MB), None, "measures afresh after resuming");
        feed(&mut eta, done + 10 * MB, 10 * MB, 600, 603);
        assert!((eta.rate().unwrap() - 10.0 * MB as f64).abs() <= 1.0, "the pause doesn't count as slow progress");
    }

    #[test]
    fn no_progress_means_no_estimate_rather_than_infinity() {
        let mut eta = Eta::new();
        feed(&mut eta, 5 * MB, 0, 0, 5);
        assert_eq!(eta.seconds_left(100 * MB), None);
        assert_eq!(Eta::format(None), "");
    }

    #[test]
    fn times_are_rounded_to_what_people_expect() {
        for (secs, text) in [
            (0, "Almost done"),
            (4, "Almost done"),
            (7, "About 10 seconds left"),
            (45, "About 45 seconds left"),
            (58, "About 1 minute left"),
            (80, "About 1 minute left"),
            (100, "About 2 minutes left"),
            (59 * 60, "About 59 minutes left"),
            (3599, "About 1 hour left"),
            (3600, "About 1 hour left"),
            (3 * 3600 + 10, "About 3 hours left"),
            (2 * 3600 + 5 * 60, "About 2 h 5 min left"),
            (30 * 3600, "More than a day left"),
            (80 * 3600, "More than 3 days left"),
        ] {
            assert_eq!(Eta::format(Some(secs)), text, "{secs} s");
        }
        assert_eq!(speed(1.5 * MB as f64), "1.50 MB/s");
    }
}
