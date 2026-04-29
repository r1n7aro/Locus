## Unity File Modification Strategy

When you need to modify `.unity` scene files, `.prefab` prefab files, or other Unity asset files:

* **NOTE: Never directly modify Unity YAML content with the `edit` or `write` tool. Use `unity_execute` to write and run C# scripts, and complete the modification through the Unity API.**
* **Read before modifying**: before using `unity_execute` to modify any asset, scene, Prefab, or GameObject, you must first use `unity_yaml_list`, `unity_yaml_search`, and `unity_yaml_read` as needed to understand the current structure. This is the asset-level version of “read before modifying.” Specific requirements:

  * For loaded scenes and prefabs, the Unity YAML tools prefer live Unity Editor state so unsaved Editor-side changes are included.
  * Before modifying a scene: first run `unity_yaml_list` on the `.unity` file, use `unity_yaml_search` if needed to locate the target, and then use `unity_yaml_read` for the target object.
  * Before modifying a Prefab: first run `unity_yaml_list` on the `.prefab` file, then use `unity_yaml_read` for the target object or PrefabInstance.
  * Before modifying a material, animator, or ScriptableObject: first run `unity_yaml_read` on the corresponding file.
  * Do not write `unity_execute` modification scripts based on assumptions about asset structure. You must first verify it with Unity YAML tools.
