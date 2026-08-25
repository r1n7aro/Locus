## Unity File Modification Strategy

When you need to modify `.unity` scene files, `.prefab` prefab files, or other Unity asset files:

* **NOTE: Never directly modify Unity YAML content with the `edit` or `write` tool. Use `unity_execute` to write and run C# scripts, and complete the modification through the Unity API.**
* **Read before modifying**: before using `unity_execute` to modify an asset, scene, Prefab, or GameObject, inspect Unity text-serialized YAML assets through `unity_yaml_read`. Start at the asset path, follow returned child paths, and use `unity_yaml_search` when the target path is unknown. Inspect importer and binary assets outside `unity_yaml_read` coverage by loading them through a read-only `unity_execute` Editor script. This is the asset-level version of “read before modifying.” Specific requirements:

  * For loaded scenes and prefabs, the Unity YAML tools prefer live Unity Editor state so unsaved Editor-side changes are included.
  * Before modifying a scene or Prefab: call `unity_yaml_read` with the `.unity` or `.prefab` asset path to inspect its compact hierarchy or bounded outline. Continue with the returned GameObject, component, or property path until the edit target and its relevant context are verified.
  * Before modifying a material, animator, or ScriptableObject: call `unity_yaml_read` with the asset path, then follow returned paths when the required field is outside the first projection.
  * Before modifying an importer or binary asset such as `.fbx`, a texture, or audio: use `unity_execute` to load the imported Unity object and print the relevant serialized or typed properties without changing it.
  * Use `unity_yaml_search` within the narrowest known asset or subtree path when a large Property Tree does not reveal the target directly. Pass the exact returned result path to `unity_yaml_read` before editing.
  * Do not write `unity_execute` modification scripts based on assumptions about asset structure. You must first verify it with Unity YAML tools.
  * `unity_yaml_read` reports a GameObject's position and size in world space, while the Transform values you write through the Unity API or serialized fields are local space. Account for parent and ancestor Transforms when converting between what you read and what you write.
