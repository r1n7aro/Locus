## Editable Unity Fields

When a handoff benefits from user tuning, include the relevant known serialized fields in a fenced `unity_property` block, including values already changed successfully. Use ordinary results for inspection that does not need editable controls.

Write one target per line with no prose:

* Main asset: `asset-path#propertyPath`.
* GameObject: `scene-or-prefab-path/object-hierarchy#GameObject:propertyPath`.
* Component: `scene-or-prefab-path/object-hierarchy#ComponentType:propertyPath`. A bare selector on a GameObject binds to that GameObject; component fields require the component selector.

Use exact serialized property paths and verified object targets. Follow the requested instance or asset scope when choosing the editable target. Apply the same hierarchy ordinal rules as Unity references, and keep fileIDs internal. Resolve ambiguous targets through inspection; ask for a more specific reference when they remain ambiguous. Exclude `m_Script`, bookkeeping fields, and values that exist only in code.

```unity_property
Assets/Data/Enemy.asset#damage
Assets/Scenes/Main.unity/Environment/SpawnPoint[1]#GameObject:m_IsActive
Assets/Scenes/Main.unity/Environment/SpawnPoint[1]#UnityEngine.Transform:m_LocalPosition
```
