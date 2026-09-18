import type {
  KnowledgeDocumentEditOperation,
  KnowledgeDocumentSection,
} from "../../types";
import {
  buildDocumentTextEditOperations,
  buildDocumentTextHunks,
  rebaseDocumentText,
  type DocumentTextConflict,
  type DocumentTextHunk,
  type DocumentTextRebaseResult,
} from "../../document/documentText";
import { normalizeKnowledgeEditorValue } from "./knowledgeEditorDrafts";

// Knowledge sections retain their established whitespace policy; the shared
// text engine preserves bytes and can also be used by source/CSV adapters.
export type KnowledgeTextHunk = DocumentTextHunk;
export type KnowledgeTextConflict = DocumentTextConflict;
export type KnowledgeTextRebaseResult = DocumentTextRebaseResult;

export function buildKnowledgeTextHunks(base: string, next: string): KnowledgeTextHunk[] {
  return buildDocumentTextHunks(
    normalizeKnowledgeEditorValue(base),
    normalizeKnowledgeEditorValue(next),
  );
}

export function rebaseKnowledgeText(
  base: string,
  local: string,
  remote: string,
): KnowledgeTextRebaseResult {
  return rebaseDocumentText(
    normalizeKnowledgeEditorValue(base),
    normalizeKnowledgeEditorValue(local),
    normalizeKnowledgeEditorValue(remote),
  );
}

export function buildKnowledgeDocumentEditOperations(
  section: KnowledgeDocumentSection,
  base: string,
  next: string,
): KnowledgeDocumentEditOperation[] {
  return buildDocumentTextEditOperations(
    normalizeKnowledgeEditorValue(base),
    normalizeKnowledgeEditorValue(next),
  ).map((operation) => ({ section, ...operation }));
}
