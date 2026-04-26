import { describe, expect, it } from "vitest";
import type { InspectorPanel } from "../types";
import {
  cleanInspectorPanelTitle,
  getInspectorPanelDisplayTitle,
  getInspectorPanelInferenceBadge,
  getInspectorPanelInferenceTooltip,
  getInspectorPanelResolveReason,
} from "../components/diff/inspectorPanelDisplay";

function makePanel(overrides: Partial<InspectorPanel>): InspectorPanel {
  return {
    panelKind: "component",
    title: "Component",
    changeKind: "modified",
    added: false,
    removed: false,
    fields: [],
    ...overrides,
  };
}

describe("inspectorPanelDisplay", () => {
  it("prefers resolved component type over raw prefab override title", () => {
    const panel = makePanel({
      title: "Component (fileID:-8679921383154817045)",
      componentType: "Transform",
      componentClassId: 4,
      componentSource: "builtin",
    });

    expect(getInspectorPanelDisplayTitle(panel)).toBe("Transform");
    expect(getInspectorPanelResolveReason(panel)).toBeNull();
  });

  it("keeps prefab target context when the backend title is more specific than the component type", () => {
    const panel = makePanel({
      title: "SettingsMenu/Buttons/Start/RectTransform",
      componentType: "RectTransform",
      componentClassId: 224,
      componentSource: "builtin",
    });

    expect(getInspectorPanelDisplayTitle(panel)).toBe("SettingsMenu/Buttons/Start/RectTransform");
  });

  it("keeps backend fileID disambiguation for repeated prefab override component titles", () => {
    const panel = makePanel({
      title: "RectTransform [fileID:881729730001825605]",
      componentType: "RectTransform",
      componentClassId: 224,
      componentSource: "builtin",
    });

    expect(getInspectorPanelDisplayTitle(panel)).toBe("RectTransform [fileID:881729730001825605]");
  });

  it("reports a fallback reason when the component still has no concrete name", () => {
    const panel = makePanel({
      title: "Component (fileID:-42)",
    });

    expect(cleanInspectorPanelTitle(panel.title)).toBe("Component");
    expect(getInspectorPanelDisplayTitle(panel)).toBe("Component");
    expect(getInspectorPanelResolveReason(panel)).toContain("fell back to raw title");
  });

  it("surfaces backend diagnostics when MonoBehaviour falls back to a generic name", () => {
    const panel = makePanel({
      title: "Component (fileID:-100)",
      componentType: "MonoBehaviour",
      componentResolveReason: "source prefab was not loaded; local stripped mapping is also missing",
    });

    expect(getInspectorPanelDisplayTitle(panel)).toBe("MonoBehaviour");
    expect(getInspectorPanelResolveReason(panel)).toBe(panel.componentResolveReason);
  });

  it("shows inferred component metadata without polluting the title", () => {
    const panel = makePanel({
      title: "Component (fileID:-7511558181221131132)",
      componentType: "Renderer",
      componentSource: "inferred",
      componentInference: {
        reasonCode: "propertyPathBuiltinFamily",
        evidence: ["m_Materials.Array.data[0]", "m_CastShadows"],
        inferredClassId: 25,
      },
      componentResolveReason: "model importer meta was loaded but no classId could be determined",
    });

    expect(getInspectorPanelDisplayTitle(panel)).toBe("Renderer");
    expect(getInspectorPanelDisplayTitle(panel)).not.toContain("?");
    expect(getInspectorPanelInferenceBadge(panel)).toBe("?");
    const tooltip = getInspectorPanelInferenceTooltip(panel);
    expect(tooltip).toContain("推测");
    expect(tooltip).toContain("Renderer");
    expect(tooltip).toContain("m_Materials.Array.data[0]");
    expect(tooltip).toContain("model importer meta was loaded but no classId could be determined");
    expect(getInspectorPanelResolveReason(panel)).toBeNull();
  });
});
