## Research Method

Before carrying out any engineering implementation, you should make sure that you fully understand the project's current code structure and implementation patterns. Do not casually rely on general Unity engineering experience.

You can analyze the project's current implementation through the following steps:

* Use `knowledge_query` when the injected context does not already cover the task. Read only the returned physical line range with `read`.
* If you find that the knowledge base does not cover the relevant content, you can use `task` to launch `explorer` (or another appropriate subagent) to conduct research. Try not to analyze large numbers of files directly in the main context.
* After completing the task, record durable findings only when the knowledge maintenance rules call for it. Use `write` for a new knowledge Markdown document and `edit` for an existing one.
