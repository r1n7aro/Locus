---
title: Unity YAML Read Extensions
tools:
  - skill_list
  - skill_reload
  - unity_yaml_read
---

# Unity YAML Read Extensions

Workflow for adding a typed reader to the built-in `unity_yaml_read` tool through a Skill package. A registered extension intercepts reads of matching YAML assets and replaces the default raw-field output with output produced by package C# running inside the connected Unity Editor.

## When the extension runs

- Dispatch applies to non-hierarchical YAML assets only: `.asset`, `.mat`, `.controller`, `.anim`, and similar single-asset files. Scene and prefab reads (`.unity`, `.prefab`) keep the existing editor live path.
- Dispatch is skipped when `detail` is `"document"` or `"prefab_overrides"`; `detail: "document"` stays the escape hatch for raw default output.
- The Unity Editor must be connected. When a matching extension cannot run (editor disconnected, compile error, invoke error, empty output), `unity_yaml_read` falls back to the default output and appends a `Note:` line naming the extension and the reason.
- Match resolution: every YAML document in the file is checked; `scriptGuids` matches beat `classIds` matches, then earlier documents beat later ones, then the first registered package wins. Keep GUID sets disjoint across packages.
- Successful extension output is prefixed with `[unity_yaml_read extension '<name>' · Skill package '<id>']` so the source is always visible.
- The extension is active while its Skill package is installed and, for plugin-owned packages, while the plugin is enabled.

## Manifest format

Declare extensions in `skill.json` at the top level, next to `tools`:

```json
{
  "unityYamlReadExtensions": [
    {
      "name": "dialogue-asset",
      "match": { "scriptGuids": ["0123456789abcdef0123456789abcdef"] },
      "path": "unity/Editor/DialogueAssetReader.cs",
      "entryType": "DialogueAssetReader",
      "method": "Read",
      "description": "Typed reader for DialogueAsset ScriptableObjects."
    },
    {
      "name": "animator-controller",
      "match": { "classIds": [91] },
      "path": "unity/Editor/AnimatorControllerReader.cs"
    }
  ]
}
```

- `path` is required: a package-relative `.cs` file.
- `match` requires at least one of `scriptGuids` (32-char lowercase hex, validated) or `classIds` (Unity class ids).
- `name` defaults to the file stem of `path`; `entryType` defaults to the file stem; `method` defaults to `Read`.
- Manifest validation runs on package load; a bad entry fails the whole package, so confirm with `skill_reload` after edits.

## Choosing the matcher

- ScriptableObject and MonoBehaviour assets serialize as class id 114 with an `m_Script` GUID. Match them with `scriptGuids`; read the GUID from the script's `.meta` file. Do not register `classIds: [114]` — it would capture every scripted asset in the project.
- Built-in YAML asset types carry no `m_Script`, so `scriptGuids` can never match them. Use `classIds` instead, for example Material 21, AnimationClip 74, AnimatorController 91. Look up ids in the Unity class id reference.
- Out of scope: binary and importer-based assets (textures, models, audio) and objects inside `unity_builtin_extra` are not YAML files on disk, so `unity_yaml_read` never reaches them.

## C# reader contract

- Extension sources compile through the in-memory Skill package pipeline together with the rest of the package C# (`unity/Editor/**/*.cs`, `capabilities.unity` paths, unity-runtime tool paths, and every `unityYamlReadExtensions.path`). Nothing is copied into the user's project; compilation is cached by source hash.
- The entry point is a static method on `entryType`. Accept exactly one parameter: either `string` (raw args JSON) or a serializable class deserialized from the args JSON.
- Return a `string`; it becomes the tool output. Empty output counts as failure and triggers fallback. Thrown exceptions surface their message in the fallback `Note:`.
- The method runs on the Editor main thread. Keep it fast and side-effect free; respect `maxFieldDepth` and `maxArrayItems` when expanding nested data.

Args payload fields:

| Field | Meaning |
| --- | --- |
| `filePath` | Path exactly as passed to `unity_yaml_read` |
| `absPath` | Absolute path with forward slashes |
| `assetPath` | Project-relative path such as `Assets/Data/Foo.asset`, or null outside the project |
| `maxFieldDepth` | Requested field depth, 1-6, default 2 |
| `maxArrayItems` | Requested array item limit, 1-200, default 20 |
| `matchedClassId` | Class id of the matched YAML document |
| `matchedScriptGuid` | `m_Script` GUID hex of the matched document, or null |
| `matchedFileId` | fileID of the matched document |

Example reader:

```csharp
using System.Text;
using UnityEditor;
using UnityEngine;

public static class DialogueAssetReader
{
    [System.Serializable]
    public class ReadArgs
    {
        public string filePath;
        public string absPath;
        public string assetPath;
        public int maxFieldDepth;
        public int maxArrayItems;
        public int matchedClassId;
        public string matchedScriptGuid;
        public long matchedFileId;
    }

    public static string Read(ReadArgs args)
    {
        var asset = AssetDatabase.LoadMainAssetAtPath(args.assetPath);
        if (asset == null)
            return "Asset could not be loaded: " + args.assetPath;

        var sb = new StringBuilder();
        sb.AppendLine(asset.name + " (" + asset.GetType().FullName + ")");
        var so = new SerializedObject(asset);
        var prop = so.GetIterator();
        bool enterChildren = true;
        while (prop.NextVisible(enterChildren))
        {
            enterChildren = prop.depth < args.maxFieldDepth;
            sb.Append(' ', (prop.depth + 1) * 2);
            sb.AppendLine(prop.displayName + ": " + prop.propertyType);
        }
        return sb.ToString();
    }
}
```

## Authoring workflow

1. Pick the target type. For a ScriptableObject, copy the script GUID from its `.cs.meta`; for a built-in type, find its class id.
2. Add the `unityYamlReadExtensions` entry to `skill.json` and the reader `.cs` under `unity/Editor/` in the Skill package.
3. Validate with `skill_reload`; fix any manifest errors it reports.
4. Test with `unity_yaml_read` on a matching asset while the Unity Editor is connected. Confirm the output starts with the `[unity_yaml_read extension ...]` header; if a `Note:` fallback appears instead, fix the reported compile or invoke error.
5. Package and publish as usual — the extension travels inside the Skill package. During the portability audit, record any project types the reader depends on in `dependencies.project`.
