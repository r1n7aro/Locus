import { reactive, ref } from "vue";
import { defineStore } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { normalizeAppError } from "../services/errors";
import { listProjectSessions } from "../services/session";
import {
  projectCollaborationSnapshot,
  PROJECT_EXPLORER_CHANGED_EVENT,
  projectExplorerApplyOperations,
  projectExplorerCreatePreset,
  projectExplorerDeletePreset,
  projectExplorerListMount,
  projectExplorerRenamePreset,
  projectExplorerSnapshot,
  projectExplorerSwitchPreset,
  projectKnowledgeList,
} from "../services/workspaceExplorer";
import type {
  ProjectExplorerMountListing,
  ProjectExplorerOperation,
  ProjectExplorerResources,
  ProjectExplorerSnapshot,
} from "../types/workbench";

const SYSTEM_RESOURCE_KIND = "system" as const;
const NEW_SESSION_SYSTEM_RESOURCE_ID = "newSession";
const KNOWLEDGE_SYSTEM_RESOURCE_ID = "knowledge";
const COLLABORATION_SYSTEM_RESOURCE_ID = "collaboration";

function emptyResources(): ProjectExplorerResources {
  return { sessions: [], knowledge: [], collaboration: null };
}

export const useWorkspaceExplorerStore = defineStore("workspaceExplorer", () => {
  const { state: displaySettings } = useDisplaySettings();
  const snapshots = reactive<Record<string, ProjectExplorerSnapshot>>({});
  const mountListings = reactive<Record<string, ProjectExplorerMountListing>>({});
  const resources = reactive<Record<string, ProjectExplorerResources>>({});
  const loading = reactive<Record<string, boolean>>({});
  const errors = reactive<Record<string, string>>({});
  const requestEpochs = new Map<string, number>();
  const sessionRequestEpochs = new Map<string, number>();
  const pendingOperationIds = new Set<string>();
  let explorerUnlisten: UnlistenFn | null = null;
  let explorerListenerStarting = false;
  const selectedNodeKey = ref<string | null>(null);
  const expandedNodeKeys = ref<Set<string>>(new Set());

  function nextEpoch(projectId: string): number {
    const epoch = (requestEpochs.get(projectId) ?? 0) + 1;
    requestEpochs.set(projectId, epoch);
    return epoch;
  }

  function isCurrent(projectId: string, epoch: number): boolean {
    return requestEpochs.get(projectId) === epoch;
  }

  function nextSessionEpoch(projectId: string): number {
    const epoch = (sessionRequestEpochs.get(projectId) ?? 0) + 1;
    sessionRequestEpochs.set(projectId, epoch);
    return epoch;
  }

  function isCurrentSessionRequest(projectId: string, epoch: number): boolean {
    return sessionRequestEpochs.get(projectId) === epoch;
  }

  function ensureExplorerListener(): void {
    if (explorerUnlisten || explorerListenerStarting) return;
    explorerListenerStarting = true;
    void listen<{
      projectId: string;
      revision: number;
      operationId: string;
      presetId: string;
    }>(PROJECT_EXPLORER_CHANGED_EVENT, ({ payload }) => {
      if (pendingOperationIds.has(payload.operationId)) return;
      const currentRevision = snapshots[payload.projectId]?.revision ?? -1;
      const currentPresetId = snapshots[payload.projectId]?.presetId;
      if (payload.presetId !== currentPresetId || payload.revision > currentRevision) {
        void loadProject(payload.projectId, true);
      }
    }).then((unlisten) => {
      explorerUnlisten = unlisten;
    }).catch(() => {
      explorerListenerStarting = false;
    });
  }

  async function loadProject(projectId: string, force = false): Promise<void> {
    ensureExplorerListener();
    if (!projectId || (loading[projectId] && !force)) return;
    if (!force && snapshots[projectId] && resources[projectId]) return;
    const epoch = nextEpoch(projectId);
    const sessionEpoch = nextSessionEpoch(projectId);
    loading[projectId] = true;
    delete errors[projectId];
    try {
      const [snapshotResult, sessionsResult, knowledgeResult, collaborationResult] = await Promise.allSettled([
        projectExplorerSnapshot(projectId),
        listProjectSessions(projectId),
        projectKnowledgeList(projectId),
        projectCollaborationSnapshot(projectId),
      ]);
      if (!isCurrent(projectId, epoch)) return;
      if (snapshotResult.status === "rejected") throw snapshotResult.reason;
      const previous = resources[projectId] ?? emptyResources();
      const previousPresetId = snapshots[projectId]?.presetId;
      snapshots[projectId] = snapshotResult.value;
      if (previousPresetId && previousPresetId !== snapshotResult.value.presetId) {
        clearProjectMounts(projectId);
      }
      resources[projectId] = {
        sessions: sessionsResult.status === "fulfilled"
          && isCurrentSessionRequest(projectId, sessionEpoch)
          ? sessionsResult.value
          : previous.sessions,
        knowledge: knowledgeResult.status === "fulfilled" ? knowledgeResult.value : previous.knowledge,
        collaboration: collaborationResult.status === "fulfilled"
          ? collaborationResult.value
          : previous.collaboration,
      };
      await placeMissingResources(projectId, epoch);
      const partialFailure = [sessionsResult, knowledgeResult, collaborationResult]
        .find((result) => result.status === "rejected");
      if (partialFailure?.status === "rejected") {
        errors[projectId] = normalizeAppError(partialFailure.reason).message;
      }
    } catch (error) {
      if (!isCurrent(projectId, epoch)) return;
      errors[projectId] = normalizeAppError(error).message;
      resources[projectId] ??= emptyResources();
    } finally {
      if (isCurrent(projectId, epoch)) loading[projectId] = false;
    }
  }

  async function refreshProjectSessions(projectId: string): Promise<void> {
    if (!projectId) return;
    ensureExplorerListener();
    const sessionEpoch = nextSessionEpoch(projectId);
    const nextSessions = await listProjectSessions(projectId);
    if (!isCurrentSessionRequest(projectId, sessionEpoch)) return;

    const previous = resources[projectId] ?? emptyResources();
    resources[projectId] = {
      ...previous,
      sessions: nextSessions,
    };

    const layoutEpoch = requestEpochs.get(projectId);
    if (layoutEpoch == null || !snapshots[projectId]) return;
    await placeMissingResources(projectId, layoutEpoch);
  }

  async function placeMissingResources(
    projectId: string,
    epoch: number,
    conflictRetries = 1,
  ): Promise<void> {
    const snapshot = snapshots[projectId];
    const projectResources = resources[projectId];
    if (!snapshot || !projectResources || !isCurrent(projectId, epoch)) return;
    const placed = new Set(
      snapshot.nodes
        .filter((node) => node.nodeKind === "resource" && node.resourceKind && node.resourceId)
        .map((node) => `${node.resourceKind}:${node.resourceId}`),
    );
    type PlacementNode = Pick<
      ProjectExplorerSnapshot["nodes"][number],
      "nodeId" | "parentNodeId" | "resourceKind" | "resourceId" | "position"
    >;
    const siblingsByParent = new Map<string | null, PlacementNode[]>();
    for (const node of snapshot.nodes) {
      const parentNodeId = node.parentNodeId ?? null;
      const siblings = siblingsByParent.get(parentNodeId) ?? [];
      siblings.push(node);
      siblingsByParent.set(parentNodeId, siblings);
    }
    for (const [parentNodeId, siblings] of siblingsByParent) {
      siblings.sort((left, right) => (
        left.position - right.position || left.nodeId.localeCompare(right.nodeId)
      ));
      siblingsByParent.set(parentNodeId, siblings);
    }
    const insertPlacementNode = (
      parentNodeId: string | null,
      position: number,
      node: PlacementNode,
    ): void => {
      const siblings = siblingsByParent.get(parentNodeId) ?? [];
      const insertion = Math.max(0, Math.min(position, siblings.length));
      siblings.splice(insertion, 0, node);
      siblingsByParent.set(parentNodeId, siblings);
    };
    const operations: ProjectExplorerOperation[] = [];
    let newSessionNode = snapshot.nodes.find((node) => (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && node.resourceId === NEW_SESSION_SYSTEM_RESOURCE_ID
    ));
    if (!placed.has(`${SYSTEM_RESOURCE_KIND}:${NEW_SESSION_SYSTEM_RESOURCE_ID}`)) {
      operations.push({
        kind: "placeResource",
        resourceKind: SYSTEM_RESOURCE_KIND,
        resourceId: NEW_SESSION_SYSTEM_RESOURCE_ID,
        position: 0,
      });
      newSessionNode = {
        nodeId: `pending:${SYSTEM_RESOURCE_KIND}:${NEW_SESSION_SYSTEM_RESOURCE_ID}`,
        projectId,
        nodeKind: "resource",
        resourceKind: SYSTEM_RESOURCE_KIND,
        resourceId: NEW_SESSION_SYSTEM_RESOURCE_ID,
        hidden: false,
        position: 0,
      };
      insertPlacementNode(null, 0, newSessionNode);
    }
    if (!placed.has(`${SYSTEM_RESOURCE_KIND}:${KNOWLEDGE_SYSTEM_RESOURCE_ID}`)) {
      const position = Math.min(1, siblingsByParent.get(null)?.length ?? 0);
      operations.push({
        kind: "placeResource",
        resourceKind: SYSTEM_RESOURCE_KIND,
        resourceId: KNOWLEDGE_SYSTEM_RESOURCE_ID,
        position,
      });
      insertPlacementNode(null, position, {
        nodeId: `pending:${SYSTEM_RESOURCE_KIND}:${KNOWLEDGE_SYSTEM_RESOURCE_ID}`,
        resourceKind: SYSTEM_RESOURCE_KIND,
        resourceId: KNOWLEDGE_SYSTEM_RESOURCE_ID,
        position,
      });
    }
    const sessionParentNodeId = newSessionNode?.parentNodeId ?? null;
    const sessionSiblings = siblingsByParent.get(sessionParentNodeId) ?? [];
    const newSessionPosition = newSessionNode
      ? sessionSiblings.findIndex((node) => node.nodeId === newSessionNode?.nodeId)
      : -1;
    const firstFollowingSessionPosition = sessionSiblings.findIndex((node, position) => (
      position > newSessionPosition && node.resourceKind === "session"
    ));
    let nextSessionPosition = firstFollowingSessionPosition >= 0
      ? firstFollowingSessionPosition
      : Math.max(0, newSessionPosition + 1);
    for (const session of projectResources.sessions) {
      if (session.sessionType === "folder" || placed.has(`session:${session.id}`)) continue;
      operations.push({
        kind: "placeResource",
        resourceKind: "session",
        resourceId: session.id,
        parentNodeId: sessionParentNodeId ?? undefined,
        position: nextSessionPosition,
      });
      insertPlacementNode(sessionParentNodeId, nextSessionPosition, {
        nodeId: `pending:session:${session.id}`,
        parentNodeId: sessionParentNodeId,
        resourceKind: "session",
        resourceId: session.id,
        position: nextSessionPosition,
      });
      nextSessionPosition += 1;
    }
    if (!placed.has(`${SYSTEM_RESOURCE_KIND}:${COLLABORATION_SYSTEM_RESOURCE_ID}`)) {
      const position = siblingsByParent.get(null)?.length ?? 0;
      operations.push({
        kind: "placeResource",
        resourceKind: SYSTEM_RESOURCE_KIND,
        resourceId: COLLABORATION_SYSTEM_RESOURCE_ID,
        position,
      });
    }
    if (operations.length === 0) return;
    try {
      const result = await projectExplorerApplyOperations(
        projectId,
        snapshot.revision,
        operations,
      );
      if (isCurrent(projectId, epoch)) snapshots[projectId] = result.snapshot;
    } catch (error) {
      const normalized = normalizeAppError(error);
      if (normalized.code !== "workspace.explorer_revision_conflict" || conflictRetries <= 0) {
        throw error;
      }
      const latest = await projectExplorerSnapshot(projectId);
      if (!isCurrent(projectId, epoch)) return;
      snapshots[projectId] = latest;
      await placeMissingResources(projectId, epoch, conflictRetries - 1);
    }
  }

  async function applyOperations(
    projectId: string,
    operations: ProjectExplorerOperation[],
  ): Promise<ProjectExplorerSnapshot> {
    ensureExplorerListener();
    let snapshot = snapshots[projectId] ?? await projectExplorerSnapshot(projectId);
    const operationId = crypto.randomUUID();
    pendingOperationIds.add(operationId);
    try {
      const result = await projectExplorerApplyOperations(
        projectId,
        snapshot.revision,
        operations,
        operationId,
      );
      snapshots[projectId] = result.snapshot;
      return result.snapshot;
    } catch (error) {
      const normalized = normalizeAppError(error);
      if (normalized.code !== "workspace.explorer_revision_conflict") throw error;
      snapshot = await projectExplorerSnapshot(projectId);
      const result = await projectExplorerApplyOperations(
        projectId,
        snapshot.revision,
        operations,
        operationId,
      );
      snapshots[projectId] = result.snapshot;
      return result.snapshot;
    } finally {
      pendingOperationIds.delete(operationId);
    }
  }

  function mountKey(projectId: string, nodeId: string): string {
    return `${projectId}:${snapshots[projectId]?.presetId ?? "unknown"}:${nodeId}`;
  }

  function clearProjectMounts(projectId: string): void {
    const prefix = `${projectId}:`;
    for (const key of Object.keys(mountListings)) {
      if (key.startsWith(prefix)) delete mountListings[key];
    }
  }

  async function loadMount(
    projectId: string,
    nodeId: string,
    force = false,
  ): Promise<ProjectExplorerMountListing> {
    const key = mountKey(projectId, nodeId);
    if (!force && mountListings[key]) return mountListings[key];
    const listing = await projectExplorerListMount(projectId, nodeId);
    mountListings[key] = listing;
    return listing;
  }

  function mountListing(
    projectId: string,
    nodeId: string,
  ): ProjectExplorerMountListing | null {
    return mountListings[mountKey(projectId, nodeId)] ?? null;
  }

  async function switchPreset(projectId: string, presetId: string): Promise<ProjectExplorerSnapshot> {
    const epoch = nextEpoch(projectId);
    const snapshot = await projectExplorerSwitchPreset(projectId, presetId);
    if (!isCurrent(projectId, epoch)) return snapshots[projectId] ?? snapshot;
    snapshots[projectId] = snapshot;
    clearProjectMounts(projectId);
    await placeMissingResources(projectId, epoch);
    return snapshots[projectId];
  }

  async function createPreset(projectId: string, name: string): Promise<ProjectExplorerSnapshot> {
    const sourcePresetId = snapshots[projectId]?.presetId ?? null;
    const snapshot = await projectExplorerCreatePreset(projectId, name, sourcePresetId);
    snapshots[projectId] = snapshot;
    clearProjectMounts(projectId);
    return snapshot;
  }

  async function renamePreset(
    projectId: string,
    presetId: string,
    name: string,
  ): Promise<ProjectExplorerSnapshot> {
    const snapshot = await projectExplorerRenamePreset(projectId, presetId, name);
    snapshots[projectId] = snapshot;
    return snapshot;
  }

  async function deletePreset(
    projectId: string,
    presetId: string,
  ): Promise<ProjectExplorerSnapshot> {
    const snapshot = await projectExplorerDeletePreset(projectId, presetId);
    snapshots[projectId] = snapshot;
    clearProjectMounts(projectId);
    return snapshot;
  }

  function toggleExpanded(key: string): void {
    const next = new Set(expandedNodeKeys.value);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedNodeKeys.value = next;
  }

  return {
    displaySettings,
    snapshots,
    mountListings,
    resources,
    loading,
    errors,
    selectedNodeKey,
    expandedNodeKeys,
    loadProject,
    refreshProjectSessions,
    applyOperations,
    loadMount,
    mountListing,
    switchPreset,
    createPreset,
    renamePreset,
    deletePreset,
    toggleExpanded,
  };
});
