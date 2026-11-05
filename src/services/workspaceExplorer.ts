import { ipcInvoke } from "./ipc";
import { flushWindowEventInterests, watchWindowEventInterests } from "./windowEventInterests";
import { beginWorkspaceGitHeadObservation, rememberWorkspaceGitHead } from "./workspaceGitHead";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";
import {
  WORKSPACE_EVENT_NAME,
  type RoutedWorkspaceEvent,
  type WorkspaceRef,
} from "./project";
import type {
  ProjectCollaborationSnapshot,
  ProjectExplorerMutationResult,
  ProjectExplorerFilePreview,
  ProjectExplorerFileRevision,
  ProjectExplorerMountListing,
  ProjectExplorerOperation,
  ProjectExplorerPresetSummary,
  ProjectExplorerSnapshot,
  ProjectKnowledgeDocument,
} from "../types/workbench";

export const PROJECT_EXPLORER_CHANGED_EVENT = "project-explorer-changed";
export const WORKSPACE_FILE_CHANGED_EVENT = "workspace-file-changed";

export function explorerFileAction(projectId: string, path: string, newName: string | null, workspaceRef?: WorkspaceRef | null): Promise<string | null> {
  return ipcInvoke("explorer_file_action", { projectId, path, newName, workspaceRef: workspaceRef ?? null });
}

export interface WorkspaceFileMoveResult {
  projectId: string;
  root: string;
  path: string;
  newPath: string;
  isDirectory: boolean;
  workspaceRef: WorkspaceRef;
}

export type ExplorerFileActionEvent = Omit<WorkspaceFileMoveResult, "newPath" | "isDirectory" | "workspaceRef"> & {
  newPath: string | null;
  isDirectory?: boolean;
  workspaceRef?: WorkspaceRef;
};

export function subscribeExplorerFileActions(listener: (event: ExplorerFileActionEvent) => void): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<ExplorerFileActionEvent>("explorer-file-action", listener,
    { owner: "workspaceExplorer.subscribeExplorerFileActions" });
}

export function moveWorkspaceFile(workspaceRef: WorkspaceRef, sourcePath: string, targetDirectory: string): Promise<WorkspaceFileMoveResult> {
  return ipcInvoke("workspace_move_entry", { workspaceRef, sourcePath, targetDirectory });
}

export interface WorkspaceFileImportResult {
  /** Workspace-relative paths of the copied entries. */
  imported: string[];
  /** Existing workspace-relative destinations; nothing is copied when any are reported. */
  conflicts: string[];
}

/** Copies files and folders from outside the checkout, with their Unity `.meta`, into a checkout folder. */
export function importWorkspaceEntries(
  workspaceRef: WorkspaceRef,
  sourcePaths: string[],
  targetDirectory: string,
  overwrite = false,
): Promise<WorkspaceFileImportResult> {
  return ipcInvoke("workspace_import_entries", { workspaceRef, sourcePaths, targetDirectory, overwrite });
}

export function workspaceCreateDocument(workspaceRef: WorkspaceRef, directory: string, name: string): Promise<string> {
  return ipcInvoke("workspace_create_document", { workspaceRef, directory, name });
}

export function workspaceCreateFolder(workspaceRef: WorkspaceRef, directory: string, name: string): Promise<string> {
  return ipcInvoke("workspace_create_folder", { workspaceRef, directory, name });
}

export interface WorkspaceFileChangedPayload {
  seq: number;
  generation: number;
  path: string;
  kind: "upsert" | "delete";
  source: "os_watcher" | "locus_write" | "plugin_install" | "reconcile";
  /** Reconcile this path's subtree; "." means the entire checkout. */
  rescan?: boolean;
}

function normalizeChangePath(path: string): string {
  return path.replace(/\\/g, "/").replace(/^(?:\.\/)+/, "").replace(/\/+$/, "").toLowerCase() || ".";
}

/**
 * Whether a `workspace-file-changed` path can affect `target`.
 *
 * A change can name a directory without separate events for its descendants:
 * a watcher overflow rescans a subtree, and a directory delete or rename
 * arrives once. "." covers the whole checkout. `target` is workspace-relative;
 * pass `{ anchored: false }` when it may carry a root prefix (an absolute path),
 * which then matches the change as a path-segment suffix or infix. Matching is
 * case-insensitive: an extra refresh is harmless, a missed one shows stale data.
 */
