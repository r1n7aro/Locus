//! Stream filter that reroutes `<think>`/`<thinking>`-tagged reasoning text
//! from the content channel into the thinking channel.
//!
//! Third-party OpenAI-compatible endpoints (vLLM, Ollama, various relays)
//! serve reasoning models that inline their chain of thought as a literal
//! `<think>...</think>` prefix of `delta.content` instead of the structured
//! `reasoning_content` field. Without rerouting, tens of kilobytes of
//! reasoning land in the transient markdown renderer and the transcript.
//!
//! Scope is deliberately narrow: only a tag at the very start of the stream
//! (leading whitespace allowed) opens redirection, so literal `<think>`
//! mentions later in prose pass through untouched. Reasoning text containing
//! a literal closing tag ends redirection early — the same limitation every
//! text-tag reasoning parser has.
//!
//! Mirrors the persisted `AppConfig::llm_strip_inline_think_tags` flag into a
//! global, same pattern as `llm::retry`.

use std::sync::atomic::{AtomicBool, Ordering};

/// Open tags checked at stream start. Longest first so `<thinking>` is not
/// consumed as `<think>` followed by literal `ing>`.
const OPEN_TAGS: [&str; 2] = ["<thinking>", "<think>"];
/// Close tags accepted while redirecting. Both variants are accepted
/// regardless of which open tag matched: mismatched pairs occur in the wild
/// and a stray unmatched close tag inside reasoning is not meaningful text.
const CLOSE_TAGS: [&str; 2] = ["</thinking>", "</think>"];

static STRIP_ENABLED: AtomicBool = AtomicBool::new(true);

/// Called once from app setup with the persisted value.
pub fn initialize(enabled: bool) {
    set_enabled(enabled);
}

/// Mirror a config change into the global.
pub fn set_enabled(enabled: bool) {
    STRIP_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn strip_enabled() -> bool {
    STRIP_ENABLED.load(Ordering::Relaxed)
}

/// Output of one `push`/`finalize` call. When both sides are non-empty the
/// thinking part precedes the content part in stream order (a close tag and
/// trailing prose arrived in the same delta).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThinkTagEmit {
    pub thinking: String,
    pub content: String,
}

impl ThinkTagEmit {
    pub fn is_empty(&self) -> bool {
        self.thinking.is_empty() && self.content.is_empty()
    }
}

enum FilterState {
    /// Stream start: buffering until the input proves or disproves an open
    /// tag prefix (leading whitespace allowed).
    Probing,
    /// Inside a think block: text flows to the thinking channel while
    /// scanning for a close tag; `buffer` holds a partial close-tag suffix.
    Redirecting,
    /// Close tag seen: swallow the whitespace that separates it from prose.
    EatWhitespace,
    /// Verbatim passthrough; no further scanning.
    Passthrough,
}

pub struct ThinkTagFilter {
    state: FilterState,
    /// `Probing`: undecided stream prefix. `Redirecting`: partial close-tag
    /// suffix held back across deltas (at most `"</thinking>".len() - 1`
    /// bytes, all ASCII). Empty in other states.
    buffer: String,
}

impl ThinkTagFilter {
    pub fn new(strip: bool) -> Self {
        Self {
            state: if strip {
                FilterState::Probing
            } else {
                FilterState::Passthrough
            },
            buffer: String::new(),
        }
    }

