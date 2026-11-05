import { ref } from "vue";
import { defineStore } from "pinia";
import { useNewSessionSettings } from "../composables/useNewSessionSettings";
import * as agentService from "../services/agent";
import type { WorkspaceRef } from "../services/project";
import {
  getWorkspaceSelection,
  saveWorkspaceSelection,
  type WorkspaceSelectionMemory,
  type WorkspaceSelectionPatch,
  type WorkspaceSelectionSnapshot,
} from "../services/selectionMemory";
import type { AgentInfo, EffortLevel } from "../types";
import { useModelStore } from "./model";
import {
  isEffortLevel,
  resolveNewSessionAgentId,
  resolveNewSessionModel,
} from "./newSessionSelection";

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeMemory(value: unknown): WorkspaceSelectionMemory {
  const record = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const agents: WorkspaceSelectionMemory["agents"] = {};
  if (record.agents && typeof record.agents === "object") {
    for (const [agentId, entry] of Object.entries(record.agents as Record<string, unknown>)) {
      const item = entry && typeof entry === "object" ? entry as Record<string, unknown> : {};
      const modelId = text(item.modelId);
      if (agentId.trim() && modelId) agents[agentId.trim()] = { modelId, effort: text(item.effort) };
    }
  }
  return {
    agentId: text(record.agentId),
    modelId: text(record.modelId),
    effort: text(record.effort),
    fastMode: typeof record.fastMode === "boolean" ? record.fastMode : null,
    agents,
    updatedAtMs: typeof record.updatedAtMs === "number" ? record.updatedAtMs : 0,
  };
}

function normalizeSnapshot(value: unknown): WorkspaceSelectionSnapshot | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const projectId = text(record.projectId);
  if (!projectId) return null;
  return {
    projectId,
    memory: normalizeMemory(record.memory),
    lastAgentId: text(record.lastAgentId),
    workspaceDefaultAgentId: text(record.workspaceDefaultAgentId) || null,
  };
}

export interface NewSessionSelection {
  agentId: string;
  modelId: string;
  effort: EffortLevel;
  fastMode: boolean;
}

/**
 * Per-workspace memory of the user's last explicit Agent/model choice, and the
 * single place that decides what a new session starts with.
 */
