---
title: Unity YAML Read Extensions
tools:
  - skill_list
  - unity_yaml_read
---

# Unity YAML Read Extensions

Workflow for adding a typed reader to the built-in `unity_yaml_read` tool through a Skill package. A registered extension replaces the default Property Tree tool text for matching asset roots with text produced by package C# running inside the connected Unity Editor. It does not extend Property Tree nodes or automatically reuse Odin Inspector.

## When the extension runs

- Dispatch applies to root reads of non-hierarchical YAML assets: `.asset`, `.mat`, `.controller`, `.anim`, and other supported asset files. For example, `{"path":"Assets/Data/Action.asset"}` can invoke an extension. Scene and prefab reads (`.unity`, `.prefab`) use Property Tree.
- Child paths such as `Assets/Data/Action.asset/events/0` always use Property Tree, even if the asset has a matching extension. A root reader must not substitute whole-asset text for a child request. Explicit serialized field names, including hidden fields, remain addressable through the live Editor.
- Use `{"path":"Assets/Data/Action.asset","reader":"default"}` to bypass extensions and read the standard live Property Tree (or disk YAML fallback). `reader` defaults to `"auto"`. This returns a structured projection, not raw YAML. Legacy `file_path` calls also dispatch; legacy `detail: "document"` / `"prefab_overrides"` bypass extensions, but `detail` is no longer part of the published schema.
- The Unity Editor must be connected. When a matching extension cannot run (editor disconnected, compile error, invoke error, empty output), `unity_yaml_read` falls back to the default output and appends a `Note:` line naming the extension and the reason.
- Match resolution: saved YAML supplies identity only; every document in the file is checked. `scriptGuids` matches beat `classIds` matches, then earlier documents beat later ones, then the first registered package wins. Keep GUID sets disjoint across packages. If saved YAML is unavailable, the default live read still runs. Readers should load the matched object from the Editor to include unsaved values.
- Successful extension output is prefixed with `[unity_yaml_read extension '<name>' · Skill package '<id>']` and `[source: live Editor]`. Only the invoke response's `result` string is used; package/assembly metadata is not rendered.
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
- Manifest validation runs on package load; a bad entry fails the whole package, so confirm the refreshed package with `skill_list` after edits.

## Choosing the matcher

- ScriptableObject and MonoBehaviour assets serialize as class id 114 with an `m_Script` GUID. Match them with `scriptGuids`; read the GUID from the script's `.meta` file. Do not register `classIds: [114]` — it would capture every scripted asset in the project.
- Built-in YAML asset types carry no `m_Script`, so `scriptGuids` can never match them. Use `classIds` instead, for example Material 21, AnimationClip 74, AnimatorController 91. Look up ids in the Unity class id reference.
- Out of scope: binary and importer-based assets (textures, models, audio) and objects inside `unity_builtin_extra` are not YAML files on disk, so `unity_yaml_read` never reaches them.

## C# reader contract

- Extension sources compile through the in-memory Skill package pipeline together with the rest of the package C# (`unity/Editor/**/*.cs`, `capabilities.unity` paths, unity-runtime tool paths, and every `unityYamlReadExtensions.path`). Nothing is copied into the user's project; compilation is cached by source hash.
- The entry point is a static method on `entryType`. Accept exactly one parameter: either `string` (raw args JSON) or a serializable class deserialized from the args JSON.
- Return a `string`; it becomes the tool output. Return null, an empty string, or whitespace to decline the read and trigger the default Property Tree fallback. Non-string/malformed responses also trigger fallback. Thrown exceptions surface their message in the fallback `Note:`. The default read retains its live/disk source label and success/error status.
- The method runs on the Editor main thread. Keep it fast and side-effect free; respect `depth` and `maxArrayItems` when expanding nested data. These are the same normalized limits as the current tool, not the old 1–6 / 20–200 contract. A custom text reader owns its expansion; it does not receive Property Tree's automatic 4,000-character expansion pass. The normal tool output budget still applies.

Args payload fields:

| Field | Meaning |
| --- | --- |
| `path` | Normalized asset-qualified request path (root only) |
| `filePath` | Normalized asset file path, without any child suffix |
| `childPath` | Empty string; child requests bypass text extensions |
| `absPath` | Absolute path with forward slashes |
| `assetPath` | Project-relative path such as `Assets/Data/Foo.asset`, or null outside the project |
| `depth` | Child expansion depth, 0–4, default 2; 0 means root summary |
| `maxFieldDepth` | Compatibility alias of `depth`, also 0–4 (not the old 1–6 range) |
| `maxArrayItems` | Maximum displayed elements per array, 1–1024, default 4 |
| `matchedClassId` | Class id of the matched YAML document |
| `matchedScriptGuid` | `m_Script` GUID hex of the matched document, or null |
| `matchedFileId` | fileID of the matched document |

Example reader:

```csharp
using Locus;
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
        public int depth;
        public int maxArrayItems;
        public int matchedClassId;
        public string matchedScriptGuid;
        public long matchedFileId;
    }

    public static string Read(ReadArgs args)
    {
        if (string.IsNullOrEmpty(args.assetPath))
            return null;
        // A match may identify a subasset rather than the main asset.
        foreach (var asset in AssetDatabase.LoadAllAssetsAtPath(args.assetPath))
        {
            if (asset == null)
                continue;
            string guid;
            long fileId;
            if (AssetDatabase.TryGetGUIDAndLocalFileIdentifier(asset, out guid, out fileId)
                && fileId == args.matchedFileId)
            {
                // Replace this formatter with your domain summary, respecting
                // depth / maxArrayItems and reading current Editor values.
                return LocusPropertyTree.Format(asset, args.depth, args.maxArrayItems);
            }
        }
        return null;
    }
}
```

## Authoring workflow

1. Pick the target type. For a ScriptableObject, copy the script GUID from its `.cs.meta`; for a built-in type, find its class id.
2. Add the `unityYamlReadExtensions` entry to `skill.json` and the reader `.cs` under `unity/Editor/` in the Skill package.
3. Confirm the automatically refreshed package with `skill_list`; fix the manifest when the package is absent or its metadata is stale.
4. Test the root with `unity_yaml_read` while the Unity Editor is connected. Confirm the `[unity_yaml_read extension ...]` header. Compare `reader: "default"`, read an exact child path, and verify empty/null/throwing readers fall back with a `Note:`. Check defaults (depth 2, four items), depth 0, and larger requested array limits. If a `Note:` reports a compile/invoke error, fix that error.
5. Package and publish as usual — the extension travels inside the Skill package. During the portability audit, record any project types the reader depends on in `dependencies.project`.
