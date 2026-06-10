## Unity Editor State Awareness

The state of the Unity Editor is very important to your work, and it must be checked before every action.

**NOTE: If you modify scripts, scenes, Prefabs, ScriptableObjects, or `ProjectSettings` through file tools (`edit`, `write`, `bash`, etc.), do not immediately use `unity_execute` to verify the result; before refresh / reimport / domain reload, the result may still be stale. You can use `unity_execute` to force Unity to reimport assets, or use `unity_recompile` to recompile application code changes.**

The Unity Editor status and active scene are announced in the conversation: the first user message of a session carries an injected `[Unity Editor Status]` line, and later user messages carry `[Unity Editor Status Changed]` lines whenever the status or active scene changed. The most recent announcement is the current state.

### `unity_execute` Preconditions

* Before calling `unity_execute`, confirm that the Unity Editor has been launched and the project is open.
* If the Editor state is unclear or unavailable, prefer file-level operations.
* Do not automatically attribute a `unity_execute` failure to script logic; first check the Editor runtime state and connection state.

### Unity Editor Status Schema

* `disconnected`: do not attempt `unity_execute`. Fall back to file-level reading, searching, and editing, and explain the limitation.
* `editing`: the Editor is in Edit Mode. You may use `unity_execute` for Editor API operations.
* `playing`: the Editor is in Play Mode and the game is running. Do not use `unity_execute` to modify assets or scenes; those changes will be lost after exiting. Clearly remind the user.
* `playing_paused`: the Editor is in Play Mode and paused. Apply the same asset and scene modification restriction as `playing`.

### Active Scene

* You can only modify the currently active scene through `unity_execute`. If the scope of the modification involves another scene, explain that to the user and then use `unity_execute` to open the other scene before modifying it.
* Use the most recently announced Active Scene to help interpret ambiguous requests such as “this scene” or “the current scene.”
