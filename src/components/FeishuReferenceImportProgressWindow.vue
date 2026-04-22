<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../i18n";
import { useCopyFeedback } from "../composables/useCopyFeedback";
import { normalizeAppError } from "../services/errors";
import {
  FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
  getFeishuReferenceImportWindowPayload,
  type FeishuReferenceImportWindowPayload,
} from "../services/feishuReferenceImportWindow";
import {
  knowledgeCancelFeishuReferenceOauthWait,
  knowledgeCancelFeishuReferenceImport,
  knowledgeCloseFeishuReferenceImportProgressWindow,
  knowledgeGetFeishuReferenceImportStatus,
  knowledgeImportFeishuReferenceDocs,
  knowledgeListFeishuReferenceSpaceNodes,
  knowledgeSaveFeishuReferenceConfig,
  knowledgeStartFeishuReferenceOauth,
  knowledgeTestFeishuReferenceConnection,
} from "../services/knowledge";
import type {
  FeishuReferenceAuthMode,
  FeishuReferenceConfigInput,
  FeishuReferenceRootSelection,
  FeishuReferenceImportStatus,
  FeishuReferenceNodeSummary,
  FeishuReferenceOauthPersistenceMode,
} from "../types";
import FileTreeList from "./explorer/FileTreeList.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseCheckbox from "./ui/BaseCheckbox.vue";
import BaseDropdown from "./ui/BaseDropdown.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";

interface SpaceOption {
  spaceId: string;
  name: string;
}

interface FeishuTreeNode {
  key: string;
  summary: FeishuReferenceNodeSummary;
  depth: number;
  pathLabel: string;
  children: FeishuTreeNode[];
  childrenLoaded: boolean;
  childrenLoading: boolean;
}

interface FeishuTreeRow {
  key: string;
  node: FeishuTreeNode;
  expanded: boolean;
  canExpand: boolean;
}

type ErrorMessageSegment =
  | {
      kind: "text";
      value: string;
    }
  | {
      kind: "link";
      value: string;
      href: string;
    };

const DEFAULT_OPEN_BASE_URL = "https://open.feishu.cn";
const AUTO_SAVE_DELAY_MS = 700;
const appWindow = getCurrentWindow();
const initialPayload = getFeishuReferenceImportWindowPayload();
type WindowResizeDirection = Parameters<
  typeof appWindow.startResizeDragging
>[0];

const statusSnapshot = ref<FeishuReferenceImportStatus | null>(null);
const persistentError = ref("");
const statusError = ref("");
const lastTestSummary = ref("");
const spaceOptions = ref<SpaceOption[]>([]);
const treeNodes = ref<FeishuTreeNode[]>([]);
const expandedNodeTokens = ref<Set<string>>(new Set());
const nodeLoading = ref(false);
const nodeError = ref("");
const copiedCallbackUrl = ref("");

const authMode = ref<FeishuReferenceAuthMode>("app_credentials");
const oauthPersistenceMode =
  ref<FeishuReferenceOauthPersistenceMode>("session");
const appId = ref("");
const appSecret = ref("");
const openBaseUrl = ref(DEFAULT_OPEN_BASE_URL);
const selectedSpaceId = ref("");
const selectedSpaceName = ref("");
const selectedRoots = ref<FeishuReferenceRootSelection[]>([]);
const selectedRootNodeToken = ref("");
const selectedRootNodeTitle = ref("");
const targetPath = ref(initialPayload.targetPath?.trim() || "");
const formDirty = ref(false);
const autoSaveQueued = ref(false);
const appSecretTouched = ref(false);

const savePending = ref(false);
const testPending = ref(false);
const authorizePending = ref(false);
const cancelAuthorizationPending = ref(false);
const importPending = ref(false);
const cancelling = ref(false);

let pollTimer: ReturnType<typeof setTimeout> | null = null;
let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
let closeRequestUnlisten: UnlistenFn | null = null;
let statusEventUnlisten: UnlistenFn | null = null;
let allowWindowClose = false;
let formRevision = 0;

const { copied: callbackCopied, copyText: copyCallbackText } =
  useCopyFeedback();

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

function normalizeRootSelections(
  roots: FeishuReferenceRootSelection[] | null | undefined,
): FeishuReferenceRootSelection[] {
  const seen = new Set<string>();
  const normalized: FeishuReferenceRootSelection[] = [];
  for (const root of roots ?? []) {
    const nodeToken = trimOrEmpty(root?.nodeToken);
    if (!nodeToken || seen.has(nodeToken)) continue;
    seen.add(nodeToken);
    normalized.push({
      nodeToken,
      nodeTitle: trimOrEmpty(root?.nodeTitle) || null,
    });
  }
  return normalized;
}

function syncPrimarySelectedRootFields() {
  const [primary] = selectedRoots.value;
  selectedRootNodeToken.value = trimOrEmpty(primary?.nodeToken);
  selectedRootNodeTitle.value = trimOrEmpty(primary?.nodeTitle);
}

function applySelectedRoots(
  roots: FeishuReferenceRootSelection[] | null | undefined,
) {
  selectedRoots.value = normalizeRootSelections(roots);
  syncPrimarySelectedRootFields();
}

function clearPersistentError() {
  persistentError.value = "";
}

function setPersistentError(value: string | null | undefined) {
  const normalized = trimOrEmpty(value);
  if (!normalized) return;
  persistentError.value = normalized;
}

function splitTrailingPunctuation(value: string): {
  core: string;
  trailing: string;
} {
  let core = value;
  let trailing = "";
  while (core.length > 0) {
    const lastChar = core[core.length - 1];
    if (!")}],.;'\"".includes(lastChar)) break;
    trailing = `${lastChar}${trailing}`;
    core = core.slice(0, -1);
  }
  return {
    core,
    trailing,
  };
}

function linkifyErrorMessage(value: string): ErrorMessageSegment[] {
  const segments: ErrorMessageSegment[] = [];
  const source = value.replace(/\r\n/g, "\n");
  const urlPattern = /https?:\/\/[^\s]+/g;
  let lastIndex = 0;

  for (const match of source.matchAll(urlPattern)) {
    const raw = match[0];
    const index = match.index ?? -1;
    if (index < 0) continue;
    if (index > lastIndex) {
      segments.push({
        kind: "text",
        value: source.slice(lastIndex, index),
      });
    }
    const { core, trailing } = splitTrailingPunctuation(raw);
    if (core) {
      segments.push({
        kind: "link",
        value: core,
        href: core,
      });
    }
    if (trailing) {
      segments.push({
        kind: "text",
        value: trailing,
      });
    }
    lastIndex = index + raw.length;
  }

  if (lastIndex < source.length) {
    segments.push({
      kind: "text",
      value: source.slice(lastIndex),
    });
  }

  if (!segments.length) {
    segments.push({
      kind: "text",
      value: source,
    });
  }

  return segments;
}

async function openErrorLink(href: string) {
  const normalized = trimOrEmpty(href);
  if (!normalized) return;
  try {
    await openUrl(normalized);
    return;
  } catch {
    // fall through to browser fallback
  }

  try {
    window.open(normalized, "_blank", "noopener,noreferrer");
  } catch {
    // keep the visible href even if the opener fails
  }
}

async function copyCallbackUrl(value: string) {
  const normalized = trimOrEmpty(value);
  if (!normalized) return;
  const copied = await copyCallbackText(normalized);
  if (copied) {
    copiedCallbackUrl.value = normalized;
    return;
  }
  setPersistentError(t("knowledge.feishuReference.window.copyCallbackFailed"));
}

function clearPollTimer() {
  if (!pollTimer) return;
  clearTimeout(pollTimer);
  pollTimer = null;
}

function clearAutoSaveTimer() {
  if (!autoSaveTimer) return;
  clearTimeout(autoSaveTimer);
  autoSaveTimer = null;
}

function schedulePoll(delay = 1200) {
  clearPollTimer();
  pollTimer = setTimeout(() => {
    pollTimer = null;
    void refreshStatus();
  }, delay);
}

function authModeLabel(value: FeishuReferenceAuthMode): string {
  return value === "oauth"
    ? t("knowledge.feishuReference.auth.oauth")
    : t("knowledge.feishuReference.auth.appCredentials");
}

function stageLabel(
  stage: FeishuReferenceImportStatus["stage"] | null | undefined,
): string {
  switch (stage) {
    case "saving_config":
      return t("knowledge.feishuReference.stage.savingConfig");
    case "authorizing":
      return t("knowledge.feishuReference.stage.authorizing");
    case "testing_connection":
      return t("knowledge.feishuReference.stage.testingConnection");
    case "listing_spaces":
      return t("knowledge.feishuReference.stage.listingSpaces");
    case "listing_nodes":
      return t("knowledge.feishuReference.stage.listingNodes");
    case "importing":
      return t("knowledge.feishuReference.stage.importing");
    case "reconciling":
      return t("knowledge.feishuReference.stage.reconciling");
    case "ready":
      return t("knowledge.feishuReference.stage.ready");
    case "error":
      return t("knowledge.feishuReference.stage.error");
    case "idle":
    default:
      return t("knowledge.feishuReference.stage.idle");
  }
}

