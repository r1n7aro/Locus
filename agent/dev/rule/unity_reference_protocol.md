## Unity Reference Protocol

The conversation UI parses the formats below to render clickable references and inline field editors. Follow them exactly in user-facing replies.

References:

* When referencing Unity assets, folders, ProjectSettings files, workspace files, or GameObjects in user-facing replies, wrap the full project-relative path with single backticks, such as `` `Assets/...` ``, `` `Packages/...` ``, or `` `ProjectSettings/...` ``. Do not add `{}` or a leading `@`.
* Use the default backticked path form for inline Unity references, such as `` `Assets/Prefabs/Player.prefab` ``.
* When a Unity reference needs more space, put the display format before the path inside the same backticks: `` `asset:row Assets/Prefabs/Player.prefab` `` for a full-row reference, `` `asset:preview Assets/Models/Hero.fbx` `` for a compact preview, or `` `asset:inspector Assets/Data/Enemy.asset` `` for an inspector-style block.
* Use a full-row Unity reference for editable assets or objects. Editable references must not use the inline form because the UI needs room for edit state and controls.
* When referencing GameObjects inside a Unity scene, use the loaded scene asset path followed by the exact hierarchy path, such as `` `Assets/Scenes/Main.unity/Environment/SpawnPoint` ``. Use exact Hierarchy names and slashes between parent/child objects so the UI can select the scene object or open it in an Inspector. Unity allows repeated sibling names; when a sibling name is repeated, use the Unity YAML 1-based ordinal suffix from the hierarchy path, such as `Enemy[1]` for the first `Enemy` sibling and `Enemy[2]` for the second.
* For interactive references, always output the full backticked project-relative path. Do not use shorthand because the UI cannot recover omitted path segments.

Editable fields (`unity_property`):

* When you change, inspect, or recommend user-adjustable Unity serialized fields, include those exact fields in a fenced `unity_property` block so the conversation UI can show compact field editors. This is required for tunable gameplay and presentation values such as speed, damage, health, cooldown, acceleration, spawn rate, volume, color, material, object reference, active/enabled state, and Transform values.
* If you already wrote the values successfully, still include a `unity_property` block for the modified fields when the user may naturally want to fine-tune them after your handoff. Keep the prose short, then provide the editable field targets.
* Only include fields whose target object and exact serialized `propertyPath` are known. Do not expose `m_Script`, internal bookkeeping fields, or guessed scene targets. If a value is controlled only by code and has no serialized field, state that briefly instead of emitting `unity_property`.
* In `unity_property`, write one field target per line. Use `object-path#propertyPath` for main asset fields, `scene-or-prefab-path/object-hierarchy#GameObject:propertyPath` for GameObject fields, and `scene-or-prefab-path/object-hierarchy#ComponentType:propertyPath` for component fields. Component type can be a short Unity type name or full name, such as `Transform` or `UnityEngine.Transform`. A bare `#propertyPath` on a scene or prefab object binds to the GameObject, so component serialized fields must include the component selector.
* When changing serialized data that could live on both a Prefab asset and a scene instance, compare the scene instance value with its Prefab source before choosing the target. If the scene instance already differs from the Prefab source for that field, modify and emit the scene object target. If the scene instance matches the Prefab source, modify and emit the Prefab target.
* For scene objects in `unity_property`, use the same Unity YAML hierarchy path and 1-based ordinal suffix rules as scene object references. If you cannot identify a unique hierarchy path for a scene object that may have repeated names, warn the user before offering editable fields that the edit target is unreliable and ask for a more specific Inspector/frontend-generated reference. Keep Unity fileID details internal; do not print fileID in agent-visible references or `unity_property` blocks.
* Use exact Unity serialized `propertyPath` values in `unity_property`, such as `m_Name`, `m_IsActive`, `m_LocalPosition`, or `m_Materials.Array.data[0]`. Do not include prose inside the code block.
* Example:
  ```unity_property
  Assets/Data/Enemy.asset#m_Name
  Assets/Data/Enemy.asset#damage
  Assets/Scenes/Main.unity/Environment/SpawnPoint[1]#GameObject:m_IsActive
  Assets/Scenes/Main.unity/Environment/SpawnPoint[1]#UnityEngine.Transform:m_LocalPosition
  Assets/Characters/Player/Player.prefab/Player#PlayerPlatformerController:maxMoveSpeed
  ```
