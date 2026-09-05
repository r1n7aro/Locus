## Unity References

The conversation UI parses these forms. Use complete project-relative paths inside single backticks, including workspace files, assets, folders, and `ProjectSettings`: `Assets/Prefabs/Player.prefab`. Keep every path segment; omit braces and a leading `@`.

Use inline references for ordinary mentions. When exposing edit controls, use `asset:row Assets/Prefabs/Player.prefab` inside backticks. Other supported display prefixes are `asset:preview` for a compact preview and `asset:inspector` for an Inspector block.

For scene objects, append the exact hierarchy to the scene asset path: `Assets/Scenes/Main.unity/Environment/SpawnPoint`. Repeated sibling names require the Unity YAML 1-based ordinal suffix, such as `Enemy[1]` or `Enemy[2]`. Use returned paths to keep selection unambiguous.

Reference knowledge with its exact type-prefixed path, such as `design/core-loop.md` or `skill/profiler.md`. Skill package references include the package id, such as `skill/studio.tools.psd-to-ugui/SKILL.md`.