function stateLabel(
  state: FeishuReferenceImportStatus["state"] | null | undefined,
): string {
  switch (state) {
    case "missing_config":
      return t("knowledge.feishuReference.state.missingConfig");
    case "needs_authorization":
      return t("knowledge.feishuReference.state.needsAuthorization");
    case "running":
      return t("knowledge.feishuReference.state.running");
    case "error":
      return t("knowledge.feishuReference.state.error");
    case "ready":
    default:
      return t("knowledge.feishuReference.state.ready");
  }
}

function formatDateTime(value: number | null | undefined): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatPercent(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return "—";
  return `${Math.round(Math.min(1, Math.max(0, value)) * 100)}%`;
}

function formatNodeType(node: FeishuReferenceNodeSummary): string {
  if (node.objType === "docx")
    return t("knowledge.feishuReference.nodeType.docx");
  if (node.objType === "doc")
    return t("knowledge.feishuReference.nodeType.doc");
  if (node.hasChild) return t("knowledge.feishuReference.nodeType.folder");
  return node.objType || t("knowledge.feishuReference.nodeType.node");
}

function nodeTitle(summary: FeishuReferenceNodeSummary): string {
  return trimOrEmpty(summary.title) || trimOrEmpty(summary.nodeToken);
}

function createTreeNode(
  summary: FeishuReferenceNodeSummary,
  parentPathLabel: string,
  depth: number,
): FeishuTreeNode {
  const title = nodeTitle(summary);
  return {
    key: summary.nodeToken,
    summary,
    depth,
    pathLabel: parentPathLabel ? `${parentPathLabel} / ${title}` : title,
    children: [],
    childrenLoaded: false,
    childrenLoading: false,
  };
}

function resetNodeTree() {
  treeNodes.value = [];
  expandedNodeTokens.value = new Set();
  nodeError.value = "";
  nodeLoading.value = false;
}

function findTreeNodeByToken(
  token: string,
  nodes: FeishuTreeNode[] = treeNodes.value,
): FeishuTreeNode | null {
  for (const node of nodes) {
    if (node.summary.nodeToken === token) return node;
    const found = findTreeNodeByToken(token, node.children);
    if (found) return found;
  }
  return null;
}

async function fetchNodeEntries(parentNodeToken: string | null) {
  return knowledgeListFeishuReferenceSpaceNodes(
    trimOrEmpty(selectedSpaceId.value),
    parentNodeToken,
  );
}

function markFormDirty() {
  formDirty.value = true;
  formRevision += 1;
}

function scheduleAutoSave(delay = AUTO_SAVE_DELAY_MS) {
  clearAutoSaveTimer();
  if (!formDirty.value) {
    autoSaveQueued.value = false;
    return;
  }
  autoSaveQueued.value = true;
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null;
    if (!formDirty.value) {
      autoSaveQueued.value = false;
      return;
    }
    void saveConfig();
  }, delay);
}

function markFormDirtyAndQueueSave(delay = AUTO_SAVE_DELAY_MS) {
  markFormDirty();
  lastTestSummary.value = "";
  scheduleAutoSave(delay);
}

function handleAppSecretInput() {
  appSecretTouched.value = true;
  markFormDirtyAndQueueSave();
}

function upsertSpaceOptions(items: SpaceOption[]) {
  const merged = new Map<string, SpaceOption>();
  for (const item of spaceOptions.value) {
    if (trimOrEmpty(item.spaceId)) merged.set(item.spaceId, item);
  }
  for (const item of items) {
    if (trimOrEmpty(item.spaceId)) {
      merged.set(item.spaceId, {
        spaceId: item.spaceId,
        name: trimOrEmpty(item.name) || item.spaceId,
      });
    }
  }
  spaceOptions.value = Array.from(merged.values()).sort((left, right) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
  );
}

function syncStatusSpaceOptions(status: FeishuReferenceImportStatus | null) {
  if (!status) return;
  const items: SpaceOption[] = [];
  if (trimOrEmpty(status.spaceId)) {
    items.push({
      spaceId: trimOrEmpty(status.spaceId),
      name: trimOrEmpty(status.spaceName) || trimOrEmpty(status.spaceId),
    });
  }
  if (trimOrEmpty(status.importedSpaceId)) {
    items.push({
      spaceId: trimOrEmpty(status.importedSpaceId),
      name:
        trimOrEmpty(status.importedSpaceName) ||
        trimOrEmpty(status.importedSpaceId),
    });
  }
  upsertSpaceOptions(items);
}

function applyStatusToForm(status: FeishuReferenceImportStatus, force = false) {
  syncStatusSpaceOptions(status);
  if (
    !force &&
    formDirty.value &&
    !status.running &&
    status.stage !== "authorizing"
  )
    return;

  authMode.value = status.authMode;
  oauthPersistenceMode.value = status.oauthPersistenceMode;
  appId.value = trimOrEmpty(status.appId);
  if (typeof status.appSecret === "string") {
    appSecret.value = trimOrEmpty(status.appSecret);
  } else if (!status.appSecretConfigured) {
    appSecret.value = "";
  }
  openBaseUrl.value = trimOrEmpty(status.openBaseUrl) || DEFAULT_OPEN_BASE_URL;
  selectedSpaceId.value = trimOrEmpty(status.spaceId);
  selectedSpaceName.value = trimOrEmpty(status.spaceName);
  applySelectedRoots(
    status.selectedRoots?.length
      ? status.selectedRoots
      : status.rootNodeToken
        ? [
            {
              nodeToken: status.rootNodeToken,
              nodeTitle: status.rootNodeTitle ?? null,
            },
          ]
        : [],
  );
  formDirty.value = false;
  autoSaveQueued.value = false;
  appSecretTouched.value = false;
}

function resolveRootSelectionTitle(
  root: FeishuReferenceRootSelection,
  options: { preferPathLabel?: boolean } = {},
): string {
  const token = trimOrEmpty(root.nodeToken);
  if (!token) return "";
  const pathLabel = options.preferPathLabel
    ? trimOrEmpty(findTreeNodeByToken(token)?.pathLabel)
    : "";
  return pathLabel || trimOrEmpty(root.nodeTitle) || token;
}

function buildSelectedRootsPayload(): FeishuReferenceRootSelection[] {
  return normalizeRootSelections(
    selectedRoots.value.map((root) => ({
      nodeToken: trimOrEmpty(root.nodeToken),
      nodeTitle: resolveRootSelectionTitle(root, { preferPathLabel: true }) || null,
    })),
  );
}

function buildConfigInput(): FeishuReferenceConfigInput {
  const normalizedAppSecret = trimOrEmpty(appSecret.value);
  const shouldPersistAppSecret = appSecretTouched.value;
  const roots = buildSelectedRootsPayload();
  return {
    targetPath: trimOrEmpty(targetPath.value) || null,
    authMode: authMode.value,
    oauthPersistenceMode: oauthPersistenceMode.value,
    appId: appId.value.trim(),
    appSecret:
      shouldPersistAppSecret && normalizedAppSecret
        ? normalizedAppSecret
        : null,
    clearAppSecret: shouldPersistAppSecret && !normalizedAppSecret,
    openBaseUrl: openBaseUrl.value.trim() || DEFAULT_OPEN_BASE_URL,
    spaceId: trimOrEmpty(selectedSpaceId.value) || null,
    spaceName: trimOrEmpty(selectedSpaceName.value) || null,
    roots,
    rootNodeToken: trimOrEmpty(selectedRootNodeToken.value) || null,
    rootNodeTitle:
      roots.length > 0 ? resolveRootSelectionTitle(roots[0]) || null : null,
  };
}

function overlayCurrentSelection(
  status: FeishuReferenceImportStatus,
): FeishuReferenceImportStatus {
  const roots = buildSelectedRootsPayload();
  return {
    ...status,
    authMode: authMode.value,
    oauthPersistenceMode: oauthPersistenceMode.value,
    appId: appId.value.trim(),
    openBaseUrl: openBaseUrl.value.trim() || DEFAULT_OPEN_BASE_URL,
    spaceId: trimOrEmpty(selectedSpaceId.value) || null,
    spaceName: trimOrEmpty(selectedSpaceName.value) || null,
    selectedRoots: roots,
    rootNodeToken: trimOrEmpty(selectedRootNodeToken.value) || null,
    rootNodeTitle:
      roots.length > 0 ? resolveRootSelectionTitle(roots[0]) || null : null,
  };
}

