import { computed, ref } from "vue";
import { defineStore } from "pinia";
import type {
  WorkbenchDropDirection,
  WorkbenchEditorGroup,
  WorkbenchEditorInput,
  WorkbenchResourceRef,
  WorkbenchSplitNode,
  WorkbenchTransferAcceptResult,
  WorkbenchWindowState,
} from "../types/workbench";

export const WORKBENCH_SCHEMA_VERSION = 1;
export const WORKBENCH_MIN_SPLIT_RATIO = 0.18;
export const WORKBENCH_MAX_SPLIT_RATIO = 0.82;
const WORKBENCH_STORAGE_PREFIX = "locus:workbench-window:";

export function isWorkbenchMarkdownPath(path: string): boolean {
  return /\.(?:md|markdown)$/i.test(path.trim());
}

export function normalizeWorkbenchResource(
  resource: WorkbenchResourceRef,
): WorkbenchResourceRef {
  if (resource.kind === "asset" && isWorkbenchMarkdownPath(resource.path)) {
    return {
      kind: "workspaceFile",
      projectId: resource.projectId,
      path: resource.path,
    };
  }
  return resource;
}

function nextStableId(prefix: string): string {
  const uuid = globalThis.crypto?.randomUUID?.();
  if (uuid) return `${prefix}-${uuid}`;
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function workbenchResourceKey(resource: WorkbenchResourceRef): string {
  resource = normalizeWorkbenchResource(resource);
  switch (resource.kind) {
    case "project": return `project:${resource.projectId}`;
    case "newSession": return `new-session:${resource.projectId}`;
    case "checkout": return `checkout:${resource.projectId}:${resource.checkoutId}`;
    case "section": return `section:${resource.projectId}:${resource.section}`;
    case "knowledgeRoot": return `knowledge-root:${resource.projectId}`;
    case "collaboration": return `collaboration:${resource.projectId}`;
    case "folder": return `folder:${resource.projectId}:${resource.nodeId}`;
    case "session": return `session:${resource.projectId}:${resource.sessionId}`;
    case "knowledge": return `knowledge:${resource.projectId}:${resource.documentId}`;
    case "workspaceFile": return `workspace-file:${resource.projectId}:${resource.path}`;
    case "asset": return `asset:${resource.projectId}:${resource.path}`;
    case "sceneObject": return [
      "scene-object",
      resource.projectId,
      resource.scenePath,
      resource.objectPath,
    ].join(":");
    case "view": return `view:${resource.projectId}:${resource.viewId}`;
    case "localDirectory": return [
      "local-directory",
      resource.projectId,
      resource.nodeId,
      resource.relativePath ?? "",
    ].join(":");
    case "localFile": return [
      "local-file",
      resource.projectId,
      resource.nodeId,
      resource.relativePath ?? "",
    ].join(":");
  }
}

export function createWorkbenchEditorInput(
  resource: WorkbenchResourceRef,
  title: string,
  options: Partial<Omit<WorkbenchEditorInput, "resource" | "title">> = {},
): WorkbenchEditorInput {
  return {
    editorId: options.editorId ?? nextStableId("editor"),
    resource: normalizeWorkbenchResource(resource),
    title,
    icon: options.icon ?? null,
    preview: options.preview ?? true,
    pinned: options.pinned ?? false,
    dirty: options.dirty ?? false,
    capabilities: options.capabilities ?? {
      split: true,
      detach: true,
      duplicate: true,
    },
    checkoutBinding: options.checkoutBinding ?? null,
    sourcePath: options.sourcePath ?? null,
    availability: options.availability ?? "available",
    unavailableReason: options.unavailableReason ?? null,
  };
}

export function shouldShowWorkbenchTabStrip(
  group: WorkbenchEditorGroup,
  showSingleTab = false,
): boolean {
  return group.tabs.length >= 2 || (showSingleTab && group.tabs.length === 1);
}

export function clampWorkbenchSplitRatio(ratio: number): number {
  if (!Number.isFinite(ratio)) return 0.5;
  return Math.min(WORKBENCH_MAX_SPLIT_RATIO, Math.max(WORKBENCH_MIN_SPLIT_RATIO, ratio));
}

function workbenchSpanUnits(
  node: WorkbenchSplitNode,
  orientation: "horizontal" | "vertical",
): number {
  if (node.kind === "group") return 1;
  const first = workbenchSpanUnits(node.first, orientation);
  const second = workbenchSpanUnits(node.second, orientation);
  return node.orientation === orientation ? first + second : Math.max(first, second);
}

function workbenchNodeContainsPane(node: WorkbenchSplitNode, paneId: string): boolean {
  if (node.kind === "group") return node.paneId === paneId;
  return workbenchNodeContainsPane(node.first, paneId)
    || workbenchNodeContainsPane(node.second, paneId);
}

function rebalanceWorkbenchSplitPath(
  node: WorkbenchSplitNode,
  paneId: string,
  orientation: "horizontal" | "vertical",
): boolean {
  if (node.kind === "group") return node.paneId === paneId;
  const target = workbenchNodeContainsPane(node.first, paneId)
    ? node.first
    : workbenchNodeContainsPane(node.second, paneId)
      ? node.second
      : null;
  if (!target || !rebalanceWorkbenchSplitPath(target, paneId, orientation)) return false;
  if (node.orientation === orientation) {
    const first = workbenchSpanUnits(node.first, orientation);
    const second = workbenchSpanUnits(node.second, orientation);
    node.ratio = clampWorkbenchSplitRatio(first / (first + second));
  }
  return true;
}

function createGroup(paneId: string, editor?: WorkbenchEditorInput | null): WorkbenchEditorGroup {
  return {
    paneId,
    tabs: editor ? [editor] : [],
    activeEditorId: editor?.editorId ?? null,
    focusedCheckoutId: editor?.checkoutBinding?.checkoutId ?? null,
  };
}

function nearestEditorMatching(
  tabs: WorkbenchEditorInput[],
  removedIndex: number,
  predicate: (editor: WorkbenchEditorInput) => boolean,
): WorkbenchEditorInput | null {
  for (let distance = 0; distance < tabs.length; distance += 1) {
    const right = tabs[removedIndex + distance];
    if (right && predicate(right)) return right;
    const left = tabs[removedIndex - distance - 1];
    if (left && predicate(left)) return left;
  }
  return null;
}

function closeFallbackEditor(
  tabs: WorkbenchEditorInput[],
  removedIndex: number,
  removed: WorkbenchEditorInput,
): WorkbenchEditorInput | null {
  const checkoutId = removed.checkoutBinding?.checkoutId;
  if (checkoutId) {
    return nearestEditorMatching(
      tabs,
      removedIndex,
      (editor) => editor.checkoutBinding?.checkoutId === checkoutId,
    );
  }
  return nearestEditorMatching(
    tabs,
    removedIndex,
    (editor) => editor.resource.projectId === removed.resource.projectId,
  );
}

function createWindowState(windowId: string): WorkbenchWindowState {
  const paneId = windowId === "main" ? "main" : nextStableId("pane");
  return {
    schemaVersion: WORKBENCH_SCHEMA_VERSION,
    windowId,
    sidebar: { width: 300, collapsed: false },
    layout: { kind: "group", paneId },
    groups: { [paneId]: createGroup(paneId) },
    focusedPaneId: paneId,
  };
}

function cloneResource(resource: WorkbenchResourceRef): WorkbenchResourceRef {
  return { ...normalizeWorkbenchResource(resource) } as WorkbenchResourceRef;
}

function isRestorableResource(value: unknown): value is WorkbenchResourceRef {
  if (!value || typeof value !== "object") return false;
  const resource = value as Record<string, unknown>;
  if (typeof resource.kind !== "string" || typeof resource.projectId !== "string") return false;
  switch (resource.kind) {
    case "project":
    case "newSession":
    case "knowledgeRoot":
    case "collaboration":
      return true;
    case "checkout": return typeof resource.checkoutId === "string";
    case "section": return ["sessions", "archived", "knowledge", "collab", "assets", "views"].includes(
      String(resource.section),
    );
    case "folder": return typeof resource.nodeId === "string";
    case "session": return typeof resource.sessionId === "string";
    case "knowledge": return typeof resource.documentId === "string";
    case "workspaceFile":
    case "asset":
      return typeof resource.path === "string" && !!resource.path.trim();
    case "sceneObject":
      return typeof resource.scenePath === "string"
        && !!resource.scenePath.trim()
        && typeof resource.objectPath === "string"
        && !!resource.objectPath.trim();
    case "view": return typeof resource.viewId === "string";
    case "localDirectory":
    case "localFile":
      return typeof resource.nodeId === "string";
    default: return false;
  }
}

function normalizeRestoredResource(resource: WorkbenchResourceRef): WorkbenchResourceRef {
  const legacy = resource as WorkbenchResourceRef & {
    checkoutId?: string;
    sourceCheckoutId?: string;
    path?: string;
  };
  if (legacy.kind === "knowledgeRoot") {
    return { kind: "section", projectId: legacy.projectId, section: "knowledge" };
  }
  if (legacy.kind === "collaboration") {
    return { kind: "section", projectId: legacy.projectId, section: "collab" };
  }
  switch (legacy.kind) {
    case "project": return { kind: "project", projectId: legacy.projectId };
    case "newSession": return { kind: "newSession", projectId: legacy.projectId };
    case "checkout": return {
      kind: "checkout",
      projectId: legacy.projectId,
      checkoutId: legacy.checkoutId,
    };
    case "section": return { ...legacy };
    case "folder": return { kind: "folder", projectId: legacy.projectId, nodeId: legacy.nodeId };
    case "session": return {
      kind: "session",
      projectId: legacy.projectId,
      sessionId: legacy.sessionId,
    };
    case "knowledge": return {
      kind: "knowledge",
      projectId: legacy.projectId,
      documentId: legacy.documentId,
    };
    case "workspaceFile": return {
      kind: "workspaceFile",
      projectId: legacy.projectId,
      path: legacy.path,
    };
    case "asset": return normalizeWorkbenchResource({
      kind: "asset",
      projectId: legacy.projectId,
      path: legacy.path,
    });
    case "sceneObject": return {
      kind: "sceneObject",
      projectId: legacy.projectId,
      scenePath: legacy.scenePath,
      objectPath: legacy.objectPath,
    };
    case "view": return { kind: "view", projectId: legacy.projectId, viewId: legacy.viewId };
    case "localDirectory": return {
      kind: "localDirectory",
      projectId: legacy.projectId,
      nodeId: legacy.nodeId,
      relativePath: legacy.relativePath ?? null,
    };
    case "localFile": return {
      kind: "localFile",
      projectId: legacy.projectId,
      nodeId: legacy.nodeId,
      relativePath: legacy.relativePath ?? null,
    };
    default: return legacy;
  }
}

function serializableEditor(editor: WorkbenchEditorInput): WorkbenchEditorInput {
  return {
    ...editor,
    resource: cloneResource(editor.resource),
    capabilities: { ...editor.capabilities },
    checkoutBinding: editor.checkoutBinding
      ? { checkoutId: editor.checkoutBinding.checkoutId }
      : null,
  };
}

function serializableWindow(state: WorkbenchWindowState): WorkbenchWindowState {
  return {
    ...state,
    sidebar: { ...state.sidebar },
    layout: JSON.parse(JSON.stringify(state.layout)) as WorkbenchSplitNode,
    groups: Object.fromEntries(Object.entries(state.groups).map(([paneId, group]) => [
      paneId,
      {
        ...group,
        tabs: group.tabs.map(serializableEditor),
      },
    ])),
  };
}

function normalizeWorkspaceScopeId(workspaceScopeId?: string | null): string | null {
  const normalized = workspaceScopeId?.trim();
  return normalized ? normalized : null;
}

function storageKey(windowId: string, workspaceScopeId?: string | null): string {
  const normalizedScopeId = normalizeWorkspaceScopeId(workspaceScopeId);
  if (!normalizedScopeId) return `${WORKBENCH_STORAGE_PREFIX}${windowId}`;
  return `${WORKBENCH_STORAGE_PREFIX}${windowId}:workspace:${encodeURIComponent(normalizedScopeId)}`;
}

function collectPaneIds(node: WorkbenchSplitNode, result = new Set<string>()): Set<string> {
  if (node.kind === "group") {
    result.add(node.paneId);
    return result;
  }
  collectPaneIds(node.first, result);
  collectPaneIds(node.second, result);
  return result;
}

function isSplitNode(value: unknown): value is WorkbenchSplitNode {
  if (!value || typeof value !== "object") return false;
  const node = value as Partial<WorkbenchSplitNode>;
  if (node.kind === "group") return typeof node.paneId === "string" && !!node.paneId.trim();
  if (node.kind !== "split") return false;
  return typeof node.splitId === "string"
    && (node.orientation === "horizontal" || node.orientation === "vertical")
    && typeof node.ratio === "number"
    && isSplitNode(node.first)
    && isSplitNode(node.second);
}

function isRestorableWindow(value: unknown, windowId: string): value is WorkbenchWindowState {
  if (!value || typeof value !== "object") return false;
  const state = value as Partial<WorkbenchWindowState>;
  if (
    state.schemaVersion !== WORKBENCH_SCHEMA_VERSION
    || state.windowId !== windowId
    || !isSplitNode(state.layout)
    || !state.groups
    || typeof state.groups !== "object"
    || typeof state.focusedPaneId !== "string"
  ) return false;
  const paneIds = collectPaneIds(state.layout);
  if (!paneIds.has(state.focusedPaneId)) return false;
  return [...paneIds].every((paneId) => {
    const group = state.groups?.[paneId];
    if (!group || group.paneId !== paneId || !Array.isArray(group.tabs)) return false;
    if (!group.tabs.every((editor) => (
      !!editor
      && typeof editor === "object"
      && typeof editor.editorId === "string"
      && typeof editor.title === "string"
      && isRestorableResource(editor.resource)
    ))) return false;
    return group.activeEditorId === null
      || group.tabs.some((editor) => editor.editorId === group.activeEditorId);
  });
}

function normalizeRestoredWindow(state: WorkbenchWindowState): WorkbenchWindowState {
  const paneIds = collectPaneIds(state.layout);
  const groups: Record<string, WorkbenchEditorGroup> = {};
  for (const paneId of paneIds) {
    const group = state.groups[paneId]!;
    const tabs: WorkbenchEditorInput[] = group.tabs.map((editor) => {
      const legacyResource = editor.resource as WorkbenchResourceRef & {
        checkoutId?: string;
        sourceCheckoutId?: string;
        path?: string;
      };
      const legacyCheckoutId = legacyResource.checkoutId ?? legacyResource.sourceCheckoutId;
      return {
      ...editor,
      resource: normalizeRestoredResource(editor.resource),
      capabilities: {
        split: editor.capabilities?.split !== false,
        detach: editor.capabilities?.detach !== false,
        duplicate: editor.capabilities?.duplicate !== false,
      },
      checkoutBinding: editor.checkoutBinding?.checkoutId || legacyCheckoutId
        ? { checkoutId: editor.checkoutBinding?.checkoutId ?? legacyCheckoutId! }
        : null,
      sourcePath: editor.sourcePath ?? legacyResource.path ?? null,
      availability: editor.availability === "unavailable"
        ? "unavailable" as const
        : "available" as const,
      preview: editor.preview === true,
      pinned: editor.pinned === true,
      dirty: editor.dirty === true,
      };
    });
    groups[paneId] = {
      paneId,
      tabs,
      activeEditorId: tabs.some((editor) => editor.editorId === group.activeEditorId)
        ? group.activeEditorId
        : tabs[0]?.editorId ?? null,
      focusedCheckoutId: group.focusedCheckoutId ?? tabs[0]?.checkoutBinding?.checkoutId ?? null,
    };
  }
  const normalizeNode = (node: WorkbenchSplitNode): WorkbenchSplitNode => node.kind === "group"
    ? { ...node }
    : {
        ...node,
        ratio: clampWorkbenchSplitRatio(node.ratio),
        first: normalizeNode(node.first),
        second: normalizeNode(node.second),
      };
  return {
    schemaVersion: WORKBENCH_SCHEMA_VERSION,
    windowId: state.windowId,
    sidebar: {
      width: Number.isFinite(state.sidebar?.width)
        ? Math.min(520, Math.max(220, state.sidebar.width))
        : 300,
      collapsed: state.sidebar?.collapsed === true,
    },
    layout: normalizeNode(state.layout),
    groups,
    focusedPaneId: state.focusedPaneId,
  };
}

function windowBelongsToWorkspaceScope(
  state: WorkbenchWindowState,
  workspaceScopeId: string,
): boolean {
  return Object.values(state.groups).every((group) => (
    (group.focusedCheckoutId === null || group.focusedCheckoutId === workspaceScopeId)
    && group.tabs.every((editor) => editor.checkoutBinding?.checkoutId === workspaceScopeId)
  ));
}

interface StoredWorkspaceCandidate {
  sourceScopeId: string;
  state: WorkbenchWindowState;
  tabCount: number;
}

function scopedStoragePrefix(windowId: string): string {
  return `${storageKey(windowId)}:workspace:`;
}

function workspaceScopeFromStorageKey(windowId: string, key: string): string | null {
  const prefix = scopedStoragePrefix(windowId);
  if (!key.startsWith(prefix)) return null;
  try {
    return normalizeWorkspaceScopeId(decodeURIComponent(key.slice(prefix.length)));
  } catch {
    return null;
  }
}

function workspaceTabCount(state: WorkbenchWindowState): number {
  return Object.values(state.groups).reduce((count, group) => count + group.tabs.length, 0);
}

function projectWindowToWorkspaceScope(
  source: WorkbenchWindowState,
  workspaceScopeId: string,
): WorkbenchWindowState {
  const state = serializableWindow(source);
  for (const group of Object.values(state.groups)) {
    group.tabs = group.tabs.filter(
      (editor) => editor.checkoutBinding?.checkoutId === workspaceScopeId,
    );
    if (!group.tabs.some((editor) => editor.editorId === group.activeEditorId)) {
      group.activeEditorId = group.tabs[0]?.editorId ?? null;
    }
    const activeEditor = group.tabs.find((editor) => editor.editorId === group.activeEditorId);
    group.focusedCheckoutId = activeEditor ? workspaceScopeId : null;
  }
  if (state.groups[state.focusedPaneId]?.tabs.length === 0) {
    state.focusedPaneId = Object.values(state.groups).find((group) => group.tabs.length > 0)?.paneId
      ?? state.focusedPaneId;
  }
  return state;
}

function mergeWorkspaceCandidates(
  windowId: string,
  workspaceScopeId: string,
  candidates: StoredWorkspaceCandidate[],
): WorkbenchWindowState {
  const ordered = [...candidates].sort((left, right) => {
    const leftScore = (left.tabCount > 0 ? 1_000_000 : 0)
      + (left.sourceScopeId === workspaceScopeId ? 100_000 : 0)
      + left.tabCount;
    const rightScore = (right.tabCount > 0 ? 1_000_000 : 0)
      + (right.sourceScopeId === workspaceScopeId ? 100_000 : 0)
      + right.tabCount;
    return rightScore - leftScore;
  });
  const primary = ordered.shift()?.state ?? createWindowState(windowId);
  const targetGroup = primary.groups[primary.focusedPaneId]
    ?? Object.values(primary.groups)[0];
  if (!targetGroup) return createWindowState(windowId);

  const knownEditorIds = new Set(
    Object.values(primary.groups).flatMap((group) => group.tabs.map((editor) => editor.editorId)),
  );
  for (const candidate of ordered) {
    for (const editor of Object.values(candidate.state.groups).flatMap((group) => group.tabs)) {
      if (knownEditorIds.has(editor.editorId)) continue;
      knownEditorIds.add(editor.editorId);
      targetGroup.tabs.push(serializableEditor(editor));
    }
  }
  if (!targetGroup.activeEditorId && targetGroup.tabs[0]) {
    targetGroup.activeEditorId = targetGroup.tabs[0].editorId;
  }
  for (const group of Object.values(primary.groups)) {
    const activeEditor = group.tabs.find((editor) => editor.editorId === group.activeEditorId);
    group.focusedCheckoutId = activeEditor ? workspaceScopeId : null;
  }
  return primary;
}

/**
 * Repairs historical per-workspace layout slots that were written under the
 * wrong checkout key. Editors are routed by their durable checkout binding;
 * an existing correctly keyed layout remains the primary layout and displaced
 * editors are merged into it.
 */
function repairStoredWorkspaceScopes(windowId: string): void {
  if (typeof window === "undefined") return;
  const prefix = scopedStoragePrefix(windowId);
  const entries: Array<{ scopeId: string; state: WorkbenchWindowState }> = [];
  for (let index = 0; index < window.localStorage.length; index += 1) {
    const key = window.localStorage.key(index);
    if (!key?.startsWith(prefix)) continue;
    const scopeId = workspaceScopeFromStorageKey(windowId, key);
    if (!scopeId) continue;
    try {
      const parsed: unknown = JSON.parse(window.localStorage.getItem(key) ?? "null");
      if (!isRestorableWindow(parsed, windowId)) continue;
      entries.push({ scopeId, state: normalizeRestoredWindow(parsed) });
    } catch {
      // Invalid entries remain isolated from valid workspace layouts.
    }
  }
  if (entries.length === 0) return;

  const candidates = new Map<string, StoredWorkspaceCandidate[]>();
  const sourceScopes = new Set(entries.map((entry) => entry.scopeId));
  let repairRequired = false;
  for (const entry of entries) {
    const checkoutIds = new Set<string>([entry.scopeId]);
    for (const group of Object.values(entry.state.groups)) {
      for (const editor of group.tabs) {
        const checkoutId = editor.checkoutBinding?.checkoutId;
        if (checkoutId) checkoutIds.add(checkoutId);
      }
    }
    if (!windowBelongsToWorkspaceScope(entry.state, entry.scopeId)) repairRequired = true;
    for (const checkoutId of checkoutIds) {
      const state = projectWindowToWorkspaceScope(entry.state, checkoutId);
      const list = candidates.get(checkoutId) ?? [];
      list.push({
        sourceScopeId: entry.scopeId,
        state,
        tabCount: workspaceTabCount(state),
      });
      candidates.set(checkoutId, list);
    }
  }
  if (!repairRequired) return;

  const destinationScopes = new Set([...sourceScopes, ...candidates.keys()]);
  for (const scopeId of destinationScopes) {
    const repaired = mergeWorkspaceCandidates(
      windowId,
      scopeId,
      candidates.get(scopeId) ?? [],
    );
    window.localStorage.setItem(storageKey(windowId, scopeId), JSON.stringify(repaired));
  }
  console.warn(`[workbench] repaired checkout-scoped layouts for window ${windowId}`);
}

export function replaceWorkbenchPane(
  node: WorkbenchSplitNode,
  paneId: string,
  replacement: WorkbenchSplitNode,
): WorkbenchSplitNode {
  if (node.kind === "group") return node.paneId === paneId ? replacement : node;
  return {
    ...node,
    first: replaceWorkbenchPane(node.first, paneId, replacement),
    second: replaceWorkbenchPane(node.second, paneId, replacement),
  };
}

export function removeWorkbenchPane(
  node: WorkbenchSplitNode,
  paneId: string,
): { layout: WorkbenchSplitNode; removed: boolean; fallbackPaneId: string | null } {
  if (node.kind === "group") {
    return { layout: node, removed: false, fallbackPaneId: node.paneId };
  }
  if (node.first.kind === "group" && node.first.paneId === paneId) {
    return {
      layout: node.second,
      removed: true,
      fallbackPaneId: collectPaneIds(node.second).values().next().value ?? null,
    };
  }
  if (node.second.kind === "group" && node.second.paneId === paneId) {
    return {
      layout: node.first,
      removed: true,
      fallbackPaneId: collectPaneIds(node.first).values().next().value ?? null,
    };
  }
  const first = removeWorkbenchPane(node.first, paneId);
  if (first.removed) return { ...first, layout: { ...node, first: first.layout } };
  const second = removeWorkbenchPane(node.second, paneId);
  if (second.removed) return { ...second, layout: { ...node, second: second.layout } };
  return { layout: node, removed: false, fallbackPaneId: null };
}

export const useWorkbenchStore = defineStore("workbench", () => {
  const windows = ref<Record<string, WorkbenchWindowState>>({});
  const workspaceScopes = ref<Record<string, string | null>>({});
  const repairedStorageWindows = new Set<string>();

  const mainWindow = computed(() => windows.value.main ?? null);

  function workspaceScope(windowId = "main"): string | null {
    return workspaceScopes.value[windowId] ?? null;
  }

  function restoreStoredWindow(
    windowId: string,
    workspaceScopeId: string | null,
  ): WorkbenchWindowState | null {
    if (typeof window === "undefined") return null;
    try {
      if (workspaceScopeId && !repairedStorageWindows.has(windowId)) {
        repairStoredWorkspaceScopes(windowId);
        repairedStorageWindows.add(windowId);
      }
      const raw = window.localStorage.getItem(storageKey(windowId, workspaceScopeId));
      const parsed: unknown = raw ? JSON.parse(raw) : null;
      if (isRestorableWindow(parsed, windowId)) {
        const restored = normalizeRestoredWindow(parsed);
        if (!workspaceScopeId || windowBelongsToWorkspaceScope(restored, workspaceScopeId)) {
          return restored;
        }
      }

      // The unscoped key predates per-workspace layouts. Adopt it once when all
      // of its tabs already belong to the workspace being activated.
      if (workspaceScopeId) {
        const legacyRaw = window.localStorage.getItem(storageKey(windowId));
        const legacyParsed: unknown = legacyRaw ? JSON.parse(legacyRaw) : null;
        if (
          isRestorableWindow(legacyParsed, windowId)
          && windowBelongsToWorkspaceScope(legacyParsed, workspaceScopeId)
        ) return normalizeRestoredWindow(legacyParsed);
      }
    } catch (error) {
      console.warn("[workbench] stored layout is unavailable", error);
    }
    return null;
  }

  function persist(windowId: string): void {
    const state = windows.value[windowId];
    if (!state || typeof window === "undefined") return;
    const scopeId = workspaceScope(windowId);
    if (scopeId && !windowBelongsToWorkspaceScope(state, scopeId)) {
      console.error(
        `[workbench] refused to persist a layout outside checkout scope ${scopeId}`,
      );
      return;
    }
    try {
      window.localStorage.setItem(
        storageKey(windowId, scopeId),
        JSON.stringify(serializableWindow(state)),
      );
    } catch (error) {
      console.warn("[workbench] layout persistence failed", error);
    }
  }

  function ensureWindow(windowId = "main"): WorkbenchWindowState {
    const existing = windows.value[windowId];
    if (existing) return existing;
    const restored = restoreStoredWindow(windowId, workspaceScope(windowId));
    const state = restored ?? createWindowState(windowId);
    windows.value[windowId] = state;
    return state;
  }

  function requireEditorWorkspaceScope(
    windowId: string,
    editor: WorkbenchEditorInput,
    operation: string,
  ): void {
    const scopeId = workspaceScope(windowId);
    if (!scopeId) return;
    const checkoutId = normalizeWorkspaceScopeId(editor.checkoutBinding?.checkoutId);
    if (checkoutId === scopeId) return;
    throw new Error(
      `[workbench] ${operation} requires checkout ${scopeId}; received ${checkoutId ?? "empty"}`,
    );
  }

  function switchWorkspaceScope(
    windowId: string,
    workspaceScopeId?: string | null,
  ): WorkbenchWindowState {
    const nextScopeId = normalizeWorkspaceScopeId(workspaceScopeId);
    const currentScopeId = workspaceScope(windowId);
    const current = ensureWindow(windowId);
    if (currentScopeId === nextScopeId) return current;

    persist(windowId);
    workspaceScopes.value[windowId] = nextScopeId;
    const restored = restoreStoredWindow(windowId, nextScopeId);
    const next = restored ?? createWindowState(windowId);
    next.sidebar = { ...current.sidebar };
    windows.value[windowId] = next;
    persist(windowId);
    return next;
  }

  function resetWindow(windowId = "main"): WorkbenchWindowState {
    const state = createWindowState(windowId);
    windows.value[windowId] = state;
    persist(windowId);
    return state;
  }

  function group(windowId: string, paneId: string): WorkbenchEditorGroup | null {
    return ensureWindow(windowId).groups[paneId] ?? null;
  }

  function activeGroup(windowId = "main"): WorkbenchEditorGroup {
    const state = ensureWindow(windowId);
    return state.groups[state.focusedPaneId] ?? Object.values(state.groups)[0]!;
  }

  function activeEditor(windowId = "main", paneId?: string | null): WorkbenchEditorInput | null {
    const target = paneId ? group(windowId, paneId) : activeGroup(windowId);
    return target?.tabs.find((editor) => editor.editorId === target.activeEditorId) ?? null;
  }

  function focusPane(windowId: string, paneId: string): boolean {
    const state = ensureWindow(windowId);
    if (!state.groups[paneId]) return false;
    state.focusedPaneId = paneId;
    persist(windowId);
    return true;
  }

  function activateEditor(
    windowId: string,
    paneId: string,
    editorId: string,
    options: { focusPane?: boolean } = {},
  ): boolean {
    const state = ensureWindow(windowId);
    const target = state.groups[paneId];
    const editor = target?.tabs.find((candidate) => candidate.editorId === editorId);
    if (!target || !editor) return false;
    requireEditorWorkspaceScope(windowId, editor, "activateEditor");
    target.activeEditorId = editorId;
    target.focusedCheckoutId = editor.checkoutBinding?.checkoutId ?? target.focusedCheckoutId ?? null;
    if (options.focusPane !== false) state.focusedPaneId = paneId;
    persist(windowId);
    return true;
  }

  function openEditor(
    windowId: string,
    input: WorkbenchEditorInput,
    options: {
      paneId?: string | null;
      pinned?: boolean;
      preview?: boolean;
      index?: number;
      replacePreview?: boolean;
      allowDuplicate?: boolean;
    } = {},
  ): WorkbenchEditorInput {
    const state = ensureWindow(windowId);
    input = {
      ...input,
      resource: cloneResource(input.resource),
    };
    requireEditorWorkspaceScope(windowId, input, "openEditor");
    const paneId = options.paneId && state.groups[options.paneId]
      ? options.paneId
      : state.focusedPaneId;
    const target = state.groups[paneId]!;
    const resourceKey = workbenchResourceKey(input.resource);
    const existing = options.allowDuplicate
      ? undefined
      : target.tabs.find(
          (editor) => workbenchResourceKey(editor.resource) === resourceKey,
        );
    if (existing) {
      const existingDirty = existing.dirty;
      Object.assign(existing, {
        ...input,
        editorId: existing.editorId,
        preview: options.preview ?? existing.preview,
        pinned: options.pinned ?? existing.pinned,
        dirty: existingDirty || input.dirty,
      });
      target.activeEditorId = existing.editorId;
      target.focusedCheckoutId = existing.checkoutBinding?.checkoutId ?? target.focusedCheckoutId ?? null;
      state.focusedPaneId = paneId;
      persist(windowId);
      return existing;
    }

    const editor: WorkbenchEditorInput = {
      ...input,
      preview: options.preview ?? input.preview,
      pinned: options.pinned ?? input.pinned,
    };
    if (editor.preview && !editor.pinned && options.replacePreview !== false) {
      const replaceIndex = target.tabs.findIndex((candidate) => candidate.preview && !candidate.pinned && !candidate.dirty);
      if (replaceIndex >= 0) target.tabs.splice(replaceIndex, 1);
    }
    const index = options.index == null
      ? target.tabs.length
      : Math.min(target.tabs.length, Math.max(0, options.index));
    target.tabs.splice(index, 0, editor);
    target.activeEditorId = editor.editorId;
    target.focusedCheckoutId = editor.checkoutBinding?.checkoutId ?? target.focusedCheckoutId ?? null;
    state.focusedPaneId = paneId;
    persist(windowId);
    return editor;
  }

  function replaceEditor(
    windowId: string,
    paneId: string,
    editorId: string,
    input: WorkbenchEditorInput,
  ): WorkbenchEditorInput | null {
    const state = ensureWindow(windowId);
    const target = state.groups[paneId];
    const index = target?.tabs.findIndex((editor) => editor.editorId === editorId) ?? -1;
    if (!target || index < 0) return null;
    const editor: WorkbenchEditorInput = {
      ...input,
      resource: cloneResource(input.resource),
      capabilities: { ...input.capabilities },
      checkoutBinding: input.checkoutBinding ? { ...input.checkoutBinding } : null,
    };
    requireEditorWorkspaceScope(windowId, editor, "replaceEditor");
    target.tabs.splice(index, 1, editor);
    target.activeEditorId = editor.editorId;
    target.focusedCheckoutId = editor.checkoutBinding?.checkoutId ?? null;
    state.focusedPaneId = paneId;
    persist(windowId);
    return editor;
  }

  function pinEditor(windowId: string, paneId: string, editorId: string): boolean {
    const editor = group(windowId, paneId)?.tabs.find((candidate) => candidate.editorId === editorId);
    if (!editor) return false;
    editor.preview = false;
    editor.pinned = true;
    persist(windowId);
    return true;
  }

  function updateEditor(
    windowId: string,
    paneId: string,
    editorId: string,
    patch: Partial<Omit<WorkbenchEditorInput, "editorId">>,
  ): WorkbenchEditorInput | null {
    const editor = group(windowId, paneId)?.tabs.find((candidate) => candidate.editorId === editorId);
    if (!editor) return null;
    requireEditorWorkspaceScope(windowId, { ...editor, ...patch, editorId }, "updateEditor");
    Object.assign(editor, patch, { editorId });
    persist(windowId);
    return editor;
  }

  function splitPane(
    windowId: string,
    paneId: string,
    direction: Exclude<WorkbenchDropDirection, "center">,
    editor?: WorkbenchEditorInput | null,
  ): string | null {
    const state = ensureWindow(windowId);
    if (!state.groups[paneId]) return null;
    if (editor) requireEditorWorkspaceScope(windowId, editor, "splitPane");
    const newPaneId = nextStableId("pane");
    const before = direction === "left" || direction === "top";
    const orientation = direction === "left" || direction === "right"
      ? "horizontal" as const
      : "vertical" as const;
    const existingLeaf: WorkbenchSplitNode = { kind: "group", paneId };
    const newLeaf: WorkbenchSplitNode = { kind: "group", paneId: newPaneId };
    const replacement: WorkbenchSplitNode = {
      kind: "split",
      splitId: nextStableId("split"),
      orientation,
      ratio: 0.5,
      first: before ? newLeaf : existingLeaf,
      second: before ? existingLeaf : newLeaf,
    };
    state.layout = replaceWorkbenchPane(state.layout, paneId, replacement);
    rebalanceWorkbenchSplitPath(state.layout, newPaneId, orientation);
    state.groups[newPaneId] = createGroup(newPaneId, editor ?? null);
    state.focusedPaneId = newPaneId;
    persist(windowId);
    return newPaneId;
  }

  function closePane(windowId: string, paneId: string): boolean {
    const state = ensureWindow(windowId);
    if (!state.groups[paneId] || Object.keys(state.groups).length <= 1) return false;
    const result = removeWorkbenchPane(state.layout, paneId);
    if (!result.removed) return false;
    state.layout = result.layout;
    delete state.groups[paneId];
    if (state.focusedPaneId === paneId) {
      state.focusedPaneId = result.fallbackPaneId ?? Object.keys(state.groups)[0]!;
    }
    persist(windowId);
    return true;
  }

  function closeEditor(windowId: string, paneId: string, editorId: string): boolean {
    const target = group(windowId, paneId);
    if (!target) return false;
    const index = target.tabs.findIndex((editor) => editor.editorId === editorId);
    if (index < 0) return false;
    const removed = target.tabs[index]!;
    const wasActive = target.activeEditorId === editorId;
    target.tabs.splice(index, 1);
    if (wasActive) {
      target.activeEditorId = closeFallbackEditor(target.tabs, index, removed)?.editorId ?? null;
    }
    const nextActive = target.tabs.find((editor) => editor.editorId === target.activeEditorId);
    target.focusedCheckoutId = nextActive?.checkoutBinding?.checkoutId ?? null;
    if (target.tabs.length === 0 && Object.keys(ensureWindow(windowId).groups).length > 1) {
      closePane(windowId, paneId);
    } else {
      persist(windowId);
    }
    return true;
  }

  function takeEditor(windowId: string, paneId: string, editorId: string): WorkbenchEditorInput | null {
    const target = group(windowId, paneId);
    if (!target) return null;
    const index = target.tabs.findIndex((editor) => editor.editorId === editorId);
    if (index < 0) return null;
    const [editor] = target.tabs.splice(index, 1);
    if (target.activeEditorId === editorId) {
      target.activeEditorId = target.tabs[index]?.editorId
        ?? target.tabs[index - 1]?.editorId
        ?? null;
    }
    const nextActive = target.tabs.find((candidate) => candidate.editorId === target.activeEditorId);
    target.focusedCheckoutId = nextActive?.checkoutBinding?.checkoutId ?? null;
    return editor ?? null;
  }

  function moveEditor(
    windowId: string,
    sourcePaneId: string,
    editorId: string,
    targetPaneId: string,
    options: { index?: number; direction?: WorkbenchDropDirection } = {},
  ): string | null {
    const state = ensureWindow(windowId);
    const source = state.groups[sourcePaneId];
    const target = state.groups[targetPaneId];
    const sourceEditor = source?.tabs.find((editor) => editor.editorId === editorId);
    if (!source || !target || !sourceEditor) return null;
    requireEditorWorkspaceScope(windowId, sourceEditor, "moveEditor");
    const direction = options.direction ?? "center";
    if (sourcePaneId === targetPaneId && direction === "center") {
      const from = source.tabs.findIndex((editor) => editor.editorId === editorId);
      const requestedIndex = options.index == null
        ? source.tabs.length
        : Math.min(source.tabs.length, Math.max(0, options.index));
      const to = from < requestedIndex ? requestedIndex - 1 : requestedIndex;
      if (from !== to) {
        const [editor] = source.tabs.splice(from, 1);
        if (editor) source.tabs.splice(Math.min(source.tabs.length, to), 0, editor);
      }
      source.activeEditorId = editorId;
      state.focusedPaneId = sourcePaneId;
      persist(windowId);
      return sourcePaneId;
    }

    const editor = takeEditor(windowId, sourcePaneId, editorId);
    if (!editor) return null;
    editor.preview = false;
    editor.pinned = true;
    let destinationPaneId = targetPaneId;
    if (direction === "center") {
      openEditor(windowId, editor, {
        paneId: targetPaneId,
        pinned: true,
        preview: false,
        index: options.index,
      });
    } else {
      destinationPaneId = splitPane(windowId, targetPaneId, direction, editor) ?? targetPaneId;
    }
    if (source.tabs.length === 0 && sourcePaneId !== destinationPaneId) closePane(windowId, sourcePaneId);
    persist(windowId);
    return destinationPaneId;
  }

  function acceptTransferredEditor(
    windowId: string,
    input: WorkbenchEditorInput,
    targetPaneId: string,
    options: {
      index?: number;
      direction?: WorkbenchDropDirection;
      allowDuplicate?: boolean;
    } = {},
  ): WorkbenchTransferAcceptResult | null {
    requireEditorWorkspaceScope(windowId, input, "acceptTransferredEditor");
    const state = ensureWindow(windowId);
    const target = state.groups[targetPaneId];
    if (!target) return null;
    const direction = options.direction ?? "center";
    if (direction === "center") {
      const resourceKey = workbenchResourceKey(input.resource);
      const existing = options.allowDuplicate
        ? undefined
        : target.tabs.find(
            (editor) => workbenchResourceKey(editor.resource) === resourceKey,
          );
      const editor = openEditor(windowId, {
        ...input,
        resource: cloneResource(input.resource),
        capabilities: { ...input.capabilities },
        checkoutBinding: input.checkoutBinding ? { ...input.checkoutBinding } : null,
        preview: false,
        pinned: true,
      }, {
        paneId: targetPaneId,
        preview: false,
        pinned: true,
        index: options.index,
        replacePreview: false,
        allowDuplicate: options.allowDuplicate,
      });
      return {
        paneId: targetPaneId,
        editorId: editor.editorId,
        inserted: !existing,
      };
    }

    const editor: WorkbenchEditorInput = {
      ...input,
      resource: cloneResource(input.resource),
      capabilities: { ...input.capabilities },
      checkoutBinding: input.checkoutBinding ? { ...input.checkoutBinding } : null,
      preview: false,
      pinned: true,
    };
    const paneId = splitPane(windowId, targetPaneId, direction, editor);
    return paneId ? { paneId, editorId: editor.editorId, inserted: true } : null;
  }

  function hasEditors(windowId: string): boolean {
    return Object.values(ensureWindow(windowId).groups)
      .some((candidate) => candidate.tabs.length > 0);
  }

  function deleteWindow(windowId: string, options: { removeStorage?: boolean } = {}): void {
    delete windows.value[windowId];
    const scope = workspaceScope(windowId);
    delete workspaceScopes.value[windowId];
    if (options.removeStorage !== true || typeof window === "undefined") return;
    window.localStorage.removeItem(storageKey(windowId, scope));
    window.localStorage.removeItem(storageKey(windowId));
  }

  function updateSplitRatio(
    windowId: string,
    splitId: string,
    ratio: number,
    options: { persist?: boolean } = {},
  ): boolean {
    const state = ensureWindow(windowId);
    let changed = false;
    const visit = (node: WorkbenchSplitNode): void => {
      if (node.kind === "group") return;
      if (node.splitId === splitId) {
        node.ratio = clampWorkbenchSplitRatio(ratio);
        changed = true;
        return;
      }
      visit(node.first);
      visit(node.second);
    };
    visit(state.layout);
    if (changed && options.persist !== false) persist(windowId);
    return changed;
  }

  function setSidebarWidth(windowId: string, width: number): void {
    const state = ensureWindow(windowId);
    state.sidebar.width = Math.min(520, Math.max(220, width));
    persist(windowId);
  }

  return {
    windows,
    workspaceScopes,
    mainWindow,
    workspaceScope,
    ensureWindow,
    switchWorkspaceScope,
    resetWindow,
    persist,
    group,
    activeGroup,
    activeEditor,
    focusPane,
    activateEditor,
    openEditor,
    replaceEditor,
    pinEditor,
    updateEditor,
    splitPane,
    closePane,
    closeEditor,
    moveEditor,
    acceptTransferredEditor,
    hasEditors,
    deleteWindow,
    updateSplitRatio,
    setSidebarWidth,
  };
});
