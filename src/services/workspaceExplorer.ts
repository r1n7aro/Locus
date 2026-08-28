import { ipcInvoke } from "./ipc";
import type {
  ProjectCollaborationSnapshot,
  ProjectExplorerMutationResult,
  ProjectExplorerFilePreview,
  ProjectExplorerMountListing,
  ProjectExplorerOperation,
  ProjectExplorerPresetSummary,
  ProjectExplorerSnapshot,
  ProjectKnowledgeDocument,
} from "../types/workbench";

export const PROJECT_EXPLORER_CHANGED_EVENT = "project-explorer-changed";

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