function syncLocalSelectionIntoStatusSnapshot() {
  if (!statusSnapshot.value) return;
  statusSnapshot.value = overlayCurrentSelection(statusSnapshot.value);
}

async function resetImportTarget(
  payload: FeishuReferenceImportWindowPayload = {},
) {
  targetPath.value = trimOrEmpty(payload.targetPath);
  statusSnapshot.value = null;
  lastTestSummary.value = "";
  statusError.value = "";
  persistentError.value = "";
  formDirty.value = false;
  autoSaveQueued.value = false;
  appSecretTouched.value = false;
  clearAutoSaveTimer();
  resetNodeTree();
  await refreshStatus();
  if (trimOrEmpty(selectedSpaceId.value)) {
    await loadRootNodes();
  }
}

async function destroyWindow() {
  clearPollTimer();
  clearAutoSaveTimer();
  if (statusSnapshot.value?.stage === "authorizing") {
    await cancelAuthorizationWait({ syncUi: false, silent: true });
  }
  allowWindowClose = true;
  closeRequestUnlisten?.();
  closeRequestUnlisten = null;
  try {
    await appWindow.setClosable(true);
  } catch {
    // ignore unsupported close state changes
  }
  try {
    await appWindow.close();
    return;
  } catch {
    // fall through to destroy
  }

  try {
    await appWindow.destroy();
    return;
  } catch {
    // fall through to backend close
  }

  try {
    await knowledgeCloseFeishuReferenceImportProgressWindow();
  } catch {
    // ignore teardown failures after local close attempts
  }
}

async function refreshStatus() {
  try {
    const nextStatus = await knowledgeGetFeishuReferenceImportStatus(targetPath.value || undefined);
    statusSnapshot.value = nextStatus;
    statusError.value = "";
    if (trimOrEmpty(nextStatus.error)) {
      setPersistentError(nextStatus.error);
    }
    syncStatusSpaceOptions(nextStatus);
    if (!formDirty.value || nextStatus.running) {
      applyStatusToForm(nextStatus, false);
    }
    if (!nextStatus.running) {
      importPending.value = false;
      cancelling.value = false;
    }
    if (nextStatus.running || nextStatus.stage === "authorizing") {
      schedulePoll(420);
    } else {
      schedulePoll(1200);
    }
  } catch (cause) {
    statusError.value = normalizeAppError(cause).message;
    setPersistentError(statusError.value);
    schedulePoll(1500);
  }
}

async function saveConfig() {
  if (savePending.value) {
    scheduleAutoSave(180);
    return null;
  }
  const requestRevision = formRevision;
  const configInput = buildConfigInput();
  clearAutoSaveTimer();
  autoSaveQueued.value = false;
  savePending.value = true;
  clearPersistentError();
  statusError.value = "";
  try {
    const status = await knowledgeSaveFeishuReferenceConfig(configInput);
    if (requestRevision === formRevision) {
      statusSnapshot.value = status;
      applyStatusToForm(status, true);
    } else {
      statusSnapshot.value = overlayCurrentSelection(status);
      scheduleAutoSave(180);
    }
    if (status.running || status.stage === "authorizing") {
      schedulePoll(420);
    } else {
      schedulePoll(1200);
    }
    return status;
  } catch (cause) {
    statusError.value = normalizeAppError(cause).message;
    setPersistentError(statusError.value);
    return null;
  } finally {
    savePending.value = false;
  }
}

async function loadRootNodes() {
  const spaceId = trimOrEmpty(selectedSpaceId.value);
  if (!spaceId) {
    resetNodeTree();
    return;
  }
  nodeLoading.value = true;
  clearPersistentError();
  nodeError.value = "";
  try {
    const entries = await fetchNodeEntries(null);
    treeNodes.value = entries.map((entry) => createTreeNode(entry, "", 0));
    expandedNodeTokens.value = new Set();
  } catch (cause) {
    nodeError.value = normalizeAppError(cause).message;
    setPersistentError(nodeError.value);
  } finally {
    nodeLoading.value = false;
  }
}

async function ensureNodeChildren(node: FeishuTreeNode) {
  if (!node.summary.hasChild || node.childrenLoaded || node.childrenLoading)
    return;
  node.childrenLoading = true;
  clearPersistentError();
  try {
    const entries = await fetchNodeEntries(node.summary.nodeToken);
    node.children = entries.map((entry) =>
      createTreeNode(entry, node.pathLabel, node.depth + 1),
    );
    node.childrenLoaded = true;
  } catch (cause) {
    const message = normalizeAppError(cause).message;
    setPersistentError(message);
    throw cause;
  } finally {
    node.childrenLoading = false;
  }
}

async function testConnection() {
  if (testPending.value) return;
  testPending.value = true;
  clearPersistentError();
  statusError.value = "";
  try {
    const saved = await saveConfig();
    if (!saved) return;
    const result = await knowledgeTestFeishuReferenceConnection(
      targetPath.value || undefined,
    );
    const resolvedOptions = result.spaces.map((item) => ({
      spaceId: item.spaceId,
      name: trimOrEmpty(item.name) || item.spaceId,
    }));
    upsertSpaceOptions(resolvedOptions);
    if (trimOrEmpty(result.resolvedSpaceId)) {
      selectedSpaceId.value = trimOrEmpty(result.resolvedSpaceId);
      selectedSpaceName.value =
        trimOrEmpty(result.resolvedSpaceName) ||
        resolvedOptions.find((item) => item.spaceId === selectedSpaceId.value)
          ?.name ||
        selectedSpaceId.value;
    } else if (
      !trimOrEmpty(selectedSpaceId.value) &&
      resolvedOptions.length === 1
    ) {
      selectedSpaceId.value = resolvedOptions[0].spaceId;
      selectedSpaceName.value = resolvedOptions[0].name;
    }
    const resolvedRootNodeToken = trimOrEmpty(result.resolvedRootNodeToken);
    const resolvedRootNodeTitle = trimOrEmpty(result.resolvedRootNodeTitle);
    if (resolvedRootNodeToken && selectedRoots.value.length > 0) {
      applySelectedRoots(
        selectedRoots.value.map((root) =>
          root.nodeToken === resolvedRootNodeToken
            ? {
                ...root,
                nodeTitle: resolvedRootNodeTitle || root.nodeTitle || null,
              }
            : root,
        ),
      );
    } else {
      applySelectedRoots(
        resolvedRootNodeToken
          ? [
              {
                nodeToken: resolvedRootNodeToken,
                nodeTitle: resolvedRootNodeTitle || null,
              },
            ]
          : [],
      );
    }
    lastTestSummary.value = result.summary;
    formDirty.value = true;
    scheduleAutoSave();
    await refreshStatus();
    if (trimOrEmpty(selectedSpaceId.value)) {
      await loadRootNodes();
    }
  } catch (cause) {
    statusError.value = normalizeAppError(cause).message;
    setPersistentError(statusError.value);
  } finally {
    testPending.value = false;
  }
}

async function startAuthorization() {
  if (authorizePending.value) return;
  authorizePending.value = true;
  clearPersistentError();
  statusError.value = "";
  try {
    const saved = await saveConfig();
    if (!saved) return;
    const result = await knowledgeStartFeishuReferenceOauth();
    await openUrl(result.authorizeUrl);
    lastTestSummary.value = t(
      "knowledge.feishuReference.window.authorizationStarted",
      result.callbackUrl,
    );
    await refreshStatus();
  } catch (cause) {
    statusError.value = normalizeAppError(cause).message;
    setPersistentError(statusError.value);
  } finally {
    authorizePending.value = false;
  }
}

async function cancelAuthorizationWait(options?: {
  syncUi?: boolean;
  silent?: boolean;
}) {
  const syncUi = options?.syncUi ?? true;
  const silent = options?.silent ?? false;
  if (cancelAuthorizationPending.value || !waitingForAuthorization.value)
    return null;
  cancelAuthorizationPending.value = true;
  if (!silent) {
    clearPersistentError();
    statusError.value = "";
  }
  try {
    const status = await knowledgeCancelFeishuReferenceOauthWait(targetPath.value || undefined);
    if (syncUi) {
      statusSnapshot.value = status;
      applyStatusToForm(status, true);
      schedulePoll(1200);
    }
    return status;
  } catch (cause) {
    if (!silent) {
      statusError.value = normalizeAppError(cause).message;
      setPersistentError(statusError.value);
    }
    return null;
  } finally {
    cancelAuthorizationPending.value = false;
  }
}

async function handleSpaceSelection(nextSpaceId: string) {
  clearPersistentError();
  selectedSpaceId.value = nextSpaceId;
  selectedSpaceName.value =
    spaceOptions.value.find((item) => item.spaceId === nextSpaceId)?.name ||
    statusSnapshot.value?.spaceName ||
    "";
  applySelectedRoots([]);
  resetNodeTree();
  syncLocalSelectionIntoStatusSnapshot();
  markFormDirtyAndQueueSave();
  if (trimOrEmpty(nextSpaceId)) {
    await loadRootNodes();
  }
}

