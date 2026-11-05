//! Background-only heartbeat observation. The Editor only increments an atomic
//! sequence; clocks, sampling and publication belong to the broker thread.
use std::time::{Duration, Instant};

pub const UNRESPONSIVE_AFTER: Duration = Duration::from_secs(30);
const OBSERVER_GAP: Duration = Duration::from_secs(10);

pub struct HeartbeatObserver {
    sequence: u64,
    generation: i64,
    last_change: Instant,
    last_sample: Instant,
    observed_pulse: bool,
}

impl HeartbeatObserver {
    pub fn new(now: Instant) -> Self {
        Self {
            sequence: 0,
            generation: 0,
            last_change: now,
            last_sample: now,
            observed_pulse: false,
        }
    }

    /// A scheduler/sleep gap is not evidence against the Editor. Require a new
    /// observation window after it; recovery also requires a real new pulse.
    pub fn sample(&mut self, sequence: u64, generation: i64, now: Instant) -> Option<Duration> {
        let gap = now.saturating_duration_since(self.last_sample) > OBSERVER_GAP;
        if sequence != self.sequence || generation != self.generation || gap {
            self.last_change = now;
        }
        if generation != self.generation || gap {
            self.observed_pulse = false;
        }
        if sequence != self.sequence && sequence != 0 {
            self.observed_pulse = true;
        }
        self.sequence = sequence;
        self.generation = generation;
        self.last_sample = now;
        let age = now.saturating_duration_since(self.last_change);
        (self.observed_pulse || (sequence != 0 && age >= UNRESPONSIVE_AFTER)).then_some(age)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_stopped_pump_and_recovers_only_on_a_new_pulse() {
        let start = Instant::now();
        let mut observer = HeartbeatObserver::new(start);
        assert_eq!(observer.sample(0, 1, start), None);
        assert_eq!(observer.sample(1, 1, start), Some(Duration::ZERO));
        for second in 1..=35 {
            assert_eq!(
                observer.sample(1, 1, start + Duration::from_secs(second)),
                Some(Duration::from_secs(second))
            );
        }
        assert_eq!(
            observer.sample(2, 1, start + Duration::from_secs(36)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn reload_and_observer_suspension_do_not_inherit_a_hang() {
        let start = Instant::now();
        let mut observer = HeartbeatObserver::new(start);
        observer.sample(10, 1, start);
        assert_eq!(observer.sample(10, 2, start + Duration::from_secs(1)), None);
        assert_eq!(
            observer.sample(11, 2, start + Duration::from_secs(2)),
            Some(Duration::ZERO)
        );
        assert_eq!(
            observer.sample(11, 2, start + Duration::from_secs(100)),
            None
        );
        assert_eq!(
            observer.sample(11, 2, start + Duration::from_secs(101)),
            None
        );
        assert_eq!(
            observer.sample(12, 2, start + Duration::from_secs(102)),
            Some(Duration::ZERO)
        );
    }
}
