//! Newline-delimited JSON envelope spoken on the broker pipe. Field names are
//! the managed `PipeEnvelope` names (`id`, `type`, `message`, `ok`, `error`,
//! `reply_to`, `processId`, `processPath`), so every client and host shares
//! one wire shape.

use serde::{Deserialize, Serialize};

/// Owned envelope used by clients (requests out, responses and events in).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipeEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(default, rename = "reply_to", skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,

    #[serde(default, rename = "type")]
    pub kind: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    #[serde(default, rename = "processId", skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,

    #[serde(default, rename = "processPath", skip_serializing_if = "Option::is_none")]
    pub process_path: Option<String>,
}

/// Borrowed envelope the broker writes (responses and unsolicited events).
#[derive(Debug, Serialize)]
pub struct OutEnvelope<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<&'a str>,
    #[serde(rename = "type")]
    pub kind: &'a str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'a str>,
    #[serde(rename = "processId", skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(rename = "processPath", skip_serializing_if = "Option::is_none")]
    pub process_path: Option<String>,
}

/// Envelope `type` of a response (requests carry their own command type).
pub const RESPONSE_KIND: &str = "response";
/// Unsolicited event telling the client a managed request was queued.
pub const REQUEST_ACCEPTED_EVENT: &str = "locus-request-accepted";
/// Request answered by the broker (capability [`crate::capabilities::REQUEST_STATE_V1`]):
/// `message` is a JSON array of the client's request ids, the response maps
/// each to `queued`, `inflight` or `none`.
pub const REQUEST_STATE_REQUEST: &str = "request_state";

/// Strip a UTF-8 BOM and surrounding ASCII whitespace from one frame.
pub fn trim_frame(line: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = line.len();
    if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
        start = 3;
    }
    while start < end && line[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &line[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_and_borrowed_envelopes_share_one_wire_shape() {
        let out = OutEnvelope {
            reply_to: Some("r1"),
            kind: RESPONSE_KIND,
            ok: true,
            message: Some("pong".into()),
            error: None,
            process_id: Some(42),
            process_path: Some("C:/Unity.exe".into()),
        };
        let json = serde_json::to_string(&out).unwrap();
        assert_eq!(
            json,
            r#"{"reply_to":"r1","type":"response","ok":true,"message":"pong","processId":42,"processPath":"C:/Unity.exe"}"#
        );
        let parsed: PipeEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.reply_to.as_deref(), Some("r1"));
        assert_eq!(parsed.kind, "response");
        assert_eq!(parsed.process_id, Some(42));
    }

    #[test]
    fn trim_frame_strips_bom_and_whitespace() {
        assert_eq!(trim_frame(b"\xEF\xBB\xBF  {\"id\":1}\r\n"), b"{\"id\":1}");
        assert_eq!(trim_frame(b"   \n"), b"");
    }
}