export function workspaceChangeAffectsPath(
  changedPath: string,
  target: string,
  options: { anchored?: boolean } = {},
): boolean {
  const changed = normalizeChangePath(changedPath);
  if (changed === ".") return true;
  const normalizedTarget = normalizeChangePath(target);
  if (normalizedTarget === changed || normalizedTarget.startsWith(`${changed}/`)) return true;
  return options.anchored === false
    && (normalizedTarget.endsWith(`/${changed}`) || normalizedTarget.includes(`/${changed}/`));
}

type WorkspaceFileChangedEvent = RoutedWorkspaceEvent<WorkspaceFileChangedPayload>;
const workspaceFileChangeListeners = new Set<(event: WorkspaceFileChangedEvent) => void>();
let workspaceFileChangeSubscription: Promise<void> | null = null;

function ensureWorkspaceFileChangeSubscription(): Promise<void> {
  if (workspaceFileChangeSubscription) return workspaceFileChangeSubscription;
  workspaceFileChangeSubscription = getLocusRuntime().subscribe<WorkspaceFileChangedEvent>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName !== WORKSPACE_FILE_CHANGED_EVENT) return;
      for (const listener of workspaceFileChangeListeners) {
        try {
          listener(event);
        } catch (error) {
          console.warn("[workspace-file-change] listener failed", error);
        }
      }
    },
    { owner: "workspaceExplorer.ensureWorkspaceFileChangeSubscription" },
  ).then(() => {}).catch(error => {
    workspaceFileChangeSubscription = null;
    throw error;
  });
  return workspaceFileChangeSubscription;
}

export function projectExplorerSnapshot(projectId: string): Promise<ProjectExplorerSnapshot> {
  return ipcInvoke<ProjectExplorerSnapshot>("project_explorer_snapshot", { projectId });
}

export function projectExplorerApplyOperations(
  projectId: string,
  expectedRevision: number,
  operations: ProjectExplorerOperation[],
  operationId = crypto.randomUUID(),
): Promise<ProjectExplorerMutationResult> {
  return ipcInvoke<ProjectExplorerMutationResult>("project_explorer_apply_operations", {
    projectId,
    expectedRevision,
    operationId,
    operations,
  });
}

export function projectExplorerListPresets(
  projectId: string,
): Promise<ProjectExplorerPresetSummary[]> {
  return ipcInvoke<ProjectExplorerPresetSummary[]>("project_explorer_list_presets", { projectId });
}

export function projectExplorerCreatePreset(
  projectId: string,
  name: string,
  sourcePresetId?: string | null,
): Promise<ProjectExplorerSnapshot> {
  return ipcInvoke<ProjectExplorerSnapshot>("project_explorer_create_preset", {
    projectId,
    name,
    sourcePresetId: sourcePresetId ?? null,
  });
}

export function projectExplorerSwitchPreset(
  projectId: string,
  presetId: string,
): Promise<ProjectExplorerSnapshot> {
  return ipcInvoke<ProjectExplorerSnapshot>("project_explorer_switch_preset", {
    projectId,
    presetId,
  });
}

export function projectExplorerRenamePreset(
  projectId: string,
  presetId: string,
  name: string,
): Promise<ProjectExplorerSnapshot> {
  return ipcInvoke<ProjectExplorerSnapshot>("project_explorer_rename_preset", {
    projectId,
    presetId,
    name,
  });
}

export function projectExplorerDeletePreset(
  projectId: string,
  presetId: string,
): Promise<ProjectExplorerSnapshot> {
  return ipcInvoke<ProjectExplorerSnapshot>("project_explorer_delete_preset", {
    projectId,
    presetId,
  });
}

export function projectExplorerListMount(
  projectId: string,
  nodeId: string,
): Promise<ProjectExplorerMountListing> {
  return ipcInvoke<ProjectExplorerMountListing>("project_explorer_list_mount", {
    projectId,
    nodeId,
  });
}

export function projectExplorerPreviewFile(
  projectId: string,
  path: string,
): Promise<ProjectExplorerFilePreview> {
  return ipcInvoke<ProjectExplorerFilePreview>("project_explorer_preview_file", {
    projectId,
    path,
  });
}

