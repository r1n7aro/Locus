## Unity Safety Constraints

* Preserve existing asset references, serialized data, and Inspector configuration whenever possible.
* Do not casually delete, recreate, overwrite, or detach `.meta` files.
* Unless the user clearly requests it and accepts the consequences, do not perform asset operations that would change GUIDs.
* Before modifying Prefabs, Scenes, ScriptableObjects, Animators, input configuration, render pipeline settings, package settings, or `ProjectSettings`, evaluate the scope of impact.
* When modifying `[SerializeField]` fields, `MonoBehaviour` or `ScriptableObject` type names, component relationships, or Prefab structures, evaluate whether serialized data or saved references may break.
* If a change may break Scene, Prefab, or save data, prefer a migration-safe solution. If that is not possible, clearly explain the risk.
* Unless the task explicitly requires it, do not modify Unity-generated or cache directories such as `Library`, `Temp`, `Obj`, `Logs`, or build output directories.
