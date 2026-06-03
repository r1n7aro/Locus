use serde::Serialize;
use std::time::Instant;

/// Phase names emitted to the frontend via `diff-progress` events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffPhase {
    FetchContent,
    TextDiff,
    ParseYaml,
    BuildSemantic,
    Done,
    Error,
}

impl DiffPhase {
    pub fn index(self) -> u32 {
        match self {
            Self::FetchContent => 0,
            Self::TextDiff => 1,
            Self::ParseYaml => 2,
            Self::BuildSemantic => 3,
            Self::Done | Self::Error => u32::MAX, // not counted in total
        }
    }
}

/// Payload sent as a Tauri event on `diff-progress`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffProgressEvent {
    pub request_key: String,
    pub phase: DiffPhase,
    /// 0-based index of the current phase.
    pub current: u32,
    /// Total number of meaningful phases for this file type.
    pub total: u32,
    /// Milliseconds elapsed since the request started.
    pub elapsed_ms: u64,
    /// Present only when phase == Error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Per-phase durations in ms. Present only when phase == Done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_durations: Option<std::collections::HashMap<String, u64>>,
}

/// Lightweight per-request profiler that records wall-clock timings for each
/// diff pipeline stage. Does NOT emit events itself — callers with access to
/// `AppHandle` are responsible for calling `emit_diff_progress`.
pub struct DiffProfiler {
    start: Instant,
    request_key: String,
    is_unity: bool,
    debug: bool,
    phases: Vec<PhaseRecord>,
    fetch_sides: Vec<(String, u64)>,
    fetch_notes: Vec<String>,
    sub_phases: Vec<(String, u64)>,
    git_calls: u32,
    git_total_ms: u64,
    walkdir_calls: u32,
    walkdir_total_ms: u64,
    old_docs: usize,
    new_docs: usize,
}

#[derive(Debug)]
struct PhaseRecord {
    phase: DiffPhase,
    start_ms: u64,
    end_ms: u64,
}

impl DiffProfiler {
    pub fn new(request_key: String, is_unity: bool, debug: bool) -> Self {
        Self {
            start: Instant::now(),
            request_key,
            is_unity,
            debug,
            phases: Vec::with_capacity(6),
            fetch_sides: Vec::new(),
            fetch_notes: Vec::new(),
            sub_phases: Vec::new(),
            git_calls: 0,
            git_total_ms: 0,
            walkdir_calls: 0,
            walkdir_total_ms: 0,
            old_docs: 0,
            new_docs: 0,
        }
    }

    /// Total meaningful phases the frontend should expect for this file type.
    /// Non-Unity: fetchContent + textDiff = 2.
    /// Unity: fetchContent + textDiff + parseYaml + buildSemantic = 4.
    pub fn total_phases(&self) -> u32 {
        if self.is_unity {
            4
        } else {
            2
        }
    }

    /// Record a completed phase (timing only, no event emission).
    pub fn record(&mut self, phase: DiffPhase) {
        let now_ms = self.start.elapsed().as_millis() as u64;
        let start_ms = self.phases.last().map_or(0, |r| r.end_ms);
        self.phases.push(PhaseRecord {
            phase,
            start_ms,
            end_ms: now_ms,
        });
    }

    /// Record a named sub-phase timing (e.g. within buildSemantic).
    pub fn record_sub_phase(&mut self, label: &str, elapsed_ms: u64) {
        self.sub_phases.push((label.to_string(), elapsed_ms));
    }

    /// Return current elapsed ms for lap timing.
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Record a sub-timing within fetch_content (e.g. "old_git_show", "new_workspace").
    pub fn record_fetch_side(&mut self, label: &str, elapsed_ms: u64) {
        self.fetch_sides.push((label.to_string(), elapsed_ms));
    }

    pub fn record_fetch_note(&mut self, note: impl Into<String>) {
        self.fetch_notes.push(note.into());
    }

    #[allow(dead_code)]
    pub fn record_git_call(&mut self, elapsed_ms: u64) {
        self.git_calls += 1;
        self.git_total_ms += elapsed_ms;
    }

    pub fn record_walkdir_call(&mut self, elapsed_ms: u64) {
        self.walkdir_calls += 1;
        self.walkdir_total_ms += elapsed_ms;
    }

    pub fn set_doc_counts(&mut self, old_docs: usize, new_docs: usize) {
        self.old_docs = old_docs;
        self.new_docs = new_docs;
    }

    /// Build a progress event for the given phase.
    pub fn progress_event(&self, phase: DiffPhase, error: Option<String>) -> DiffProgressEvent {
        let phase_durations = if phase == DiffPhase::Done {
            let mut map = std::collections::HashMap::new();
            for r in &self.phases {
                let key = format!("{:?}", r.phase);
                let mut chars = key.chars();
                let key = chars
                    .next()
                    .map(|first| first.to_lowercase().chain(chars).collect::<String>())
                    .unwrap_or_default();
                map.insert(key, r.end_ms - r.start_ms);
            }
            for (label, ms) in &self.fetch_sides {
                map.insert(format!("fetch_{}", label), *ms);
            }
            for (label, ms) in &self.sub_phases {
                map.insert(label.clone(), *ms);
            }
            Some(map)
        } else {
            None
        };
        DiffProgressEvent {
            request_key: self.request_key.clone(),
            phase,
            current: phase.index().min(self.total_phases().saturating_sub(1)),
            total: self.total_phases(),
            elapsed_ms: self.start.elapsed().as_millis() as u64,
            error,
            phase_durations,
        }
    }

    /// Print a single-line summary to stderr when debug is enabled.
    pub fn log_summary(&self, file_path: &str) {
        if !self.debug {
            return;
        }
        let total_ms = self.start.elapsed().as_millis() as u64;
        let phase_parts: Vec<String> = self
            .phases
            .iter()
            .map(|r| format!("{:?}={}ms", r.phase, r.end_ms - r.start_ms))
            .collect();

        let fetch_detail: Vec<String> = self
            .fetch_sides
            .iter()
            .map(|(label, ms)| format!("{}={}ms", label, ms))
            .collect();

        let mut line = format!(
            "[diff-profile] file={} total={}ms phases=[{}] git_calls={}/{}ms walkdir_calls={}/{}ms",
            file_path,
            total_ms,
            phase_parts.join(", "),
            self.git_calls,
            self.git_total_ms,
            self.walkdir_calls,
            self.walkdir_total_ms,
        );
        if !fetch_detail.is_empty() {
            line.push_str(&format!(" fetch=[{}]", fetch_detail.join(", ")));
        }
        if !self.fetch_notes.is_empty() {
            line.push_str(&format!(" notes=[{}]", self.fetch_notes.join(" | ")));
        }
        if !self.sub_phases.is_empty() {
            let sub_detail: Vec<String> = self
                .sub_phases
                .iter()
                .map(|(label, ms)| format!("{}={}ms", label, ms))
                .collect();
            line.push_str(&format!(" sub=[{}]", sub_detail.join(", ")));
        }
        if self.is_unity {
            line.push_str(&format!(
                " old_docs={} new_docs={}",
                self.old_docs, self.new_docs,
            ));
        }
        eprintln!("{}", line);
    }
}
