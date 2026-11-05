import { reactive, ref } from "vue";
import { defineStore } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  acknowledgeSessionPromotions, attentionLayoutKey, pendingSessionPromotions,
  sessionAttention, sessionPromotionOperations, updateSessionAttention,
} from "../services/sessionAttention";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { normalizeAppError } from "../services/errors";
import { projectKnowledgeEnabled, type RoutedWorkspaceEvent } from "../services/project";
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
import type { KnowledgeChangedEvent } from "../types";
import { listenWorkspaceEvent } from "../services/workspaceEventHub";
import { newSessionInsertionIndex, type SessionInsertionRules } from "./workspaceSessionPlacement";
import { useWorkspaceEventScope } from "../composables/useWorkspaceEventScope";
import { useWorkspaceContextStore } from "./workspaceContext";
import { viewList } from "../services/view";
import type { WorkspaceViewReference } from "../components/view/viewWorkspaceDrag";

const SYSTEM_RESOURCE_KIND = "system" as const;
const NEW_SESSION_SYSTEM_RESOURCE_ID = "newSession";
const KNOWLEDGE_SYSTEM_RESOURCE_ID = "knowledge";
const COLLABORATION_SYSTEM_RESOURCE_ID = "collaboration";
const ASSETS_SYSTEM_RESOURCE_ID = "assets";
const VIEWS_SYSTEM_RESOURCE_ID = "views";
const AGENTS_SYSTEM_RESOURCE_ID = "agents";
const ARCHIVED_SYSTEM_RESOURCE_ID = "archived";

function emptyResources(): ProjectExplorerResources {
  return { sessions: [], knowledge: [], collaboration: null };
}

