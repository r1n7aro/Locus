import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("scene object mention search", () => {
  it("searches the cached on-disk scene hierarchy and validates only on selection", () => {
    const input = read("src/components/chat/RichChatInput.vue");
    const assetService = read("src/services/asset.ts");
    const unityService = read("src/services/unity.ts");
    const assetCommands = read("src-tauri/src/commands/asset.rs");
    const workspaceCommands = read("src-tauri/src/commands/workspace.rs");
    const rustBridge = read("src-tauri/src/unity_bridge/mod.rs");
    const unityBridge = read("locus_unity/Editor/LocusBridge.cs");
    const unityTypes = read("locus_unity/Editor/LocusBridge.Types.cs");
    const app = read("src-tauri/src/lib.rs");
    const popup = read("src/components/chat/MentionPopup.vue");

    expect(input).toContain("searchWorkspaceSceneObjects");
    expect(input).toContain("projectStore.unityConnectionStatus?.scenePath");
    expect(input).toContain("await validateUnitySceneObject(workspaceRef, target.scenePath, target.objectPath)");
    expect(input).toContain('entryKind: "sceneObject"');
    expect(input).toContain("meta: relPath");
    expect(input).toContain("shouldContinueMentionWithSpace");
    expect(input).toContain("escapedMentionAnchor.value = activeOperator.value?.kind");
    expect(input).toContain('event.key === "Escape"');
    expect(assetService).toContain('"search_workspace_scene_objects"');
    expect(app).toContain("commands::search_workspace_scene_objects");
    expect(app).toContain("commands::validate_unity_scene_object");
    expect(assetCommands).toContain("parse_scene_hierarchy_objects");
    expect(assetCommands).toContain("build_hierarchy_path_map");
    expect(assetCommands).toContain("scene_hierarchy_build_lock");
    expect(assetCommands).toContain("spawn_blocking");
    expect(unityService).toContain('"validate_unity_scene_object"');
    expect(workspaceCommands).toContain("pub async fn validate_unity_scene_object");
    expect(rustBridge).toContain('send_message(project_path, "validate_scene_object"');
    expect(unityBridge).toContain('case "validate_scene_object"');
    expect(unityTypes).toContain("ResolveSceneObject(scenePath, objectPath);");
    expect(popup).toContain(':title="entry.meta || entry.parentPath"');
    expect(rustBridge).not.toContain('send_message(project_path, "search_scene_objects"');
  });
});
