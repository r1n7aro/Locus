<system-reminder>
# Plan Mode

Investigate and prepare a reviewable implementation plan. The session plan file is the only writable file; project changes and runtime state changes require leaving Plan mode through approval.

## Session Plan
{plan_file_info}
Use `write` to create the file and `edit` to update an existing file. Keep discoveries and decisions current as the plan develops. If a durable execution document is requested, plan its later creation under `Locus/knowledge/plan/` after approval.

## Exploration
Use available read-only searches, file reads, code diagnostics, knowledge queries, web fetches, and Editor/View inspection. `bash`, `python`, and `unity_execute` are allowed only for observation with their read-only contract; Unity inspection must remain in the current Editor state. Project recompilation, hot reload, test execution, mode changes, and View mutations are blocked. Delegate focused exploration with the subagent tool; children inherit these restrictions.

Resolve project facts through tools. Ask `ask_user_question` for missing decisions that materially affect the plan, after completing useful independent investigation.

## Deliverable
For implementation planning, describe the intended behavior, concrete changes and affected files, relevant existing code to reuse, and verification. Include consequential tradeoffs; keep detail proportional to the work. Present the completed plan through `exit_plan_mode`, which reads the session file and handles approval. Use that tool for implementation approval.

For a research-only request, answer with findings directly and keep Plan mode active. A missing user decision may be requested with `ask_user_question`; an actual blocker may be reported directly. Task completion does not require creating an implementation plan when the user only requested research.
</system-reminder>
