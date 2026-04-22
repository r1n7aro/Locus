import type { KnowledgeDocument, KnowledgeDocumentSection } from "../../types";

export interface KnowledgeEditorDraftValues {
  summary: string;
  maintenanceRules: string;
  body: string;
}

export const KNOWLEDGE_EDITOR_SECTIONS: readonly KnowledgeDocumentSection[] = [
  "summary",
  "maintenanceRules",
  "body",
];

export function normalizeKnowledgeEditorValue(value: string): string {
  return value.replace(/\r\n/g, "\n").trimEnd();
}

export function createKnowledgeEditorDraftValues(
  document: KnowledgeDocument | null,
): KnowledgeEditorDraftValues {
  return {
    summary: document?.summary ?? "",
    maintenanceRules: document?.maintenanceRules ?? "",
    body: document?.body ?? "",
  };
}

export function getKnowledgeDocumentSectionValue(
  document: KnowledgeDocument | null,
  section: KnowledgeDocumentSection,
): string {
  if (section === "summary") return document?.summary ?? "";
  if (section === "maintenanceRules") return document?.maintenanceRules ?? "";
  return document?.body ?? "";
}

export function getKnowledgeEditorDraftValue(
  drafts: KnowledgeEditorDraftValues,
  section: KnowledgeDocumentSection,
): string {
  if (section === "summary") return drafts.summary;
  if (section === "maintenanceRules") return drafts.maintenanceRules;
  return drafts.body;
}

export function mergeKnowledgeEditorDraftValues(
  document: KnowledgeDocument | null,
  drafts: KnowledgeEditorDraftValues,
  dirtySections: ReadonlySet<KnowledgeDocumentSection>,
  force = false,
): {
  drafts: KnowledgeEditorDraftValues;
  dirtySections: Set<KnowledgeDocumentSection>;
} {
  if (force) {
    return {
      drafts: createKnowledgeEditorDraftValues(document),
      dirtySections: new Set(),
    };
  }

  const nextDrafts: KnowledgeEditorDraftValues = { ...drafts };
  for (const section of KNOWLEDGE_EDITOR_SECTIONS) {
    if (dirtySections.has(section)) continue;
    if (section === "summary") nextDrafts.summary = getKnowledgeDocumentSectionValue(document, section);
    else if (section === "maintenanceRules") nextDrafts.maintenanceRules = getKnowledgeDocumentSectionValue(document, section);
    else nextDrafts.body = getKnowledgeDocumentSectionValue(document, section);
  }

  const nextDirtySections = new Set(dirtySections);
  for (const section of nextDirtySections) {
    if (
      normalizeKnowledgeEditorValue(getKnowledgeEditorDraftValue(nextDrafts, section))
      === normalizeKnowledgeEditorValue(getKnowledgeDocumentSectionValue(document, section))
    ) {
      nextDirtySections.delete(section);
    }
  }

  return {
    drafts: nextDrafts,
    dirtySections: nextDirtySections,
  };
}
