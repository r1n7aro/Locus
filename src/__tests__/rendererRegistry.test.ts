import { describe, expect, it } from "vitest";
import type { InspectorField, InspectorPanel } from "../types";
import { getRendererConfig, partitionFields } from "../components/diff/rendererRegistry";

function makeField(
  propertyPath: string,
  overrides: Partial<InspectorField> = {},
): InspectorField {
  return {
    id: propertyPath,
    label: propertyPath,
    propertyPath,
    valueType: overrides.children?.length ? "group" : "string",
    changeKind: "modified",
    ...overrides,
  };
}

function makeBuiltinPanel(componentType: string, fields: InspectorField[]): InspectorPanel {
  return {
    panelKind: "component",
    title: componentType,
    changeKind: "modified",
    added: false,
    removed: false,
    componentType,
    componentSource: "builtin",
    fields,
  };
}

describe("rendererRegistry", () => {
  it("keeps Transform optimized mode flat while still applying filtering", () => {
    const panel = makeBuiltinPanel("Transform", [
      makeField("m_LocalPosition"),
      makeField("m_ConstrainProportionsScale"),
      makeField("m_Father"),
    ]);

    const config = getRendererConfig(panel);
    expect(config).not.toBeNull();
    expect(config?.grouping).toBeUndefined();

    const partition = partitionFields(panel.fields, config!);
    expect(partition.otherFields).toHaveLength(0);
    expect(partition.sections).toHaveLength(0);
    expect(partition.flatFields.map((field) => field.propertyPath)).toEqual([
      "m_LocalPosition",
      "m_ConstrainProportionsScale",
    ]);
    expect(partition.hiddenCount).toBe(1);
  });

  it("groups renderer static shadow caster under lighting and shadow", () => {
    const panel = makeBuiltinPanel("MeshRenderer", [
      makeField("m_StaticShadowCaster"),
    ]);

    const config = getRendererConfig(panel);
    expect(config).not.toBeNull();
    expect(config?.grouping).toBeDefined();

    const partition = partitionFields(panel.fields, config!);
    expect(partition.flatFields).toHaveLength(0);
    expect(partition.otherFields).toHaveLength(0);
    expect(partition.sections).toHaveLength(1);
    expect(partition.sections[0].titleKey).toBe("diff.optimized.lightingShadow");
    expect(partition.sections[0].fields[0]?.propertyPath).toBe("m_StaticShadowCaster");
  });

  it("keeps collider layer override fields in filtering and hides nested serializedVersion", () => {
    const panel = makeBuiltinPanel("BoxCollider", [
      makeField("m_IncludeLayers", {
        children: [
          makeField("m_IncludeLayers.m_Bits"),
          makeField("m_IncludeLayers.serializedVersion", {
            before: "2",
            after: "2",
            changeKind: "unchanged",
          }),
        ],
      }),
      makeField("m_ExcludeLayers", {
        children: [
          makeField("m_ExcludeLayers.m_Bits"),
        ],
      }),
      makeField("m_LayerOverridePriority"),
      makeField("m_ProvidesContacts"),
    ]);

    const config = getRendererConfig(panel);
    expect(config).not.toBeNull();
    expect(config?.grouping).toBeDefined();

    const partition = partitionFields(panel.fields, config!);
    const filteringSection = partition.sections.find(
      (section) => section.titleKey === "diff.optimized.filtering",
    );

    expect(filteringSection).toBeDefined();
    expect(partition.flatFields).toHaveLength(0);
    expect(partition.otherFields).toHaveLength(0);
    expect(filteringSection?.fields.map((field) => field.propertyPath)).toEqual([
      "m_IncludeLayers",
      "m_ExcludeLayers",
      "m_LayerOverridePriority",
      "m_ProvidesContacts",
    ]);
    expect(filteringSection?.fields[0]?.children?.map((field) => field.propertyPath)).toEqual([
      "m_IncludeLayers.m_Bits",
    ]);
    expect(partition.hiddenCount).toBe(1);
  });
});
