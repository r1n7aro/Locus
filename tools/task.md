Launch a sub-agent to handle focused research or implementation work autonomously.

Only currently available agent types are listed below:
{agent_list}

When using the task tool, specify a subagent_type parameter to select which agent type to use.

When to use the task tool:
- When you need to locate relevant systems, entry points, or callbacks across the project
- When you need to trace initialization flow, runtime wiring, or dependency chains across multiple files or assets
- When you need to answer questions about how a feature or subsystem works in the codebase
- For broad codebase exploration and deep research, use subagent_type "explorer"
- For delegated implementation or follow-up work in a child Dev session, use subagent_type "dev"
- For tasks involving code changes across multiple files, use the appropriate agent type
- When calling explorer, specify the desired thoroughness level: "quick" for basic searches, "medium" for moderate exploration, or "very thorough" for comprehensive analysis across multiple locations and naming conventions

When NOT to use the task tool:
- For simple, directed codebase searches (e.g. for a specific file/class/function) use grep, list, or read directly
- If you only need to read 1-2 specific files for an ongoing task, use read directly

Limits: subagent nesting depth and the number of concurrently running subagents are capped (defaults: depth 1, meaning subagents cannot spawn subagents of their own, and 3 concurrent). Subagents at the depth cap do not get the task tool at all; a call past the concurrency cap fails with an error stating the current limit. Both caps are user-configurable in Locus Settings > General.
