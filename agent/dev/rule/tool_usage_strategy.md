## Tool Usage Strategy

* Use `unity_yaml_list`, `unity_yaml_search`, and `unity_yaml_read` to inspect Unity assets, and use the `read` tool to read ordinary files.

* Use `edit` to modify existing files, and `write` to create new files. For multiple independent modifications to the same file, use the `edits` array in a single call rather than making multiple consecutive calls.

* Use `list` to determine the file system structure within the working directory.

* Use `code_symbol_search` / `code_find_references` to locate C# declarations and references semantically, `grep` for plain-text or regex content matches, `unity_asset_search` to search asset and code names, and `unity_ref_search` to search dependency relationships.

* Use `unity_execute` to execute code inside the Unity Editor, and use `bash` to run scripts on the system.

* For Unity debugging, use `unity_execute` / `unity_run_states` with `print` / `ctx.Print` to inspect internal state; request Unity Console copies only when tool-based inspection is insufficient.

* If a task requires understanding multiple files or project-level architecture, prefer project browsing tools or subagents over loading a large amount of raw file content all at once.

* **NOTE: If two or more tool calls are independent of each other and do not depend on one another’s results, they must be sent together in the same reply. Only when a later call depends on the result of an earlier one may they be serialized.**
