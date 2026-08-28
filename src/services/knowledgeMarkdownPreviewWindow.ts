import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { KnowledgeDocumentType } from "../types";
import type { WorkspaceRef } from "./project";

export const KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_LABEL = "knowledge-markdown-preview";
export const KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_EVENT = "knowledge-markdown-preview:payload";
export const KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_FLAG = "knowledgeMarkdownPreview";

export interface KnowledgeMarkdownPreviewWindowPayload {
  workspaceRef: WorkspaceRef;
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
  const checkoutId = params.get("checkoutId")?.trim() ?? "";
  const generation = Number(params.get("workspaceGeneration"));
  return docType && path && checkoutId
    ? {
        docType,
        path,
        workspaceRef: {
          checkoutId,
          expectedGeneration: Number.isSafeInteger(generation) && generation > 0
            ? generation
            : undefined,
        },
      }
    : null;
}

export function buildKnowledgeMarkdownPreviewWindowQuery(
  payload: KnowledgeMarkdownPreviewWindowPayload,
): string {
  return new URLSearchParams({
    [KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_FLAG]: "1",
    docType: payload.docType,
    path: payload.path.trim(),
    checkoutId: payload.workspaceRef.checkoutId,
    ...(payload.workspaceRef.expectedGeneration != null
      ? { workspaceGeneration: String(payload.workspaceRef.expectedGeneration) }
      : {}),
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
    workspaceRef: { ...payload.workspaceRef },
    docType: payload.docType,
    path: payload.path.trim(),
  };
  if (!normalizedPayload.path) return false;

  const title = documentTitle(normalizedPayload.path);
  const result = await openSubWindow({
    kind: `${KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_LABEL}-${safeWindowScope(payload.workspaceRef.checkoutId)}`,
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

function safeWindowScope(value: string): string {
  return Array.from(new TextEncoder().encode(value))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}