    /// Feed one content delta; returns the rerouted output. Memory use is
    /// O(1): only an undecided prefix or a partial close tag is held back.
    pub fn push(&mut self, input: &str) -> ThinkTagEmit {
        let mut emit = ThinkTagEmit::default();
        let mut rest = input.to_string();

        loop {
            match self.state {
                FilterState::Passthrough => {
                    emit.content.push_str(&rest);
                    return emit;
                }
                FilterState::Probing => {
                    self.buffer.push_str(&rest);
                    rest.clear();

                    let after_whitespace = self.buffer.trim_start();
                    if after_whitespace.is_empty() {
                        // Nothing but whitespace so far — keep probing.
                        return emit;
                    }

                    let matched_open = OPEN_TAGS
                        .iter()
                        .find(|tag| after_whitespace.starts_with(**tag));
                    if let Some(tag) = matched_open {
                        // Leading whitespace and the tag itself are dropped.
                        rest = after_whitespace[tag.len()..].to_string();
                        self.buffer.clear();
                        self.state = FilterState::Redirecting;
                        continue;
                    }

                    if OPEN_TAGS
                        .iter()
                        .any(|tag| tag.starts_with(after_whitespace))
                    {
                        // Could still become an open tag (e.g. "<thi").
                        return emit;
                    }

                    // Proven ordinary prose: release the held prefix verbatim.
                    emit.content.push_str(&self.buffer);
                    self.buffer.clear();
                    self.state = FilterState::Passthrough;
                    return emit;
                }
                FilterState::Redirecting => {
                    let haystack = if self.buffer.is_empty() {
                        std::mem::take(&mut rest)
                    } else {
                        let joined = format!("{}{}", self.buffer, rest);
                        self.buffer.clear();
                        rest.clear();
                        joined
                    };

                    let earliest_close = CLOSE_TAGS
                        .iter()
                        .filter_map(|tag| haystack.find(*tag).map(|pos| (pos, *tag)))
                        .min_by_key(|(pos, _)| *pos);

                    if let Some((pos, tag)) = earliest_close {
                        emit.thinking.push_str(&haystack[..pos]);
                        rest = haystack[pos + tag.len()..].to_string();
                        self.state = FilterState::EatWhitespace;
                        continue;
                    }

                    // No full close tag: hold back a suffix that could still
                    // grow into one. Close tags are ASCII, so the held suffix
                    // is ASCII and the split point is a char boundary.
                    let hold = longest_partial_close_suffix(&haystack);
                    let flush_end = haystack.len() - hold;
                    emit.thinking.push_str(&haystack[..flush_end]);
                    self.buffer = haystack[flush_end..].to_string();
                    return emit;
                }
                FilterState::EatWhitespace => {
                    let trimmed = rest.trim_start();
                    if trimmed.is_empty() {
                        return emit;
                    }
                    rest = trimmed.to_string();
                    self.state = FilterState::Passthrough;
                }
            }
        }
    }

    /// Flush at stream end. An undecided probe prefix was never proven to be
    /// a tag, so it is ordinary content; a dangling partial close tag is
    /// reasoning text that never completed.
    pub fn finalize(&mut self) -> ThinkTagEmit {
        let mut emit = ThinkTagEmit::default();
        match self.state {
            FilterState::Probing => emit.content = std::mem::take(&mut self.buffer),
            FilterState::Redirecting => emit.thinking = std::mem::take(&mut self.buffer),
            FilterState::EatWhitespace | FilterState::Passthrough => {}
        }
        self.state = FilterState::Passthrough;
        emit
    }
}

