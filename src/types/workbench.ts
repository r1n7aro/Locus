import type { KnowledgeDocumentSummary, SessionSummary } from "../types";

export type WorkspaceDisplayMode = "single" | "multi";

export interface ProjectKnowledgeDocument extends KnowledgeDocumentSummary {
  sourceCheckoutId: string;
  sourceWorkspaceGeneration?: number | null;
  sourceRoot: string;
  availableCheckoutIds: string[];
}

export interface ProjectCollaborationCheckout {
  checkoutId: string;
  workspaceGeneration?: number | null;
  root: string;
  branchRef?: string | null;
  headOid?: string | null;
}

export interface ProjectCollaborationSnapshot {
  projectId: string;
  checkouts: ProjectCollaborationCheckout[];
}

export interface ProjectExplorerNode {
  nodeId: string;
  projectId: string;
  nodeKind: "folder" | "resource";
  parentNodeId?: string | null;
  resourceKind?: "session" | "knowledge" | string | null;
  resourceId?: string | null;
  folderName?: string | null;
  hidden: boolean;
  sourcePath?: string | null;
  sourceKind?: "local" | "knowledge" | string | null;
  position: number;
}

export interface ProjectExplorerPresetSummary {
  presetId: string;
  name: string;
  revision: number;
  active: boolean;
  filePath: string;
}

export interface ProjectExplorerSnapshot {
  projectId: string;
  presetId: string;
  presetName: string;
  manifestPath: string;
  revision: number;
  nodes: ProjectExplorerNode[];
  presets: ProjectExplorerPresetSummary[];
}

export type ProjectExplorerOperation =
  | { kind: "createFolder"; nodeId?: string | null; parentNodeId?: string | null; name: string; position: number }
  | { kind: "renameFolder"; nodeId: string; name: string }
  | { kind: "deleteFolder"; nodeId: string }
  | { kind: "moveNode"; nodeId: string; parentNodeId?: string | null; position: number }
  | { kind: "placeResource"; resourceKind: "session" | "knowledge" | "system"; resourceId: string; sourceKind?: "knowledge" | string | null; parentNodeId?: string | null; position: number }
  | { kind: "removeResourcePlacement"; resourceKind: "session" | "knowledge" | "system"; resourceId: string }
  | { kind: "mountPath"; nodeId?: string | null; parentNodeId?: string | null; path: string; sourceKind?: "local" | "knowledge" | null; name?: string | null; position: number }
  | { kind: "setNodeHidden"; nodeId: string; hidden: boolean }
  | { kind: "removeNode"; nodeId: string };

export interface ProjectExplorerMutationResult {
  operationId: string;
  snapshot: ProjectExplorerSnapshot;
}

export interface ProjectExplorerResources {
  sessions: SessionSummary[];
  knowledge: ProjectKnowledgeDocument[];
  collaboration: ProjectCollaborationSnapshot | null;
}

export interface ProjectExplorerMountEntry {
  nodeId: string;
  relativePath: string;
  absolutePath: string;
  name: string;
  isDir: boolean;
  depth: number;
}

export interface ProjectExplorerMountListing {
  nodeId: string;
  rootPath: string;
  entries: ProjectExplorerMountEntry[];
  truncated: boolean;
}

export type ProjectExplorerFilePreviewKind =
  | "text"
  | "image"
  | "pdf"
  | "audio"
  | "video"
  | "binary"
  | "unity";

export interface ProjectExplorerFilePreview {
  path: string;
  name: string;
  extension: string;
  size: number;
  kind: ProjectExplorerFilePreviewKind;
  mimeType: string;
  text?: string;
  dataUrl?: string;
  totalLines?: number;
  truncated: boolean;
  checkoutId?: string;
  workspaceGeneration?: number;
  workspaceRelativePath?: string;
}

export type DevelopmentResourceRef =
  | { kind: "project"; projectId: string }
  | { kind: "newSession"; projectId: string; checkoutId: string }
  | { kind: "checkout"; projectId: string; checkoutId: string }
  | { kind: "knowledgeRoot"; projectId: string; checkoutId: string }
  | { kind: "collaboration"; projectId: string }
  | { kind: "folder"; projectId: string; nodeId: string }
  | { kind: "session"; projectId: string; sessionId: string; checkoutId: string }
  | { kind: "knowledge"; projectId: string; documentId: string; sourceCheckoutId: string }
  | { kind: "localFile"; projectId: string; path: string; nodeId: string };
