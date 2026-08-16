import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function read(path: string): string {
  return readFileSync(path, "utf8");
}

describe("Unity YAML Property Tree", () => {
  it("publishes one asset-qualified progressive read DSL", () => {
    const schema = JSON.parse(read("tools/unity_yaml_read.json"));
    expect(schema.parameters.required).toEqual(["path"]);
    expect(schema.parameters.properties.path).toBeDefined();
    expect(schema.parameters.properties.depth.maximum).toBe(4);
    expect(schema.parameters.properties.file_path).toBeUndefined();
    expect(schema.parameters.properties.detail).toBeUndefined();
    expect(schema.description).toContain("4,000 characters");
    expect(schema.description).toContain("live Editor object graph first");
    expect(schema.description).toContain("all non-array descendants");
    expect(schema.description).toContain("at most four items");
    expect(schema.description).toContain("Subasset");
    expect(schema.description).toContain("never returns a whole raw YAML file");
  });

  it("uses the same path DSL for search results and read targets", () => {
    const schema = JSON.parse(read("tools/unity_yaml_search.json"));
    expect(schema.parameters.required).toEqual(["path", "query"]);
    expect(schema.description).toContain("passed to unity_yaml_read unchanged");
    expect(schema.description).toContain("shallowest matching node");
    expect(schema.description).toContain("explicit evidence");

    const implementation = read(
      "src-tauri/src/unity_serialized_property/property_tree.rs",
    );
    expect(implementation).toContain("pub struct PropertyTreePath");
    expect(implementation).toContain("AGENT_PROPERTY_TREE_ARRAY_LIMIT: usize = 4");
    expect(implementation).toContain(
      "AGENT_PROPERTY_TREE_AUTO_EXPAND_CHAR_LIMIT: usize = 4_000",
    );
    expect(implementation).toContain("read_complete_within_budget");
    expect(implementation).toContain("record_node");
    expect(implementation).toContain("pub fn search(");
    expect(implementation).toContain("pub async fn search_live_property_tree(");
    expect(implementation).toContain("pub fn read(");

    const agent = read("src-tauri/src/agent/instance/mod.rs");
    expect(agent).toContain("unity_property_tree_auto_expand_char_limit");
    const readFlow = agent.slice(
      agent.indexOf("async fn execute_unity_property_tree_read"),
      agent.indexOf("fn unity_property_tree_search_options"),
    );
    expect(readFlow).toContain("read_live_property_tree_with_limits");
    expect(readFlow).toContain("read_complete_within_budget(&path, auto_expand_limit)");
    expect(
      readFlow.indexOf("read_live_property_tree_with_limits"),
    ).toBeLessThan(
      readFlow.indexOf("read_complete_within_budget(&path, auto_expand_limit)"),
    );
    expect(agent).toContain("async fn execute_unity_property_tree_search");
    expect(agent).toContain("search_live_property_tree");
  });

  it("hosts the Unity bridge implementation in the PropertyTree file", () => {
    expect(existsSync("locus_unity/Editor/LocusBridge.PropertyTree.cs")).toBe(true);
    expect(existsSync("locus_unity/Editor/LocusBridge.ViewBindings.cs")).toBe(false);
    const bridge = read("locus_unity/Editor/LocusBridge.PropertyTree.cs");
    expect(bridge).toContain("HandlePropertyTreeRead");
    expect(bridge).toContain("property_tree_read");
    expect(bridge).toContain("referenceTarget");
  });

  it("keeps GameObject semantics and compact Unity values in the shared tree", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.PropertyTree.cs");
    expect(bridge).toContain("BuildPropertyTreeDisplaySections");
    expect(bridge).toContain("BuildPropertyTreeHierarchyDisplaySection");
    expect(bridge).toContain("BuildPropertyTreeTransformDisplaySection");
    expect(bridge).toContain("BuildPropertyTreeSceneSnapshot");
    expect(bridge).toContain("BuildPropertyTreeSceneHierarchyNode");
    expect(bridge).toContain("TryDiscoverPropertyTreeScene");
    expect(bridge).toContain("DiscoverPropertyTreeGameObjectHierarchy");
    expect(bridge).toContain(
      "CollectPropertyTreeGameObjectObjectRootsDiscoverMatches",
    );
    expect(bridge).toContain("PropertyTreeObjectSearchEvidence");
    expect(bridge).toContain("PropertyTreeDiscoverTraversalState");
    expect(bridge).toContain("TryDiscoverPropertyTreeAssetWithSubassets");
    expect(bridge).toContain("BuildPropertyTreeSubassetRecords");
    const subassetOwnership = bridge.slice(
      bridge.indexOf(
        "private static List<PropertyTreeSubassetRecord> BuildPropertyTreeSubassetRecords",
      ),
      bridge.indexOf(
        "private static string PropertyTreeUniqueSubassetSegment",
      ),
    );
    expect(subassetOwnership).toContain("cursor.Next(enterChildren)");
    expect(subassetOwnership).toContain(
      "SerializedPropertyType.ObjectReference",
    );
    expect(subassetOwnership).toContain("cursor.objectReferenceValue");
    expect(subassetOwnership).not.toContain("GetFields(");
    expect(bridge).toContain('nodeKind = "hierarchy"');
    const sceneDiscover = bridge.slice(
      bridge.indexOf("private static bool TryDiscoverPropertyTreeScene"),
      bridge.indexOf("private static PropertyTreeSearchFieldSet BuildPropertyTreeSearchFieldSet"),
    );
    expect(sceneDiscover).toContain(
      "CollectPropertyTreeGameObjectNodeDiscoverMatches",
    );
    const objectRootDiscover = bridge.slice(
      bridge.indexOf(
        "private static void CollectPropertyTreeGameObjectObjectRootsDiscoverMatches",
      ),
      bridge.indexOf(
        "private static PropertyTreeTarget PropertyTreeHierarchyGameObjectTarget",
      ),
    );
    expect(objectRootDiscover).toContain("go.GetComponents<Component>()");
    expect(objectRootDiscover).toContain(
      "CollectPropertyTreeObjectDiscoverMatches",
    );
    expect(bridge).not.toContain("properties.Add(BuildPropertyTreeHierarchySnapshot");
    expect(bridge).toContain("PropertyTreeGameObjectStaticPropertyPath");
    expect(bridge).toContain('"Source Prefab: "');
    expect(bridge).toContain('"Script"');

    const snapshots = read(
      "locus_unity/Editor/LocusBridge.SerializedProperties.cs",
    );
    expect(snapshots).toContain("IsSerializedPropertyCompactValue");
    expect(snapshots).toContain("SerializedObjectReferenceDisplay");
    expect(snapshots).toContain('return "None";');

    const projection = read(
      "src-tauri/src/unity_serialized_property/property_tree.rs",
    );
    expect(projection).toContain("is_compact_unity_value");
    expect(projection).toContain("compact_yaml_unity_value");
    expect(projection).toContain("format_display_sections");
    expect(projection).toContain("format_subassets");
    expect(projection).toContain("serialized_local_reference_order");
    expect(projection).toContain("entry.children.as_slice()");
    expect(projection).toContain("if !child.name.is_empty()");

    const agent = read("src-tauri/src/agent/instance/mod.rs");
    expect(agent).not.toContain("editor_eligible && !hierarchy_outline");
  });

  it("exposes the shared Property Tree formatter to unity_execute", () => {
    const api = read("locus_unity/Editor/LocusPropertyTree.cs");
    const context = read(
      "locus_unity/Editor/ExecuteCodeAsync/LocusBridge.ExecuteCodeAsync.cs",
    );
    const definition = JSON.parse(read("tools/unity_execute.json"));

    expect(api).toContain("public static class LocusPropertyTree");
    expect(api).toContain("FormatPropertyTreeForExecute");
    expect(context).toContain("public string PropertyTree(");
    expect(definition.description).toContain(
      "ctx.PropertyTree(target, depth, maxArrayItems)",
    );
    expect(definition.description).toContain("LocusPropertyTree.Format");
  });

  it("removes the list tool while retaining historical session restoration", () => {
    expect(existsSync("tools/unity_yaml_list.json")).toBe(false);
    const builtins = read("src-tauri/src/tool/builtins/mod.rs");
    expect(builtins).not.toContain("unity::unity_yaml_list()");
    const prompt = read("src-tauri/src/prompt.rs");
    expect(prompt).not.toContain("UNITY_YAML_LIST");
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    expect(bridge).not.toContain('case "list_yaml"');
    const devConfig = JSON.parse(read("agent/dev/config.json"));
    expect(devConfig.tools).not.toContain("unity_yaml_list");
    const compact = read("src-tauri/src/compact.rs");
    expect(compact).toContain(
      "exact `unity_yaml_list` result preserved from the pre-compact tool output",
    );
    const filesystem = read("src-tauri/src/tool/builtins/filesystem.rs");
    expect(filesystem).toContain("Direct raw reads are disabled for Unity YAML asset");
    expect(filesystem).not.toContain("repeat the same `read` call once more");
  });
});