function useSpaceRoot() {
  applySelectedRoots([]);
  syncLocalSelectionIntoStatusSnapshot();
  markFormDirtyAndQueueSave();
}

function toggleNodeSelection(node: FeishuTreeNode) {
  if (statusSnapshot.value?.running) return;
  const nextToken = node.summary.nodeToken;
  const nextTitle = node.pathLabel;
  const exists = selectedRoots.value.some((item) => item.nodeToken === nextToken);
  applySelectedRoots(
    exists
      ? selectedRoots.value.filter((item) => item.nodeToken !== nextToken)
      : [
          ...selectedRoots.value,
          {
            nodeToken: nextToken,
            nodeTitle: nextTitle || null,
          },
        ],
  );
  syncLocalSelectionIntoStatusSnapshot();
  markFormDirtyAndQueueSave();
}

async function toggleNodeExpansion(node: FeishuTreeNode) {
  if (!node.summary.hasChild) return;
  const nextExpanded = new Set(expandedNodeTokens.value);
  if (nextExpanded.has(node.summary.nodeToken)) {
    nextExpanded.delete(node.summary.nodeToken);
    expandedNodeTokens.value = nextExpanded;
    return;
  }
  try {
    await ensureNodeChildren(node);
    nextExpanded.add(node.summary.nodeToken);
    expandedNodeTokens.value = nextExpanded;
  } catch {
    // the inline error is handled through the persistent error banner
  }
}

async function startImport() {
  if (importPending.value || statusSnapshot.value?.running) return;
  importPending.value = true;
  clearPersistentError();
  statusError.value = "";
  try {
    const saved = await saveConfig();
    if (!saved) return;
    const roots = buildSelectedRootsPayload();
    const request = {
      targetPath: trimOrEmpty(targetPath.value) || null,
      spaceId: trimOrEmpty(selectedSpaceId.value),
      spaceName: trimOrEmpty(selectedSpaceName.value) || null,
      roots,
      rootNodeToken: trimOrEmpty(selectedRootNodeToken.value) || null,
      rootNodeTitle:
        roots.length > 0 ? resolveRootSelectionTitle(roots[0]) || null : null,
    };
    const status = await knowledgeImportFeishuReferenceDocs(request);
    statusSnapshot.value = status;
    applyStatusToForm(status, true);
    schedulePoll(260);
  } catch (cause) {
    statusError.value = normalizeAppError(cause).message;
    setPersistentError(statusError.value);
  } finally {
    importPending.value = false;
  }
}

async function cancelImport() {
  if (cancelling.value) return;
  cancelling.value = true;
  clearPersistentError();
  statusError.value = "";
  try {
    const status = await knowledgeCancelFeishuReferenceImport(targetPath.value || undefined);
    statusSnapshot.value = status;
    schedulePoll(260);
  } catch (cause) {
    statusError.value = normalizeAppError(cause).message;
    setPersistentError(statusError.value);
    cancelling.value = false;
  }
}

async function initializeWindow() {
  try {
    await appWindow.setClosable(false);
  } catch {
    // ignore unsupported close state changes
  }

  try {
    closeRequestUnlisten = await appWindow.onCloseRequested((event) => {
      if (allowWindowClose || canCloseWindow.value) {
        return;
      }
      event.preventDefault();
    });
    statusEventUnlisten = await appWindow.listen<FeishuReferenceImportWindowPayload>(
      FEISHU_REFERENCE_IMPORT_WINDOW_STATUS_EVENT,
      (event) => {
        void resetImportTarget(event.payload ?? {});
      },
    );
  } catch {
    // keep local controls available even if close interception is unavailable
  }

  await refreshStatus();
  if (trimOrEmpty(selectedSpaceId.value)) {
    await loadRootNodes();
  }
}

function summarizeRootSelections(
  roots: FeishuReferenceRootSelection[] | null | undefined,
  fallbackToken: string | null | undefined,
  fallbackTitle: string | null | undefined,
  emptyLabel: string,
): string {
  const normalized = normalizeRootSelections(
    roots?.length
      ? roots
      : fallbackToken
        ? [
            {
              nodeToken: fallbackToken,
              nodeTitle: fallbackTitle ?? null,
            },
          ]
        : [],
  );
  if (!normalized.length) return emptyLabel;
  if (normalized.length === 1) {
    return (
      resolveRootSelectionTitle(normalized[0], { preferPathLabel: true }) || emptyLabel
    );
  }
  return t("knowledge.feishuReference.window.selectedRootCount", normalized.length);
}

const authModeOptions = computed(() => [
  {
    value: "app_credentials",
    label: t("knowledge.feishuReference.auth.appCredentials"),
  },
  {
    value: "oauth",
    label: t("knowledge.feishuReference.auth.oauth"),
  },
]);
const oauthPersistenceModeOptions = computed(() => [
  {
    value: "session",
    label: t("knowledge.feishuReference.window.persistenceSession"),
  },
  {
    value: "offline",
    label: t("knowledge.feishuReference.window.persistenceOffline"),
  },
]);

const spaceDropdownOptions = computed(() =>
  spaceOptions.value.map((item) => ({
    value: item.spaceId,
    label: item.name,
    hint: item.spaceId,
  })),
);

