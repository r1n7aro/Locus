import type { InjectionKey } from "vue";
import type { WorkspaceRef } from "./project";
import { ipcInvoke } from "./ipc";
import type { KnowledgeDocumentType } from "../types";

export interface KnowledgeDocumentSource {
  path: string;
  content: string;
}

export interface KnowledgeQuoteSelection {
  path: string;
  name: string;
  content: string;
}

export const KNOWLEDGE_QUOTE_SELECTION_KEY: InjectionKey<
  (workspaceRef: WorkspaceRef, quote: KnowledgeQuoteSelection) => Promise<void>
> = Symbol("knowledge-quote-selection");

export function readKnowledgeDocumentSource(
  workspaceRef: WorkspaceRef,
  type: KnowledgeDocumentType,
  path: string,
): Promise<KnowledgeDocumentSource> {
  return ipcInvoke("knowledge_document_source", { workspaceRef, request: { docType: type, path, kind: "document" } });
}
