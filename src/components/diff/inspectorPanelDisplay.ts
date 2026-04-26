import { t } from "../../i18n";
import type { InspectorComponentInference } from "../../types";

const GENERIC_COMPONENT_TITLE_RE = /^Component(?:\s*\(fileID:-?\d+\))?$/;

interface InspectorPanelDisplayLike {
  title: string;
  panelKind?: string;
  scriptClass?: string;
  componentType?: string;
  componentResolveReason?: string;
  componentInference?: InspectorComponentInference;
}

export function cleanInspectorPanelTitle(title: string): string {
  return title.replace(/\s*\(fileID:-?\d+\)\s*/g, "").trim();
}

export function getInspectorPanelDisplayTitle(panel: InspectorPanelDisplayLike): string {
  if (panel.scriptClass) return panel.scriptClass;
  const cleanedTitle = cleanInspectorPanelTitle(panel.title);
  if (panel.componentType) {
    const titleAddsTargetContext = cleanedTitle
      && cleanedTitle !== panel.componentType
      && !GENERIC_COMPONENT_TITLE_RE.test(cleanedTitle);
    return titleAddsTargetContext ? cleanedTitle : panel.componentType;
  }
  return cleanedTitle;
}

export function getInspectorPanelResolveReason(panel: InspectorPanelDisplayLike): string | null {
  if (panel.componentInference) return null;
  if (panel.componentResolveReason) return panel.componentResolveReason;
  if (panel.panelKind !== "component") return null;
  if (panel.scriptClass || panel.componentType) return null;

  const cleanedTitle = cleanInspectorPanelTitle(panel.title);
  if (!GENERIC_COMPONENT_TITLE_RE.test(cleanedTitle)) return null;

  return `missing componentType/scriptClass, fell back to raw title (${panel.title})`;
}

export function getInspectorPanelInference(
  panel: InspectorPanelDisplayLike,
): InspectorComponentInference | null {
  return panel.componentInference ?? null;
}

export function getInspectorPanelInferenceBadge(panel: InspectorPanelDisplayLike): string {
  return getInspectorPanelInference(panel) ? t("diff.componentInference.badge") : "";
}

export function getInspectorPanelInferenceTooltip(panel: InspectorPanelDisplayLike): string {
  const inference = getInspectorPanelInference(panel);
  if (!inference) return "";
  const component = getInspectorPanelDisplayTitle(panel);
  const evidence = inference.evidence.join(", ");
  const head = evidence
    ? t("diff.componentInference.tooltipWithEvidence", component, evidence)
    : t("diff.componentInference.tooltip", component);
  if (panel.componentResolveReason) {
    return `${head}\n${t("diff.componentInference.tooltipReason", panel.componentResolveReason)}`;
  }
  return head;
}
