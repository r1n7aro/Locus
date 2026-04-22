import type { KnowledgeDocument, KnowledgeDocumentSection } from "../../types";
import { defaultSummaryEnabledForType } from "./knowledgeEditMode";

export interface KnowledgeDocumentEditorSections
  extends Record<KnowledgeDocumentSection, boolean> {}

function hasSectionContent(value?: string | null): boolean {
  return !!value?.trim();
}

function isSummaryEnabledByDefault(
  type?: KnowledgeDocument["type"] | null,
  summaryEnabled?: boolean,
): boolean {
  if (summaryEnabled !== undefined) return summaryEnabled;
  return type ? defaultSummaryEnabledForType(type) : false;
}

function typeNeedsMaintenanceRules(type?: KnowledgeDocument["type"] | null): boolean {
  return type === "memory";
}

function isExplicitMaintenanceRulesEnabled(
  document: Pick<
    KnowledgeDocument,
    "type" | "maintenanceRules" | "aiMaintained" | "explicitMaintenanceRules"
  >,
): boolean {
  if (typeof document.explicitMaintenanceRules === "boolean") {
    return document.explicitMaintenanceRules;
  }
  return (
    typeNeedsMaintenanceRules(document.type)
    || !!document.aiMaintained
    || hasSectionContent(document.maintenanceRules)
  );
}

export function getKnowledgeDocumentEditorSections(
  document: Pick<
    KnowledgeDocument,
    | "type"
    | "summary"
    | "summaryEnabled"
    | "maintenanceRules"
    | "aiMaintained"
    | "explicitMaintenanceRules"
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
    summary: isSummaryEnabledByDefault(document.type, document.summaryEnabled),
    maintenanceRules:
      isExplicitMaintenanceRulesEnabled(document)
      || !!document.aiMaintained,
    body: true,
  };
}
