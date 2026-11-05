//! RTT of the existing SSH stdio heartbeat, never a separate connection/probe.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const FRESH_FOR: Duration = Duration::from_secs(15);
// Shorter than the hooks' 45s lease, including the first 5s ping delay.
const RESPONSE_LEASE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct Heartbeat {
    pending: VecDeque<(u64, Instant)>,
    sample: Option<(u64, f64, Instant)>,
    // Survives eviction from the bounded pending queue. Missing RTT alone is
    // not a disconnect: require a full lease of unanswered, correlated pings.
    last_progress: Option<Instant>,
    last_reply: Option<Instant>,
}
impl Heartbeat {
    /// Dedicated bulk connections use transfer progress while busy. Old pings
    /// queued behind that transfer must not poison the next idle lease.
    pub fn suspend(&mut self) {
        self.pending.clear();
        self.last_progress = None;
        self.sample = None;
    }
    pub fn sent(&mut self, id: u64, now: Instant) {
        if self.pending.is_empty() { self.last_progress = Some(now); }
        self.pending.push_back((id, now));
        while self.pending.len() > 4 { self.pending.pop_front(); }
    }
    pub fn received(&mut self, id: u64, now: Instant) {
        let Some(index) = self.pending.iter().position(|(pending, _)| *pending == id) else { return; };
        let (_, sent) = self.pending.remove(index).unwrap();
        self.last_progress = Some(now);
        self.last_reply = Some(now);
        let elapsed = now.saturating_duration_since(sent);
        if elapsed <= FRESH_FOR && self.sample.is_none_or(|(last, _, _)| id > last) {
            self.sample = Some((id, elapsed.as_secs_f64() * 1000.0, now));
        }
    }
    pub fn latency_ms(&self, now: Instant) -> Option<f64> {
        self.sample.filter(|(_, _, at)| now.saturating_duration_since(*at) < FRESH_FOR).map(|(_, ms, _)| ms)
    }
    pub fn diagnostic(&self, now: Instant) -> (Option<u64>, usize) {
        (self.last_reply.map(|at| now.saturating_duration_since(at).as_millis() as u64), self.pending.len())
    }
    pub fn unresponsive(&self, now: Instant) -> bool {
        !self.pending.is_empty()
            && self.last_progress.is_some_and(|at| now.saturating_duration_since(at) >= RESPONSE_LEASE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_bulk_progress_lease_does_not_leave_expired_pings_in_the_next_idle_lease() {
        let now = Instant::now();
        let mut heartbeat = Heartbeat::default();
        heartbeat.sent(1, now);
        heartbeat.suspend();
        let idle = now + Duration::from_secs(300);
        assert!(!heartbeat.unresponsive(idle));
        heartbeat.received(1, idle); // delayed pre-transfer pong is not a new sample
        assert_eq!(heartbeat.latency_ms(idle), None);
        heartbeat.sent(2, idle);
        assert!(!heartbeat.unresponsive(idle + Duration::from_secs(29)));
        assert!(heartbeat.unresponsive(idle + Duration::from_secs(30)));
        heartbeat.received(2, idle + Duration::from_secs(30));
        assert!(!heartbeat.unresponsive(idle + Duration::from_secs(31)));
    }
    #[test]
    fn silence_requires_sent_pings_and_survives_pending_queue_eviction() {
        let now = Instant::now();
        let mut heartbeat = Heartbeat::default();
        assert!(!heartbeat.unresponsive(now + Duration::from_secs(60)));
        // No RTT yet is not evidence of failure.
        heartbeat.sent(1, now);
        assert!(!heartbeat.unresponsive(now + Duration::from_secs(15)));
        for id in 2..=8 { heartbeat.sent(id, now + Duration::from_secs((id - 1) * 5)); }
        assert_eq!(heartbeat.pending.len(), 4);
        heartbeat.received(99, now + Duration::from_secs(35));
        assert!(heartbeat.unresponsive(now + Duration::from_secs(35)));
        heartbeat.received(8, now + Duration::from_millis(35100));
        assert!(!heartbeat.unresponsive(now + Duration::from_secs(36)));
        assert_eq!(heartbeat.latency_ms(now + Duration::from_secs(36)), Some(100.0));
    }

    #[test]
    fn responsive_or_late_correlated_replies_are_not_transport_loss() {
        let now = Instant::now();
        let mut heartbeat = Heartbeat::default();
        for id in 1..=20 {
            let at = now + Duration::from_secs(id * 5);
            heartbeat.sent(id, at);
            heartbeat.received(id, at + Duration::from_millis(285));
            assert!(!heartbeat.unresponsive(at + Duration::from_secs(4)));
        }
        // An old RTT ages out without declaring an idle channel disconnected.
        assert_eq!(heartbeat.latency_ms(now + Duration::from_secs(130)), None);
        assert!(!heartbeat.unresponsive(now + Duration::from_secs(130)));
        heartbeat.sent(21, now + Duration::from_secs(130));
        assert!(!heartbeat.unresponsive(now + Duration::from_secs(131)));
        heartbeat.received(21, now + Duration::from_secs(150));
        assert_eq!(heartbeat.latency_ms(now + Duration::from_secs(150)), None);
        assert!(!heartbeat.unresponsive(now + Duration::from_secs(150)));
    }

    #[test]
    fn samples_roundtrip_and_discards_unknown_late_and_stale_replies() {
        let now = Instant::now();
        let mut heartbeat = Heartbeat::default();
        heartbeat.received(99, now);
        assert_eq!(heartbeat.latency_ms(now), None);
        heartbeat.sent(1, now);
        heartbeat.sent(2, now + Duration::from_secs(5));
        heartbeat.received(2, now + Duration::from_millis(5042));
        assert_eq!(heartbeat.latency_ms(now + Duration::from_secs(6)), Some(42.0));
        heartbeat.received(1, now + Duration::from_secs(7));
        assert_eq!(heartbeat.latency_ms(now + Duration::from_secs(7)), Some(42.0));
        assert_eq!(heartbeat.latency_ms(now + Duration::from_secs(21)), None);
        heartbeat.sent(3, now);
        heartbeat.received(3, now + Duration::from_secs(22));
        assert_eq!(heartbeat.latency_ms(now + Duration::from_secs(22)), None);
    }
}