const currentStageLabel = computed(() =>
  stageLabel(statusSnapshot.value?.stage),
);
const currentStateLabel = computed(() =>
  stateLabel(statusSnapshot.value?.state),
);
const selectedAuthModeLabel = computed(() => authModeLabel(authMode.value));
const currentIdentityLabel = computed(() => authModeLabel(authMode.value));
const oauthCallbackUrls = computed(
  () => statusSnapshot.value?.callbackUrls ?? [],
);
const persistenceModeLabel = computed(() =>
  oauthPersistenceMode.value === "offline"
    ? t("knowledge.feishuReference.window.persistenceOffline")
    : t("knowledge.feishuReference.window.persistenceSession"),
);
const selectedSpaceLabel = computed(
  () =>
    trimOrEmpty(selectedSpaceName.value) ||
    trimOrEmpty(selectedSpaceId.value) ||
    "—",
);
const selectedRootTokenSet = computed(
  () =>
    new Set(
      selectedRoots.value
        .map((root) => trimOrEmpty(root.nodeToken))
        .filter(Boolean),
    ),
);
const selectedRootLabel = computed(() => {
  if (!trimOrEmpty(selectedSpaceId.value)) return "—";
  return summarizeRootSelections(
    selectedRoots.value,
    selectedRootNodeToken.value,
    selectedRootNodeTitle.value,
    t("knowledge.feishuReference.window.spaceRoot"),
  );
});
const selectedScopeLabel = computed(() => {
  const spaceLabel = selectedSpaceLabel.value;
  if (spaceLabel === "—") return spaceLabel;
  const hasSelectedRoots = selectedRoots.value.length > 0;
  return hasSelectedRoots ? `${spaceLabel} / ${selectedRootLabel.value}` : spaceLabel;
});
const importedScopeLabel = computed(() => {
  const spaceName =
    trimOrEmpty(statusSnapshot.value?.importedSpaceName) ||
    trimOrEmpty(statusSnapshot.value?.importedSpaceId);
  if (!spaceName) return "—";
  const rootLabel = summarizeRootSelections(
    statusSnapshot.value?.importedRoots,
    statusSnapshot.value?.importedRootNodeToken,
    statusSnapshot.value?.importedRootNodeTitle,
    "",
  );
  return rootLabel ? `${spaceName} / ${rootLabel}` : spaceName;
});
const visibleTreeRows = computed<FeishuTreeRow[]>(() => {
  const rows: FeishuTreeRow[] = [];
  const walk = (nodes: FeishuTreeNode[]) => {
    for (const node of nodes) {
      const expanded = expandedNodeTokens.value.has(node.summary.nodeToken);
      rows.push({
        key: node.key,
        node,
        expanded,
        canExpand: node.summary.hasChild,
      });
      if (expanded && node.children.length) {
        walk(node.children);
      }
    }
  };
  walk(treeNodes.value);
  return rows;
});
const busy = computed(
  () =>
    autoSaveQueued.value ||
    savePending.value ||
    testPending.value ||
    authorizePending.value ||
    cancelAuthorizationPending.value ||
    importPending.value ||
    cancelling.value ||
    nodeLoading.value,
);
const hasConfiguredSecret = computed(
  () =>
    !!trimOrEmpty(appSecret.value) ||
    !!statusSnapshot.value?.appSecretConfigured,
);
const waitingForAuthorization = computed(
  () => statusSnapshot.value?.stage === "authorizing",
);
const oauthAuthorized = computed(
  () =>
    authMode.value === "oauth" &&
    statusSnapshot.value?.authMode === "oauth" &&
    !!statusSnapshot.value?.authorized,
);
const showTestConnection = computed(
  () => authMode.value !== "oauth" || oauthAuthorized.value,
);
const canTestConnection = computed(() => {
  return (
    showTestConnection.value &&
    !busy.value &&
    !waitingForAuthorization.value &&
    !!trimOrEmpty(appId.value) &&
    hasConfiguredSecret.value
  );
});
const canAuthorize = computed(
  () =>
    authMode.value === "oauth" &&
    !busy.value &&
    !statusSnapshot.value?.running &&
    !!trimOrEmpty(appId.value) &&
    hasConfiguredSecret.value,
);
const canCancelAuthorizationWait = computed(
  () =>
    waitingForAuthorization.value &&
    !cancelAuthorizationPending.value &&
    !importPending.value &&
    !cancelling.value,
);
const canImport = computed(() => {
  if (busy.value || statusSnapshot.value?.running) return false;
  if (!trimOrEmpty(selectedSpaceId.value)) return false;
  if (authMode.value === "oauth" && !statusSnapshot.value?.authorized)
    return false;
  return !!trimOrEmpty(appId.value) && hasConfiguredSecret.value;
});
const canCloseWindow = computed(
  () =>
    !statusSnapshot.value?.running &&
    !authorizePending.value &&
    !cancelAuthorizationPending.value &&
    !importPending.value &&
    !cancelling.value,
);
const statusHeading = computed(() => {
  if (statusSnapshot.value?.running)
    return t("knowledge.feishuReference.window.titleRunning");
  if (statusSnapshot.value?.stage === "authorizing") {
    return t("knowledge.feishuReference.window.titleAuthorizing");
  }
  if (statusSnapshot.value?.state === "error")
    return t("knowledge.feishuReference.window.titleError");
  return t("knowledge.feishuReference.window.title");
});
const titlebarStatus = computed(() => {
  if (statusSnapshot.value?.running) {
    return `${currentStageLabel.value} · ${formatPercent(statusSnapshot.value?.progress)}`;
  }
  return currentStateLabel.value;
});
const persistentErrorSegments = computed(() =>
  linkifyErrorMessage(persistentError.value).filter(
    (segment) => segment.value.length > 0,
  ),
);
const summaryText = computed(() => {
  if (
    statusSnapshot.value?.running ||
    statusSnapshot.value?.stage === "authorizing"
  ) {
    return (
      statusSnapshot.value?.message?.trim() ||
      t("knowledge.feishuReference.window.subtitle")
    );
  }
  if (lastTestSummary.value.trim()) return lastTestSummary.value.trim();
  return (
    statusSnapshot.value?.message?.trim() ||
    t("knowledge.feishuReference.window.subtitle")
  );
});
const processedLabel = computed(() => {
  const processed = statusSnapshot.value?.processedDocs ?? 0;
  const totalDocs = statusSnapshot.value?.totalDocs;
  return totalDocs == null ? `${processed}` : `${processed} / ${totalDocs}`;
});
const managedPathLabel = computed(() =>
  trimOrEmpty(statusSnapshot.value?.managedPath) ||
  (trimOrEmpty(targetPath.value) ? `reference/${trimOrEmpty(targetPath.value)}` : "—"),
);
const currentItemLabel = computed(() => {
  return (
    trimOrEmpty(statusSnapshot.value?.currentTitle) ||
    trimOrEmpty(statusSnapshot.value?.currentPath) ||
    "—"
  );
});
const authorizedTokenPresent = computed(
  () =>
    !!statusSnapshot.value?.grantedScopes?.length ||
    statusSnapshot.value?.accessTokenExpiresAt != null ||
    statusSnapshot.value?.refreshTokenExpiresAt != null,
);
const missingScopeLabel = computed(() =>
  (statusSnapshot.value?.missingScopes ?? []).join("、"),
);
const appSecretStateLabel = computed(() => {
  if (savePending.value) {
    return t("knowledge.feishuReference.window.saving");
  }
  if (autoSaveQueued.value) {
    return t("knowledge.editor.autosavePending");
  }
  if (formDirty.value && !!trimOrEmpty(appSecret.value))
    return t("knowledge.feishuReference.window.appSecretPending");
  if (statusSnapshot.value?.appSecretConfigured) {
    return t("knowledge.feishuReference.window.appSecretSaved");
  }
  return t("knowledge.feishuReference.window.appSecretMissing");
});
const showAuthorizedUser = computed(() => authMode.value === "oauth");
const showMissingScopeHint = computed(
  () =>
    authMode.value === "oauth" &&
    statusSnapshot.value?.authMode === "oauth" &&
    authorizedTokenPresent.value &&
    !!statusSnapshot.value?.missingScopes?.length,
);
const authorizedUserLabel = computed(() => {
  if (authMode.value !== "oauth") return "—";
  if (statusSnapshot.value?.authMode !== "oauth") {
    return t("knowledge.feishuReference.window.authorizedUserNeedsSave");
  }
  if (!statusSnapshot.value?.authorized) {
    return t("knowledge.feishuReference.window.authorizedUserMissing");
  }
  const primary = trimOrEmpty(statusSnapshot.value?.authorizedUserName);
  const secondary =
    trimOrEmpty(statusSnapshot.value?.authorizedUserEmail) ||
    trimOrEmpty(statusSnapshot.value?.authorizedUserOpenId);
  if (primary && secondary) return `${primary} · ${secondary}`;
  if (primary) return primary;
  if (secondary) return secondary;
  return t("knowledge.feishuReference.window.authorizedUserReady");
});
const oauthBindingLabel = computed(() =>
  t(
    "knowledge.feishuReference.window.oauthBindingHint",
    trimOrEmpty(appId.value) || "—",
    trimOrEmpty(openBaseUrl.value) || DEFAULT_OPEN_BASE_URL,
  ),
);
const oauthPersistenceHint = computed(() =>
  oauthPersistenceMode.value === "offline"
    ? t("knowledge.feishuReference.window.persistenceOfflineHint")
    : t("knowledge.feishuReference.window.persistenceSessionHint"),
);
const showCurrentItem = computed(
  () =>
    !!trimOrEmpty(statusSnapshot.value?.currentTitle) ||
    !!trimOrEmpty(statusSnapshot.value?.currentPath),
);

function treeIndentPx(depth: number): number {
  return 12 + depth * 16;
}

function nodeMetaLabel(node: FeishuTreeNode): string {
  const parts = [formatNodeType(node.summary)];
  if (node.childrenLoading) {
    parts.push(t("common.loading"));
  } else if (node.summary.hasChild) {
    parts.push(t("knowledge.feishuReference.window.hasChildren"));
  }
  return parts.join(" · ");
}

function asTreeRow(item: { key: string }): FeishuTreeRow {
  return item as FeishuTreeRow;
}

watch(
  statusHeading,
  (nextTitle) => {
    void appWindow.setTitle(nextTitle).catch(() => {
      // ignore unsupported title updates
    });
  },
  { immediate: true },
);

async function requestWindowClose() {
  if (!canCloseWindow.value) return;
  await destroyWindow();
}

async function startWindowResize(
  direction: WindowResizeDirection = "SouthEast",
) {
  try {
    await appWindow.startResizeDragging(direction);
  } catch {
    // ignore unsupported resize dragging
  }
}

onMounted(() => {
  void initializeWindow();
});

onUnmounted(() => {
  clearPollTimer();
  clearAutoSaveTimer();
  statusEventUnlisten?.();
  closeRequestUnlisten?.();
});
</script>

