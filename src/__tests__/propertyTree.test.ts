import { describe, expect, it } from "vitest";
import { defineComponent, isVNode } from "vue";
import {
  createInspectorPropertyTreeBinding,
  createPropertyTree,
  createInspectorPropertyDrawerLibrary,
  defineInspectorPropertyDrawers,
  projectInspectorPropertyDrawerLibrary,
  propertyTreeService,
  publicInspectorPropertyDrawerLibrary,
  registerInspectorAttributeDrawer,
  registerInspectorPropertyDrawer,
  registerInspectorValueDrawer,
  resolveInspectorDrawer,
  type InspectorPropertySnapshot,
} from "../services/propertyTree";

function makeSnapshot(): InspectorPropertySnapshot {
  return {
    propertyPath: "root",
    displayName: "Root",
    type: "Generic",
    valueType: "Generic",
    editable: false,
    children: [
      {
        propertyPath: "root.enabled",
        displayName: "Enabled",
        type: "Boolean",
        valueType: "Boolean",
        value: true,
        editable: true,
      },
      {
        propertyPath: "root.tint",
        displayName: "Tint",
        type: "Color",
        valueType: "Color",
        value: "#ffffff",
        editable: true,
      },
      {
        propertyPath: "root.points",
        displayName: "Points",
        type: "Generic",
        valueType: "Generic",
        isArray: true,
        arraySize: 2,
        editable: true,
        children: [
          {
            propertyPath: "root.points.Array.data[0]",
            displayName: "Element 0",
            type: "Vector3",
            valueType: "Vector3",
            value: { x: 1, y: 2, z: 3 },
            editable: true,
          },
        ],
      },
    ],
  };
}

