import type { ExplorerNode } from "../../composables/useKnowledgeState";
import type {
  KnowledgeDirectoryConfigRecord,
  KnowledgeDocumentSummary,
  KnowledgeDocumentType,
} from "../../types";
import type {
  InternalDragController,
  InternalDragFinishReason,
  InternalDragSource,
} from "../../composables/useInternalDrag";
export const KNOWLEDGE_INTERNAL_DRAG_TYPE = "locus/knowledge";

export interface KnowledgeWorkspaceDragEntry {
  kind: "document" | "folder";
  type: KnowledgeDocumentType;
  path: string;
  relativePath?: string;
  name: string;
  documentId?: string;
}

export interface KnowledgeWorkspaceDragPayload {
  version: 1;
  entries: KnowledgeWorkspaceDragEntry[];
}

export interface KnowledgeInternalDragData {
  payload: KnowledgeWorkspaceDragPayload;
  nodes?: ExplorerNode[];
}

export function knowledgeInternalDragSource(
  data: KnowledgeInternalDragData,
  callbacks: {
    onActivated?: () => void;
    onFinished?: (result: { dropped: boolean; reason: InternalDragFinishReason }) => void;
  } = {},
): InternalDragSource<KnowledgeInternalDragData> {
  const first = data.payload.entries[0];
  return {
    id: `knowledge:${data.payload.entries.map((entry) => entry.path).join("|")}`,
    payload: { type: KNOWLEDGE_INTERNAL_DRAG_TYPE, data },
    preview: {
      label: first?.name ?? "",
      count: data.payload.entries.length,
      kind: first?.kind === "folder" ? "folder" : "file",
    },
    allowedOperations: ["move", "copy"],
    ...callbacks,
  };
}

export function startKnowledgeInternalDrag(
  controller: InternalDragController,
  event: PointerEvent,
  data: KnowledgeInternalDragData,
  callbacks: Parameters<typeof knowledgeInternalDragSource>[1] = {},
): boolean {
  return controller.start(event, knowledgeInternalDragSource(data, callbacks));
}

function normalizeRelativePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

function fullKnowledgePath(type: KnowledgeDocumentType, path: string): string {
  const normalized = normalizeRelativePath(path);
  return normalized ? `${type}/${normalized}` : type;
}

function pathName(path: string, fallback: string): string {
  const normalized = normalizeRelativePath(path);
  return normalized.split("/").filter(Boolean).pop() || fallback;
}

export function buildKnowledgeDocumentWorkspaceDragPayload(
  document: KnowledgeDocumentSummary,
): KnowledgeWorkspaceDragPayload {
  return {
    version: 1,
    entries: [{
      kind: "document",
      type: document.type,
      path: fullKnowledgePath(document.type, document.path),
      name: pathName(document.path, document.title),
      documentId: document.id,
    }],
  };
}

export function buildKnowledgeFolderWorkspaceDragPayload(
  directory: KnowledgeDirectoryConfigRecord,
): KnowledgeWorkspaceDragPayload | null {
  const relativePath = normalizeRelativePath(directory.path);
  if (!relativePath) return null;
  return {
    version: 1,
    entries: [{
      kind: "folder",
      type: directory.type,
      path: fullKnowledgePath(directory.type, relativePath),
      relativePath,
      name: pathName(relativePath, directory.type),
    }],
  };
}

export function buildKnowledgeWorkspaceDragPayload(
  nodes: ExplorerNode[],
): KnowledgeWorkspaceDragPayload {
  return {
    version: 1,
    entries: nodes.flatMap((node): KnowledgeWorkspaceDragEntry[] => {
      if (node.kind === "document") {
        return [{
          kind: "document",
          type: node.type,
          path: node.path,
          name: node.name,
          documentId: node.document.id,
        }];
      }
      if (node.kind === "folder") {
        return [{
          kind: "folder",
          type: node.type,
          path: node.path,
          relativePath: node.relativePath,
          name: node.name,
        }];
      }
      return [];
    }),
  };
}
