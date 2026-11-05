//! Pairing sessions: one at a time, short-lived, single-use.
//!
//! A session carries two credentials: a 6-digit code (typed on a phone that
//! reaches the desktop through a CA-trusted address) and a 128-bit secret
//! (inside the QR code, together with the certificate fingerprint). Wrong
//! guesses for either count against one shared attempt budget.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::RngExt;
use tokio::sync::oneshot;

use locus_remote_protocol::{encode_pairing_secret, PAIRING_SECRET_BYTES};

pub const PAIRING_TTL: Duration = Duration::from_secs(120);
pub const MAX_ATTEMPTS: u32 = 5;
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

struct Session {
    id: String,
    code: String,
    secret: String,
    expires_at: Instant,
    expires_at_ms: i64,
    attempts: u32,
    /// A request with valid credentials is waiting for the desktop decision.
    awaiting_confirmation: bool,
}

/// Credentials handed to the settings UI when pairing starts.
#[derive(Debug, Clone)]
pub struct PairingTicket {
    pub session_id: String,
    pub code: String,
    pub secret: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairCheck {
    Accepted { session_id: String },
    Rejected { remaining: u32 },
    /// No session, expired, or exhausted.
    Closed,
    /// Another device holds this session while the desktop decides.
    Busy,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingStatus {
    pub session_id: String,
    pub expires_at_ms: i64,
    pub attempts_left: u32,
    pub awaiting_confirmation: bool,
}

#[derive(Default)]
pub struct PairingState {
    current: Mutex<Option<Session>>,
    confirmations: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl PairingState {
    fn session(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.current.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Starts a new session, replacing any previous one.
    pub fn start(&self, now_ms: i64) -> PairingTicket {
        let mut rng = rand::rng();
        let code = format!("{:06}", rng.random_range(0..1_000_000u32));
        let mut secret = [0u8; PAIRING_SECRET_BYTES];
        rng.fill(&mut secret);
        let ticket = PairingTicket {
            session_id: uuid::Uuid::new_v4().simple().to_string(),
            code,
            secret: encode_pairing_secret(&secret),
            expires_at_ms: now_ms + PAIRING_TTL.as_millis() as i64,
        };
        *self.session() = Some(Session {
            id: ticket.session_id.clone(),
            code: ticket.code.clone(),
            secret: ticket.secret.clone(),
            expires_at: Instant::now() + PAIRING_TTL,
            expires_at_ms: ticket.expires_at_ms,
            attempts: 0,
            awaiting_confirmation: false,
        });
        ticket
    }

    pub fn cancel(&self) {
        *self.session() = None;
        self.deny_all_confirmations();
    }

    pub fn status(&self) -> Option<PairingStatus> {
        let mut session = self.session();
        if session.as_ref().is_some_and(|current| current.expires_at <= Instant::now()
            && !current.awaiting_confirmation)
        {
            *session = None;
        }
        session.as_ref().map(|current| PairingStatus {
            session_id: current.id.clone(),
            expires_at_ms: current.expires_at_ms,
            attempts_left: MAX_ATTEMPTS.saturating_sub(current.attempts),
            awaiting_confirmation: current.awaiting_confirmation,
        })
    }

    pub fn is_open(&self) -> bool {
        self.status().is_some_and(|status| !status.awaiting_confirmation)
    }

    /// Checks one pairing attempt. Exactly one credential must be supplied.
    pub fn verify(&self, secret: Option<&str>, code: Option<&str>) -> PairCheck {
        let mut guard = self.session();
        let Some(session) = guard.as_mut() else {
            return PairCheck::Closed;
        };
        if session.awaiting_confirmation {
            return PairCheck::Busy;
        }
        if session.expires_at <= Instant::now() {
            *guard = None;
            return PairCheck::Closed;
        }
        let matched = match (secret, code) {
            (Some(secret), None) => constant_time_eq(secret.as_bytes(), session.secret.as_bytes()),
            (None, Some(code)) => constant_time_eq(code.as_bytes(), session.code.as_bytes()),
            _ => false,
        };
        if matched {
            session.awaiting_confirmation = true;
            return PairCheck::Accepted { session_id: session.id.clone() };
        }
        session.attempts += 1;
        let remaining = MAX_ATTEMPTS.saturating_sub(session.attempts);
        if remaining == 0 {
            *guard = None;
        }
        PairCheck::Rejected { remaining }
    }

    /// Ends the session after an accepted attempt. A denied or timed-out
    /// confirmation also closes it: an unexpected request is the signal that
    /// someone else knows the credentials.
    pub fn finish(&self, session_id: &str) {
        let mut session = self.session();
        if session.as_ref().is_some_and(|current| current.id == session_id) {
            *session = None;
        }
    }

    /// Registers a pending desktop decision and returns its receiver.
    pub fn await_confirmation(&self, request_id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.confirmations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(request_id.to_string(), tx);
        rx
    }

    /// Delivers the desktop decision. Returns false when the request is gone.
    pub fn resolve_confirmation(&self, request_id: &str, allow: bool) -> bool {
        let sender = self
            .confirmations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(request_id);
        sender.is_some_and(|sender| sender.send(allow).is_ok())
    }

    pub fn drop_confirmation(&self, request_id: &str) {
        self.confirmations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(request_id);
    }

    fn deny_all_confirmations(&self) {
        let pending: Vec<_> = self
            .confirmations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .drain()
            .collect();
        for (_, sender) in pending {
            let _ = sender.send(false);
        }
    }
}

/// Length-checked comparison without early exit on the first difference.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Renders the pairing link as an SVG QR code for the settings dialog.
pub fn qr_svg(text: &str) -> Result<String, String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .map_err(|error| format!("无法生成二维码：{error}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_six_digits_and_secret_is_128_bits() {
        let state = PairingState::default();
        let ticket = state.start(0);
        assert!(locus_remote_protocol::is_pairing_code(&ticket.code));
        assert_eq!(ticket.secret.len(), 22);
        assert_eq!(ticket.expires_at_ms, PAIRING_TTL.as_millis() as i64);
        assert!(state.is_open());
    }

    #[test]
    fn five_wrong_attempts_close_the_session() {
        let state = PairingState::default();
        let ticket = state.start(0);
        let wrong = if ticket.code == "000000" { "000001" } else { "000000" };
        for remaining in (1..MAX_ATTEMPTS).rev() {
            assert_eq!(state.verify(None, Some(wrong)), PairCheck::Rejected { remaining });
        }
        assert_eq!(state.verify(Some("bogus"), None), PairCheck::Rejected { remaining: 0 });
        assert_eq!(state.verify(None, Some(&ticket.code)), PairCheck::Closed);
    }

    #[test]
    fn accepted_attempt_blocks_others_until_finished() {
        let state = PairingState::default();
        let ticket = state.start(0);
        let accepted = state.verify(Some(&ticket.secret), None);
        let PairCheck::Accepted { session_id } = accepted else {
            panic!("secret should be accepted");
        };
        assert_eq!(state.verify(None, Some(&ticket.code)), PairCheck::Busy);
        assert!(!state.is_open());
        state.finish(&session_id);
        assert_eq!(state.verify(Some(&ticket.secret), None), PairCheck::Closed);
    }

    #[test]
    fn requires_exactly_one_credential() {
        let state = PairingState::default();
        let ticket = state.start(0);
        assert_eq!(
            state.verify(Some(&ticket.secret), Some(&ticket.code)),
            PairCheck::Rejected { remaining: MAX_ATTEMPTS - 1 }
        );
        assert_eq!(state.verify(None, None), PairCheck::Rejected { remaining: MAX_ATTEMPTS - 2 });
    }

    #[test]
    fn confirmations_resolve_once_and_cancel_denies() {
        let state = PairingState::default();
        let mut rx = state.await_confirmation("r1");
        assert!(state.resolve_confirmation("r1", true));
        assert_eq!(rx.try_recv(), Ok(true));
        assert!(!state.resolve_confirmation("r1", true));
        let mut rx = state.await_confirmation("r2");
        state.cancel();
        assert_eq!(rx.try_recv(), Ok(false));
    }

    #[test]
    fn constant_time_eq_and_qr() {
        assert!(constant_time_eq(b"123456", b"123456"));
        assert!(!constant_time_eq(b"123456", b"123457"));
        assert!(!constant_time_eq(b"12345", b"123456"));
        let svg = qr_svg("https://192.168.1.2:47831/m/#p=abc").unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
    }
}
