import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";
import type { KnowledgeDocumentType } from "../types";

export type GlobalSearchSource = "knowledgeTitle" | "knowledgeContent" | "sessionTitle" | "sessionContent";
export interface GlobalSearchHit {
  kind: "knowledge" | "session";
  id: string;
  title: string;
  /** Matching content when available; otherwise a short document/message preview. */
  excerpt: string;
  field: "title" | "content";
  docType: KnowledgeDocumentType | null;
  path: string | null;
  messageId: string | null;
  checkoutId?: string | null;
  archived: boolean;
  /** Last document/session modification, in epoch milliseconds. */
  modifiedAt: number;
}
export interface GlobalSearchPage {
  matches: GlobalSearchHit[];
  nextCursor: string | null;
}
export interface GlobalSearchTarget {
  projectId: string;
  projectName: string;
  workspaceRef: WorkspaceRef;
}
export interface GlobalSearchResult extends GlobalSearchHit { target: GlobalSearchTarget }
export interface GlobalSearchRequest {
  workspaceRef: WorkspaceRef;
  query: string;
  source: GlobalSearchSource;
  archived: boolean;
  cursor: string | null;
}
export function searchGlobalPage(request: GlobalSearchRequest): Promise<GlobalSearchPage> {
  return ipcInvoke("global_search", { ...request });
}

export function globalSearchHitKey(hit: GlobalSearchHit): string {
  return JSON.stringify([hit.kind, hit.id]);
}

function foldSearchText(value: string): string {
  return value.replace(/[A-Z]/g, (char) => char.toLowerCase());
}

// Literal ASCII folding matches the history service and preserves UTF-16
// offsets, including Chinese, emoji and code punctuation. Render as text nodes.
export function searchHighlightParts(text: string, query: string, contextBefore?: number): { text: string; match: boolean }[] {
  const needle = foldSearchText(query.trim());
  if (!needle) return [{ text, match: false }];
  const haystack = foldSearchText(text);
  const parts: { text: string; match: boolean }[] = [];
  let position = 0;
  for (let index = haystack.indexOf(needle); index !== -1; index = haystack.indexOf(needle, position)) {
    if (index > position) parts.push({ text: text.slice(position, index), match: false });
    position = index + needle.length;
    parts.push({ text: text.slice(index, position), match: true });
  }
  if (position < text.length) parts.push({ text: text.slice(position), match: false });
  // Keep the first match visible in a one-line result even with wide CJK text.
  // Iterate code points so shortening context never splits a surrogate pair.
  const prefix = parts[0];
  if (contextBefore !== undefined && prefix && !prefix.match && parts.some((part) => part.match)) {
    const chars = Array.from(prefix.text);
    if (chars.length > contextBefore) prefix.text = `…${chars.slice(-contextBefore).join("")}`;
  }
  return parts;
}