export const useWorkspaceExplorerStore = defineStore("workspaceExplorer", () => {
  const workspaceEventSignal = useWorkspaceEventScope();
  const { state: displaySettings } = useDisplaySettings();
  const snapshots = reactive<Record<string, ProjectExplorerSnapshot>>({});
  const mountListings = reactive<Record<string, ProjectExplorerMountListing>>({});
  const resources = reactive<Record<string, ProjectExplorerResources>>({});
  const viewResources = reactive<Record<string, WorkspaceViewReference[]>>({});
  const viewRequestEpochs = new Map<string, number>();
  const loading = reactive<Record<string, boolean>>({});
  const errors = reactive<Record<string, string>>({});
  const requestEpochs = new Map<string, number>();
  const sessionRequestEpochs = new Map<string, number>();
  const knowledgeRequestEpochs = new Map<string, number>();
  const knowledgeCatalogLoaded = new Set<string>();
  const queuedKnowledgeRefreshes = new Set<string>();
  const activeKnowledgeRefreshes = new Map<string, Promise<void>>();
  const pendingOperationIds = new Set<string>();
  let explorerUnlisten: UnlistenFn | null = null;
  let explorerListenerStarting = false;
  let knowledgeUnlisten: UnlistenFn | null = null;
  let knowledgeListenerStarting = false;
  const selectedNodeKey = ref<string | null>(null);
  const expandedNodeKeys = ref<Set<string>>(new Set());

  function knowledgeEnabled(projectId: string): boolean {
    return projectKnowledgeEnabled(useWorkspaceContextStore().projectsById[projectId]);
  }

  function acceptSnapshot(projectId: string, snapshot: ProjectExplorerSnapshot): ProjectExplorerSnapshot {
    const current = snapshots[projectId];
    if (current?.presetId === snapshot.presetId && current.revision > snapshot.revision) return current;
    snapshots[projectId] = snapshot;
    return snapshot;
  }

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

  function nextKnowledgeEpoch(projectId: string): number {
    const epoch = (knowledgeRequestEpochs.get(projectId) ?? 0) + 1;
    knowledgeRequestEpochs.set(projectId, epoch);
    return epoch;
  }

  function isCurrentKnowledgeRequest(projectId: string, epoch: number): boolean {
    return knowledgeRequestEpochs.get(projectId) === epoch;
  }

  function ensureKnowledgeListener(): void {
    if (knowledgeUnlisten || knowledgeListenerStarting) return;
    knowledgeListenerStarting = true;
    void listenWorkspaceEvent<RoutedWorkspaceEvent<KnowledgeChangedEvent>>(
      "workspaceExplorer.ensureKnowledgeListener",
      ({ payload: event }) => {
        if (event.eventName === "view-package-reloaded" || event.eventName === "view-tree-changed") {
          if (snapshots[event.projectId]?.nodes.some((node) => node.resourceKind === "view")) {
            void refreshProjectViews(event.projectId);
          }
          return;
        }
        if (event.eventName !== "knowledge-changed") return;
        if (!snapshots[event.projectId] || !resources[event.projectId]) return;
        scheduleProjectKnowledgeRefresh(event.projectId);
      },
      { signal: workspaceEventSignal },
    ).then((unlisten) => {
      knowledgeUnlisten = unlisten;
    }).catch(() => {
      knowledgeListenerStarting = false;
    });
  }

  function ensureExplorerListener(): void {
    ensureKnowledgeListener();
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
    const knowledgeEpoch = nextKnowledgeEpoch(projectId);
    loading[projectId] = true;
    delete errors[projectId];
    try {
      const [snapshotResult, sessionsResult, knowledgeResult, collaborationResult] = await Promise.allSettled([
        projectExplorerSnapshot(projectId),
        listProjectSessions(projectId),
        knowledgeEnabled(projectId) ? projectKnowledgeList(projectId) : Promise.resolve([]),
        projectCollaborationSnapshot(projectId),
      ]);
      if (!isCurrent(projectId, epoch)) return;
      if (snapshotResult.status === "rejected") throw snapshotResult.reason;
      const previous = resources[projectId] ?? emptyResources();
      const previousPresetId = snapshots[projectId]?.presetId;
      acceptSnapshot(projectId, snapshotResult.value);
      if (previousPresetId && previousPresetId !== snapshotResult.value.presetId) {
        clearProjectMounts(projectId);
      }
      resources[projectId] = {
        sessions: sessionsResult.status === "fulfilled"
          && isCurrentSessionRequest(projectId, sessionEpoch)
          ? sessionsResult.value
          : previous.sessions,
        knowledge: knowledgeResult.status === "fulfilled"
          && isCurrentKnowledgeRequest(projectId, knowledgeEpoch)
          ? knowledgeResult.value
          : previous.knowledge,
        collaboration: collaborationResult.status === "fulfilled"
          ? collaborationResult.value
          : previous.collaboration,
      };
      if (snapshotResult.value.nodes.some((node) => node.resourceKind === "view")) {
        await refreshProjectViews(projectId);
        if (!isCurrent(projectId, epoch)) return;
      }
      if (
        knowledgeResult.status === "fulfilled"
        && isCurrentKnowledgeRequest(projectId, knowledgeEpoch)
      ) {
        knowledgeCatalogLoaded.add(projectId);
      }
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

  function rememberView(reference: WorkspaceViewReference): void {
    viewRequestEpochs.set(reference.projectId, (viewRequestEpochs.get(reference.projectId) ?? 0) + 1);
    const current = viewResources[reference.projectId] ?? [];
    viewResources[reference.projectId] = [
      ...current.filter((item) => item.view.id !== reference.view.id
        || item.workspaceRef.checkoutId !== reference.workspaceRef.checkoutId),
      reference,
    ];
  }

  async function refreshProjectViews(projectId: string): Promise<void> {
    const contexts = useWorkspaceContextStore();
    const project = contexts.projectsById[projectId];
    if (!project) return;
    const epoch = (viewRequestEpochs.get(projectId) ?? 0) + 1;
    viewRequestEpochs.set(projectId, epoch);
    const checkouts = project.checkouts.filter((checkout) => checkout.available !== false && checkout.runtime);
    const results = await Promise.allSettled(checkouts.map(async (checkout) => {
      const workspaceRef = {
        checkoutId: checkout.checkoutId,
        expectedGeneration: checkout.runtime!.workspaceGeneration,
        expectedMaterializationEpoch: checkout.runtime!.materializationEpoch,
      };
      const views = await viewList(workspaceRef);
      return views.map((view) => ({ projectId, workspaceRef, view }));
    }));
    if (viewRequestEpochs.get(projectId) !== epoch) return;
    const previous = viewResources[projectId] ?? [];
    viewResources[projectId] = results.flatMap((result, index) => result.status === "fulfilled"
      ? result.value
      : previous.filter((item) => item.workspaceRef.checkoutId === checkouts[index]?.checkoutId));
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

  function newlyCreatedPlanDesignDocuments(
    previous: ProjectExplorerResources["knowledge"],
    next: ProjectExplorerResources["knowledge"],
  ): ProjectExplorerResources["knowledge"] {
    const previousIds = new Set(previous.map((document) => document.id));
    return next
      .filter((document) => (
        (document.type === "plan" || document.type === "design")
        && document.storageSource !== "app"
        && !previousIds.has(document.id)
      ))
      .sort((left, right) => (
        right.modifiedAt - left.modifiedAt
        || left.path.localeCompare(right.path, undefined, { sensitivity: "base", numeric: true })
      ));
  }

  async function placeKnowledgeDocumentsBelowSpecialNode(
    projectId: string,
    documents: ProjectExplorerResources["knowledge"],
  ): Promise<void> {
    const snapshot = snapshots[projectId];
    if (!snapshot || documents.length === 0) return;
    const knowledgeNode = snapshot.nodes.find((node) => (
      node.resourceKind === SYSTEM_RESOURCE_KIND
      && node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID
    ));
    if (!knowledgeNode) return;

    const placedDocumentIds = new Set(snapshot.nodes.flatMap((node) => (
      node.resourceKind === "knowledge" && node.resourceId ? [node.resourceId] : []
    )));
    const documentsToPlace = documents.filter((document) => !placedDocumentIds.has(document.id));
    if (documentsToPlace.length === 0) return;

    const parentNodeId = knowledgeNode.parentNodeId ?? null;
    const siblings = snapshot.nodes
      .filter((node) => (node.parentNodeId ?? null) === parentNodeId)
      .sort((left, right) => left.position - right.position || left.nodeId.localeCompare(right.nodeId));
    const knowledgePosition = siblings.findIndex((node) => node.nodeId === knowledgeNode.nodeId);
    if (knowledgePosition < 0) return;

    await applyOperations(projectId, documentsToPlace.map((document, index) => ({
      kind: "placeResource" as const,
      resourceKind: "knowledge" as const,
      resourceId: document.id,
      sourceKind: "knowledge",
      parentNodeId: parentNodeId ?? undefined,
      position: knowledgePosition + 1 + index,
    })));
  }

  async function refreshProjectKnowledge(projectId: string): Promise<void> {
    const previousResources = resources[projectId];
    if (!projectId || !previousResources || !snapshots[projectId]) return;
    const hadLoadedCatalog = knowledgeCatalogLoaded.has(projectId);
    const epoch = nextKnowledgeEpoch(projectId);
    const nextKnowledge = knowledgeEnabled(projectId) ? await projectKnowledgeList(projectId) : [];
    if (!isCurrentKnowledgeRequest(projectId, epoch)) return;

    const addedDocuments = hadLoadedCatalog
      ? newlyCreatedPlanDesignDocuments(previousResources.knowledge, nextKnowledge)
      : [];
    resources[projectId] = {
      ...previousResources,
      knowledge: nextKnowledge,
    };
    knowledgeCatalogLoaded.add(projectId);

    if (
      displaySettings.autoPlaceNewPlanDesignKnowledgeDocuments
      && addedDocuments.length > 0
    ) {
      await placeKnowledgeDocumentsBelowSpecialNode(projectId, addedDocuments);
    }
  }

  function scheduleProjectKnowledgeRefresh(projectId: string): void {
    queuedKnowledgeRefreshes.add(projectId);
    if (activeKnowledgeRefreshes.has(projectId)) return;
    const task = (async () => {
      while (queuedKnowledgeRefreshes.delete(projectId)) {
        await refreshProjectKnowledge(projectId);
      }
    })().catch((error) => {
      errors[projectId] = normalizeAppError(error).message;
    }).finally(() => {
      if (activeKnowledgeRefreshes.get(projectId) === task) {
        activeKnowledgeRefreshes.delete(projectId);
      }
      if (queuedKnowledgeRefreshes.has(projectId)) scheduleProjectKnowledgeRefresh(projectId);
    });
    activeKnowledgeRefreshes.set(projectId, task);
  }

  function isEntryNodeShown(projectId: string, node: { resourceId?: string | null; hidden?: boolean }): boolean {
    if (node.hidden) return false;
    const sections = displaySettings.workspaceSectionVisibility;
    switch (node.resourceId) {
      case KNOWLEDGE_SYSTEM_RESOURCE_ID: return knowledgeEnabled(projectId) && sections.knowledge;
      case COLLABORATION_SYSTEM_RESOURCE_ID: return sections.collab;
      case ASSETS_SYSTEM_RESOURCE_ID: return sections.assets;
      case VIEWS_SYSTEM_RESOURCE_ID: return sections.views;
      case AGENTS_SYSTEM_RESOURCE_ID: return displaySettings.showAgentTab;
      default: return true;
    }
  }

  function sessionInsertionRules<Node extends { nodeId: string; resourceKind?: string | null; resourceId?: string | null; hidden?: boolean }>(
    projectId: string,
    snapshot: ProjectExplorerSnapshot,
    sessionIds: { has(sessionId: string): boolean },
  ): SessionInsertionRules<Node> {
    const pinned = new Set((snapshot.itemStates ?? [])
      .filter((state) => state.pinned && !state.relativePath)
      .map((state) => state.nodeId));
    return {
      isSessionAnchor: (node) => node.resourceKind === "session" && !!node.resourceId
        && sessionIds.has(node.resourceId) && !node.hidden && !pinned.has(node.nodeId),
      isShownEntry: (node) => isEntryNodeShown(projectId, node),
    };
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
    > & { hidden?: boolean };
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
    // Missing special nodes join the root special block. Knowledge closes it, so
    // the documents placed below it sit between the special nodes and sessions.
    const specialNodeInsertionIndex = (resourceId: string): number => {
      const roots = siblingsByParent.get(null) ?? [];
      let end = 0;
      for (const [index, node] of roots.entries()) {
        if (node.resourceKind !== SYSTEM_RESOURCE_KIND) continue;
        if (node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID && resourceId !== KNOWLEDGE_SYSTEM_RESOURCE_ID) return index;
        end = index + 1;
      }
      return end;
    };
    // Include every default special node before choosing the first session's position.
    for (const resourceId of [
      COLLABORATION_SYSTEM_RESOURCE_ID,
      ASSETS_SYSTEM_RESOURCE_ID,
      VIEWS_SYSTEM_RESOURCE_ID,
      AGENTS_SYSTEM_RESOURCE_ID,
      ARCHIVED_SYSTEM_RESOURCE_ID,
      ...(knowledgeEnabled(projectId) ? [KNOWLEDGE_SYSTEM_RESOURCE_ID] : []),
    ]) {
      if (placed.has(`${SYSTEM_RESOURCE_KIND}:${resourceId}`)) continue;
      const position = specialNodeInsertionIndex(resourceId);
      operations.push({
        kind: "placeResource",
        resourceKind: SYSTEM_RESOURCE_KIND,
        resourceId,
        position,
      });
      insertPlacementNode(null, position, {
        nodeId: `pending:${SYSTEM_RESOURCE_KIND}:${resourceId}`,
        resourceKind: SYSTEM_RESOURCE_KIND,
        resourceId,
        position,
      });
    }
    const sessionsById = new Map(
      projectResources.sessions
        .filter((session) => session.sessionType !== "folder")
        .map((session) => [session.id, session]),
    );
    const sessionParentNodeId = newSessionNode?.parentNodeId ?? null;
    const sessionSiblings = siblingsByParent.get(sessionParentNodeId) ?? [];
    const newSessionPosition = newSessionNode
      ? sessionSiblings.findIndex((node) => node.nodeId === newSessionNode?.nodeId)
      : -1;
    let nextSessionPosition = newSessionInsertionIndex(
      sessionSiblings,
      newSessionPosition,
      sessionInsertionRules(projectId, snapshot, sessionsById),
    );
    const sessionPlacementById = new Map<string, PlacementNode>(
      snapshot.nodes.flatMap((node) => (
        node.resourceKind === "session" && node.resourceId ? [[node.resourceId, node] as const] : []
      )),
    );
    const pendingSessions = new Map(
      [...sessionsById.values()]
        .filter((session) => !placed.has(`session:${session.id}`))
        .map((session) => [session.id, session]),
    );
    const plannedNodeIds = new Map(
      [...pendingSessions.keys()].map((sessionId) => [
        sessionId,
        `resource:session:${crypto.randomUUID()}`,
      ]),
    );
    const placeSession = (sessionId: string, parentNodeId: string | null): void => {
      const session = pendingSessions.get(sessionId);
      const nodeId = plannedNodeIds.get(sessionId);
      if (!session || !nodeId) return;
      const position = parentNodeId === sessionParentNodeId
        ? nextSessionPosition++
        : siblingsByParent.get(parentNodeId)?.length ?? 0;
      operations.push({
        kind: "placeResource",
        nodeId,
        resourceKind: "session",
        resourceId: session.id,
        parentNodeId: parentNodeId ?? undefined,
        position,
      });
      const placement: PlacementNode = {
        nodeId,
        parentNodeId,
        resourceKind: "session",
        resourceId: session.id,
        position,
      };
      insertPlacementNode(parentNodeId, position, placement);
      sessionPlacementById.set(session.id, placement);
      pendingSessions.delete(session.id);
    };
    while (pendingSessions.size > 0) {
      let placedInPass = 0;
      for (const session of [...pendingSessions.values()]) {
        const parentSessionId = session.parentSessionId ?? null;
        const parentPlacement = parentSessionId
          ? sessionPlacementById.get(parentSessionId)
          : undefined;
        if (parentSessionId && pendingSessions.has(parentSessionId) && !parentPlacement) continue;
        placeSession(session.id, parentPlacement?.nodeId ?? sessionParentNodeId);
        placedInPass += 1;
      }
      if (placedInPass > 0) continue;
      const cyclicSession = pendingSessions.values().next().value;
      if (!cyclicSession) break;
      placeSession(cyclicSession.id, sessionParentNodeId);
    }
    if (operations.length === 0) return;
    try {
      const result = await projectExplorerApplyOperations(
        projectId,
        snapshot.revision,
        operations,
      );
      if (isCurrent(projectId, epoch)) acceptSnapshot(projectId, result.snapshot);
    } catch (error) {
      const normalized = normalizeAppError(error);
      if (normalized.code !== "workspace.explorer_revision_conflict" || conflictRetries <= 0) {
        throw error;
      }
      const latest = await projectExplorerSnapshot(projectId);
      if (!isCurrent(projectId, epoch)) return;
      acceptSnapshot(projectId, latest);
      await placeMissingResources(projectId, epoch, conflictRetries - 1);
    }
  }

  async function applyOperations(
    projectId: string,
    operations: ProjectExplorerOperation[],
  ): Promise<ProjectExplorerSnapshot> {
    const startedSequence = sessionAttention.value.sequence;
    const result = await applyOperationsUnlocked(projectId, operations);
    await updateSessionAttention((attention) => {
      // Explicit layout changes win over completions already received, including
      // a moved folder's descendants. A later run can still promote normally.
      const moved = new Set(operations.flatMap((operation) => (
        operation.kind === "moveNode" || operation.kind === "setItemState" ? [operation.nodeId] : []
      )));
      const sessions = result.nodes.filter((node) => {
        let current: typeof node | undefined = node;
        const visited = new Set<string>();
        while (current && !visited.has(current.nodeId)) {
          if (moved.has(current.nodeId)) return true;
          visited.add(current.nodeId);
          current = result.nodes.find((item) => item.nodeId === current!.parentNodeId);
        }
        return false;
      }).flatMap((node) => node.resourceKind === "session" && node.resourceId ? [node.resourceId] : []);
      acknowledgeSessionPromotions(attention, result, sessions.filter((id) => (attention.sessions[id]?.completionSequence ?? 0) <= startedSequence));
    });
    return result;
  }

  async function promoteCompletedSessions(projectId: string, canMove: () => boolean): Promise<void> {
    await updateSessionAttention(async (attention) => {
      if (!canMove()) return;
      let snapshot = snapshots[projectId];
      if (!snapshot) return;
      const layoutKey = attentionLayoutKey(snapshot);
      if (!attention.promoted[layoutKey]) {
        acknowledgeSessionPromotions(attention, snapshot, pendingSessionPromotions(attention, snapshot));
        return;
      }
      if (!pendingSessionPromotions(attention, snapshot).length) return;
      snapshot = await projectExplorerSnapshot(projectId);
      if (attentionLayoutKey(snapshot) !== layoutKey || !canMove()) return;
      for (let attempt = 0; attempt < 2; attempt += 1) {
        const pending = pendingSessionPromotions(attention, snapshot);
        const operations = sessionPromotionOperations(snapshot, displaySettings.autoPromoteCompletedSessions
          ? pending : []);
        try {
          if (operations.length) {
            const result = await projectExplorerApplyOperations(projectId, snapshot.revision, operations, crypto.randomUUID());
            snapshot = result.snapshot;
          }
          acknowledgeSessionPromotions(attention, snapshot, pending);
          acceptSnapshot(projectId, snapshot);
          return;
        } catch (error) {
          if (attempt || normalizeAppError(error).code !== "workspace.explorer_revision_conflict") throw error;
          snapshot = await projectExplorerSnapshot(projectId);
          if (!canMove() || attentionLayoutKey(snapshot) !== layoutKey) return;
        }
      }
    });
  }

  async function applyOperationsUnlocked(
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
      return acceptSnapshot(projectId, result.snapshot);
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
      return acceptSnapshot(projectId, result.snapshot);
    } finally {
      pendingOperationIds.delete(operationId);
    }
  }

  function mountKey(projectId: string, nodeId: string): string {
    return `${projectId}:${snapshots[projectId]?.presetId ?? "unknown"}:${nodeId}`;
  }

  async function pinResources(projectId: string, placements: ProjectExplorerOperation[]): Promise<ProjectExplorerSnapshot> {
    const snapshot = snapshots[projectId] ?? await projectExplorerSnapshot(projectId);
    const operations: ProjectExplorerOperation[] = [];
    const resolved = new Map<string, string>();
    const pathKey = (path: string) => path.replace(/\\/g, "/").replace(/\/+$/, "").toLocaleLowerCase();
    for (const placement of placements) {
      if (placement.kind !== "placeResource" && placement.kind !== "mountPath") continue;
      const key = placement.kind === "mountPath" ? `path:${pathKey(placement.path)}`
        : `${placement.resourceKind}:${placement.resourceId}`;
      if (resolved.has(key)) continue;
      const existing = snapshot.nodes.find((node) => placement.kind === "mountPath"
        ? !!node.sourcePath && pathKey(node.sourcePath) === pathKey(placement.path)
        : node.resourceKind === placement.resourceKind && node.resourceId === placement.resourceId);
      const nodeId = existing?.nodeId ?? placement.nodeId ?? `pinned:${crypto.randomUUID()}`;
      resolved.set(key, nodeId);
      if (!existing) operations.push({ ...placement, nodeId });
      operations.push({ kind: "setItemState", nodeId, pinned: true });
    }
    return operations.length ? applyOperations(projectId, operations) : snapshot;
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
    if (snapshot.nodes.some((node) => node.resourceKind === "view")) await refreshProjectViews(projectId);
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
    viewResources,
    loading,
    errors,
    selectedNodeKey,
    expandedNodeKeys,
    loadProject,
    refreshProjectSessions,
    refreshProjectKnowledge,
    refreshProjectViews,
    rememberView,
    applyOperations,
    promoteCompletedSessions,
    pinResources,
    loadMount,
    mountListing,
    switchPreset,
    createPreset,
    renamePreset,
    deletePreset,
    toggleExpanded,
  };
});
