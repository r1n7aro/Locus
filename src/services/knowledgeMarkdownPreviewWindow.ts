import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { KnowledgeDocumentType } from "../types";

export const KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_LABEL = "knowledge-markdown-preview";
export const KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_EVENT = "knowledge-markdown-preview:payload";
export const KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_FLAG = "knowledgeMarkdownPreview";

export interface KnowledgeMarkdownPreviewWindowPayload {
  docType: KnowledgeDocumentType;
  path: string;
}

function toKnowledgeDocumentType(value: string | null): KnowledgeDocumentType | null {
  return value === "design"
    || value === "memory"
    || value === "skill"
    || value === "reference"
    ? value
    : null;
}

function documentTitle(path: string): string {
  const fileName = path.trim().replace(/\\/g, "/").split("/").pop() || "Memory";
  return fileName.replace(/\.md$/i, "") || "Memory";
}

export function isKnowledgeMarkdownPreviewWindowLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  return new URLSearchParams(locationLike.search).get(KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_FLAG) === "1";
}

export function getKnowledgeMarkdownPreviewWindowPayload(
  search = window.location.search,
): KnowledgeMarkdownPreviewWindowPayload | null {
  const params = new URLSearchParams(search);
  const docType = toKnowledgeDocumentType(params.get("docType"));
  const path = params.get("path")?.trim() ?? "";
  return docType && path ? { docType, path } : null;
}

export function buildKnowledgeMarkdownPreviewWindowQuery(
  payload: KnowledgeMarkdownPreviewWindowPayload,
): string {
  return new URLSearchParams({
    [KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_FLAG]: "1",
    docType: payload.docType,
    path: payload.path.trim(),
  }).toString();
}

export function buildKnowledgeMarkdownPreviewWindowUrl(
  payload: KnowledgeMarkdownPreviewWindowPayload,
): string {
  return buildSubWindowUrl(buildKnowledgeMarkdownPreviewWindowQuery(payload));
}

export async function openKnowledgeMarkdownPreviewWindow(
  payload: KnowledgeMarkdownPreviewWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;
  const normalizedPayload = {
    docType: payload.docType,
    path: payload.path.trim(),
  };
  if (!normalizedPayload.path) return false;

  const title = documentTitle(normalizedPayload.path);
  const result = await openSubWindow({
    kind: KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_LABEL,
    title: `Locus - ${title}`,
    width: 920,
    height: 720,
    minWidth: 600,
    minHeight: 420,
    resizable: true,
    maximizable: true,
    minimizable: false,
  }, buildKnowledgeMarkdownPreviewWindowQuery(normalizedPayload));
  if (result.existing) {
    await result.window?.emit(KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_EVENT, normalizedPayload);
  }
  return true;
}
