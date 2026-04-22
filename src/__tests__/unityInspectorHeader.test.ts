import { describe, expect, it } from "vitest";
import type { InspectorField, InspectorPanel } from "../types";
import {
  buildGameObjectHeaderSummary,
  parsePrefabSourceSummary,
} from "../components/diff/unityInspectorHeader";

function makeField(
  propertyPath: string,
  label: string,
  value: string,
): InspectorField {
  return {
    id: propertyPath,
    label,
    propertyPath,
    valueType: "string",
    changeKind: "unchanged",
    after: value,
    children: [],
  };
}

function makeGameObjectPanel(fields: InspectorField[]): InspectorPanel {
  return {
    panelKind: "gameObjectHeader",
    title: "GameObject",
    changeKind: "unchanged",
    added: false,
    removed: false,
    fields,
  };
}

describe("unityInspectorHeader", () => {
  it("extracts a prefab parent path from prefab instance subtitles", () => {
    const source = parsePrefabSourceSummary("Prefab Instance · Assets/Characters/Hero.prefab");

    expect(source).toEqual({
      kind: "prefab",
      path: "Assets/Characters/Hero.prefab",
      name: "Hero",
      extension: "prefab",
    });
  });

  it("identifies fbx parents explicitly", () => {
    const source = parsePrefabSourceSummary("Prefab Instance · Assets/Characters/Hero.fbx");

    expect(source?.kind).toBe("fbx");
    expect(source?.name).toBe("Hero");
  });

  it("builds a compact gameobject summary from the header panel", () => {
    const panel = makeGameObjectPanel([
      makeField("m_Name", "Name", "Teacher_Rig"),
      makeField("m_TagString", "Tag", "Untagged"),
      makeField("m_Layer", "Layer", "0"),
      makeField("m_IsActive", "Active", "1"),
      makeField("m_StaticEditorFlags", "Static", "0"),
    ]);

    expect(buildGameObjectHeaderSummary(panel)).toEqual({
      name: "Teacher_Rig",
      tag: "Untagged",
      layer: "0",
      active: true,
      isStatic: false,
    });
  });
});
