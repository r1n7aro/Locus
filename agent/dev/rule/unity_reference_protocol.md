## Unity Reference Protocol

The conversation UI parses the formats below to render clickable references. Follow them exactly in user-facing replies.

* When referencing Unity assets, folders, ProjectSettings files, workspace files, or GameObjects in user-facing replies, wrap the full project-relative path with single backticks, such as `` `Assets/...` ``, `` `Packages/...` ``, or `` `ProjectSettings/...` ``. Do not add `{}` or a leading `@`.
* Use the default backticked path form for inline Unity references, such as `` `Assets/Prefabs/Player.prefab` ``.
* When a Unity reference needs more space, put the display format before the path inside the same backticks: `` `asset:row Assets/Prefabs/Player.prefab` `` for a full-row reference, `` `asset:preview Assets/Models/Hero.fbx` `` for a compact preview, or `` `asset:inspector Assets/Data/Enemy.asset` `` for an inspector-style block.
* Use a full-row Unity reference for editable assets or objects. Editable references must not use the inline form because the UI needs room for edit state and controls.
* When referencing GameObjects inside a Unity scene, use the loaded scene asset path followed by the exact hierarchy path, such as `` `Assets/Scenes/Main.unity/Environment/SpawnPoint` ``. Use exact Hierarchy names and slashes between parent/child objects so the UI can select the scene object or open it in an Inspector. Unity allows repeated sibling names; when a sibling name is repeated, use the Unity YAML 1-based ordinal suffix from the hierarchy path, such as `Enemy[1]` for the first `Enemy` sibling and `Enemy[2]` for the second.
* For interactive references, always output the full backticked project-relative path. Do not use shorthand because the UI cannot recover omitted path segments.