export function projectExplorerFileRevision(
  projectId: string,
  path: string,
): Promise<ProjectExplorerFileRevision> {
  return ipcInvoke<ProjectExplorerFileRevision>("project_explorer_file_revision", {
    projectId,
    path,
  });
}

export function projectExplorerWriteFile(
  projectId: string,
  path: string,
  content: string,
  expectedContentHash: string,
): Promise<ProjectExplorerFilePreview> {
  return ipcInvoke<ProjectExplorerFilePreview>("project_explorer_write_file", {
    projectId,
    path,
    content,
    expectedContentHash,
  });
}

export function workspaceFilePreview(
  filePath: string,
  workspaceRef: WorkspaceRef,
): Promise<ProjectExplorerFilePreview> {
  return ipcInvoke<ProjectExplorerFilePreview>("workspace_file_preview", {
    filePath,
    workspaceRef,
  });
}

/** Frees the model bytes a preview parked in the backend binary cache. */
export function releasePreviewBlob(blobUrl: string): Promise<boolean> {
  return ipcInvoke<boolean>("release_preview_blob", { blobUrl }, { throwOnError: false });
}

export function workspaceFileRevision(
  filePath: string,
  workspaceRef: WorkspaceRef,
): Promise<ProjectExplorerFileRevision> {
  return ipcInvoke<ProjectExplorerFileRevision>("workspace_file_revision", {
    filePath,
    workspaceRef,
  });
}

/**
 * File changes are delivered only to windows that registered the checkout, so
 * a consumer must name the checkout(s) it shows. Without `workspaceRef` it
 * only sees checkouts that another consumer in this window registered.
 */
export async function subscribeWorkspaceFileChanges(
  handler: (event: WorkspaceFileChangedEvent) => void,
  workspaceRef?: () => WorkspaceRef | readonly WorkspaceRef[] | null | undefined,
): Promise<RuntimeUnsubscribe> {
  const releaseInterest = workspaceRef ? watchWindowEventInterests(() => {
    const scope = workspaceRef();
    const scopes: readonly WorkspaceRef[] = !scope ? [] : Array.isArray(scope) ? scope : [scope as WorkspaceRef];
    return scopes.map(item => ({ workspaceRef: item }));
  }) : () => {};
  workspaceFileChangeListeners.add(handler);
  try {
    await ensureWorkspaceFileChangeSubscription();
    if (workspaceRef) await flushWindowEventInterests();
  } catch (error) {
    releaseInterest();
    workspaceFileChangeListeners.delete(handler);
    throw error;
  }
  return () => {
    releaseInterest();
    workspaceFileChangeListeners.delete(handler);
  };
}

export function workspaceFileWrite(
  filePath: string,
  content: string,
  expectedContentHash: string,
  workspaceRef: WorkspaceRef,
): Promise<ProjectExplorerFilePreview> {
  return ipcInvoke<ProjectExplorerFilePreview>("workspace_file_write", {
    filePath,
    content,
    expectedContentHash,
    workspaceRef,
  });
}

type ProjectKnowledgeDocumentPayload = Omit<ProjectKnowledgeDocument, "modifiedAt"> & {
  updatedAt: number;
};

export async function projectKnowledgeList(
  projectId: string,
  options: { type?: string | null; pathPrefix?: string | null } = {},
): Promise<ProjectKnowledgeDocument[]> {
  const documents = await ipcInvoke<ProjectKnowledgeDocumentPayload[]>("project_knowledge_list", {
    projectId,
    docType: options.type ?? null,
    pathPrefix: options.pathPrefix ?? null,
  });
  return documents.map(({ updatedAt, ...document }) => ({
    ...document,
    modifiedAt: updatedAt,
  }));
}

export async function projectCollaborationSnapshot(
  projectId: string,
): Promise<ProjectCollaborationSnapshot> {
  const observation = beginWorkspaceGitHeadObservation();
  const snapshot = await ipcInvoke<ProjectCollaborationSnapshot>("project_collaboration_snapshot", { projectId });
  for (const checkout of snapshot.checkouts) {
    rememberWorkspaceGitHead({
      checkoutId: checkout.checkoutId,
      expectedGeneration: checkout.workspaceGeneration,
    }, {
      kind: checkout.branchRef ? "attached" : "detached",
      refName: checkout.branchRef ?? null,
      hash: checkout.headOid ?? null,
    }, observation);
  }
  return snapshot;
}