export const useSelectionMemoryStore = defineStore("selectionMemory", () => {
  const { state: settings } = useNewSessionSettings();
  const modelStore = useModelStore();
  /** Snapshots keyed by checkout id; worktrees of one project share content. */
  const snapshots = ref<Record<string, WorkspaceSelectionSnapshot>>({});
  const loads = new Map<string, Promise<WorkspaceSelectionSnapshot | null>>();
  const revisions = new Map<string, number>();
  let saveQueue: Promise<unknown> = Promise.resolve();

  function snapshotFor(checkoutId: string | null | undefined): WorkspaceSelectionSnapshot | null {
    return checkoutId ? snapshots.value[checkoutId] ?? null : null;
  }

  function bumpRevision(checkoutId: string) {
    revisions.set(checkoutId, (revisions.get(checkoutId) ?? 0) + 1);
  }

  function storeSnapshot(checkoutId: string, snapshot: WorkspaceSelectionSnapshot) {
    const next = { ...snapshots.value, [checkoutId]: snapshot };
    for (const [id, existing] of Object.entries(next)) {
      if (id === checkoutId || existing.projectId !== snapshot.projectId) continue;
      next[id] = { ...snapshot, workspaceDefaultAgentId: existing.workspaceDefaultAgentId };
    }
    snapshots.value = next;
  }

  async function load(workspaceRef: WorkspaceRef | null | undefined): Promise<WorkspaceSelectionSnapshot | null> {
    const checkoutId = workspaceRef?.checkoutId;
    if (!checkoutId) return null;
    const pending = loads.get(checkoutId);
    if (pending) return pending;
    const revision = revisions.get(checkoutId) ?? 0;
    const target = { ...workspaceRef };
    const request = Promise.resolve()
      .then(() => getWorkspaceSelection(target))
      .then((result) => {
        const snapshot = normalizeSnapshot(result);
        // A local choice made while loading wins; its save refreshes the snapshot.
        if (snapshot && (revisions.get(checkoutId) ?? 0) === revision) storeSnapshot(checkoutId, snapshot);
        return snapshotFor(checkoutId) ?? snapshot;
      })
      .catch((error: unknown) => {
        console.warn("[selection] get_workspace_selection:", error);
        return snapshotFor(checkoutId);
      })
      .finally(() => loads.delete(checkoutId));
    loads.set(checkoutId, request);
    return request;
  }

  function applyLocally(checkoutId: string, patch: WorkspaceSelectionPatch) {
    const current = snapshotFor(checkoutId);
    if (!current) return;
    const memory: WorkspaceSelectionMemory = {
      ...current.memory,
      agents: { ...current.memory.agents },
      updatedAtMs: Date.now(),
    };
    if (patch.agentId) memory.agentId = patch.agentId;
    if (patch.modelId) memory.modelId = patch.modelId;
    if (patch.effort) memory.effort = patch.effort;
    if (typeof patch.fastMode === "boolean") memory.fastMode = patch.fastMode;
    // Only a choice that names the model describes the Agent's model.
    if (patch.modelId && memory.agentId) {
      memory.agents[memory.agentId] = { modelId: memory.modelId, effort: memory.effort };
    }
    storeSnapshot(checkoutId, {
      ...current,
      memory,
      lastAgentId: patch.agentId || current.lastAgentId,
    });
  }

  /** Records an explicit user choice; omitted fields keep their remembered value. */
  function remember(
    workspaceRef: WorkspaceRef | null | undefined,
    selection: { agentId?: string; modelId?: string; effort?: EffortLevel | null; fastMode?: boolean | null },
  ): Promise<unknown> {
    const checkoutId = workspaceRef?.checkoutId;
    if (!checkoutId) return Promise.resolve();
    const patch: WorkspaceSelectionPatch = {};
    const agentId = selection.agentId?.trim();
    const modelId = selection.modelId?.trim();
    if (agentId) patch.agentId = agentId;
    if (modelId) patch.modelId = modelId;
    if (selection.effort && isEffortLevel(selection.effort)) patch.effort = selection.effort;
    if (typeof selection.fastMode === "boolean") patch.fastMode = selection.fastMode;
    if (!patch.agentId && !patch.modelId && !patch.effort && patch.fastMode === undefined) return Promise.resolve();

    bumpRevision(checkoutId);
    applyLocally(checkoutId, patch);
    if (modelId) modelStore.noteRememberedSelection(agentId ?? "", modelId, patch.effort);
    const target = { ...workspaceRef };
    saveQueue = saveQueue
      .catch(() => undefined)
      .then(() => saveWorkspaceSelection(target, patch))
      .then((result) => {
        const snapshot = normalizeSnapshot(result);
        if (snapshot) storeSnapshot(checkoutId, snapshot);
      })
      .catch((error: unknown) => console.warn("[selection] save_workspace_selection:", error));
    return saveQueue;
  }

  /** Makes an Agent the workspace default; it also becomes the user's current choice. */
  async function setWorkspaceDefaultAgent(workspaceRef: WorkspaceRef, agentId: string): Promise<void> {
    await agentService.setWorkspaceDefaultAgent({ ...workspaceRef }, agentId);
    const checkoutId = workspaceRef.checkoutId;
    bumpRevision(checkoutId);
    applyLocally(checkoutId, { agentId });
    const current = snapshotFor(checkoutId);
    if (current) storeSnapshot(checkoutId, { ...current, workspaceDefaultAgentId: agentId });
    await load(workspaceRef);
  }

  function resolveAgentId(
    checkoutId: string | null | undefined,
    agents: readonly Pick<AgentInfo, "id" | "isDefault">[],
    options: { forNewSession?: boolean } = {},
  ): string {
    if (options.forNewSession === false) return resolveNewSessionAgentId({ agents, mode: "workspace" });
    return resolveNewSessionAgentId({
      agents,
      mode: settings.agentMode,
      fixedAgentId: settings.defaultAgentId,
      snapshot: snapshotFor(checkoutId),
      legacyAgentId: settings.legacyAgentId,
    });
  }

  function fallbackEffort(agent: Pick<AgentInfo, "defaultEffort"> | null | undefined): EffortLevel {
    return modelStore.hasUserDefaultEffort ? modelStore.defaultEffort : (agent?.defaultEffort ?? "none");
  }

  function clampEffort(modelId: string, level: EffortLevel): EffortLevel {
    return modelStore.clampEffortForModel(modelId, level);
  }

  /** Model, effort and fast mode a new session with this Agent starts with. */
  function resolveExecution(
    checkoutId: string | null | undefined,
    agent: Pick<AgentInfo, "id" | "defaultEffort"> | null | undefined,
  ): Omit<NewSessionSelection, "agentId"> {
    const snapshot = snapshotFor(checkoutId);
    const resolved = resolveNewSessionModel({
      models: modelStore.availableModels,
      agentId: agent?.id ?? "",
      pinnedModelId: modelStore.modelDefaults.mainModel,
      snapshot,
      agentPreferences: modelStore.agentModelPreferences,
      lastModelId: modelStore.lastModelId,
    });
    const remembered = snapshot?.memory.fastMode;
    return {
      modelId: resolved.modelId,
      effort: clampEffort(resolved.modelId, resolved.effort ?? fallbackEffort(agent)),
      fastMode: typeof remembered === "boolean" ? remembered : modelStore.defaultCodexFastMode,
    };
  }

  function resolveNewSession(
    checkoutId: string | null | undefined,
    agents: readonly Pick<AgentInfo, "id" | "isDefault" | "defaultEffort">[],
  ): NewSessionSelection {
    const agentId = resolveAgentId(checkoutId, agents);
    return { agentId, ...resolveExecution(checkoutId, agents.find((agent) => agent.id === agentId)) };
  }

  return {
    snapshots,
    snapshotFor,
    load,
    remember,
    setWorkspaceDefaultAgent,
    resolveAgentId,
    resolveExecution,
    resolveNewSession,
    fallbackEffort,
    clampEffort,
  };
});
