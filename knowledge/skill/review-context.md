---
id: kd_skill_review_context
injectMode: excerpt
summary: >-
  Use when `/review-context` creates a YAML audit review session or the user asks to analyze a Locus exported context trajectory. Ignore ordinary code review, current-session status summaries, and non-Locus logs.
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /review-context
tools:
  - read
  - grep
---

# Review Context

## Instructions

Analyze the Locus YAML context audit named in the user message. Keep the review read-only. Treat the audit as evidence and the current repository only as optional supporting context; do not modify project files.

### Evidence procedure

1. Inspect `format`, `format_version`, `export`, `source`, and every session entry before drawing conclusions.
2. For large files, page with `read` and locate sections with `grep`. Cover every `context_attempts` entry and every compaction boundary; do not silently analyze only the visible prefix.
3. Use `session_id`, `run_id`, `iteration`, `attempt`, message IDs, tool-call IDs, and YAML field paths as evidence anchors.
4. Label each material statement as `Fact` or `Inference`. Explain the evidence limit whenever a value is `empty`, capture quality is partial/reconstructed, output is truncated, or an encrypted compaction cannot be inspected.
5. Do not count duplicated representations twice. In particular, `provider_request` may repeat fields already normalized under `prompt`.
6. Prefer each attempt's exported `context_budget` metrics. For older exports without that field, derive the same measures from `prompt` and state that they were reconstructed during review.

### Reconstruct the trajectory

- Recover the user goal, constraints, acceptance criteria, stages, decisions, branches, retries, interruptions, and final outcome.
- Build a chronological tool-call trace containing call purpose, arguments, returned result, agent interpretation, subsequent action, status, and contribution to the outcome.
- Connect each provider attempt to the exact system prompt, user/history messages, tool schemas, model, effort, response, and following observable action.
- Identify where the trajectory first diverged from the shortest reliable path.

### Diagnose failures and tool feedback

For every failed, invalid, cancelled, retried, or ineffective attempt, identify the immediate symptom, root cause, recovery behavior, avoidability, and a regression case.

Audit tool results for:

- errors presented as success, empty or partial results treated as complete, stale state, ambiguous exit status, truncation, persisted-output placeholders, and missing verification;
- outputs whose wording or schema encouraged a wrong conclusion;
- correct outputs that the agent misread, ignored, or contradicted later;
- redundant calls, overly broad reads/searches, premature writes, slow feedback loops, and repeated calls that produced no new evidence.

Separate tool-design defects from agent-interpretation defects. Cite the result field and the later evidence that confirms the mismatch.

### Quantify context use

Always include a `Context budget` table. Report these measures for each session and, where possible, each attempt:

1. Actual context occupancy: `token_usage.contextTokens / token_usage.contextLimit × 100%`. Mark it unavailable when either value is `empty` or the limit is zero.
2. Tool-result prompt share: `context_budget.tool_result_share_percent` with its character numerator and `denominator_chars`.
3. Largest-result share: the five entries in `context_budget.largest_tool_results`, including path, character count, and prompt share.
4. Prompt component share: `system_share_percent`, `history_share_percent`, `tool_schema_share_percent`, and `tool_result_share_percent`.

Label character-based percentages as `character-share proxy`, keep numerator and denominator visible, and avoid presenting them as tokenizer-accurate. Call out any single result above 10%, all tool results above 35%, repeated payloads, and content retained after it stopped influencing decisions. Explain what should be summarized, paged, persisted by reference, filtered, or removed.

### Evaluate compaction continuity

At every compaction boundary:

1. Compare the pre-compaction goals, constraints, decisions, unresolved work, file/tool evidence, and acceptance criteria with the exported post-compaction context.
2. Trace later attempts for signs of forgetting: repeated discovery, reversed decisions, lost constraints, duplicated work, missing verification, invented state, or requests for information already known before compaction.
3. Classify each important item as preserved, weakened, lost, or incorrectly transformed.
4. Distinguish a handoff-generation defect from a post-compact reasoning defect.
5. Report the first downstream consequence and propose the exact handoff field or reminder that would prevent recurrence.

When compaction evidence is encrypted or partial, state which comparison cannot be proven and use later behavior only as inference.

### Evaluate harness and prompts

Assess:

- harness sequencing, tool availability and schemas, permissions, confirmation boundaries, result transport, retry policy, cancellation, runtime snapshots, compaction, and observability;
- system-prompt instruction priority, conflicts, redundancy, missing guardrails, and verification requirements;
- user-prompt specificity, acceptance criteria, hidden assumptions, and missing decision context;
- model and effort suitability for the observed task and failure mode;
- whether the final answer is supported by tool evidence and satisfies the original request.

### Required report

Produce these sections:

1. `Verdict`: outcome, capture quality, and the three highest-impact findings.
2. `Trajectory`: chronological table of attempts and decisive tool calls.
3. `Failure cases`: symptom, cause, recovery, prevention, evidence, and regression test.
4. `Tool-result audit`: misleading outputs and agent misinterpretations.
5. `Context budget`: required percentages, largest contributors, and reduction actions.
6. `Compaction continuity`: boundary-by-boundary preservation and forgetting analysis.
7. `Prompt and harness findings`: system prompt, user prompt, tools, model/effort, and evaluation.
8. `Prioritized changes`: P0–P2 changes with owner surface, expected impact, implementation sketch, and measurable acceptance metric.
9. `Improved run`: a reproducible tool/prompt sequence and a compact regression suite.

Prefer a small number of causal findings over a long list of stylistic observations. Every priority item must connect to an observed failure or measurable context cost.