<template>
  <div class="feishu-reference-window-root">
    <div class="feishu-reference-window-titlebar">
      <div class="feishu-reference-window-titlebar-label">
        {{ statusHeading }}
      </div>
      <div class="feishu-reference-window-titlebar-actions">
        <div class="feishu-reference-window-titlebar-progress">
          {{ titlebarStatus }}
        </div>
        <button
          class="feishu-reference-window-close"
          type="button"
          :aria-label="t('common.close')"
          :title="t('common.close')"
          :disabled="!canCloseWindow"
          @click="void requestWindowClose()"
        >
          <svg
            viewBox="0 0 16 16"
            fill="currentColor"
            width="14"
            height="14"
            aria-hidden="true"
          >
            <path
              d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"
            />
          </svg>
        </button>
      </div>
    </div>

    <div
      v-if="persistentErrorSegments.length"
      class="feishu-reference-window-error-wrap"
    >
      <div class="feishu-reference-window-error">
        <template
          v-for="(segment, index) in persistentErrorSegments"
          :key="`${segment.kind}-${index}`"
        >
          <span
            v-if="segment.kind === 'text'"
            class="feishu-reference-window-error-text"
          >
            {{ segment.value }}
          </span>
          <button
            v-else
            type="button"
            class="feishu-reference-window-error-link"
            @click.stop="void openErrorLink(segment.href)"
          >
            {{ segment.value }}
          </button>
        </template>
      </div>
    </div>

    <div
      class="feishu-reference-window-body"
      :class="{ 'with-fixed-error': persistentErrorSegments.length > 0 }"
    >
      <div class="feishu-reference-window-scroll">
        <div class="feishu-reference-window-summary">{{ summaryText }}</div>

        <section class="feishu-reference-card">
          <div class="feishu-reference-card-header">
            <div>
              <div class="feishu-reference-card-title">
                {{ t("knowledge.feishuReference.window.connectionTitle") }}
              </div>
              <div class="feishu-reference-card-hint">
                {{ t("knowledge.feishuReference.window.connectionHint") }}
              </div>
            </div>
            <BaseSegmented
              v-model="authMode"
              size="sm"
              :options="authModeOptions"
              @update:model-value="markFormDirtyAndQueueSave(0)"
            />
          </div>

          <div class="feishu-reference-form-grid">
            <label class="feishu-reference-field">
              <span class="feishu-reference-field-label">{{
                t("knowledge.feishuReference.window.appId")
              }}</span>
              <input
                v-model="appId"
                class="feishu-reference-input"
                type="text"
                :placeholder="
                  t('knowledge.feishuReference.window.appIdPlaceholder')
                "
                :disabled="statusSnapshot?.running"
                @input="markFormDirtyAndQueueSave()"
              />
            </label>

            <label class="feishu-reference-field">
              <span class="feishu-reference-field-label">{{
                t("knowledge.feishuReference.window.appSecret")
              }}</span>
              <input
                v-model="appSecret"
                class="feishu-reference-input"
                type="password"
                autocomplete="off"
                :placeholder="
                  t('knowledge.feishuReference.window.appSecretPlaceholder')
                "
                :disabled="statusSnapshot?.running"
                @input="handleAppSecretInput()"
              />
              <span class="feishu-reference-field-meta">{{
                appSecretStateLabel
              }}</span>
            </label>

            <label class="feishu-reference-field feishu-reference-field-wide">
              <span class="feishu-reference-field-label">{{
                t("knowledge.feishuReference.window.openBaseUrl")
              }}</span>
              <input
                v-model="openBaseUrl"
                class="feishu-reference-input"
                type="text"
                :disabled="statusSnapshot?.running"
                @input="markFormDirtyAndQueueSave()"
              />
            </label>

            <div
              v-if="authMode === 'oauth'"
              class="feishu-reference-field feishu-reference-field-wide"
            >
              <span class="feishu-reference-field-label">{{
                t("knowledge.feishuReference.window.persistenceMode")
              }}</span>
              <BaseSegmented
                v-model="oauthPersistenceMode"
                size="sm"
                :options="oauthPersistenceModeOptions"
                @update:model-value="markFormDirtyAndQueueSave(0)"
              />
              <span class="feishu-reference-field-meta">{{
                oauthPersistenceHint
              }}</span>
            </div>
          </div>

          <div class="feishu-reference-inline-note">
            {{
              t(
                "knowledge.feishuReference.window.connectionNote",
                selectedAuthModeLabel,
              )
            }}
          </div>
          <div
            v-if="authMode === 'oauth'"
            class="feishu-reference-inline-note feishu-reference-inline-note-stack"
          >
            <span>{{
              t("knowledge.feishuReference.window.oauthAdminHint")
            }}</span>
            <span>{{
              t("knowledge.feishuReference.window.oauthRedirectHint")
            }}</span>
            <button
              v-for="callbackUrl in oauthCallbackUrls"
              :key="callbackUrl"
              type="button"
              class="feishu-reference-copy-row"
              :class="{
                copied: callbackCopied && copiedCallbackUrl === callbackUrl,
              }"
              :title="
                callbackCopied && copiedCallbackUrl === callbackUrl
                  ? t('common.copied')
                  : t('common.clickToCopy')
              "
              @click="void copyCallbackUrl(callbackUrl)"
            >
              <span class="feishu-reference-path-value">{{ callbackUrl }}</span>
              <span class="feishu-reference-copy-indicator">
                {{
                  callbackCopied && copiedCallbackUrl === callbackUrl
                    ? t("common.copied")
                    : t("common.clickToCopy")
                }}
              </span>
            </button>
          </div>
          <div v-if="authMode === 'oauth'" class="feishu-reference-inline-note">
            {{ oauthBindingLabel }}
          </div>
          <div
            v-if="showMissingScopeHint"
            class="feishu-reference-inline-note feishu-reference-inline-note-warning"
          >
            {{
              t(
                "knowledge.feishuReference.window.missingScopesHint",
                missingScopeLabel,
              )
            }}
          </div>

          <div class="feishu-reference-actions">
            <BaseButton
              v-if="showTestConnection"
              :disabled="!canTestConnection"
              @click="void testConnection()"
            >
              {{
                testPending
                  ? t("knowledge.feishuReference.window.testing")
                  : t("knowledge.feishuReference.window.testConnection")
              }}
            </BaseButton>
            <BaseButton
              v-if="authMode === 'oauth' && waitingForAuthorization"
              :disabled="!canCancelAuthorizationWait"
              @click="void cancelAuthorizationWait()"
            >
              {{
                cancelAuthorizationPending
                  ? t(
                      "knowledge.feishuReference.window.cancelAuthorizationPending",
                    )
                  : t("knowledge.feishuReference.window.cancelAuthorization")
              }}
            </BaseButton>
            <BaseButton
              v-else-if="authMode === 'oauth'"
              :disabled="!canAuthorize"
              @click="void startAuthorization()"
            >
              {{
                authorizePending
                  ? t("knowledge.feishuReference.window.authorizing")
                  : t("knowledge.feishuReference.window.authorize")
              }}
            </BaseButton>
          </div>
        </section>

        <section class="feishu-reference-card">
          <div class="feishu-reference-card-header">
            <div>
              <div class="feishu-reference-card-title">
                {{ t("knowledge.feishuReference.window.scopeTitle") }}
              </div>
              <div class="feishu-reference-card-hint">
                {{ t("knowledge.feishuReference.window.scopeHint") }}
              </div>
            </div>
          </div>

          <div class="feishu-reference-scope-grid">
            <div class="feishu-reference-field">
              <span class="feishu-reference-field-label">{{
                t("knowledge.feishuReference.window.space")
              }}</span>
              <BaseDropdown
                :model-value="selectedSpaceId"
                size="md"
                :disabled="
                  !spaceDropdownOptions.length || statusSnapshot?.running
                "
                :options="spaceDropdownOptions"
                :placeholder="
                  t('knowledge.feishuReference.window.selectSpacePlaceholder')
                "
                :aria-label="t('knowledge.feishuReference.window.space')"
                @update:model-value="
                  (value) => void handleSpaceSelection(value)
                "
              />
            </div>
            <div class="feishu-reference-field">
              <span class="feishu-reference-field-label">{{
                t("knowledge.feishuReference.window.selectedRoot")
              }}</span>
              <div class="feishu-reference-selection-value">
                {{ selectedRootLabel }}
              </div>
              <span class="feishu-reference-field-meta">
                {{
                  t(
                    "knowledge.feishuReference.window.selectedSpaceValue",
                    selectedScopeLabel,
                  )
                }}
              </span>
            </div>
          </div>

          <div class="feishu-reference-browser-toolbar">
            <div class="feishu-reference-browser-title">
              <span>{{
                t("knowledge.feishuReference.window.selectedRoot")
              }}</span>
              <span class="feishu-reference-browser-title-value">{{
                selectedRootLabel
              }}</span>
            </div>
            <div class="feishu-reference-browser-actions">
              <BaseButton
                :disabled="!selectedSpaceId || statusSnapshot?.running"
                @click="useSpaceRoot()"
              >
                {{ t("knowledge.feishuReference.window.useSpaceRoot") }}
              </BaseButton>
              <BaseButton
                :disabled="
                  !selectedSpaceId || nodeLoading || statusSnapshot?.running
                "
                @click="void loadRootNodes()"
              >
                {{ t("knowledge.referenceImport.refresh") }}
              </BaseButton>
            </div>
          </div>

          <div class="feishu-reference-browser">
            <div v-if="!selectedSpaceId" class="feishu-reference-browser-empty">
              {{ t("knowledge.feishuReference.window.selectSpaceFirst") }}
            </div>
            <div v-else-if="nodeError" class="feishu-reference-browser-empty">
              {{ nodeError }}
            </div>
            <div v-else-if="nodeLoading" class="feishu-reference-browser-empty">
              {{ t("common.loading") }}
            </div>
            <div
              v-else-if="!visibleTreeRows.length"
              class="feishu-reference-browser-empty"
            >
              {{ t("knowledge.feishuReference.window.emptyNodes") }}
            </div>
            <FileTreeList
              v-else
              class="feishu-reference-browser-list"
              :items="visibleTreeRows"
              :row-height="32"
            >
              <template #item="{ item }">
                <div
                  v-for="entry in [asTreeRow(item)]"
                  :key="entry.key"
                  class="feishu-reference-tree-row-shell"
                  :class="{
                    active: selectedRootTokenSet.has(entry.node.summary.nodeToken),
                  }"
                  :style="{
                    paddingLeft: `${treeIndentPx(entry.node.depth)}px`,
                  }"
                >
                  <button
                    v-if="entry.canExpand"
                    type="button"
                    class="feishu-reference-tree-branch"
                    :aria-label="
                      entry.expanded
                        ? t(
                            'merge.tree.toggleCollapse',
                            nodeTitle(entry.node.summary),
                          )
                        : t(
                            'merge.tree.toggleExpand',
                            nodeTitle(entry.node.summary),
                          )
                    "
                    :title="
                      entry.expanded
                        ? t(
                            'merge.tree.toggleCollapse',
                            nodeTitle(entry.node.summary),
                          )
                        : t(
                            'merge.tree.toggleExpand',
                            nodeTitle(entry.node.summary),
                          )
                    "
                    :disabled="entry.node.childrenLoading"
                    @click.stop="void toggleNodeExpansion(entry.node)"
                  >
                    <svg
                      class="feishu-reference-tree-chevron"
                      :class="{ open: entry.expanded }"
                      viewBox="0 0 16 16"
                      width="10"
                      height="10"
                      fill="currentColor"
                      aria-hidden="true"
                    >
                      <path
                        d="M6.22 3.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L9.94 8 6.22 4.28a.75.75 0 0 1 0-1.06z"
                      />
                    </svg>
                  </button>
                  <span
                    v-else
                    class="feishu-reference-tree-spacer-slot"
                    aria-hidden="true"
                  ></span>
                  <span
                    class="feishu-reference-tree-kind-icon"
                    :class="{
                      folder: entry.canExpand,
                      open: entry.canExpand && entry.expanded,
                    }"
                    aria-hidden="true"
                  >
                    <svg
                      v-if="entry.canExpand"
                      viewBox="0 0 16 16"
                      width="13"
                      height="13"
                      fill="none"
                    >
                      <path
                        d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linejoin="round"
                      />
                    </svg>
                    <svg
                      v-else
                      viewBox="0 0 16 16"
                      width="13"
                      height="13"
                      fill="none"
                    >
                      <path
                        d="M5 2.75h4.55c.3 0 .58.12.8.33l1.57 1.57c.21.22.33.5.33.8V12A1.25 1.25 0 0 1 11 13.25H5A1.25 1.25 0 0 1 3.75 12V4A1.25 1.25 0 0 1 5 2.75Z"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linejoin="round"
                      />
                      <path
                        d="M9.5 2.9V5a.5.5 0 0 0 .5.5h2.1"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </svg>
                  </span>
                  <BaseCheckbox
                    :model-value="
                      selectedRootTokenSet.has(entry.node.summary.nodeToken)
                    "
                    :disabled="statusSnapshot?.running"
                    :aria-label="
                      t(
                        'knowledge.feishuReference.window.toggleNodeSelection',
                        nodeTitle(entry.node.summary),
                      )
                    "
                    @update:model-value="
                      () => toggleNodeSelection(entry.node)
                    "
                  />
                  <button
                    type="button"
                    class="feishu-reference-tree-row"
                    @click="toggleNodeSelection(entry.node)"
                  >
                    <span class="feishu-reference-tree-title">{{
                      nodeTitle(entry.node.summary)
                    }}</span>
                    <span class="feishu-reference-tree-meta">{{
                      nodeMetaLabel(entry.node)
                    }}</span>
                  </button>
                </div>
              </template>
            </FileTreeList>
          </div>
        </section>

        <section class="feishu-reference-card">
          <div class="feishu-reference-card-header">
            <div>
              <div class="feishu-reference-card-title">
                {{ t("knowledge.feishuReference.window.importTitle") }}
              </div>
              <div class="feishu-reference-card-hint">
                {{ t("knowledge.feishuReference.window.importHint") }}
              </div>
            </div>
          </div>

          <div class="feishu-reference-meta-grid">
            <div class="feishu-reference-row">
              <span>{{ t("knowledge.feishuReference.window.state") }}</span>
              <span>{{ currentStateLabel }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{ t("knowledge.feishuReference.window.authMode") }}</span>
              <span>{{ currentIdentityLabel }}</span>
            </div>
            <div v-if="authMode === 'oauth'" class="feishu-reference-row">
              <span>{{
                t("knowledge.feishuReference.window.persistenceMode")
              }}</span>
              <span>{{ persistenceModeLabel }}</span>
            </div>
            <div v-if="showAuthorizedUser" class="feishu-reference-row">
              <span>{{
                t("knowledge.feishuReference.window.authorizedUser")
              }}</span>
              <span>{{ authorizedUserLabel }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{ t("knowledge.dashboard.knowledge.rebuildStage") }}</span>
              <span>{{ currentStageLabel }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{
                t("knowledge.feishuReference.window.selectedScope")
              }}</span>
              <span>{{ selectedScopeLabel }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{
                t("knowledge.feishuReference.window.importedScope")
              }}</span>
              <span>{{ importedScopeLabel }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{ t("knowledge.feishuReference.window.progress") }}</span>
              <span>{{ formatPercent(statusSnapshot?.progress) }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{ t("knowledge.referenceImport.window.processed") }}</span>
              <span>{{ processedLabel }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{ t("knowledge.referenceImport.importedCount") }}</span>
              <span>{{ statusSnapshot?.importedDocCount ?? 0 }}</span>
            </div>
            <div class="feishu-reference-row">
              <span>{{ t("knowledge.referenceImport.importedAt") }}</span>
              <span>{{ formatDateTime(statusSnapshot?.importedAt) }}</span>
            </div>
            <div class="feishu-reference-row feishu-reference-row-wide">
              <span>{{ t("knowledge.referenceImport.managedPath") }}</span>
              <span class="feishu-reference-path-value">{{
                managedPathLabel
              }}</span>
            </div>
            <div
              v-if="showCurrentItem"
              class="feishu-reference-row feishu-reference-row-wide"
            >
              <span>{{
                t("knowledge.feishuReference.window.currentItem")
              }}</span>
              <span class="feishu-reference-path-value">{{
                currentItemLabel
              }}</span>
            </div>
          </div>
        </section>

        <div class="feishu-reference-window-footer">
          <BaseButton
            v-if="!statusSnapshot?.running"
            :disabled="!canCloseWindow"
            @click="void destroyWindow()"
          >
            {{ t("common.close") }}
          </BaseButton>
          <BaseButton
            v-if="statusSnapshot?.running"
            :disabled="cancelling"
            @click="void cancelImport()"
          >
            {{
              cancelling
                ? t("knowledge.referenceImport.window.cancelling")
                : t("common.cancel")
            }}
          </BaseButton>
          <BaseButton
            variant="primary"
            :disabled="!canImport"
            @click="void startImport()"
          >
            {{
              importPending
                ? t("knowledge.referenceImport.window.starting")
                : t("knowledge.referenceImport.action.import")
            }}
          </BaseButton>
          <button
            type="button"
            class="feishu-reference-window-resize-handle"
            :aria-label="t('knowledge.feishuReference.window.resizeWindow')"
            :title="t('knowledge.feishuReference.window.resizeWindow')"
            @mousedown.prevent="void startWindowResize('SouthEast')"
          >
            <span
              aria-hidden="true"
              class="feishu-reference-window-resize-icon"
            >
              <span></span>
              <span></span>
              <span></span>
            </span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.feishu-reference-window-root {
  position: relative;
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
  border: 1px solid var(--border-strong);
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, white 8%, transparent),
    inset 0 0 0 1px color-mix(in srgb, var(--border-strong) 82%, transparent);
  overflow: hidden;
}

