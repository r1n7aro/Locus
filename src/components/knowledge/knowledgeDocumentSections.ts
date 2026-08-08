import type { KnowledgeDocument, KnowledgeDocumentSection } from "../../types";

export interface KnowledgeDocumentEditorSections
  extends Record<KnowledgeDocumentSection, boolean> {}

function hasSectionContent(value?: string | null): boolean {
  return !!value?.trim();
}

function typeNeedsMaintenanceRules(type?: KnowledgeDocument["type"] | null): boolean {
  return type === "memory";
}

function isExplicitMaintenanceRulesEnabled(
  document: Pick<
    KnowledgeDocument,
    "type" | "maintenanceRules" | "aiMaintained" | "effectiveAiMaintained"
  >,
): boolean {
  return (
    typeNeedsMaintenanceRules(document.type)
    || document.effectiveAiMaintained
    || hasSectionContent(document.maintenanceRules)
  );
}

export function getKnowledgeDocumentEditorSections(
  document: Pick<
    KnowledgeDocument,
    | "type"
    | "summary"
    | "maintenanceRules"
    | "aiMaintained"
    | "effectiveAiMaintained"
  > | null | undefined,
): KnowledgeDocumentEditorSections {
  if (!document) {
    return {
      summary: false,
      maintenanceRules: false,
      body: true,
    };
  }

  return {
    summary: hasSectionContent(document.summary),
    maintenanceRules:
      isExplicitMaintenanceRulesEnabled(document),
    body: true,
  };
}
