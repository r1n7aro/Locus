//! Outbound frames of one pipe connection.
//!
//! A reply (a response, an error, or the acceptance of the client's own
//! request) is never dropped: losing one leaves the client waiting for an
//! answer that will not come. Broadcast events are snapshots of host state, so
//! while the client is not reading only the latest few are worth keeping: past
//! [`EVENT_FRAME_LIMIT`] queued events a new event replaces the oldest queued
//! one of its type (or the oldest event). A client that leaves more than
//! [`REPLY_BYTE_LIMIT`] of replies unread is disconnected instead, so its
//! pending requests fail and it reconnects.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::Value;
use tokio::sync::Notify;

use crate::broker::now_ms;

/// Queued broadcast events per connection before newer ones replace older.
pub(crate) const EVENT_FRAME_LIMIT: usize = 64;
/// Queued broadcast event bytes per connection before newer replace older.
pub(crate) const EVENT_BYTE_LIMIT: usize = 8 * 1024 * 1024;
/// Unread reply bytes after which the connection is closed. A single reply
/// larger than this is still delivered.
#[cfg(not(test))]
pub(crate) const REPLY_BYTE_LIMIT: usize = 256 * 1024 * 1024;
#[cfg(test)]
pub(crate) const REPLY_BYTE_LIMIT: usize = 1024 * 1024;

enum Class {
    Reply,
    Event(String),
}

struct Frame {
    class: Class,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct State {
    frames: VecDeque<Frame>,
    reply_bytes: usize,
    event_bytes: usize,
    events: usize,
    closed: bool,
    /// Events replaced since the client last caught up.
    superseded: u64,
    superseded_total: u64,
    /// When the frame being written was taken, 0 while the writer is idle.
    write_started_ms: i64,
}

impl State {
    fn remove(&mut self, index: usize) {
        if let Some(frame) = self.frames.remove(index) {
            match frame.class {
                Class::Reply => self.reply_bytes -= frame.bytes.len(),
                Class::Event(_) => {
                    self.event_bytes -= frame.bytes.len();
                    self.events -= 1;
                }
            }
        }
    }

    /// Index of the event the newest `kind` event supersedes: the oldest
    /// queued one of the same type, else the oldest event of any type.
    fn superseded_event(&self, kind: &str) -> Option<usize> {
        let newest = self.frames.len().checked_sub(1)?;
        let is_event = |frame: &Frame| matches!(frame.class, Class::Event(_));
        self.frames
            .iter()
            .take(newest)
            .position(|frame| matches!(&frame.class, Class::Event(k) if k == kind))
            .or_else(|| self.frames.iter().take(newest).position(is_event))
    }
}

pub(crate) struct Outbox {
    client: u64,
    state: Mutex<State>,
    ready: Notify,
    closed: Notify,
}

impl Outbox {
    pub(crate) fn new(client: u64) -> Self {
        Self {
            client,
            state: Mutex::new(State::default()),
            ready: Notify::new(),
            closed: Notify::new(),
        }
    }

    /// Queue a reply. Returns `false` when the client already left more than
    /// [`REPLY_BYTE_LIMIT`] unread; the caller then closes the connection.
    pub(crate) fn push_reply(&self, bytes: Vec<u8>) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.closed {
            return true;
        }
        if !state.frames.is_empty()
            && state.reply_bytes.saturating_add(bytes.len()) > REPLY_BYTE_LIMIT
        {
            return false;
        }
        state.reply_bytes += bytes.len();
        state.frames.push_back(Frame {
            class: Class::Reply,
            bytes,
        });
        drop(state);
        self.ready.notify_one();
        true
    }

    /// Queue a broadcast event, superseding older events past the limits.
    pub(crate) fn push_event(&self, kind: &str, bytes: Vec<u8>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.closed {
            return;
        }
        state.events += 1;
        state.event_bytes += bytes.len();
        state.frames.push_back(Frame {
            class: Class::Event(kind.to_string()),
            bytes,
        });
        let mut started = false;
        while state.events > EVENT_FRAME_LIMIT
            || (state.events > 1 && state.event_bytes > EVENT_BYTE_LIMIT)
        {
            let Some(index) = state.superseded_event(kind) else {
                break;
            };
            state.remove(index);
            started |= state.superseded == 0;
            state.superseded += 1;
            state.superseded_total += 1;
        }
        let queued = state.frames.len();
        let blocked_ms = match state.write_started_ms {
            0 => 0,
            started_ms => now_ms().saturating_sub(started_ms),
        };
        drop(state);
        if started {
            eprintln!(
                "[locus-native] client {} is not reading its pipe ({} frames queued, writer blocked {} ms); \
                 its replies are kept and only its latest events",
                self.client, queued, blocked_ms
            );
        }
        self.ready.notify_one();
    }

    /// Stop accepting frames and drop the queued ones.
    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.frames.clear();
            state.reply_bytes = 0;
            state.event_bytes = 0;
            state.events = 0;
        }
        self.ready.notify_one();
        self.closed.notify_one();
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.state.lock().map(|state| state.closed).unwrap_or(true)
    }

    /// Resolves once [`Outbox::close`] was called.
    pub(crate) async fn closed(&self) {
        loop {
            let notified = self.closed.notified();
            if self.is_closed() {
                return;
            }
            notified.await;
        }
    }

    /// The next frame to write, marking the writer busy until
    /// [`Outbox::written`]. `None` once closed.
    pub(crate) fn take(&self) -> Option<Vec<u8>> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        if state.closed {
            return None;
        }
        let frame = state.frames.pop_front()?;
        match frame.class {
            Class::Reply => state.reply_bytes -= frame.bytes.len(),
            Class::Event(_) => {
                state.event_bytes -= frame.bytes.len();
                state.events -= 1;
            }
        }
        state.write_started_ms = now_ms();
        let caught_up = state.frames.is_empty() && state.superseded > 0;
        let superseded = std::mem::take(&mut state.superseded);
        if !caught_up {
            state.superseded = superseded;
        }
        drop(state);
        if caught_up {
            eprintln!(
                "[locus-native] client {} caught up; {} events were superseded while it was not reading",
                self.client, superseded
            );
        }
        Some(frame.bytes)
    }

    pub(crate) async fn next(&self) -> Option<Vec<u8>> {
        loop {
            let notified = self.ready.notified();
            if let Some(bytes) = self.take() {
                return Some(bytes);
            }
            if self.is_closed() {
                return None;
            }
            notified.await;
        }
    }

    /// The frame from [`Outbox::take`] reached the pipe.
    pub(crate) fn written(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.write_started_ms = 0;
        }
    }

    pub(crate) fn status_json(&self, now: i64) -> Value {
        let Ok(state) = self.state.lock() else {
            return Value::Null;
        };
        serde_json::json!({
            "id": self.client,
            "queuedReplies": state.frames.len() - state.events,
            "queuedEvents": state.events,
            "queuedBytes": state.reply_bytes + state.event_bytes,
            "supersededEvents": state.superseded_total,
            "writeBlockedMs": match state.write_started_ms {
                0 => 0,
                started => now.saturating_sub(started).max(0),
            },
        })
    }
}