.feishu-reference-window-titlebar {
  -webkit-app-region: drag;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 38px;
  padding: 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
  box-shadow: inset 0 1px 0 color-mix(in srgb, white 6%, transparent);
}

.feishu-reference-window-titlebar-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.feishu-reference-window-titlebar-progress {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.feishu-reference-window-titlebar-actions {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  -webkit-app-region: no-drag;
}

.feishu-reference-window-close {
  -webkit-app-region: no-drag;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition:
    background 0.15s ease,
    color 0.15s ease,
    opacity 0.15s ease;
}

.feishu-reference-window-close:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.feishu-reference-window-close:disabled {
  opacity: 0.45;
  cursor: default;
}

.feishu-reference-window-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 16px 18px 18px;
  background: var(--panel-bg);
  overflow: hidden;
}

.feishu-reference-window-body.with-fixed-error {
  padding-top: 14px;
}

.feishu-reference-window-error-wrap {
  flex-shrink: 0;
  padding: 16px 18px 0;
  background: var(--panel-bg);
}

.feishu-reference-window-scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.feishu-reference-window-summary {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.feishu-reference-window-error {
  flex-shrink: 0;
  max-height: min(32vh, 240px);
  overflow: auto;
  padding: 10px 12px;
  border: 1px solid var(--status-danger-border);
  border-radius: 8px;
  background: color-mix(
    in srgb,
    var(--status-danger-bg) 88%,
    var(--panel-bg) 12%
  );
  color: var(--status-danger-fg);
  font-size: 12px;
  line-height: 1.65;
  white-space: pre-wrap;
  word-break: break-word;
}

.feishu-reference-window-error-link {
  display: inline;
  padding: 0;
  border: none;
  background: transparent;
  color: inherit;
  font: inherit;
  line-height: inherit;
  text-decoration-line: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 0.16em;
  cursor: pointer;
}

.feishu-reference-window-error-link:hover {
  text-decoration-thickness: 2px;
}

.feishu-reference-window-error-link:focus-visible {
  outline: none;
  text-decoration-thickness: 2px;
}

.feishu-reference-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.feishu-reference-card-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.feishu-reference-card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.feishu-reference-card-hint {
  margin-top: 4px;
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.feishu-reference-form-grid,
.feishu-reference-scope-grid,
.feishu-reference-meta-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.feishu-reference-field {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.feishu-reference-field-wide,
.feishu-reference-row-wide {
  grid-column: 1 / -1;
}

.feishu-reference-field-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.feishu-reference-field-meta {
  font-size: 11px;
  line-height: 1.45;
  color: var(--text-secondary);
}

.feishu-reference-input,
.feishu-reference-selection-value {
  width: 100%;
  min-height: 32px;
  box-sizing: border-box;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--bg-color) 24%);
  color: var(--text-color);
  font-size: 13px;
}

.feishu-reference-input {
  padding: 0 11px;
  transition:
    border-color 0.15s ease,
    box-shadow 0.15s ease,
    background 0.15s ease;
}

.feishu-reference-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.feishu-reference-selection-value {
  display: flex;
  align-items: center;
  padding: 0 11px;
  font-weight: 600;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.feishu-reference-inline-note {
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-secondary);
}

.feishu-reference-inline-note-stack {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.feishu-reference-inline-note-warning {
  color: var(--status-warn-fg);
}

.feishu-reference-actions,
.feishu-reference-browser-actions,
.feishu-reference-window-footer {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.feishu-reference-browser-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.feishu-reference-browser-title {
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 6px;
  font-size: 11px;
  color: var(--text-secondary);
}

.feishu-reference-browser-title-value {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.feishu-reference-browser {
  display: flex;
  flex-direction: column;
  min-height: 196px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--input-bg) 22%);
  overflow: hidden;
}

.feishu-reference-browser-empty {
  min-height: 196px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
  text-align: center;
}

.feishu-reference-browser-list {
  flex: 1;
  min-height: 196px;
  padding: 4px 0;
}

.feishu-reference-tree-row-shell {
  border-bottom: 1px solid
    color-mix(in srgb, var(--border-color) 72%, transparent);
  display: flex;
  align-items: stretch;
  gap: 2px;
  min-height: 32px;
  min-width: 0;
  transition: background 0.12s ease;
}

.feishu-reference-tree-row-shell:hover {
  background: var(--hover-bg);
}

.feishu-reference-tree-row-shell.active {
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--accent-soft) 30%);
}