/// Length of the longest suffix of `haystack` that is a proper prefix of any
/// close tag. Bounded by the longest close tag, so the scan is O(1) per call.
fn longest_partial_close_suffix(haystack: &str) -> usize {
    let bytes = haystack.as_bytes();
    let max_len = CLOSE_TAGS
        .iter()
        .map(|tag| tag.len() - 1)
        .max()
        .unwrap_or(0)
        .min(bytes.len());
    for hold in (1..=max_len).rev() {
        let suffix = &bytes[bytes.len() - hold..];
        if CLOSE_TAGS
            .iter()
            .any(|tag| tag.len() > hold && &tag.as_bytes()[..hold] == suffix)
        {
            return hold;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push each delta through a fresh filter and collect the merged output,
    /// including the finalize flush.
    fn run(deltas: &[&str]) -> (String, String) {
        let mut filter = ThinkTagFilter::new(true);
        let mut thinking = String::new();
        let mut content = String::new();
        for delta in deltas {
            let emit = filter.push(delta);
            thinking.push_str(&emit.thinking);
            content.push_str(&emit.content);
        }
        let emit = filter.finalize();
        thinking.push_str(&emit.thinking);
        content.push_str(&emit.content);
        (thinking, content)
    }

    #[test]
    fn plain_text_passes_through() {
        let (thinking, content) = run(&["hello ", "world"]);
        assert_eq!(thinking, "");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn think_block_in_single_delta_is_rerouted() {
        let (thinking, content) = run(&["<think>reasoning</think>answer"]);
        assert_eq!(thinking, "reasoning");
        assert_eq!(content, "answer");
    }

    #[test]
    fn thinking_variant_is_rerouted() {
        let (thinking, content) = run(&["<thinking>深度思考</thinking>好的"]);
        assert_eq!(thinking, "深度思考");
        assert_eq!(content, "好的");
    }

    #[test]
    fn open_tag_split_across_deltas() {
        let (thinking, content) = run(&["<thi", "nk>a", "b</th", "ink>ok"]);
        assert_eq!(thinking, "ab");
        assert_eq!(content, "ok");
    }

    #[test]
    fn close_tag_split_across_many_deltas() {
        let (thinking, content) = run(&["<think>abc", "</", "t", "hink", ">rest"]);
        assert_eq!(thinking, "abc");
        assert_eq!(content, "rest");
    }

    #[test]
    fn mismatched_close_variant_still_closes() {
        let (thinking, content) = run(&["<think>abc</thinking>rest"]);
        assert_eq!(thinking, "abc");
        assert_eq!(content, "rest");
    }

    #[test]
    fn leading_whitespace_before_tag_is_dropped() {
        let (thinking, content) = run(&["\n\n  <think>a</think>b"]);
        assert_eq!(thinking, "a");
        assert_eq!(content, "b");
    }

    #[test]
    fn whitespace_between_close_and_prose_is_eaten() {
        let (thinking, content) = run(&["<think>a</think>", "\n\n", "\nb c"]);
        assert_eq!(thinking, "a");
        assert_eq!(content, "b c");
    }

    #[test]
    fn unclosed_block_flushes_to_thinking() {
        let (thinking, content) = run(&["<think>abc", "def"]);
        assert_eq!(thinking, "abcdef");
        assert_eq!(content, "");
    }

    #[test]
    fn dangling_partial_close_flushes_to_thinking() {
        let (thinking, content) = run(&["<think>abc</thi"]);
        assert_eq!(thinking, "abc</thi");
        assert_eq!(content, "");
    }

    #[test]
    fn literal_tag_mid_prose_is_untouched() {
        let (thinking, content) = run(&["hello <think>x</think>"]);
        assert_eq!(thinking, "");
        assert_eq!(content, "hello <think>x</think>");
    }

    #[test]
    fn second_block_after_close_is_untouched() {
        let (thinking, content) = run(&["<think>a</think>b<think>c</think>"]);
        assert_eq!(thinking, "a");
        assert_eq!(content, "b<think>c</think>");
    }

    #[test]
    fn probe_prefix_that_never_becomes_a_tag_is_content() {
        let (thinking, content) = run(&["<thin", "dex> stuff"]);
        assert_eq!(thinking, "");
        assert_eq!(content, "<thindex> stuff");
    }

    #[test]
    fn probe_prefix_left_at_stream_end_is_content() {
        let (thinking, content) = run(&["<thi"]);
        assert_eq!(thinking, "");
        assert_eq!(content, "<thi");
    }

    #[test]
    fn whitespace_only_stream_is_content() {
        let (thinking, content) = run(&["\n ", " \n"]);
        assert_eq!(thinking, "");
        assert_eq!(content, "\n  \n");
    }

    #[test]
    fn empty_think_block() {
        let (thinking, content) = run(&["<think></think>x"]);
        assert_eq!(thinking, "");
        assert_eq!(content, "x");
    }

    #[test]
    fn multibyte_text_around_held_suffix_is_safe() {
        // A partial close suffix after CJK text must not split a char.
        let (thinking, content) = run(&["<think>中文思考</t", "hink>中文正文"]);
        assert_eq!(thinking, "中文思考");
        assert_eq!(content, "中文正文");
    }

    #[test]
    fn cjk_delta_ending_before_partial_close_flushes_cleanly() {
        let (thinking, content) = run(&["<think>思考а</", "х</think>done"]);
        // "</" then a non-tag char: everything is reasoning text.
        assert_eq!(thinking, "思考а</х");
        assert_eq!(content, "done");
    }

    #[test]
    fn disabled_filter_passes_everything_through() {
        let mut filter = ThinkTagFilter::new(false);
        let emit = filter.push("<think>a</think>b");
        assert_eq!(emit.thinking, "");
        assert_eq!(emit.content, "<think>a</think>b");
        assert!(filter.finalize().is_empty());
    }

    #[test]
    fn streaming_thinking_emits_incrementally() {
        // Reasoning must flow out per delta, not accumulate until close.
        let mut filter = ThinkTagFilter::new(true);
        assert!(filter.push("<think>").is_empty());
        let first = filter.push("chunk one ");
        assert_eq!(first.thinking, "chunk one ");
        let second = filter.push("chunk two");
        assert_eq!(second.thinking, "chunk two");
        let close = filter.push("</think>\nprose");
        assert_eq!(close.thinking, "");
        assert_eq!(close.content, "prose");
    }
}
