//! Bounded, resumable literal search. Each text field is searched once, using the
//! existing (session_id, rowid) index; no text UNION or hit sort.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Instant;

const MAX_SCAN_MESSAGES: u32 = 512;
const MAX_SCAN_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SCAN_SESSIONS: u32 = 128;
const SCAN_TIME: Duration = Duration::from_millis(100);
const MESSAGE_BATCH: u32 = 16;
const MESSAGE_QUERY: &str =
    "SELECT m.rowid, m.id, m.role, m.content, m.thinking_content, m.tool_calls
     FROM messages m JOIN sessions s ON s.id = m.session_id
     WHERE m.session_id = ?1 AND m.rowid <= ?2
       AND s.default_checkout_id = ?4 AND (s.archived_at IS NOT NULL) = ?5
     ORDER BY m.rowid DESC LIMIT ?3";

struct LiteralMatcher {
    needle: String,
    fold_ascii: bool,
    scratch: String,
}

impl LiteralMatcher {
    fn new(query: &str) -> Self {
        Self {
            needle: query.to_ascii_lowercase(),
            fold_ascii: query.bytes().any(|byte| byte.is_ascii_alphabetic()),
            scratch: String::new(),
        }
    }

    fn find(&mut self, text: &str) -> Option<usize> {
        if !self.fold_ascii {
            return text.find(&self.needle);
        }
        // std's literal search is substantially faster than the case-insensitive
        // regex automaton in development builds. Reuse one field-sized buffer;
        // ASCII folding preserves UTF-8 byte offsets and the SDK's case semantics.
        self.scratch.clear();
        self.scratch.push_str(text);
        self.scratch.make_ascii_lowercase();
        self.scratch.find(&self.needle)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct SessionKey {
    id: String,
    updated_at: i64,
}

#[derive(Serialize, Deserialize)]
struct SessionPosition {
    key: SessionKey,
    // Inclusive row boundary; field resumes within a message with several hits.
    row_id: i64,
    field: u8,
    title_done: bool,
}

#[derive(Serialize, Deserialize)]
struct SearchCursor {
    version: u8,
    fingerprint: String,
    max_message_row_id: i64,
    after_session: Option<SessionKey>,
    current: Option<SessionPosition>,
}

struct ScanBudget {
    started: Instant,
    messages: u32,
    bytes: u64,
    sessions: u32,
}

impl ScanBudget {
    fn exhausted(&self) -> bool {
        self.messages >= MAX_SCAN_MESSAGES
            || self.bytes >= MAX_SCAN_BYTES
            || self.sessions >= MAX_SCAN_SESSIONS
            || self.started.elapsed() >= SCAN_TIME
    }
}

fn excerpt(text: &str, match_start: usize) -> String {
    // Work only on the nearby characters, not a full-string char count/copy.
    let start = text[..match_start]
        .char_indices()
        .rev()
        .nth(78)
        .map_or(0, |(i, _)| i);
    let end = text[start..]
        .char_indices()
        .nth(320)
        .map_or(text.len(), |(i, _)| start + i);
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        &text[start..end],
        if end < text.len() { "…" } else { "" }
    )
}

fn text_column<'a>(row: &'a rusqlite::Row<'_>, column: usize) -> Result<&'a str, String> {
    match row.get_ref(column).map_err(|e| e.to_string())? {
        rusqlite::types::ValueRef::Null => Ok(""),
        value => value.as_str().map_err(|e| e.to_string()),
    }
}

fn finish(
    matches: Vec<SessionSearchMatch>,
    cursor: Option<&SearchCursor>,
    budget: &ScanBudget,
) -> Result<SessionSearchPage, String> {
    Ok(SessionSearchPage {
        matches,
        next_cursor: cursor
            .map(|value| {
                serde_json::to_vec(value)
                    .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
                    .map_err(|e| e.to_string())
            })
            .transpose()?,
        scanned_messages: budget.messages,
        scanned_bytes: budget.bytes,
    })
}

impl SessionStore {
    /// Short-lived independent reads avoid the writer mutex. A cursor caps the
    /// message snapshot and resumes at a field boundary, including on empty pages.
    pub fn search_session_history(
        &self,
        checkout_id: &str,
        query: &str,
        archived: bool,
        session_id: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<SessionSearchPage, String> {
        if query.trim().is_empty() || query.chars().count() > 1_000 || query.contains('\0') {
            return Err("query must contain 1..1000 characters and no NUL".into());
        }
        if !(1..=100).contains(&limit) {
            return Err("limit must be 1..100".into());
        }
        let fingerprint = URL_SAFE_NO_PAD.encode(Sha256::digest(
            serde_json::to_vec(&(checkout_id, query, archived, session_id))
                .map_err(|e| e.to_string())?,
        ));
        let mut state: Option<SearchCursor> = cursor
            .map(|token| {
                if token.len() > 4096 {
                    return Err("Invalid search cursor".to_string());
                }
                let bytes = URL_SAFE_NO_PAD
                    .decode(token)
                    .map_err(|_| "Invalid search cursor")?;
                let value: SearchCursor =
                    serde_json::from_slice(&bytes).map_err(|_| "Invalid search cursor")?;
                if value.version != 1
                    || value.fingerprint != fingerprint
                    || value.max_message_row_id < 0
                    || value.current.as_ref().is_some_and(|s| {
                        s.field > 2 || s.row_id < 0 || s.row_id > value.max_message_row_id
                    })
                {
                    return Err("Search cursor does not match this query or workspace".into());
                }
                Ok(value)
            })
            .transpose()?;
        let conn =
            Connection::open_with_flags(&self.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| format!("Failed to open history reader: {e}"))?;
        conn.busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        if let Some(id) = session_id {
            Self::require_history_session(&conn, checkout_id, id)?;
        }
        if state.is_none() {
            let max_message_row_id = conn
                .query_row("SELECT COALESCE(MAX(rowid), 0) FROM messages", [], |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string())?;
            state = Some(SearchCursor {
                version: 1,
                fingerprint,
                max_message_row_id,
                after_session: None,
                current: None,
            });
        }
        let mut state = state.unwrap();
        let mut matcher = LiteralMatcher::new(query);
        let mut budget = ScanBudget {
            started: Instant::now(),
            messages: 0,
            bytes: 0,
            sessions: 0,
        };
        let mut matches = Vec::new();
        let mut pending = VecDeque::<SessionKey>::new();
        loop {
            if state.current.is_none() {
                if pending.is_empty() {
                    // Only small session metadata is ordered, never message text or hits.
                    let mut statement = conn
                        .prepare(
                            "SELECT id, updated_at FROM sessions WHERE default_checkout_id = ?1
                         AND (archived_at IS NOT NULL) = ?2 AND (?3 IS NULL OR id = ?3)
                         AND (?4 IS NULL OR updated_at < ?4 OR (updated_at = ?4 AND id > ?5))
                         ORDER BY updated_at DESC, id ASC LIMIT 64",
                        )
                        .map_err(|e| e.to_string())?;
                    let rows = statement
                        .query_map(
                            params![
                                checkout_id,
                                archived,
                                session_id,
                                state.after_session.as_ref().map(|k| k.updated_at),
                                state.after_session.as_ref().map(|k| &k.id)
                            ],
                            |row| {
                                Ok(SessionKey {
                                    id: row.get(0)?,
                                    updated_at: row.get(1)?,
                                })
                            },
                        )
                        .map_err(|e| e.to_string())?;
                    pending = rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
                }
                let Some(key) = pending.pop_front() else {
                    return finish(matches, None, &budget);
                };
                state.current = Some(SessionPosition {
                    key,
                    row_id: state.max_message_row_id,
                    field: 0,
                    title_done: false,
                });
            }
            let position = state.current.as_mut().unwrap();
            // Revalidate resumed IDs; a caller cannot use a forged cursor to widen scope.
            // Fetch by ID so appending a message (updated_at changes) cannot skip the session.
            let title: Option<String> = conn
                .query_row(
                    "SELECT title FROM sessions WHERE id = ?1 AND default_checkout_id = ?2
                 AND (archived_at IS NOT NULL) = ?3 AND (?4 IS NULL OR id = ?4)",
                    params![position.key.id, checkout_id, archived, session_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            budget.sessions += 1;
            let Some(title) = title else {
                state.after_session = state.current.take().map(|s| s.key);
                if budget.exhausted() {
                    return finish(matches, Some(&state), &budget);
                }
                continue;
            };
            if !position.title_done {
                position.title_done = true;
                budget.bytes += title.len() as u64;
                if let Some(hit) = matcher.find(&title) {
                    matches.push(SessionSearchMatch {
                        session_id: position.key.id.clone(),
                        session_title: title.clone(),
                        message_id: None,
                        message_row_id: None,
                        role: None,
                        field: "title".into(),
                        excerpt: excerpt(&title, hit),
                    });
                }
                if matches.len() >= limit as usize || budget.exhausted() {
                    return finish(matches, Some(&state), &budget);
                }
            }
            loop {
                // Drop each 16-row statement before requesting the next batch. Even
                // rollback-journal databases get frequent opportunities to commit writes.
                let mut found = false;
                {
                    let mut statement = conn
                        .prepare_cached(MESSAGE_QUERY)
                        .map_err(|e| e.to_string())?;
                    let mut rows = statement
                        .query(params![
                            position.key.id,
                            position.row_id,
                            MESSAGE_BATCH,
                            checkout_id,
                            archived
                        ])
                        .map_err(|e| e.to_string())?;
                    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                        found = true;
                        budget.messages += 1;
                        let row_id: i64 = row.get(0).map_err(|e| e.to_string())?;
                        let start_field = if row_id == position.row_id {
                            position.field
                        } else {
                            0
                        };
                        for field in start_field..3 {
                            let text = text_column(row, 3 + field as usize)?;
                            budget.bytes += text.len() as u64;
                            if let Some(hit) = matcher.find(text) {
                                matches.push(SessionSearchMatch {
                                    session_id: position.key.id.clone(),
                                    session_title: title.clone(),
                                    message_id: Some(row.get(1).map_err(|e| e.to_string())?),
                                    message_row_id: Some(row_id),
                                    role: Some(row.get(2).map_err(|e| e.to_string())?),
                                    field: ["content", "thinking", "tool_calls"][field as usize]
                                        .into(),
                                    excerpt: excerpt(text, hit),
                                });
                            }
                            if field == 2 {
                                position.row_id = row_id - 1;
                                position.field = 0;
                            } else {
                                position.row_id = row_id;
                                position.field = field + 1;
                            }
                            if matches.len() >= limit as usize || budget.exhausted() {
                                return finish(matches, Some(&state), &budget);
                            }
                        }
                    }
                }
                if !found {
                    break;
                }
            }
            state.after_session = state.current.take().map(|s| s.key);
            if budget.exhausted() {
                return finish(matches, Some(&state), &budget);
            }
        }
    }
}

#[cfg(test)]
#[path = "history_search_tests.rs"]
mod tests;
