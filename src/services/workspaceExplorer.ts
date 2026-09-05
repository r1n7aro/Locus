import { ipcInvoke } from "./ipc";
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

export interface WorkspaceFileChangedPayload {
  seq: number;
  generation: number;
  path: string;
  kind: "upsert" | "delete";
  source: "os_watcher" | "locus_write" | "plugin_install" | "reconcile";
}

type WorkspaceFileChangedEvent = RoutedWorkspaceEvent<WorkspaceFileChangedPayload>;
const workspaceFileChangeListeners = new Set<(event: WorkspaceFileChangedEvent) => void>();
let workspaceFileChangeSubscriptionStarted = false;

function ensureWorkspaceFileChangeSubscription(): void {
  if (workspaceFileChangeSubscriptionStarted) return;
  workspaceFileChangeSubscriptionStarted = true;
  void getLocusRuntime().subscribe<WorkspaceFileChangedEvent>(
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
  ).catch(() => {
    workspaceFileChangeSubscriptionStarted = false;
  });
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

export function workspaceFileRevision(
  filePath: string,
  workspaceRef: WorkspaceRef,
): Promise<ProjectExplorerFileRevision> {
  return ipcInvoke<ProjectExplorerFileRevision>("workspace_file_revision", {
    filePath,
    workspaceRef,
  });
}

export function subscribeWorkspaceFileChanges(
  handler: (event: WorkspaceFileChangedEvent) => void,
): Promise<RuntimeUnsubscribe> {
  workspaceFileChangeListeners.add(handler);
  ensureWorkspaceFileChangeSubscription();
  return Promise.resolve(() => {
    workspaceFileChangeListeners.delete(handler);
  });
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

export function projectKnowledgeList(
  projectId: string,
  options: { type?: string | null; pathPrefix?: string | null } = {},
): Promise<ProjectKnowledgeDocument[]> {
  return ipcInvoke<ProjectKnowledgeDocument[]>("project_knowledge_list", {
    projectId,
    docType: options.type ?? null,
    pathPrefix: options.pathPrefix ?? null,
  });
}

export function projectCollaborationSnapshot(
  projectId: string,
): Promise<ProjectCollaborationSnapshot> {
  return ipcInvoke<ProjectCollaborationSnapshot>("project_collaboration_snapshot", { projectId });
}
