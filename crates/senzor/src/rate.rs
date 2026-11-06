//! Messages per second, as the status bar shows it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How far back the rate looks.
const WINDOW: Duration = Duration::from_secs(5);

/// The rate at which the engine publishes, worked out from the sequence number of its newest
/// logged message, sampled once a frame.
#[derive(Debug, Default)]
pub struct MessageRate {
    /// When each sample was taken and the newest sequence number then, oldest first.
    samples: VecDeque<(Instant, u64)>,
}

impl MessageRate {
    /// Records that the newest message at `now` has sequence number `seq` (`None` before the
    /// first message), and forgets samples older than the window.
    pub fn record(&mut self, now: Instant, seq: Option<u64>) {
        let Some(seq) = seq else {
            return;
        };
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > WINDOW)
        {
            self.samples.pop_front();
        }
        // Keep the oldest sample of a quiet stretch, so the rate falls back to 0 over the window.
        let unchanged = self.samples.back().is_some_and(|(_, last)| *last == seq);
        if !unchanged || self.samples.len() < 2 {
            self.samples.push_back((now, seq));
        }
    }

    /// Messages per second over the last few seconds, at `now`.
    pub fn per_second(&self, now: Instant) -> f64 {
        let (Some((oldest_at, oldest)), Some((_, newest))) =
            (self.samples.front(), self.samples.back())
        else {
            return 0.0;
        };
        let elapsed = now.duration_since(*oldest_at).max(Duration::from_secs(1));
        (newest - oldest) as f64 / elapsed.as_secs_f64()
    }

    /// Starts over, for another engine.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn no_messages_is_zero() {
        let mut rate = MessageRate::default();
        let start = Instant::now();
        rate.record(start, None);
        assert_eq!(rate.per_second(at(start, 2000)), 0.0);
    }

    #[test]
    fn a_steady_stream_gives_its_rate() {
        let mut rate = MessageRate::default();
        let start = Instant::now();
        for second in 0..=4 {
            rate.record(at(start, second * 1000), Some(10 + second * 20));
        }
        assert_eq!(rate.per_second(at(start, 4000)), 20.0);
    }

    #[test]
    fn the_rate_falls_to_zero_once_publishing_stops() {
        let mut rate = MessageRate::default();
        let start = Instant::now();
        for second in 0..=2 {
            rate.record(at(start, second * 1000), Some(second * 10));
        }
        for second in 3..=9 {
            rate.record(at(start, second * 1000), Some(20));
        }
        assert_eq!(rate.per_second(at(start, 9000)), 0.0);
    }

    #[test]
    fn the_first_second_does_not_overstate_the_rate() {
        let mut rate = MessageRate::default();
        let start = Instant::now();
        rate.record(start, Some(1));
        rate.record(at(start, 100), Some(6));
        assert_eq!(rate.per_second(at(start, 100)), 5.0);
    }
}