.feishu-reference-tree-branch,
.feishu-reference-tree-spacer-slot,
.feishu-reference-tree-kind-icon {
  width: 16px;
  min-width: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  align-self: center;
}

.feishu-reference-tree-branch {
  margin-left: 4px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition:
    color 0.15s ease,
    background 0.15s ease;
}

.feishu-reference-tree-branch:hover:not(:disabled) {
  color: var(--text-color);
  background: color-mix(in srgb, var(--hover-bg) 82%, transparent);
}

.feishu-reference-tree-branch:focus-visible {
  outline: none;
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.feishu-reference-tree-branch:disabled {
  cursor: progress;
  opacity: 0.7;
}

.feishu-reference-tree-chevron {
  opacity: 0.64;
  transition: transform 0.15s ease;
}

.feishu-reference-tree-chevron.open {
  transform: rotate(90deg);
}

.feishu-reference-tree-kind-icon {
  color: color-mix(in srgb, var(--text-secondary) 88%, transparent);
}

.feishu-reference-tree-kind-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 42%, var(--text-secondary) 58%);
}

.feishu-reference-tree-kind-icon.folder.open {
  color: color-mix(in srgb, var(--accent-color) 56%, var(--text-secondary) 44%);
}

.feishu-reference-tree-row {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  padding: 4px 12px 4px 0;
  text-align: left;
  cursor: pointer;
  color: inherit;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.feishu-reference-tree-row:focus-visible {
  outline: none;
  box-shadow: inset 0 0 0 2px
    color-mix(in srgb, var(--accent-color) 18%, transparent);
  border-radius: 6px;
}

.feishu-reference-tree-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.35;
}

.feishu-reference-tree-meta {
  flex-shrink: 0;
  white-space: nowrap;
  font-size: 11px;
  line-height: 1.35;
  color: var(--text-secondary);
}

.feishu-reference-row {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-bottom: 8px;
  border-bottom: 1px solid
    color-mix(in srgb, var(--border-color) 72%, transparent);
  font-size: 12px;
  color: var(--text-secondary);
}

.feishu-reference-row span:last-child {
  text-align: right;
  color: var(--text-color);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.feishu-reference-path-value {
  max-width: min(100%, 420px);
  word-break: break-word;
  font-family: var(--font-mono-identifier);
}

.feishu-reference-copy-row {
  width: 100%;
  min-width: 0;
  padding: 0;
  border: none;
  background: transparent;
  color: inherit;
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  text-align: left;
  cursor: pointer;
  transition: color 0.15s ease;
}

.feishu-reference-copy-row .feishu-reference-path-value {
  flex: 1 1 auto;
  min-width: 0;
  max-width: none;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  word-break: normal;
}

.feishu-reference-copy-row:hover .feishu-reference-path-value,
.feishu-reference-copy-row:focus-visible .feishu-reference-path-value {
  color: var(--text-color);
  text-decoration: underline;
  text-underline-offset: 0.16em;
}

.feishu-reference-copy-row.copied .feishu-reference-copy-indicator {
  color: var(--accent-color);
}

.feishu-reference-copy-row:focus-visible {
  outline: none;
}

.feishu-reference-copy-indicator {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  transition: color 0.15s ease;
}

.feishu-reference-window-footer {
  justify-content: flex-end;
}

.feishu-reference-window-resize-handle {
  -webkit-app-region: no-drag;
  margin-left: 12px;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--sidebar-bg) 18%);
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: nwse-resize;
  transition:
    background 0.15s ease,
    color 0.15s ease,
    border-color 0.15s ease;
}

.feishu-reference-window-resize-handle:hover {
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--hover-bg) 30%);
  color: var(--text-color);
  border-color: var(--border-strong);
}

.feishu-reference-window-resize-handle:focus-visible {
  outline: none;
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.feishu-reference-window-resize-icon {
  width: 12px;
  height: 12px;
  position: relative;
  display: inline-flex;
  align-items: flex-end;
  justify-content: flex-end;
}

.feishu-reference-window-resize-icon span {
  position: absolute;
  right: 0;
  height: 1px;
  border-radius: 999px;
  background: currentColor;
  transform-origin: right center;
  transform: rotate(-45deg);
}

.feishu-reference-window-resize-icon span:nth-child(1) {
  bottom: 1px;
  width: 5px;
}

.feishu-reference-window-resize-icon span:nth-child(2) {
  bottom: 4px;
  width: 8px;
}

.feishu-reference-window-resize-icon span:nth-child(3) {
  bottom: 7px;
  width: 11px;
}

@media (max-width: 820px) {
  .feishu-reference-card-header,
  .feishu-reference-browser-toolbar {
    flex-direction: column;
    align-items: stretch;
  }

  .feishu-reference-form-grid,
  .feishu-reference-scope-grid,
  .feishu-reference-meta-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .feishu-reference-row {
    flex-direction: column;
    align-items: flex-start;
  }

  .feishu-reference-row span:last-child {
    text-align: left;
  }
}
</style>
