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
  | { kind: "placeResource"; nodeId?: string | null; resourceKind: "session" | "knowledge" | "system"; resourceId: string; sourceKind?: "knowledge" | string | null; parentNodeId?: string | null; position: number }
  | { kind: "removeResourcePlacement"; resourceKind: "knowledge"; resourceId: string }
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

export interface ProjectExplorerFileRevision {
  exists: boolean;
  size: number;
  modifiedAtNanos: string;
  key: string;
}

export interface ProjectExplorerFilePreview {
  path: string;
  name: string;
  extension: string;
  size: number;
  kind: ProjectExplorerFilePreviewKind;
  mimeType: string;
  text?: string;
  contentHash?: string;
  dataUrl?: string;
  totalLines?: number;
  truncated: boolean;
  editable: boolean;
  checkoutId?: string;
  workspaceGeneration?: number;
  workspaceRelativePath?: string;
  revision: ProjectExplorerFileRevision;
}

export type WorkspaceSectionKind =
  | "sessions"
  | "archived"
  | "knowledge"
  | "collab"
  | "assets"
  | "views";

/**
 * Stable editor identity. Runtime generations and physical titles are kept on
 * the editor input so a restored tab can resolve them again.
 */
export type WorkbenchResourceRef =
  | { kind: "project"; projectId: string }
  | { kind: "newSession"; projectId: string }
  | { kind: "checkout"; projectId: string; checkoutId: string }
  | { kind: "section"; projectId: string; section: WorkspaceSectionKind }
  | { kind: "knowledgeRoot"; projectId: string }
  | { kind: "collaboration"; projectId: string }
  | { kind: "folder"; projectId: string; nodeId: string }
  | { kind: "session"; projectId: string; sessionId: string }
  | { kind: "knowledge"; projectId: string; documentId: string }
  | { kind: "workspaceFile"; projectId: string; path: string }
  | { kind: "asset"; projectId: string; path: string }
  | { kind: "sceneObject"; projectId: string; scenePath: string; objectPath: string }
  | { kind: "view"; projectId: string; viewId: string }
  | { kind: "localDirectory"; projectId: string; nodeId: string; relativePath?: string | null }
  | { kind: "localFile"; projectId: string; nodeId: string; relativePath?: string | null };

export type DevelopmentResourceRef = WorkbenchResourceRef;

export interface EditorCheckoutBinding {
  checkoutId: string;
  expectedGeneration?: number | null;
}

export interface EditorCapabilities {
  split: boolean;
  detach: boolean;
  duplicate: boolean;
}

export type WorkbenchEditorAvailability = "available" | "unavailable";

export interface WorkbenchEditorInput {
  editorId: string;
  resource: WorkbenchResourceRef;
  title: string;
  icon?: string | null;
  preview: boolean;
  pinned: boolean;
  dirty: boolean;
  capabilities: EditorCapabilities;
  checkoutBinding?: EditorCheckoutBinding | null;
  /** Resolved locator for local-file and mounted-file adapters. */
  sourcePath?: string | null;
  availability: WorkbenchEditorAvailability;
  unavailableReason?: string | null;
}

export interface WorkbenchEditorGroup {
  paneId: string;
  tabs: WorkbenchEditorInput[];
  activeEditorId: string | null;
  focusedCheckoutId?: string | null;
}

export type WorkbenchSplitOrientation = "horizontal" | "vertical";

export type WorkbenchSplitNode =
  | {
      kind: "group";
      paneId: string;
    }
  | {
      kind: "split";
      splitId: string;
      orientation: WorkbenchSplitOrientation;
      ratio: number;
      first: WorkbenchSplitNode;
      second: WorkbenchSplitNode;
    };

export interface WorkbenchSidebarState {
  width: number;
  collapsed: boolean;
}

export interface WorkbenchWindowState {
  schemaVersion: number;
  windowId: string;
  sidebar: WorkbenchSidebarState;
  layout: WorkbenchSplitNode;
  groups: Record<string, WorkbenchEditorGroup>;
  focusedPaneId: string;
}

export type WorkbenchDropDirection = "center" | "left" | "right" | "top" | "bottom";

export interface WorkbenchEditorDragData {
  windowId: string;
  paneId: string;
  editorId: string;
}

export interface WorkbenchWindowDropIntent {
  windowId: string;
  paneId: string;
  direction: WorkbenchDropDirection;
  index?: number;
}

export type WorkbenchEditorTransferSnapshot =
  | {
      kind: "session";
      composerDraft?: unknown;
    }
  | {
      kind: "workspaceFile";
      text: string;
      contentHash: string;
      originalLineEnding: "\n" | "\r\n" | "\r";
      selection?: { anchor: number; head: number } | null;
      scrollTop?: number | null;
    }
  | {
      kind: "resource";
    };

export interface WorkbenchEditorTransferRecord {
  version: 1;
  token: string;
  sourceWindowId: string;
  sourcePaneId: string;
  sourceEditorId: string;
  editor: WorkbenchEditorInput;
  snapshot?: WorkbenchEditorTransferSnapshot | null;
  target?: WorkbenchWindowDropIntent | null;
  allowDuplicate?: boolean;
  createdAt: number;
  dragStartedAt: number;
}

export interface WorkbenchTransferAcceptResult {
  paneId: string;
  editorId: string;
  inserted: boolean;
}
