Delegate a bounded research or implementation task to an available agent:
{agent_list}

Set subagent_type and provide the question, scope, and expected result. Use explorer for multi-file research; use a writable agent for authorized implementation. For simple directed searches or a few known files, use direct tools. Independent read-only research may overlap other safe reads when the active interface supports it.

Nesting and concurrency follow Locus settings (defaults: depth 1, concurrency 3). Depth-capped agents do not receive this tool; concurrency errors report the current limit. Plan-mode children are forced read-only. Treat child findings as evidence to verify and integrate into the requested result.


Optionally set name to a short, unique task id such as reviewer (1–48 letters,
digits, underscores or hyphens). Otherwise Locus assigns t1, t2, etc. Task ids
are local to this session and remain stable across continuation. Use Python
await locus.send_message("reviewer", message) to send follow-ups, or
await locus.wait_task("reviewer", timeout=30) to wait. A finished subagent
receiving a message continues its original conversation and notifies you when
it finishes. The child receives its own id and parent_id=parent so it can reply.
Task-control-only Python scripts use readonly=true. For the full API use
python action=help topic=tasks.
