use std::collections::{HashMap, HashSet};

use rusqlite::params;

use super::SessionStore;
use crate::commands::{SessionTimingUsage, TokenUsage};
use crate::session::models::{ChatMessage, MessageRole};

type Interval = (i64, i64);

#[derive(Default)]
struct ToolTiming {
    start: Option<i64>,
    end: Option<i64>,
    waits: Vec<Interval>,
}

struct RunTiming {
    start: i64,
    end: i64,
    has_events: bool,
    tools: HashMap<String, ToolTiming>,
    questions: HashMap<String, (String, i64)>,
}

fn merge_intervals(mut intervals: Vec<Interval>) -> Vec<Interval> {
    intervals.retain(|(start, end)| end > start);
    intervals.sort_unstable();
    let mut merged: Vec<Interval> = Vec::new();
    for (start, end) in intervals {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn duration_ms(intervals: Vec<Interval>) -> u64 {
    merge_intervals(intervals)
        .into_iter()
        .map(|(start, end)| end.saturating_sub(start) as u64)
        .fold(0u64, u64::saturating_add)
        .saturating_mul(1_000)
}

fn exclude_waits((start, end): Interval, waits: Vec<Interval>) -> Vec<Interval> {
    let mut cursor = start;
    let mut active = Vec::new();
    for (wait_start, wait_end) in merge_intervals(waits) {
        let wait_start = wait_start.clamp(start, end);
        let wait_end = wait_end.clamp(start, end);
        if wait_start > cursor {
            active.push((cursor, wait_start));
        }
        cursor = cursor.max(wait_end);
    }
    if cursor < end {
        active.push((cursor, end));
    }
    active
}

impl RunTiming {
    fn record(
        &mut self,
        kind: &str,
        tool_id: Option<String>,
        question_id: Option<String>,
        at: i64,
    ) {
        self.has_events = true;
        let at = at.clamp(self.start, self.end);
        match kind {
            "toolCallStart" => {
                if let Some(id) = tool_id {
                    let tool = self.tools.entry(id).or_default();
                    if tool.end.is_none() {
                        // The first start can announce streamed arguments. The
                        // repeated start immediately before dispatch is the
                        // local call boundary, so always keep the latest one.
                        tool.start = Some(at);
                    }
                }
            }
            "toolCallDone" => {
                if let Some(id) = tool_id {
                    self.tools.entry(id).or_default().end.get_or_insert(at);
                }
            }
            "askUser" | "toolConfirm" => {
                if let (Some(tool), Some(question)) = (tool_id, question_id) {
                    self.questions.entry(question).or_insert((tool, at));
                }
            }
            "inputAnswered" => {
                if let Some((tool, start)) = question_id.and_then(|id| self.questions.remove(&id)) {
                    self.tools.entry(tool).or_default().waits.push((start, at));
                }
            }
            _ => {}
        }
    }

    fn local_intervals(mut self, server_tools: &HashSet<&str>) -> Option<Vec<Interval>> {
        if !self.has_events {
            return None;
        }
        for (_, (tool, start)) in self.questions {
            self.tools
                .entry(tool)
                .or_default()
                .waits
                .push((start, self.end));
        }
        let mut intervals = Vec::new();
        for (id, tool) in self.tools {
            if server_tools.contains(id.as_str()) {
                continue;
            }
            // A result without its start is incomplete historical data.
            let start = tool.start?;
            let end = tool.end.unwrap_or(self.end).max(start);
            intervals.extend(exclude_waits((start, end), tool.waits));
        }
        Some(intervals)
    }
}

impl SessionStore {
    /// A read-only view of existing timing records; no schema or export changes.
    /// Run and event timestamps have second precision. Tool intervals measure
    /// dispatch through completion (including scheduling / awaiting a result).
    pub fn get_session_timing_usage(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
        usage: &TokenUsage,
    ) -> Result<SessionTimingUsage, String> {
        self.event_writer.flush()?;
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        let mut runs = HashMap::new();
        let mut run_query = conn.prepare(
            "SELECT run_id, started_at, COALESCE(finished_at,
                CASE WHEN status IN ('running', 'waiting_input', 'cancelling') THEN ?2 ELSE updated_at END)
             FROM session_runs WHERE session_id = ?1",
        ).map_err(|error| error.to_string())?;
        let rows = run_query
            .query_map(params![session_id, now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (id, start, end) = row.map_err(|error| error.to_string())?;
            runs.insert(
                id,
                RunTiming {
                    start,
                    end: end.max(start),
                    has_events: false,
                    tools: HashMap::new(),
                    questions: HashMap::new(),
                },
            );
        }

        // Project only identifiers; tool outputs and streamed content can be
        // large. Do not load all payloads or use the paginated replay limit.
        let mut event_query = conn
            .prepare(
                "SELECT run_id, event_type, json_extract(payload_json, '$.toolCallId'),
                    json_extract(payload_json, '$.questionId'), created_at
             FROM session_events WHERE session_id = ?1 AND event_type IN (
                'runStart', 'done', 'cancelled', 'error', 'toolCallStart', 'toolCallDone',
                'askUser', 'toolConfirm', 'inputAnswered') ORDER BY seq",
            )
            .map_err(|error| error.to_string())?;
        let rows = event_query
            .query_map([session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (run, kind, tool, question, at) = row.map_err(|error| error.to_string())?;
            if let Some(run) = runs.get_mut(&run) {
                run.record(&kind, tool, question, at);
            }
        }

        let has_history = messages
            .iter()
            .any(|message| message.role != MessageRole::Tool);
        let server_tools: HashSet<&str> = messages
            .iter()
            .flat_map(|message| message.tool_calls.iter().flatten())
            .filter(|tool| tool.is_server_tool())
            .map(|tool| tool.id.as_str())
            .collect();
        let recorded_tools: HashSet<&str> = runs
            .values()
            .flat_map(|run| run.tools.keys().map(String::as_str))
            .collect();
        let all_tools_recorded = messages
            .iter()
            .flat_map(|message| message.tool_calls.iter().flatten())
            .filter(|tool| !tool.is_server_tool())
            .all(|tool| recorded_tools.contains(tool.id.as_str()));
        let has_runs = !runs.is_empty();
        let total_duration_ms = (has_runs || !has_history)
            .then(|| duration_ms(runs.values().map(|run| (run.start, run.end)).collect()));
        let local_tool_duration_ms = if all_tools_recorded && (has_runs || !has_history) {
            runs.into_values()
                .map(|run| run.local_intervals(&server_tools))
                .collect::<Option<Vec<_>>>()
                .map(|intervals| duration_ms(intervals.into_iter().flatten().collect()))
        } else {
            None
        };
        let remote_output_duration_ms = if usage.model_active_duration_ms > 0 {
            Some(usage.model_active_duration_ms)
        } else if usage.total_output_tokens == 0 && !has_history {
            Some(0)
        } else {
            None
        };
        Ok(SessionTimingUsage {
            remote_output_duration_ms,
            local_tool_duration_ms,
            total_duration_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: i64, end: i64) -> RunTiming {
        RunTiming {
            start,
            end,
            has_events: true,
            tools: HashMap::new(),
            questions: HashMap::new(),
        }
    }

    fn tool_event(run: &mut RunTiming, kind: &str, id: &str, at: i64) {
        run.record(kind, Some(id.into()), None, at);
    }

    #[test]
    fn local_timing_excludes_streaming_arguments_and_merges_parallel_calls() {
        let mut run = run(100, 140);
        tool_event(&mut run, "toolCallStart", "a", 101);
        tool_event(&mut run, "toolCallStart", "a", 110);
        tool_event(&mut run, "toolCallStart", "b", 111);
        tool_event(&mut run, "toolCallDone", "a", 120);
        tool_event(&mut run, "toolCallDone", "b", 125);
        assert_eq!(
            duration_ms(run.local_intervals(&HashSet::new()).unwrap()),
            15_000
        );
    }

    #[test]
    fn confirmation_wait_does_not_remove_other_parallel_work() {
        let mut run = run(0, 30);
        tool_event(&mut run, "toolCallStart", "a", 1);
        run.record("toolConfirm", Some("a".into()), Some("question".into()), 2);
        tool_event(&mut run, "toolCallStart", "b", 5);
        tool_event(&mut run, "toolCallDone", "b", 10);
        run.record("inputAnswered", None, Some("question".into()), 20);
        tool_event(&mut run, "toolCallDone", "a", 25);
        assert_eq!(
            duration_ms(run.local_intervals(&HashSet::new()).unwrap()),
            11_000
        );
    }

    #[test]
    fn cancellation_closes_open_tools_and_pending_confirmation() {
        let mut run = run(0, 15);
        tool_event(&mut run, "toolCallStart", "a", 2);
        tool_event(&mut run, "toolCallStart", "b", 3);
        run.record("askUser", Some("b".into()), Some("question".into()), 4);
        assert_eq!(
            duration_ms(run.local_intervals(&HashSet::new()).unwrap()),
            13_000
        );
    }

    #[test]
    fn server_tools_are_excluded_and_missing_local_starts_are_unknown() {
        let mut server = run(0, 30);
        tool_event(&mut server, "toolCallDone", "web", 20);
        assert_eq!(
            duration_ms(server.local_intervals(&HashSet::from(["web"])).unwrap()),
            0
        );
        let mut incomplete = run(0, 30);
        tool_event(&mut incomplete, "toolCallDone", "bash", 20);
        assert!(incomplete.local_intervals(&HashSet::new()).is_none());
    }

    #[test]
    fn total_timing_excludes_idle_gaps_and_deduplicates_overlapping_runs() {
        assert_eq!(
            duration_ms(vec![(100, 130), (120, 140), (500, 510)]),
            50_000
        );
        assert_eq!(duration_ms(vec![(10, 9), (10, 10)]), 0);
    }

    #[test]
    fn persisted_timing_reads_all_events_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let session = store
            .create_session("Timing", None, None, "chat", None)
            .unwrap();
        store.try_start_run(&session, "run").unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE session_runs SET started_at=100, updated_at=160,
                finished_at=160, status='done' WHERE run_id='run'",
                [],
            )
            .unwrap();
            // More than one replay page precedes the actual timing events.
            conn.execute(
                "WITH RECURSIVE sequence(n) AS (
                SELECT 1 UNION ALL SELECT n+1 FROM sequence WHERE n<2100)
                INSERT INTO session_events(session_id,run_id,seq,event_type,payload_json,created_at)
                SELECT ?1,'run',n,'textDelta','{}',100 FROM sequence",
                [&session],
            )
            .unwrap();
            for (seq, kind, at) in [
                (2101, "toolCallStart", 110),
                (2102, "toolCallStart", 120),
                (2103, "toolCallDone", 150),
            ] {
                conn.execute("INSERT INTO session_events(session_id,run_id,seq,event_type,payload_json,created_at)
                    VALUES (?1,'run',?2,?3,'{\"toolCallId\":\"bash-1\"}',?4)",
                    params![session, seq, kind, at]).unwrap();
            }
        }
        let mut usage = store.get_token_usage(&session).unwrap();
        usage.model_active_duration_ms = 25_500;
        let timing = store
            .get_session_timing_usage(&session, &[], &usage)
            .unwrap();
        assert_eq!(timing.remote_output_duration_ms, Some(25_500));
        assert_eq!(timing.local_tool_duration_ms, Some(30_000));
        assert_eq!(timing.total_duration_ms, Some(60_000));
        drop(store);
        let reopened = SessionStore::new(dir.path()).unwrap();
        let timing = reopened
            .get_session_timing_usage(&session, &[], &usage)
            .unwrap();
        assert_eq!(timing.local_tool_duration_ms, Some(30_000));
        assert_eq!(timing.total_duration_ms, Some(60_000));
    }

    #[test]
    fn empty_sessions_are_zero_and_legacy_sessions_without_records_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let session = store
            .create_session("Legacy", None, None, "chat", None)
            .unwrap();
        let usage = store.get_token_usage(&session).unwrap();
        let empty = store
            .get_session_timing_usage(&session, &[], &usage)
            .unwrap();
        assert_eq!(empty.remote_output_duration_ms, Some(0));
        assert_eq!(empty.local_tool_duration_ms, Some(0));
        assert_eq!(empty.total_duration_ms, Some(0));
        store
            .add_message(&session, MessageRole::User, "Historical message")
            .unwrap();
        let messages = store.get_messages(&session).unwrap();
        let legacy = store
            .get_session_timing_usage(&session, &messages, &usage)
            .unwrap();
        assert_eq!(legacy.remote_output_duration_ms, None);
        assert_eq!(legacy.local_tool_duration_ms, None);
        assert_eq!(legacy.total_duration_ms, None);
    }
}
