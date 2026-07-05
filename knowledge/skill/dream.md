---
id: kd_skill_dream
type: skill
path: dream.md
title: Dream
injectMode: excerpt
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /dream
argumentHint: "[scope]"
tools:
  - knowledge_list
  - knowledge_query
  - knowledge_read
  - knowledge_edit
createdAt: 1783036800000
updatedAt: 1783036800000
---

# Dream

## Summary
Use when the user invokes `/dream` or explicitly asks to consolidate, clean up, or audit long-term Memory. Ignore requests that add new knowledge, answer project questions, or edit Design content.

## Content
## Instructions

Command arguments: `[scope]` optionally narrows the pass to one Memory document or directory (for example `memory/project-mistake-note.md`); default is every Memory document, project level and user level.

A dream pass is a dedicated consolidation session, separate from normal task work. End-of-run maintenance happens while the context is full of task noise; a dream pass starts clean, reads Memory as its primary subject, and converges it. Consolidation only removes, merges, or tightens — never invent new facts during a dream pass.

1. Inventory.
   - List all Memory documents in scope with `knowledge_list`, then read each one with `knowledge_read`, including its maintenance rules.
   - Note each document's approximate size before changes so the report can show the delta.
   - Skip documents whose AI edit mode is read-only. For documents in proposal mode, collect suggested edits for the report instead of writing.

2. Consolidate each document under its own maintenance rules.
   - The document's maintenance rules govern; the steps below apply only where the rules do not say otherwise.
   - Merge duplicate or conflicting entries into the latest verified conclusion.
   - Delete expired conclusions, one-off task residue, temporary investigation traces, and unsupported guesses.
   - Tighten wordy entries; keep each entry focused on a single lesson, fact, or constraint.
   - Enforce entry caps from the rules (for example, keep the list within 20 items) by merging or dropping the lowest-value entries.

3. Verify entries against the current project.
   - Spot-check entries that reference concrete paths, assets, systems, or constraints: confirm the referenced thing still exists and the stated conclusion still holds. Use project search and `knowledge_query` where needed.
   - Rewrite stale entries to the current fact, or delete them when no longer relevant.
   - When an entry cannot be verified cheaply, keep it and append a short `(unverified as of <date>)` marker rather than guessing.

4. Check placement across documents.
   - Design intent found in Memory (requirements, gameplay decisions, product direction) belongs in Design. Do not write Design directly from a dream pass — Design changes go through proposals; list these entries in the report as suggested moves.
   - Project-specific habits found in user-level preferences belong in project Memory; move them when both documents are auto-editable, otherwise list the move in the report.

5. Apply and report.
   - Write consolidated content with `knowledge_edit` only to documents whose AI edit mode allows automatic editing.
   - Never delete a whole document; when one has become empty or redundant, say so in the report and let the user decide.
   - End with a short report: per document, the counts of merged, deleted, and rewritten entries with the before and after size; then the suggested Design moves, entries kept but marked unverified, and anything left for the user to handle.
