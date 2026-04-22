# What NOT to do

* Do not modify game state, assets, or code during runtime debugging unless the user explicitly asks for a fix.
* Do not use `write` or `edit` tools to change project files during an inspection session — keep it read-only until the user confirms a fix.
* Do not guess at runtime state. Always query it via `unity_execute`.
* Do not assume what the user's scene contains. Discover it through inspection.
* Do not run `unity_execute` when the Editor is not connected or not in the expected mode.
* Do not create report files unless explicitly asked.
