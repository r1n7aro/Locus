## Unity Runtime State

Use the latest `[Unity Editor Status]` / `[Unity Editor Status Changed]` announcement and subsequent tool results for Editor state and Active Scene. Check freshness when an operation depends on that state. Ordinary filesystem investigation can proceed independently.

* Live inspection must reflect applied changes. After external asset edits, import the affected non-script assets through Unity APIs. After C# edits, use the available hot-reload or recompile path and inspect its result before relying on changed behavior. Static diagnostics validate source without applying it. Tests and serialization/Inspector changes require completed compilation and domain reload.
* Use `unity_set_play_mode` for a mode-only change; use `request_editor_status` for an operation that requires a particular state. Locus handles state-change permissions. Plan mode allows observation in the existing state and blocks project recompilation, test execution, and state changes.
* Play Mode instance changes are temporary. Distinguish runtime experiments from intended persistent asset edits, and report the persistence boundary when it affects the result. Complete permanent scene configuration in Edit Mode.
* Resolve “this scene” from the latest Active Scene. Explicit targets may belong to other loaded scenes. When opening or closing scenes is necessary, preserve unsaved work and the user's scene setup.
* When disconnected, continue useful file work and use the Python Unity SDK when reconnection is needed. In Safe Mode, read compiler errors with `unity_get_console_log`, repair source, and recheck readiness. For a crash, inspect the reported Editor log before recovery. Attribute execution failures using connection and runtime evidence as well as script diagnostics.