describe("propertyTree", () => {
  it("builds indexed inspector properties from Unity snapshots", () => {
    const tree = createPropertyTree(makeSnapshot());

    expect(tree.rootProperty?.label).toBe("Root");
    expect(tree.getProperty("root.enabled")?.parent?.propertyPath).toBe("root");
    expect(tree.getProperty("root.enabled")?.drawer.kind).toBe("boolean");
    expect(tree.getProperty("root.tint")?.drawer.kind).toBe("color");
    expect(tree.getProperty("root.points")?.drawer.kind).toBe("array");
    expect(tree.getProperty("root.points.Array.data[0]")?.drawer.kind).toBe("vector");
  });

  it("keeps Unity structured values editable when snapshots include component children", () => {
    const tree = createPropertyTree({
      propertyPath: "root.transform",
      displayName: "Transform",
      valueType: "Generic",
      children: [
        {
          propertyPath: "root.transform.m_LocalPosition",
          displayName: "Local Position",
          valueType: "Vector3",
          value: { x: 1, y: 2, z: 3 },
          editable: true,
          children: [
            { propertyPath: "root.transform.m_LocalPosition.x", displayName: "X", valueType: "Float", value: 1 },
            { propertyPath: "root.transform.m_LocalPosition.y", displayName: "Y", valueType: "Float", value: 2 },
            { propertyPath: "root.transform.m_LocalPosition.z", displayName: "Z", valueType: "Float", value: 3 },
          ],
        },
        {
          propertyPath: "root.transform.m_LocalRotation",
          displayName: "Local Rotation",
          valueType: "Quaternion",
          value: { x: 0, y: 0, z: 0, w: 1 },
          displayValue: "0, 0, 0",
          editable: true,
          children: [
            { propertyPath: "root.transform.m_LocalRotation.x", displayName: "X", valueType: "Float", value: 0 },
            { propertyPath: "root.transform.m_LocalRotation.y", displayName: "Y", valueType: "Float", value: 0 },
            { propertyPath: "root.transform.m_LocalRotation.z", displayName: "Z", valueType: "Float", value: 0 },
            { propertyPath: "root.transform.m_LocalRotation.w", displayName: "W", valueType: "Float", value: 1 },
          ],
        },
      ],
    });

    const position = tree.requireProperty("root.transform.m_LocalPosition");
    const rotation = tree.requireProperty("root.transform.m_LocalRotation");

    expect(position.drawer).toMatchObject({ kind: "vector", container: false, valueType: "Vector3" });
    expect(rotation.drawer).toMatchObject({ kind: "vector", container: false, valueType: "Quaternion" });
    expect(tree.requireProperty("root.transform").drawer.kind).toBe("object");
  });

  it("tracks selection, changed fields, validation messages, and commit events", () => {
    const tree = createPropertyTree(makeSnapshot(), {
      selectedPath: "root.tint",
      changedPaths: ["root.tint"],
      errors: { "root.tint": "Invalid color" },
    });
    const property = tree.requireProperty("root.tint");

    expect(property.state.selected).toBe(true);
    expect(property.state.changed).toBe(true);
    expect(property.state.error).toBe("Invalid color");
    expect(property.createCommit("#000000")).toMatchObject({
      propertyPath: "root.tint",
      value: "#000000",
      snapshot: { propertyPath: "root.tint" },
    });
  });

  it("filters visible properties by search while keeping matching ancestors", () => {
    const tree = createPropertyTree(makeSnapshot(), {
      searchQuery: "element 0",
    });

    expect(tree.getProperty("root")?.state.visible).toBe(true);
    expect(tree.getProperty("root.points")?.state.visible).toBe(true);
    expect(tree.getProperty("root.points.Array.data[0]")?.state.matchesSearch).toBe(true);
    expect(tree.getProperty("root.enabled")?.state.visible).toBe(false);
    expect(tree.visibleProperties().map((property) => property.propertyPath)).toEqual([
      "root",
      "root.points",
      "root.points.Array.data[0]",
    ]);
  });

  it("allows custom drawer resolvers and state updaters", () => {
    const tree = createPropertyTree(makeSnapshot(), {
      drawerResolvers: [
        (property) =>
          property.propertyPath === "root.tint"
            ? { kind: "unsupported", commitMode: "none", container: false, valueType: "Color" }
            : null,
      ],
      stateUpdaters: [
        (property) =>
          property.propertyPath === "root.enabled"
            ? { readonly: true, editable: false }
            : null,
      ],
    });

    expect(tree.getProperty("root.tint")?.drawer.kind).toBe("unsupported");
    expect(tree.getProperty("root.enabled")?.canEdit).toBe(false);
  });

  it("draws registered property components by serialized type", () => {
    const StatDrawer = defineComponent({ name: "StatDrawer" });
    const tree = createPropertyTree({
      propertyPath: "root.stat",
      displayName: "Stat",
      type: "Demo.Stat",
      valueType: "Demo.Stat",
      value: { current: 3, max: 10 },
      editable: true,
      tooltip: "Current stat value",
      hasRange: true,
      rangeMin: 0,
      rangeMax: 10,
      numberStep: 1,
      multiline: true,
      minLines: 2,
      maxLines: 4,
      referenceTypeFullName: "Demo.StatAsset",
      referenceTypeAssembly: "Assembly-CSharp",
      attributes: [
        { type: "RangeAttribute", displayName: "Range", value: "0..10" },
      ],
    }, {
      propertyDrawers: {
        "Demo.Stat": StatDrawer,
      },
    });
    const property = tree.requireProperty("root.stat");
    const commits: unknown[] = [];
    const drawn = property.draw({
      onCommit: (commit) => commits.push(commit),
    });

    expect(property.hasPropertyDrawer()).toBe(true);
    expect(property.propertyDrawerComponent()).toBe(StatDrawer);
    expect(isVNode(drawn)).toBe(true);
    expect(isVNode(drawn) ? drawn.type : null).toBe(StatDrawer);
    const props = isVNode(drawn) ? drawn.props as Record<string, unknown> : {};
    expect(props.propertyPath).toBe("root.stat");
    expect(props.propertyType).toBe("Demo.Stat");
    expect(props.tooltip).toBe("Current stat value");
    expect(props.hasRange).toBe(true);
    expect(props.rangeMax).toBe(10);
    expect(props.multiline).toBe(true);
    expect(props.referenceTypeFullName).toBe("Demo.StatAsset");
    expect(props.attributes).toMatchObject([{ type: "RangeAttribute" }]);
    expect(typeof props.commit).toBe("function");

    (props.commit as (value: unknown) => void)(5);

    expect(commits).toMatchObject([
      {
        propertyPath: "root.stat",
        value: 5,
        snapshot: { propertyPath: "root.stat" },
      },
    ]);
  });

  it("supports property drawer helpers, dynamic libraries, global registrations, and tree drawing", () => {
    const NumberDrawer = defineComponent({ name: "NumberDrawer" });
    const TextDrawer = defineComponent({ name: "TextDrawer" });
    const GlobalBoolDrawer = defineComponent({ name: "GlobalBoolDrawer" });
    const unregister = registerInspectorPropertyDrawer("Boolean", GlobalBoolDrawer);
    const propertyDrawers = createInspectorPropertyDrawerLibrary(defineInspectorPropertyDrawers([
      {
        type: "Integer",
        drawer: NumberDrawer,
      },
    ]));

    try {
      const tree = createPropertyTree([
        {
          propertyPath: "count",
          valueType: "Integer",
          value: 2,
          editable: true,
        },
        {
          propertyPath: "name",
          valueType: "String",
          value: "Player",
          editable: true,
        },
        {
          propertyPath: "enabled",
          valueType: "Boolean",
          value: true,
          editable: true,
        },
      ], {
        propertyDrawers,
      });

      expect(tree.requireProperty("count").propertyDrawerComponent()).toBe(NumberDrawer);
      expect(tree.requireProperty("name").propertyDrawerComponent()).toBe(null);
      propertyDrawers.register("String", TextDrawer);
      expect(tree.requireProperty("name").propertyDrawerComponent()).toBe(TextDrawer);
      expect(tree.requireProperty("enabled").propertyDrawerComponent()).toBe(GlobalBoolDrawer);
      const treeDrawn = tree.draw();
      expect(isVNode(treeDrawn)).toBe(true);
      expect(isVNode(treeDrawn) ? treeDrawn.type : null).toBe("div");
    } finally {
      unregister();
    }
  });

  it("matches property drawers by value, attribute, property path, and priority", () => {
    const RangeDrawer = defineComponent({ name: "RangeDrawer" });
    const FloatDrawer = defineComponent({ name: "FloatDrawer" });
    const PathDrawer = defineComponent({ name: "PathDrawer" });
    const registerFloat = registerInspectorValueDrawer("Float", FloatDrawer);
    const registerRange = registerInspectorAttributeDrawer("RangeAttribute", RangeDrawer, {
      valueType: "Float",
      priority: 10,
    });

    try {
      const tree = createPropertyTree([
        {
          propertyPath: "speed",
          valueType: "Float",
          value: 2,
          editable: true,
          attributes: [{ type: "UnityEngine.RangeAttribute", displayName: "Range" }],
        },
        {
          propertyPath: "damage",
          valueType: "Float",
          value: 5,
          editable: true,
        },
      ], {
        propertyDrawers: [
          {
            propertyPath: "damage",
            drawer: PathDrawer,
            priority: 20,
          },
        ],
      });

      expect(tree.requireProperty("speed").propertyDrawerComponent()).toBe(RangeDrawer);
      expect(tree.requireProperty("damage").propertyDrawerComponent()).toBe(PathDrawer);
    } finally {
      registerRange();
      registerFloat();
    }
  });

  it("uses the public draw library as the default cross-package draw surface", () => {
    const SharedDrawer = defineComponent({ name: "SharedDrawer" });
    const tree = createPropertyTree({
      propertyPath: "shared",
      valueType: "Generic",
      fieldTypeFullName: "Project.SharedStat",
      fieldTypeAssembly: "Assembly-CSharp",
      value: { current: 1 },
      editable: true,
    });
    const property = tree.requireProperty("shared");

    expect(property.propertyDrawerComponent()).toBe(null);
    expect(propertyTreeService.publicPropertyDrawerLibrary).toBe(publicInspectorPropertyDrawerLibrary);
    expect(projectInspectorPropertyDrawerLibrary).toBe(publicInspectorPropertyDrawerLibrary);

    const unregister = publicInspectorPropertyDrawerLibrary.register("Project.SharedStat", SharedDrawer);
    try {
      expect(property.propertyDrawerComponent()).toBe(SharedDrawer);
      expect(property.searchText).toContain("assembly-csharp");
      expect(isVNode(property.draw())).toBe(true);
    } finally {
      unregister();
    }
  });

  it("creates property tree bindings as the edit source and commit sink", async () => {
    const snapshot = makeSnapshot();
    const tree = createPropertyTree(snapshot);
    const property = tree.requireProperty("root.enabled");
    const commits: unknown[] = [];
    const binding = createInspectorPropertyTreeBinding({
      id: "demo-tree",
      targetId: "asset:demo",
      snapshots: snapshot,
      loading: true,
      commit: (commit) => {
        commits.push(commit);
      },
    });

    expect(binding.id).toBe("demo-tree");
    expect(binding.targetId).toBe("asset:demo");
    expect(binding.snapshots).toBe(snapshot);
    expect(binding.disabled).toBe(true);
    expect(binding.readonly).toBe(false);
    expect(binding.editable).toBe(true);

    await binding.commit(property.createCommit(false));

    expect(commits).toMatchObject([
      {
        propertyPath: "root.enabled",
        value: false,
        snapshot: { propertyPath: "root.enabled" },
      },
    ]);
    expect(propertyTreeService.createBinding({ snapshots: null }).snapshots).toBeNull();
  });

  it("resolves default drawer metadata without a tree", () => {
    const tree = createPropertyTree({
      propertyPath: "speed",
      valueType: "Float",
      editable: true,
    });
    const property = tree.requireProperty("speed");

    expect(resolveInspectorDrawer(property)).toEqual({
      kind: "number",
      commitMode: "blur",
      container: false,
      valueType: "Float",
    });
  });

  it("normalizes managed reference metadata and commands", () => {
    const tree = createPropertyTree({
      propertyPath: "root.behaviour",
      displayName: "Behaviour",
      valueType: "ManagedReference",
      editable: true,
      isManagedReference: true,
      managedReferenceFullTypename: "Game Demo.CurrentBehaviour",
      managedReferenceFieldTypename: "Game Demo.IBehaviour",
      managedReferenceDisplayName: "CurrentBehaviour",
      managedReferenceTypes: [
        {
          label: "OtherBehaviour",
          value: "Game Demo.OtherBehaviour",
          fullName: "Demo.OtherBehaviour",
          assembly: "Game",
        },
      ],
    });
    const property = tree.requireProperty("root.behaviour");

    expect(property.drawer.kind).toBe("managedReference");
    expect(property.selectedManagedReferenceType).toMatchObject({
      label: "CurrentBehaviour",
      value: "Game Demo.CurrentBehaviour",
      current: true,
      unavailable: true,
    });
    expect(property.searchText).toContain("demo.currentbehaviour");
    expect(tree.search("ibehaviour").map((item) => item.propertyPath)).toEqual(["root.behaviour"]);
    expect(property.searchManagedReferenceTypes("other")).toMatchObject([
      { value: "Game Demo.OtherBehaviour" },
    ]);
    expect(property.createManagedReferenceTypeCommit(property.managedReferenceTypes[1])).toMatchObject({
      value: {
        action: "setType",
        typeName: "Game Demo.OtherBehaviour",
        fullName: "Demo.OtherBehaviour",
        assembly: "Game",
      },
    });
    expect(tree.createManagedReferenceTypeCommit("root.behaviour", "")).toMatchObject({
      value: { action: "clear" },
    });
  });

  it("normalizes enum and flags metadata", () => {
    const tree = createPropertyTree({
      propertyPath: "root.mask",
      valueType: "Enum",
      editable: true,
      isFlagsEnum: true,
      enumValueIndex: -1,
      enumValueFlag: 5,
      enumOptions: [
        { label: "None", value: "0", name: "None", index: 0, numericValue: 0 },
        { label: "Read", value: "1", name: "Read", index: 1, numericValue: 1 },
        { label: "Write", value: "2", name: "Write", index: 2, numericValue: 4 },
      ],
    });
    const property = tree.requireProperty("root.mask");

    expect(property.drawer.kind).toBe("flags");
    expect(property.enumValueFlag).toBe(5);
    expect(property.enumOptions).toEqual([
      { label: "None", value: "0", name: "None", index: 0, numericValue: 0 },
      { label: "Read", value: "1", name: "Read", index: 1, numericValue: 1 },
      { label: "Write", value: "2", name: "Write", index: 2, numericValue: 4 },
    ]);
  });
});
