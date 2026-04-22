<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { useNotificationStore } from "../../stores/notification";
import {
  knowledgeCancelFeishuReferenceImport,
  knowledgeCancelFeishuReferenceOauthWait,
  knowledgeCancelUnityReferenceImport,
  knowledgeCreate,
  knowledgeFindUnityReferenceDirectory,
  knowledgeGetFeishuReferenceImportStatus,
  knowledgeGetUnityReferenceImportStatus,
  knowledgeImportFeishuReferenceDocs,
  knowledgeImportUnityReferenceDocs,
  knowledgeListFeishuReferenceSpaceNodes,
  knowledgeListDirectories,
  knowledgeSaveFeishuReferenceConfig,
  knowledgeStartFeishuReferenceOauth,
  knowledgeTestFeishuReferenceConnection,
} from "../../services/knowledge";
import type {
  FeishuReferenceAuthMode,
  FeishuReferenceConfigInput,
  FeishuReferenceImportRequest,
  FeishuReferenceImportStatus,
  FeishuReferenceNodeSummary,
  FeishuReferenceOauthPersistenceMode,
  FeishuReferenceRootSelection,
  KnowledgeDirectoryConfigRecord,
  UnityReferenceImportLocale,
  UnityReferenceImportStatus,
} from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import ReferenceExternalImportFeishuWindowFlow from "./externalImport/ReferenceExternalImportFeishuWindowFlow.vue";
import ReferenceExternalImportUnityWindowPane from "./externalImport/ReferenceExternalImportUnityWindowPane.vue";
import {
  resolveStableExternalImportTargetPath,
} from "./referenceExternalImportPaths";
import type {
  ReferenceExternalImportFeishuTreeRowModel,
  ReferenceExternalImportFeishuWindowModel,
  ReferenceExternalImportUnityWindowModel,
} from "./externalImport/referenceExternalImportModels";

const notificationStore = useNotificationStore();

export type ExternalImportSource = "feishu" | "unity";

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

const props = withDefaults(defineProps<{
  mode?: "dialog" | "directory" | "window";
  parentDir?: string;
  fixedTargetPath?: string | null;
  initialSource?: ExternalImportSource;
  directory?: KnowledgeDirectoryConfigRecord | null;
  pathExists?: ((path: string) => boolean) | null;
  ensureDirectory?: ((path: string) => Promise<boolean>) | null;
  selectDirectory?: ((path: string) => Promise<void>) | null;
  refreshKnowledge?: (() => Promise<void>) | null;
  deleteFeishuImport?: ((path: string) => Promise<void>) | null;
  deleteUnityImport?: ((path: string) => Promise<void>) | null;
}>(), {
  mode: "dialog",
  parentDir: "",
  fixedTargetPath: null,
  initialSource: "feishu",
  directory: null,
  pathExists: null,
  ensureDirectory: null,
  selectDirectory: null,
  refreshKnowledge: null,
  deleteFeishuImport: null,
  deleteUnityImport: null,
});

const emit = defineEmits<{
  (e: "close"): void;
  (e: "runningChange", value: boolean): void;
}>();

const DEFAULT_FEISHU_BASE_DIR = "feishu-knowledge-base";
const DEFAULT_FEISHU_OPEN_BASE_URL = "https://open.feishu.cn";
const DEFAULT_UNITY_DIR = "unity-official-docs";
const UNITY_IMPORT_STAGE_ORDER = [
  "resolving_source",
  "downloading",
  "extracting",
  "converting",
  "reconciling",
] as const;
const knownReferenceDirectories = ref<Set<string>>(new Set());

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

function normalizeRelativePath(path: string | null | undefined): string {
  return trimOrEmpty(path).replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

function joinRelativePath(parentDir: string, name: string): string {
  const normalizedParent = normalizeRelativePath(parentDir);
  const normalizedName = normalizeRelativePath(name);
  return normalizedParent ? `${normalizedParent}/${normalizedName}` : normalizedName;
}

function isUnityManagedTargetPath(path: string | null | undefined): boolean {
  return normalizeRelativePath(path) === DEFAULT_UNITY_DIR;
}

function unityRequestTargetPath(path: string | null | undefined): string | undefined {
  const normalized = normalizeRelativePath(path);
  if (!normalized || isUnityManagedTargetPath(normalized)) {
    return undefined;
  }
  return normalized;
}

function sanitizePathSegment(value: string, fallback: string): string {
  const sanitized = trimOrEmpty(value)
    .replace(/[\\/:*?"<>|]+/g, "-")
    .replace(/\s+/g, "-")
    .replace(/\.+$/g, "")
    .replace(/^-+|-+$/g, "");
  return sanitized || fallback;
}

function referencePathLabel(path: string | null | undefined): string {
  const normalized = normalizeRelativePath(path);
  return normalized ? `reference/${normalized}` : "reference";
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

function formatBytes(bytes: number | null | undefined): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 100 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function formatPercent(value: number): string {
  return `${Math.round(Math.max(0, Math.min(1, value)) * 100)}%`;
}

function clampProgress(value: number): number {
  return Math.max(0, Math.min(1, value));
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

function normalizeSingleRootSelection(
  roots: FeishuReferenceRootSelection[] | null | undefined,
): FeishuReferenceRootSelection[] {
  const [primary] = normalizeRootSelections(roots);
  return primary ? [primary] : [];
}

function preferredSourceFromDirectory(
  directory: KnowledgeDirectoryConfigRecord | null,
): ExternalImportSource | null {
  const sources = Array.isArray(directory?.externalSources)
    ? directory?.externalSources ?? []
    : [];
  if (sources.some((source) => source.provider === "unity")) return "unity";
  if (sources.some((source) => source.provider === "feishu")) return "feishu";
  return null;
}

function localeLabel(locale: UnityReferenceImportLocale | null | undefined): string {
  switch (locale) {
    case "zh-CN":
      return t("knowledge.referenceImport.locale.zhCn");
    case "en":
    default:
      return t("knowledge.referenceImport.locale.en");
  }
}

function unityStageLabel(stage: string | null | undefined): string {
  switch (stage) {
    case "resolving_source":
      return t("knowledge.referenceImport.stage.resolvingSource");
    case "downloading":
      return t("knowledge.referenceImport.stage.downloading");
    case "extracting":
      return t("knowledge.referenceImport.stage.extracting");
    case "converting":
      return t("knowledge.referenceImport.stage.converting");
    case "reconciling":
      return t("knowledge.referenceImport.stage.reconciling");
    case "ready":
      return t("knowledge.referenceImport.stage.ready");
    case "error":
      return t("knowledge.referenceImport.stage.error");
    case "idle":
    default:
      return t("knowledge.referenceImport.stage.idle");
  }
}

function feishuStageLabel(stage: FeishuReferenceImportStatus["stage"] | null | undefined): string {
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

function feishuStateLabel(state: FeishuReferenceImportStatus["state"] | null | undefined): string {
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

function unityProgressRatioForStatus(status: UnityReferenceImportStatus | null): number | null {
  if (!status) return null;
  if (status.stage === "ready" || status.state === "ready") return 1;
  if (typeof status.progress === "number") return clampProgress(status.progress);
  if (status.stage === "downloading" && status.totalBytes && typeof status.downloadedBytes === "number") {
    return clampProgress(status.downloadedBytes / status.totalBytes);
  }
  if (status.stage === "converting" && status.totalDocs) {
    return clampProgress(status.processedDocs / status.totalDocs);
  }
  return null;
}

function feishuProgressRatioForStatus(status: FeishuReferenceImportStatus | null): number | null {
  if (!status) return null;
  if (typeof status.progress === "number") return clampProgress(status.progress);
  if (status.totalDocs && status.processedDocs > 0) {
    return clampProgress(status.processedDocs / status.totalDocs);
  }
  if (status.stage === "ready" && status.importedDocCount > 0) return 1;
  return null;
}

const normalizedParentDir = computed(() => normalizeRelativePath(props.parentDir));
const fixedTargetPath = computed(() => normalizeRelativePath(props.fixedTargetPath));
const sourceTouched = ref(false);
const activeSource = ref<ExternalImportSource>(props.initialSource);

watch(
  () => [props.initialSource, props.directory?.path, JSON.stringify(props.directory?.externalSources ?? [])],
  () => {
    if (sourceTouched.value) return;
    activeSource.value = preferredSourceFromDirectory(props.directory) ?? props.initialSource;
  },
  { immediate: true },
);

function setActiveSource(value: string) {
  activeSource.value = value as ExternalImportSource;
  sourceTouched.value = true;
}

function setFeishuAuthMode(value: string) {
  feishuAuthMode.value = value as FeishuReferenceAuthMode;
  feishuConnectionVerified.value = false;
  feishuOauthRequestedInSession.value = false;
}

function setFeishuAppId(value: string) {
  feishuAppId.value = value;
  feishuConnectionVerified.value = false;
  feishuOauthRequestedInSession.value = false;
}

function setFeishuAppSecret(value: string) {
  feishuAppSecret.value = value;
  feishuAppSecretTouched.value = true;
  feishuConnectionVerified.value = false;
  feishuOauthRequestedInSession.value = false;
}

function setFeishuOpenBaseUrl(value: string) {
  feishuOpenBaseUrl.value = value;
  feishuConnectionVerified.value = false;
  feishuOauthRequestedInSession.value = false;
}

function setFeishuOauthPersistenceMode(value: string) {
  feishuOauthPersistenceMode.value = value as FeishuReferenceOauthPersistenceMode;
  feishuConnectionVerified.value = false;
  feishuOauthRequestedInSession.value = false;
}

function setFeishuSelectedSpaceId(value: string) {
  feishuSelectedSpaceId.value = trimOrEmpty(value);
}

function localPathExists(path: string): boolean {
  return knownReferenceDirectories.value.has(normalizeRelativePath(path));
}

async function refreshKnownReferenceDirectories() {
  try {
    const directories = await knowledgeListDirectories("reference");
    knownReferenceDirectories.value = new Set(
      directories
        .map((path) => normalizeRelativePath(path))
        .filter(Boolean),
    );
  } catch {
    // ignore listing failures in standalone mode and fall back to base names
  }
}

async function ensureDirectoryReady(path: string): Promise<boolean> {
  const normalized = normalizeRelativePath(path);
  if (!normalized) return false;
  if (props.ensureDirectory) {
    return props.ensureDirectory(normalized);
  }
  if (localPathExists(normalized)) return true;
  let createCause: unknown = null;
  try {
    await knowledgeCreate({
      kind: "directory",
      type: "reference",
      path: normalized,
    });
  } catch (cause) {
    createCause = cause;
  }
  await refreshKnownReferenceDirectories();
  if (localPathExists(normalized)) return true;
  if (createCause) throw createCause;
  return false;
}

async function focusDirectory(path: string, refresh = false) {
  const normalized = normalizeRelativePath(path);
  if (!normalized) return;
  if (refresh && props.refreshKnowledge) {
    await props.refreshKnowledge();
  }
  if (props.selectDirectory) {
    await props.selectDirectory(normalized);
  }
}

function externalSourceProviders(): ExternalImportSource[] {
  const sources = Array.isArray(props.directory?.externalSources)
    ? props.directory?.externalSources ?? []
    : [];
  const providers = new Set<ExternalImportSource>();
  for (const source of sources) {
    if (source.provider === "feishu") providers.add("feishu");
    if (source.provider === "unity") providers.add("unity");
  }
  return Array.from(providers.values());
}

const boundProviders = computed(() => externalSourceProviders());

// Unity
const unitySelectedLocale = ref<UnityReferenceImportLocale>("en");
const unityStatus = ref<UnityReferenceImportStatus | null>(null);
const unityError = ref("");
const unityExistingDirectory = ref<KnowledgeDirectoryConfigRecord | null>(null);
const unityMaterializedTargetPath = ref("");
const unityExistingLoading = ref(false);
const unityStartPending = ref(false);
const unityCancelPending = ref(false);
const unityImportSessionStarted = ref(false);
const unityCloseAfterSuccess = ref(false);
let unityPollTimer: ReturnType<typeof setTimeout> | null = null;

function clearUnityPollTimer() {
  if (!unityPollTimer) return;
  clearTimeout(unityPollTimer);
  unityPollTimer = null;
}

function scheduleUnityPoll(delay = 600) {
  clearUnityPollTimer();
  unityPollTimer = setTimeout(() => {
    unityPollTimer = null;
    void refreshUnityStatus();
  }, delay);
}

async function loadUnityExistingDirectory() {
  unityExistingLoading.value = true;
  try {
    unityExistingDirectory.value = await knowledgeFindUnityReferenceDirectory();
  } catch (cause) {
    unityError.value = normalizeAppError(cause).message;
  } finally {
    unityExistingLoading.value = false;
  }
}

const unityExistingPath = computed(() =>
  normalizeRelativePath(unityExistingDirectory.value?.path),
);

const unityTargetPath = computed(() => {
  return resolveStableExternalImportTargetPath({
    fixedTargetPath: isUnityManagedTargetPath(fixedTargetPath.value)
      ? fixedTargetPath.value
      : null,
    preferredTargetPath: unityExistingPath.value,
    materializedTargetPath: isUnityManagedTargetPath(unityMaterializedTargetPath.value)
      ? unityMaterializedTargetPath.value
      : null,
    basePath: DEFAULT_UNITY_DIR,
    pathExists: props.pathExists ?? localPathExists,
  });
});

const unityTargetPathLabel = computed(() => referencePathLabel(unityTargetPath.value));
const unityHasForeignBinding = computed(() =>
  !!fixedTargetPath.value
  && !!unityExistingPath.value
  && unityExistingPath.value !== fixedTargetPath.value,
);
const unityBoundHere = computed(() =>
  boundProviders.value.includes("unity")
  || (!!fixedTargetPath.value && unityExistingPath.value === fixedTargetPath.value),
);
const unityActionLabel = computed(() => {
  if (unityStatus.value?.running) return t("knowledge.referenceImport.action.running");
  if (unityBoundHere.value || unityExistingPath.value === unityTargetPath.value) {
    return t("knowledge.referenceImport.action.reimport");
  }
  return t("knowledge.referenceImport.action.import");
});

function applyUnityStatus(
  status: UnityReferenceImportStatus | null,
  options: { startedSession?: boolean } = {},
) {
  unityStatus.value = status;
  if (status?.selectedLocale) {
    unitySelectedLocale.value = status.selectedLocale;
  }
  if (options.startedSession) {
    unityImportSessionStarted.value = true;
  }
  if (status?.running || status?.state === "running") {
    unityImportSessionStarted.value = true;
    unityCloseAfterSuccess.value = false;
    return;
  }
  if (unityImportSessionStarted.value && status?.state === "ready") {
    unityCloseAfterSuccess.value = true;
    return;
  }
  if (!status || status.state !== "ready") {
    unityCloseAfterSuccess.value = false;
  }
}

async function refreshUnityStatus() {
  const targetPath = unityRequestTargetPath(unityTargetPath.value);
  try {
    const status = await knowledgeGetUnityReferenceImportStatus(targetPath);
    applyUnityStatus(status);
    unityError.value = "";
    if (status.running) {
      scheduleUnityPoll();
      return;
    }
  } catch (cause) {
    unityError.value = normalizeAppError(cause).message;
  }
}

async function startUnityImport() {
  if (unityStartPending.value || unityCancelPending.value) return;
  if (unityHasForeignBinding.value && unityExistingPath.value) {
    await focusDirectory(unityExistingPath.value, true);
    return;
  }
  unityStartPending.value = true;
  unityError.value = "";
  unityImportSessionStarted.value = true;
  unityCloseAfterSuccess.value = false;
  try {
    const targetPath = unityTargetPath.value;
    unityMaterializedTargetPath.value = targetPath;
    const ready = isUnityManagedTargetPath(targetPath)
      ? true
      : await ensureDirectoryReady(targetPath);
    if (!ready) {
      throw new Error(t("knowledge.referenceFolder.external.targetPath"));
    }
    await focusDirectory(targetPath, true);
    const status = await knowledgeImportUnityReferenceDocs(
      unityRequestTargetPath(targetPath),
      unitySelectedLocale.value,
    );
    applyUnityStatus(status, { startedSession: true });
    if (status.running) scheduleUnityPoll();
  } catch (cause) {
    unityError.value = normalizeAppError(cause).message;
  } finally {
    unityStartPending.value = false;
  }
}

async function cancelUnityImport() {
  if (unityCancelPending.value) return;
  unityCancelPending.value = true;
  unityError.value = "";
  try {
    applyUnityStatus(await knowledgeCancelUnityReferenceImport(
      unityRequestTargetPath(unityTargetPath.value),
    ));
    scheduleUnityPoll(200);
  } catch (cause) {
    unityError.value = normalizeAppError(cause).message;
  } finally {
    unityCancelPending.value = false;
  }
}

async function deleteUnityImport() {
  if (!props.deleteUnityImport || !unityTargetPath.value) return;
  unityError.value = "";
  try {
    await props.deleteUnityImport(unityTargetPath.value);
    unityImportSessionStarted.value = false;
    unityCloseAfterSuccess.value = false;
    await loadUnityExistingDirectory();
    await refreshUnityStatus();
  } catch (cause) {
    unityError.value = normalizeAppError(cause).message;
  }
}

const unityLocaleOptions = computed(() => [
  {
    value: "zh-CN",
    label: t("knowledge.referenceImport.locale.zhCn"),
  },
  {
    value: "en",
    label: t("knowledge.referenceImport.locale.en"),
  },
]);

const unityProgressRatio = computed(() => unityProgressRatioForStatus(unityStatus.value));
const unityProgressLabel = computed(() =>
  unityProgressRatio.value == null ? "—" : formatPercent(unityProgressRatio.value),
);
const unityImportedLocaleLabel = computed(() =>
  localeLabel(unityStatus.value?.importedLocale ?? unityStatus.value?.selectedLocale),
);
const unityCurrentStage = computed(() => unityStageLabel(unityStatus.value?.stage));
const unitySummaryMessage = computed(() =>
  trimOrEmpty(unityError.value)
  || trimOrEmpty(unityStatus.value?.error)
  || trimOrEmpty(unityStatus.value?.message)
  || t("knowledge.referenceImport.window.setupHint"),
);
const unityDisableInputs = computed(() =>
  unityStartPending.value || unityCancelPending.value || !!unityStatus.value?.running,
);
const unityCanDelete = computed(() =>
  !!props.deleteUnityImport && !!fixedTargetPath.value && unityBoundHere.value && !unityStatus.value?.running,
);
const unityWindowPrimaryCloses = computed(() =>
  props.mode === "window"
  && unityCloseAfterSuccess.value
  && !unityStatus.value?.running,
);
const unityWindowPrimaryLabel = computed(() =>
  unityWindowPrimaryCloses.value ? t("common.close") : unityActionLabel.value,
);

// Feishu
const feishuStatus = ref<FeishuReferenceImportStatus | null>(null);
const feishuError = ref("");
const feishuLastMessage = ref("");
const feishuMaterializedTargetPath = ref("");
const feishuAuthMode = ref<FeishuReferenceAuthMode>("app_credentials");
const feishuOauthPersistenceMode = ref<FeishuReferenceOauthPersistenceMode>("session");
const feishuAppId = ref("");
const feishuAppSecret = ref("");
const feishuAppSecretTouched = ref(false);
const feishuOpenBaseUrl = ref(DEFAULT_FEISHU_OPEN_BASE_URL);
const feishuSelectedSpaceId = ref("");
const feishuSelectedSpaceName = ref("");
const feishuSelectedRoots = ref<FeishuReferenceRootSelection[]>([]);
const feishuSpaceOptions = ref<SpaceOption[]>([]);
const feishuTreeNodes = ref<FeishuTreeNode[]>([]);
const feishuExpandedTokens = ref<Set<string>>(new Set());
const feishuNodeLoading = ref(false);
const feishuNodeError = ref("");
const feishuConnectionVerified = ref(false);
const feishuOauthRequestedInSession = ref(false);
const feishuSavePending = ref(false);
const feishuTestPending = ref(false);
const feishuAuthorizePending = ref(false);
const feishuCancelAuthorizationPending = ref(false);
const feishuImportPending = ref(false);
const feishuCancelImportPending = ref(false);
let feishuPollTimer: ReturnType<typeof setTimeout> | null = null;

function clearFeishuPollTimer() {
  if (!feishuPollTimer) return;
  clearTimeout(feishuPollTimer);
  feishuPollTimer = null;
}

function scheduleFeishuPoll(delay = 800) {
  clearFeishuPollTimer();
  feishuPollTimer = setTimeout(() => {
    feishuPollTimer = null;
    void refreshFeishuStatus();
  }, delay);
}

function upsertFeishuSpaceOptions(items: SpaceOption[]) {
  const merged = new Map<string, SpaceOption>();
  for (const item of feishuSpaceOptions.value) {
    if (trimOrEmpty(item.spaceId)) {
      merged.set(item.spaceId, item);
    }
  }
  for (const item of items) {
    if (!trimOrEmpty(item.spaceId)) continue;
    merged.set(item.spaceId, {
      spaceId: item.spaceId,
      name: trimOrEmpty(item.name) || item.spaceId,
    });
  }
  feishuSpaceOptions.value = Array.from(merged.values()).sort((left, right) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
  );
}

function applyFeishuStatusToForm(status: FeishuReferenceImportStatus) {
  feishuAuthMode.value = status.authMode;
  feishuOauthPersistenceMode.value = status.oauthPersistenceMode;
  feishuAppId.value = trimOrEmpty(status.appId);
  if (typeof status.appSecret === "string") {
    feishuAppSecret.value = trimOrEmpty(status.appSecret);
  } else if (!status.appSecretConfigured) {
    feishuAppSecret.value = "";
  }
  feishuOpenBaseUrl.value = trimOrEmpty(status.openBaseUrl) || DEFAULT_FEISHU_OPEN_BASE_URL;
  feishuSelectedSpaceId.value = trimOrEmpty(status.spaceId);
  feishuSelectedSpaceName.value = trimOrEmpty(status.spaceName);
  feishuAppSecretTouched.value = false;
  feishuSelectedRoots.value = normalizeSingleRootSelection(
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
  upsertFeishuSpaceOptions([
    ...(trimOrEmpty(status.spaceId)
      ? [{
          spaceId: trimOrEmpty(status.spaceId),
          name: trimOrEmpty(status.spaceName) || trimOrEmpty(status.spaceId),
        }]
      : []),
    ...(trimOrEmpty(status.importedSpaceId)
      ? [{
          spaceId: trimOrEmpty(status.importedSpaceId),
          name: trimOrEmpty(status.importedSpaceName) || trimOrEmpty(status.importedSpaceId),
        }]
      : []),
  ]);
  if (
    feishuAuthMode.value === "oauth"
    && feishuOauthRequestedInSession.value
    && status.authMode === "oauth"
    && !!status.authorized
  ) {
    feishuOauthRequestedInSession.value = false;
    notificationStore.addNotice(
      "success",
      t("knowledge.feishuReference.window.authorizationSucceeded"),
      {
        operation: "feishuReferenceAuthorizationSuccess",
        replaceOperation: true,
      },
    );
  }
}

function resetFeishuTree() {
  feishuTreeNodes.value = [];
  feishuExpandedTokens.value = new Set();
}

function createFeishuTreeNode(
  summary: FeishuReferenceNodeSummary,
  parentPath: string,
  depth: number,
): FeishuTreeNode {
  const title = trimOrEmpty(summary.title) || summary.nodeToken;
  return {
    key: `${summary.nodeToken}:${depth}`,
    summary,
    depth,
    pathLabel: parentPath ? `${parentPath}/${title}` : title,
    children: [],
    childrenLoaded: false,
    childrenLoading: false,
  };
}

function findFeishuTreeNodeByToken(
  token: string,
  nodes: FeishuTreeNode[] = feishuTreeNodes.value,
): FeishuTreeNode | null {
  for (const node of nodes) {
    if (node.summary.nodeToken === token) return node;
    const child = findFeishuTreeNodeByToken(token, node.children);
    if (child) return child;
  }
  return null;
}

function resolveFeishuRootTitle(root: FeishuReferenceRootSelection): string {
  const token = trimOrEmpty(root.nodeToken);
  if (!token) return "";
  const treeNode = findFeishuTreeNodeByToken(token);
  return trimOrEmpty(treeNode?.pathLabel) || trimOrEmpty(root.nodeTitle) || token;
}

function buildFeishuSelectedRootsPayload(): FeishuReferenceRootSelection[] {
  return normalizeSingleRootSelection(
    feishuSelectedRoots.value.map((root) => ({
      nodeToken: trimOrEmpty(root.nodeToken),
      nodeTitle: resolveFeishuRootTitle(root) || root.nodeTitle || null,
    })),
  );
}

async function fetchFeishuNodeEntries(parentNodeToken?: string | null) {
  const spaceId = trimOrEmpty(feishuSelectedSpaceId.value);
  if (!spaceId) return [];
  return knowledgeListFeishuReferenceSpaceNodes(spaceId, parentNodeToken ?? null);
}

async function loadFeishuRootNodes() {
  const spaceId = trimOrEmpty(feishuSelectedSpaceId.value);
  if (!spaceId) {
    resetFeishuTree();
    return;
  }
  feishuNodeLoading.value = true;
  feishuNodeError.value = "";
  try {
    const entries = await fetchFeishuNodeEntries(null);
    feishuTreeNodes.value = entries.map((entry) => createFeishuTreeNode(entry, "", 0));
    feishuExpandedTokens.value = new Set();
  } catch (cause) {
    feishuNodeError.value = normalizeAppError(cause).message;
  } finally {
    feishuNodeLoading.value = false;
  }
}

async function ensureFeishuNodeChildren(node: FeishuTreeNode) {
  if (!node.summary.hasChild || node.childrenLoaded || node.childrenLoading) return;
  node.childrenLoading = true;
  try {
    const entries = await fetchFeishuNodeEntries(node.summary.nodeToken);
    node.children = entries.map((entry) =>
      createFeishuTreeNode(entry, node.pathLabel, node.depth + 1),
    );
    node.childrenLoaded = true;
  } finally {
    node.childrenLoading = false;
  }
}

async function toggleFeishuNode(row: FeishuTreeRow) {
  if (!row.canExpand) return;
  const next = new Set(feishuExpandedTokens.value);
  if (next.has(row.node.summary.nodeToken)) {
    next.delete(row.node.summary.nodeToken);
    feishuExpandedTokens.value = next;
    return;
  }
  try {
    await ensureFeishuNodeChildren(row.node);
    next.add(row.node.summary.nodeToken);
    feishuExpandedTokens.value = next;
  } catch (cause) {
    feishuNodeError.value = normalizeAppError(cause).message;
  }
}

function toggleFeishuRootSelection(node: FeishuTreeNode) {
  const token = trimOrEmpty(node.summary.nodeToken);
  if (!token) return;
  feishuSelectedRoots.value = [{
    nodeToken: token,
    nodeTitle: node.pathLabel || node.summary.title || token,
  }];
}

const feishuVisibleRows = computed<FeishuTreeRow[]>(() => {
  const rows: FeishuTreeRow[] = [];
  const walk = (nodes: FeishuTreeNode[]) => {
    for (const node of nodes) {
      const expanded = feishuExpandedTokens.value.has(node.summary.nodeToken);
      rows.push({
        key: node.key,
        node,
        expanded,
        canExpand: !!node.summary.hasChild,
      });
      if (expanded) {
        walk(node.children);
      }
    }
  };
  walk(feishuTreeNodes.value);
  return rows;
});

const feishuSelectedRootTokenSet = computed(() =>
  new Set(feishuSelectedRoots.value.map((root) => trimOrEmpty(root.nodeToken)).filter(Boolean)),
);

function feishuScopeLabelForRoots(
  spaceName: string,
  roots: FeishuReferenceRootSelection[] | null | undefined,
  fallbackRootToken?: string | null,
  fallbackRootTitle?: string | null,
): string {
  const normalizedRoots = normalizeRootSelections(
    roots?.length
      ? roots
      : trimOrEmpty(fallbackRootToken)
        ? [
            {
              nodeToken: trimOrEmpty(fallbackRootToken),
              nodeTitle: trimOrEmpty(fallbackRootTitle) || null,
            },
          ]
        : [],
  );
  const prefix = trimOrEmpty(spaceName);
  if (!normalizedRoots.length) {
    return prefix
      ? `${prefix} / ${t("knowledge.feishuReference.window.spaceRoot")}`
      : t("knowledge.feishuReference.window.spaceRoot");
  }
  if (normalizedRoots.length === 1) {
    const label = resolveFeishuRootTitle(normalizedRoots[0])
      || trimOrEmpty(normalizedRoots[0].nodeTitle)
      || trimOrEmpty(normalizedRoots[0].nodeToken);
    return prefix ? `${prefix} / ${label}` : label;
  }
  const countLabel = t("knowledge.feishuReference.window.selectedRootCount", normalizedRoots.length);
  return prefix ? `${prefix} / ${countLabel}` : countLabel;
}

const feishuBaseName = computed(() => {
  const roots = buildFeishuSelectedRootsPayload();
  const spaceName = trimOrEmpty(feishuSelectedSpaceName.value);
  if (roots.length === 1) {
    return sanitizePathSegment(resolveFeishuRootTitle(roots[0]), DEFAULT_FEISHU_BASE_DIR);
  }
  if (roots.length > 1) {
    const prefix = sanitizePathSegment(spaceName || DEFAULT_FEISHU_BASE_DIR, DEFAULT_FEISHU_BASE_DIR);
    return sanitizePathSegment(`${prefix}-${roots.length}-folders`, `${DEFAULT_FEISHU_BASE_DIR}-${roots.length}`);
  }
  if (spaceName) {
    return sanitizePathSegment(spaceName, DEFAULT_FEISHU_BASE_DIR);
  }
  return "";
});

const feishuComputedTargetPath = computed(() => {
  const baseName = feishuBaseName.value;
  if (!baseName) return "";
  const basePath = joinRelativePath(normalizedParentDir.value, baseName);
  return resolveStableExternalImportTargetPath({
    fixedTargetPath: fixedTargetPath.value,
    materializedTargetPath: feishuMaterializedTargetPath.value,
    basePath,
    pathExists: props.pathExists ?? localPathExists,
  });
});

const feishuTargetPath = computed(() =>
  fixedTargetPath.value || feishuComputedTargetPath.value,
);
const feishuTargetPathLabel = computed(() =>
  feishuTargetPath.value
    ? referencePathLabel(feishuTargetPath.value)
    : t("knowledge.referenceFolder.external.targetPending"),
);
const feishuCurrentStatusTargetPath = computed(() =>
  fixedTargetPath.value || feishuMaterializedTargetPath.value || "",
);
const feishuSelectedScopeLabel = computed(() =>
  feishuScopeLabelForRoots(
    feishuSelectedSpaceName.value,
    feishuSelectedRoots.value,
    null,
    null,
  ),
);
const feishuImportedScopeLabel = computed(() =>
  feishuScopeLabelForRoots(
    trimOrEmpty(feishuStatus.value?.importedSpaceName) || trimOrEmpty(feishuStatus.value?.spaceName),
    feishuStatus.value?.importedRoots,
    feishuStatus.value?.importedRootNodeToken,
    feishuStatus.value?.importedRootNodeTitle,
  ),
);
const feishuSpaceDropdownOptions = computed(() =>
  feishuSpaceOptions.value.map((item) => ({
    value: item.spaceId,
    label: item.name,
  })),
);
const feishuProgressRatio = computed(() => feishuProgressRatioForStatus(feishuStatus.value));
const feishuProgressLabel = computed(() =>
  feishuProgressRatio.value == null ? "—" : formatPercent(feishuProgressRatio.value),
);
const feishuWaitingForAuthorization = computed(() =>
  feishuStatus.value?.stage === "authorizing" && !feishuStatus.value?.authorized,
);
const feishuHasConfiguredSecret = computed(() =>
  !!trimOrEmpty(feishuAppSecret.value) || !!feishuStatus.value?.appSecretConfigured,
);
const feishuOauthAuthorized = computed(() =>
  feishuAuthMode.value === "oauth"
  && feishuStatus.value?.authMode === "oauth"
  && !!feishuStatus.value?.authorized,
);
const feishuOauthAuthorizedForCurrentConfig = computed(() => {
  if (!feishuOauthAuthorized.value || feishuAppSecretTouched.value) return false;
  const openBaseUrl = trimOrEmpty(feishuOpenBaseUrl.value) || DEFAULT_FEISHU_OPEN_BASE_URL;
  const statusOpenBaseUrl = trimOrEmpty(feishuStatus.value?.openBaseUrl) || DEFAULT_FEISHU_OPEN_BASE_URL;
  return trimOrEmpty(feishuAppId.value) === trimOrEmpty(feishuStatus.value?.appId)
    && openBaseUrl === statusOpenBaseUrl
    && feishuOauthPersistenceMode.value === feishuStatus.value?.oauthPersistenceMode;
});
const feishuShowTestConnection = computed(() =>
  feishuAuthMode.value !== "oauth" || feishuOauthAuthorizedForCurrentConfig.value,
);
const feishuDisableInputs = computed(() =>
  !!feishuStatus.value?.running
  || feishuWaitingForAuthorization.value
  || feishuSavePending.value
  || feishuTestPending.value
  || feishuAuthorizePending.value
  || feishuImportPending.value
  || feishuCancelImportPending.value
  || feishuCancelAuthorizationPending.value,
);
const feishuCanTestConnection = computed(() =>
  feishuShowTestConnection.value
  && !feishuDisableInputs.value
  && !!trimOrEmpty(feishuAppId.value)
  && feishuHasConfiguredSecret.value,
);
const feishuCanAuthorize = computed(() =>
  feishuAuthMode.value === "oauth"
  && !feishuDisableInputs.value
  && !feishuStatus.value?.running
  && !!trimOrEmpty(feishuAppId.value)
  && feishuHasConfiguredSecret.value,
);
const feishuCanContinueConnectionStep = computed(() =>
  feishuConnectionVerified.value
  && (feishuAuthMode.value !== "oauth" || feishuOauthAuthorizedForCurrentConfig.value),
);
const feishuCanDelete = computed(() =>
  !!props.deleteFeishuImport
  && !!fixedTargetPath.value
  && boundProviders.value.includes("feishu")
  && !feishuStatus.value?.running,
);
const feishuActionLabel = computed(() => {
  if (feishuStatus.value?.running) return t("knowledge.referenceImport.action.running");
  if (boundProviders.value.includes("feishu")) {
    return t("knowledge.referenceImport.action.reimport");
  }
  return t("knowledge.feishuReference.window.startImport");
});

function buildFeishuConfigInput(targetPath?: string | null): FeishuReferenceConfigInput {
  const roots = buildFeishuSelectedRootsPayload();
  const normalizedSecret = trimOrEmpty(feishuAppSecret.value);
  return {
    targetPath: trimOrEmpty(targetPath) || null,
    authMode: feishuAuthMode.value,
    oauthPersistenceMode: feishuOauthPersistenceMode.value,
    appId: trimOrEmpty(feishuAppId.value),
    appSecret: feishuAppSecretTouched.value && normalizedSecret ? normalizedSecret : null,
    clearAppSecret: feishuAppSecretTouched.value && !normalizedSecret,
    openBaseUrl: trimOrEmpty(feishuOpenBaseUrl.value) || DEFAULT_FEISHU_OPEN_BASE_URL,
    spaceId: trimOrEmpty(feishuSelectedSpaceId.value) || null,
    spaceName: trimOrEmpty(feishuSelectedSpaceName.value) || null,
    roots,
    rootNodeToken: roots[0]?.nodeToken ?? null,
    rootNodeTitle: roots[0]?.nodeTitle ?? null,
  };
}

function onFeishuAppSecretInput(event: Event) {
  const target = event.target as HTMLInputElement | null;
  feishuAppSecret.value = target?.value ?? "";
  feishuAppSecretTouched.value = true;
}

function buildFeishuImportRequest(targetPath?: string | null): FeishuReferenceImportRequest {
  const roots = buildFeishuSelectedRootsPayload();
  return {
    targetPath: trimOrEmpty(targetPath) || null,
    spaceId: trimOrEmpty(feishuSelectedSpaceId.value),
    spaceName: trimOrEmpty(feishuSelectedSpaceName.value) || null,
    roots,
    rootNodeToken: roots[0]?.nodeToken ?? null,
    rootNodeTitle: roots[0]?.nodeTitle ?? null,
  };
}

async function refreshFeishuStatus() {
  const targetPath = trimOrEmpty(feishuCurrentStatusTargetPath.value) || undefined;
  try {
    const status = await knowledgeGetFeishuReferenceImportStatus(targetPath);
    feishuStatus.value = status;
    feishuError.value = "";
    applyFeishuStatusToForm(status);
    if (trimOrEmpty(status.message)) {
      feishuLastMessage.value = status.message.trim();
    }
    if (status.running || status.stage === "authorizing") {
      scheduleFeishuPoll(status.stage === "authorizing" ? 600 : 900);
      return;
    }
  } catch (cause) {
    feishuError.value = normalizeAppError(cause).message;
  }
}

async function saveFeishuConfig(targetPath?: string | null) {
  if (feishuSavePending.value) return null;
  feishuSavePending.value = true;
  feishuError.value = "";
  try {
    const status = await knowledgeSaveFeishuReferenceConfig(buildFeishuConfigInput(targetPath));
    feishuStatus.value = status;
    applyFeishuStatusToForm(status);
    if (status.running || status.stage === "authorizing") {
      scheduleFeishuPoll(status.stage === "authorizing" ? 600 : 900);
    }
    return status;
  } catch (cause) {
    feishuError.value = normalizeAppError(cause).message;
    return null;
  } finally {
    feishuSavePending.value = false;
  }
}

async function testFeishuConnection() {
  if (feishuTestPending.value) return;
  feishuTestPending.value = true;
  feishuError.value = "";
  try {
    const targetPath = fixedTargetPath.value || null;
    const saved = await saveFeishuConfig(targetPath);
    if (!saved) return;
    const result = await knowledgeTestFeishuReferenceConnection(targetPath || undefined);
    feishuLastMessage.value = result.summary;
    upsertFeishuSpaceOptions(
      result.spaces.map((item) => ({
        spaceId: item.spaceId,
        name: trimOrEmpty(item.name) || item.spaceId,
      })),
    );
    if (trimOrEmpty(result.resolvedSpaceId)) {
      feishuSelectedSpaceId.value = trimOrEmpty(result.resolvedSpaceId);
      feishuSelectedSpaceName.value = trimOrEmpty(result.resolvedSpaceName)
        || feishuSpaceOptions.value.find((item) => item.spaceId === feishuSelectedSpaceId.value)?.name
        || feishuSelectedSpaceId.value;
    } else if (!trimOrEmpty(feishuSelectedSpaceId.value) && result.spaces.length === 1) {
      feishuSelectedSpaceId.value = result.spaces[0].spaceId;
      feishuSelectedSpaceName.value = trimOrEmpty(result.spaces[0].name) || result.spaces[0].spaceId;
    }
    if (trimOrEmpty(feishuSelectedSpaceId.value)) {
      await loadFeishuRootNodes();
    }
    feishuConnectionVerified.value = true;
    notificationStore.addNotice(
      "success",
      t("knowledge.feishuReference.window.connectionSucceeded"),
      {
        operation: "feishuReferenceConnectionSuccess",
        replaceOperation: true,
      },
    );
    await refreshFeishuStatus();
  } catch (cause) {
    feishuConnectionVerified.value = false;
    feishuError.value = normalizeAppError(cause).message;
  } finally {
    feishuTestPending.value = false;
  }
}

async function startFeishuAuthorization() {
  if (feishuAuthorizePending.value) return;
  feishuAuthorizePending.value = true;
  feishuError.value = "";
  feishuConnectionVerified.value = false;
  feishuOauthRequestedInSession.value = true;
  try {
    const targetPath = fixedTargetPath.value || null;
    const saved = await saveFeishuConfig(targetPath);
    if (!saved) return;
    const result = await knowledgeStartFeishuReferenceOauth();
    await openUrl(result.authorizeUrl);
    feishuLastMessage.value = t("knowledge.feishuReference.window.authorizationStarted", result.callbackUrl);
    await refreshFeishuStatus();
  } catch (cause) {
    feishuOauthRequestedInSession.value = false;
    feishuError.value = normalizeAppError(cause).message;
  } finally {
    feishuAuthorizePending.value = false;
  }
}

async function cancelFeishuAuthorization() {
  if (feishuCancelAuthorizationPending.value) return;
  feishuCancelAuthorizationPending.value = true;
  feishuError.value = "";
  feishuConnectionVerified.value = false;
  feishuOauthRequestedInSession.value = false;
  try {
    feishuStatus.value = await knowledgeCancelFeishuReferenceOauthWait(
      (trimOrEmpty(feishuCurrentStatusTargetPath.value) || undefined),
    );
    scheduleFeishuPoll(200);
  } catch (cause) {
    feishuError.value = normalizeAppError(cause).message;
  } finally {
    feishuCancelAuthorizationPending.value = false;
  }
}

async function startFeishuImport() {
  if (feishuImportPending.value) return;
  feishuImportPending.value = true;
  feishuError.value = "";
  try {
    const targetPath = feishuTargetPath.value;
    if (!trimOrEmpty(feishuSelectedSpaceId.value)) {
      throw new Error(t("knowledge.feishuReference.window.selectSpaceFirst"));
    }
    if (!targetPath) {
      throw new Error(t("knowledge.referenceFolder.external.targetPending"));
    }
    const ready = await ensureDirectoryReady(targetPath);
    if (!ready) {
      throw new Error(t("knowledge.referenceFolder.external.targetPending"));
    }
    feishuMaterializedTargetPath.value = targetPath;
    await focusDirectory(targetPath, true);
    const saved = await saveFeishuConfig(targetPath);
    if (!saved) return;
    const status = await knowledgeImportFeishuReferenceDocs(buildFeishuImportRequest(targetPath));
    feishuStatus.value = status;
    if (status.running) scheduleFeishuPoll();
  } catch (cause) {
    feishuError.value = normalizeAppError(cause).message;
  } finally {
    feishuImportPending.value = false;
  }
}

async function cancelFeishuImport() {
  if (feishuCancelImportPending.value) return;
  feishuCancelImportPending.value = true;
  feishuError.value = "";
  try {
    feishuStatus.value = await knowledgeCancelFeishuReferenceImport(
      trimOrEmpty(feishuCurrentStatusTargetPath.value) || undefined,
    );
    scheduleFeishuPoll(200);
  } catch (cause) {
    feishuError.value = normalizeAppError(cause).message;
  } finally {
    feishuCancelImportPending.value = false;
  }
}

async function deleteFeishuImport() {
  if (!props.deleteFeishuImport || !fixedTargetPath.value) return;
  feishuError.value = "";
  try {
    await props.deleteFeishuImport(fixedTargetPath.value);
    feishuMaterializedTargetPath.value = "";
    await refreshFeishuStatus();
  } catch (cause) {
    feishuError.value = normalizeAppError(cause).message;
  }
}

const headerPathLabel = computed(() =>
  props.mode === "directory"
    ? referencePathLabel(fixedTargetPath.value)
    : referencePathLabel(normalizedParentDir.value),
);
const showPanelHeader = computed(() => props.mode !== "window");
const panelHint = computed(() =>
  props.mode === "directory"
    ? t("knowledge.referenceFolder.external.hint")
    : t("knowledge.referenceFolder.external.dialogHint"),
);
const isRunning = computed(() =>
  !!unityStatus.value?.running
  || !!feishuStatus.value?.running
  || feishuWaitingForAuthorization.value,
);
const isWindowBusy = computed(() =>
  isRunning.value
  || unityStartPending.value
  || unityCancelPending.value
  || feishuSavePending.value
  || feishuImportPending.value
  || feishuCancelImportPending.value,
);
const canClose = computed(() => !unityStartPending.value && !feishuSavePending.value);
const disableSourceSwitch = computed(() => isRunning.value);
const inlineSourceOptions = computed(() => [
  {
    value: "feishu",
    label: t("knowledge.referenceFolder.external.sourceFeishu"),
    disabled: disableSourceSwitch.value,
  },
  {
    value: "unity",
    label: t("knowledge.referenceFolder.external.sourceUnity"),
    disabled: disableSourceSwitch.value,
  },
]);
const windowSourceOptions = computed(() => [
  {
    value: "feishu",
    label: t("knowledge.feishuReference.title"),
    disabled: disableSourceSwitch.value,
  },
  {
    value: "unity",
    label: t("knowledge.referenceImport.title"),
    disabled: disableSourceSwitch.value,
  },
]);
const currentUnityVersionLabel = computed(() =>
  trimOrEmpty(unityStatus.value?.projectVersion) || "—",
);
const currentUnityDocsVersionLabel = computed(() =>
  trimOrEmpty(unityStatus.value?.docsVersion) || "—",
);
const unityImportedAtLabel = computed(() => formatDateTime(unityStatus.value?.importedAt));
const feishuImportedAtLabel = computed(() => formatDateTime(feishuStatus.value?.importedAt));
const feishuSummaryMessage = computed(() =>
  trimOrEmpty(feishuError.value)
  || trimOrEmpty(feishuStatus.value?.error)
  || trimOrEmpty(feishuLastMessage.value)
  || trimOrEmpty(feishuStatus.value?.message)
  || t("knowledge.feishuReference.window.subtitle"),
);
const windowTargetPathLabel = computed(() =>
  activeSource.value === "unity" ? unityTargetPathLabel.value : feishuTargetPathLabel.value,
);
const windowTargetPathHint = computed(() => {
  if (activeSource.value === "unity" && unityExistingPath.value && unityExistingPath.value === unityTargetPath.value) {
    return t("knowledge.referenceFolder.external.unityReuseHint", referencePathLabel(unityExistingPath.value));
  }
  return "";
});
const unityTransferredLabel = computed(() =>
  `${formatBytes(unityStatus.value?.downloadedBytes)} / ${formatBytes(unityStatus.value?.totalBytes)}`,
);
const unityProcessedLabel = computed(() =>
  unityStatus.value?.totalDocs == null
    ? `${unityStatus.value?.processedDocs ?? 0}`
    : `${unityStatus.value?.processedDocs ?? 0} / ${unityStatus.value?.totalDocs ?? 0}`,
);
const unityWindowStageItems = computed(() => {
  const currentStage = unityStatus.value?.stage;
  const currentIndex = UNITY_IMPORT_STAGE_ORDER.indexOf(
    currentStage as typeof UNITY_IMPORT_STAGE_ORDER[number],
  );
  const activeProgress = unityProgressRatio.value ?? 0;
  return UNITY_IMPORT_STAGE_ORDER.map((stage, index) => {
    const complete = currentStage === "ready"
      || (currentIndex >= 0 && currentStage !== "error" && index < currentIndex);
    const current = currentStage === stage;
    return {
      key: stage,
      label: unityStageLabel(stage),
      complete,
      current,
      error: currentStage === "error" && current,
      progress: complete ? 1 : current ? activeProgress : 0,
      statusText: current
        ? unityProgressLabel.value
        : complete
          ? t("knowledge.referenceImport.stage.ready")
          : "—",
    };
  });
});
const feishuCurrentItemLabel = computed(() =>
  trimOrEmpty(feishuStatus.value?.currentTitle)
  || trimOrEmpty(feishuStatus.value?.currentPath)
  || "—",
);
const feishuWindowTreeRows = computed<ReferenceExternalImportFeishuTreeRowModel[]>(() =>
  feishuVisibleRows.value.map((row) => ({
    key: row.key,
    depth: row.node.depth,
    canExpand: row.canExpand,
    expanded: row.expanded,
    title: row.node.summary.title || row.node.summary.nodeToken,
    pathLabel: row.node.pathLabel,
    selected: feishuSelectedRootTokenSet.value.has(row.node.summary.nodeToken),
    disabled: feishuDisableInputs.value,
  })),
);

function findFeishuVisibleRowByKey(key: string): FeishuTreeRow | null {
  return feishuVisibleRows.value.find((row) => row.key === key) ?? null;
}

async function toggleFeishuWindowRow(key: string) {
  const row = findFeishuVisibleRowByKey(key);
  if (!row) return;
  await toggleFeishuNode(row);
}

function toggleFeishuWindowSelection(key: string) {
  const row = findFeishuVisibleRowByKey(key);
  if (!row) return;
  toggleFeishuRootSelection(row.node);
}

const unityWindowModel = computed<ReferenceExternalImportUnityWindowModel>(() => ({
  summary: t("knowledge.referenceImport.subtitle"),
  locale: unitySelectedLocale.value,
  localeOptions: unityLocaleOptions.value,
  localeDisabled: unityDisableInputs.value || unityHasForeignBinding.value || disableSourceSwitch.value,
  foreignBindingText: unityHasForeignBinding.value
    ? t("knowledge.referenceFolder.external.unityExistingConflict", referencePathLabel(unityExistingPath.value))
    : "",
  canOpenExisting: !!props.selectDirectory && !!unityExistingPath.value,
  stageTitle: unityCurrentStage.value,
  stageCaption: t("knowledge.referenceImport.window.stageProgress"),
  progressLabel: unityProgressLabel.value,
  progressRatio: unityProgressRatio.value ?? 0,
  stageItems: unityWindowStageItems.value,
  rows: [
    { label: t("knowledge.referenceImport.managedPath"), value: unityTargetPathLabel.value, mono: true },
    { label: t("knowledge.referenceImport.projectVersion"), value: currentUnityVersionLabel.value },
    { label: t("knowledge.referenceImport.docsVersion"), value: currentUnityDocsVersionLabel.value },
    { label: t("knowledge.referenceImport.locale"), value: unityImportedLocaleLabel.value },
    { label: t("knowledge.referenceImport.importedAt"), value: unityImportedAtLabel.value },
    { label: t("knowledge.overview.documentsUnit"), value: `${unityStatus.value?.importedDocCount ?? 0}` },
    { label: t("knowledge.referenceImport.window.transferred"), value: unityTransferredLabel.value },
    { label: t("knowledge.referenceImport.window.processed"), value: unityProcessedLabel.value },
  ],
  detail: unitySummaryMessage.value,
  currentPath: trimOrEmpty(unityStatus.value?.currentPath),
  currentPathLabel: t("knowledge.referenceImport.window.currentPath"),
  canDelete: unityCanDelete.value,
  canCancel: !!unityStatus.value?.running,
  cancelDisabled: unityCancelPending.value,
  primaryDisabled: unityDisableInputs.value || disableSourceSwitch.value,
  primaryClosesWindow: unityWindowPrimaryCloses.value,
  primaryLabel: unityWindowPrimaryLabel.value,
  cancelLabel: unityCancelPending.value ? t("knowledge.referenceImport.window.cancelling") : t("common.cancel"),
  deleteLabel: t("common.delete"),
  openExistingLabel: t("knowledge.referenceFolder.external.openExistingUnity"),
}));

const feishuWindowModel = computed<ReferenceExternalImportFeishuWindowModel>(() => ({
  summary: t("knowledge.feishuReference.window.subtitle"),
  steps: [
    { key: "connection", label: t("knowledge.feishuReference.window.connectionTitle") },
    { key: "scope", label: t("knowledge.feishuReference.window.scopeTitle") },
    { key: "import", label: t("knowledge.feishuReference.window.importTitle") },
  ],
  authMode: feishuAuthMode.value,
  authModeOptions: [
    { value: "app_credentials", label: t("knowledge.feishuReference.auth.appCredentials"), disabled: feishuDisableInputs.value || disableSourceSwitch.value },
    { value: "oauth", label: t("knowledge.feishuReference.auth.oauth"), disabled: feishuDisableInputs.value || disableSourceSwitch.value },
  ],
  authDisabled: feishuDisableInputs.value,
  appId: feishuAppId.value,
  appIdPlaceholder: t("knowledge.feishuReference.window.appIdPlaceholder"),
  appSecret: feishuAppSecret.value,
  appSecretPlaceholder: t("knowledge.feishuReference.window.appSecretPlaceholder"),
  openBaseUrl: feishuOpenBaseUrl.value,
  persistenceMode: feishuOauthPersistenceMode.value,
  persistenceModeOptions: [
    { value: "session", label: t("knowledge.feishuReference.window.persistenceSession"), disabled: feishuDisableInputs.value },
    { value: "offline", label: t("knowledge.feishuReference.window.persistenceOffline"), disabled: feishuDisableInputs.value },
  ],
  showOauthSettings: feishuAuthMode.value === "oauth",
  persistenceHint: feishuOauthPersistenceMode.value === "offline"
    ? t("knowledge.feishuReference.window.persistenceOfflineHint")
    : t("knowledge.feishuReference.window.persistenceSessionHint"),
  callbackUrls: feishuStatus.value?.callbackUrls ?? [],
  oauthAdminHint: t("knowledge.feishuReference.window.oauthAdminHint"),
  oauthRedirectHint: t("knowledge.feishuReference.window.oauthRedirectHint"),
  showTest: feishuShowTestConnection.value,
  canTest: feishuCanTestConnection.value,
  testLabel: feishuTestPending.value ? t("knowledge.feishuReference.window.testing") : t("knowledge.feishuReference.window.testConnection"),
  authorized: feishuOauthAuthorizedForCurrentConfig.value,
  showAuthorize: feishuAuthMode.value === "oauth" && !feishuWaitingForAuthorization.value,
  canAuthorize: feishuCanAuthorize.value,
  authorizeLabel: feishuAuthorizePending.value ? t("knowledge.feishuReference.window.authorizing") : t("knowledge.feishuReference.window.authorize"),
  canContinueConnection: feishuCanContinueConnectionStep.value,
  missingScopesHint: feishuStatus.value?.missingScopes?.length
    ? t("knowledge.feishuReference.window.missingScopesHint", feishuStatus.value.missingScopes.join(", "))
    : "",
  spaceId: feishuSelectedSpaceId.value,
  spaceOptions: feishuSpaceDropdownOptions.value,
  spacePlaceholder: t("knowledge.feishuReference.window.selectSpacePlaceholder"),
  selectedScopeLabel: feishuSelectedScopeLabel.value,
  selectedScopeHint: t("knowledge.feishuReference.window.selectedSpaceValue", feishuSelectedScopeLabel.value),
  canUseSpaceRoot: !feishuDisableInputs.value && !!feishuSelectedSpaceId.value,
  useSpaceRootLabel: t("knowledge.feishuReference.window.useSpaceRoot"),
  nodeLoading: feishuNodeLoading.value,
  nodeError: feishuNodeError.value,
  treeEmptyText: !feishuSelectedSpaceId.value
    ? t("knowledge.feishuReference.window.selectSpaceFirst")
    : t("knowledge.feishuReference.window.emptyNodes"),
  treeRows: feishuWindowTreeRows.value,
  stageTitle: feishuStageLabel(feishuStatus.value?.stage),
  progressLabel: feishuProgressLabel.value,
  progressRatio: feishuProgressRatio.value ?? 0,
  detail: feishuSummaryMessage.value,
  rows: [
    { label: t("knowledge.feishuReference.window.state"), value: feishuStateLabel(feishuStatus.value?.state) },
    { label: t("knowledge.dashboard.knowledge.rebuildStage"), value: feishuStageLabel(feishuStatus.value?.stage) },
    { label: t("knowledge.feishuReference.window.selectedScope"), value: feishuSelectedScopeLabel.value },
    { label: t("knowledge.feishuReference.window.importedScope"), value: feishuImportedScopeLabel.value },
    { label: t("knowledge.referenceImport.window.processed"), value: `${feishuStatus.value?.processedDocs ?? 0} / ${feishuStatus.value?.totalDocs ?? 0}` },
    { label: t("knowledge.referenceImport.importedCount"), value: `${feishuStatus.value?.importedDocCount ?? 0}` },
    { label: t("knowledge.referenceImport.importedAt"), value: feishuImportedAtLabel.value },
    { label: t("knowledge.referenceImport.managedPath"), value: feishuTargetPathLabel.value, mono: true },
  ],
  currentItem: feishuCurrentItemLabel.value === "—" ? "" : feishuCurrentItemLabel.value,
  currentItemLabel: t("knowledge.feishuReference.window.currentItem"),
  isRunning: !!feishuStatus.value?.running,
  waitingForAuthorization: feishuWaitingForAuthorization.value,
  canDelete: feishuCanDelete.value,
  canCancelAuthorization: feishuWaitingForAuthorization.value,
  cancelAuthorizationDisabled: feishuCancelAuthorizationPending.value,
  cancelAuthorizationLabel: feishuCancelAuthorizationPending.value
    ? t("knowledge.feishuReference.window.cancelAuthorizationPending")
    : t("knowledge.feishuReference.window.cancelAuthorization"),
  canCancelImport: !!feishuStatus.value?.running,
  cancelImportDisabled: feishuCancelImportPending.value,
  cancelImportLabel: feishuCancelImportPending.value ? t("knowledge.referenceImport.window.cancelling") : t("common.cancel"),
  primaryDisabled: feishuDisableInputs.value || disableSourceSwitch.value,
  primaryLabel: feishuActionLabel.value,
  deleteLabel: t("common.delete"),
}));

watch(
  () => activeSource.value,
  (source) => {
    if (source === "unity") {
      void loadUnityExistingDirectory().then(() => refreshUnityStatus());
      return;
    }
    void refreshFeishuStatus();
    if (trimOrEmpty(feishuSelectedSpaceId.value)) {
      void loadFeishuRootNodes();
    }
  },
  { immediate: true },
);

watch(
  () => feishuSelectedSpaceId.value,
  (spaceId, previous) => {
    const normalized = trimOrEmpty(spaceId);
    const prior = trimOrEmpty(previous);
    feishuSelectedSpaceName.value = feishuSpaceOptions.value.find((item) => item.spaceId === normalized)?.name || feishuSelectedSpaceName.value;
    if (normalized === prior) return;
    feishuSelectedRoots.value = [];
    resetFeishuTree();
    if (normalized) {
      void loadFeishuRootNodes();
    }
  },
);

watch(
  () => props.fixedTargetPath,
  () => {
    unityMaterializedTargetPath.value = "";
    unityImportSessionStarted.value = false;
    unityCloseAfterSuccess.value = false;
    feishuMaterializedTargetPath.value = "";
    if (activeSource.value === "unity") {
      void loadUnityExistingDirectory().then(() => refreshUnityStatus());
      return;
    }
    void refreshFeishuStatus();
  },
);

watch(isWindowBusy, (value) => {
  emit("runningChange", value);
}, { immediate: true });

onMounted(() => {
  if (!props.pathExists || !props.ensureDirectory) {
    void refreshKnownReferenceDirectories();
  }
});

onUnmounted(() => {
  clearUnityPollTimer();
  clearFeishuPollTimer();
});
</script>

<template>
  <div class="reference-external-panel" :class="[`mode-${mode}`]">
    <template v-if="mode === 'window'">
      <div class="reference-external-window-tabs">
        <BaseSegmented
          :model-value="activeSource"
          size="md"
          class="reference-external-window-source-tabs"
          :options="windowSourceOptions"
          :aria-label="t('knowledge.referenceFolder.external.source')"
          @update:model-value="setActiveSource"
        />
      </div>

      <div class="reference-external-window-meta">
        <div class="reference-external-window-meta-grid">
          <div class="reference-external-window-meta-field">
            <span class="reference-external-label">{{ t("knowledge.referenceFolder.external.parentFolder") }}</span>
            <span class="reference-external-window-meta-value">{{ headerPathLabel }}</span>
          </div>
          <div class="reference-external-window-meta-field">
            <span class="reference-external-label">{{ t("knowledge.referenceFolder.external.targetPath") }}</span>
            <span class="reference-external-window-meta-value">{{ windowTargetPathLabel }}</span>
          </div>
        </div>
      <div v-if="windowTargetPathHint" class="reference-external-window-meta-hint">
          {{ windowTargetPathHint }}
      </div>
      </div>

      <ReferenceExternalImportUnityWindowPane
        v-if="activeSource === 'unity'"
        :model="unityWindowModel"
        @update:locale="unitySelectedLocale = $event"
        @open-existing="void focusDirectory(unityExistingPath, true)"
        @delete="void deleteUnityImport()"
        @cancel="void cancelUnityImport()"
        @close="emit('close')"
        @start="void startUnityImport()"
      />

      <ReferenceExternalImportFeishuWindowFlow
        v-else
        :model="feishuWindowModel"
        @update:auth-mode="setFeishuAuthMode"
        @update:app-id="setFeishuAppId"
        @update:app-secret="setFeishuAppSecret"
        @update:open-base-url="setFeishuOpenBaseUrl"
        @update:persistence-mode="setFeishuOauthPersistenceMode"
        @test="void testFeishuConnection()"
        @authorize="void startFeishuAuthorization()"
        @update:space-id="setFeishuSelectedSpaceId"
        @use-space-root="feishuSelectedRoots = []"
        @toggle-node="void toggleFeishuWindowRow($event)"
        @toggle-selection="toggleFeishuWindowSelection($event)"
        @delete="void deleteFeishuImport()"
        @cancel-authorization="void cancelFeishuAuthorization()"
        @cancel-import="void cancelFeishuImport()"
        @start-import="void startFeishuImport()"
      />
    </template>

    <template v-else>
    <section class="reference-external-card">
      <div v-if="showPanelHeader" class="reference-external-topbar">
        <div class="reference-external-copy">
          <div class="reference-external-title">
            {{ t("knowledge.referenceFolder.external.createAction") }}
          </div>
          <div class="reference-external-hint">
            {{ panelHint }}
          </div>
        </div>
        <BaseButton
          v-if="mode === 'dialog'"
          size="sm"
          :disabled="!canClose"
          @click="emit('close')"
        >
          {{ isRunning ? t("knowledge.referenceFolder.external.keepInBackground") : t("common.cancel") }}
        </BaseButton>
      </div>

      <div class="reference-external-grid" :class="{ 'with-header': showPanelHeader }">
        <div class="reference-external-field">
          <span class="reference-external-label">{{ t("knowledge.referenceFolder.external.source") }}</span>
          <BaseSegmented
            :model-value="activeSource"
            size="sm"
            :options="inlineSourceOptions"
            :aria-label="t('knowledge.referenceFolder.external.source')"
            @update:model-value="setActiveSource"
          />
        </div>

        <div class="reference-external-field">
          <span class="reference-external-label">
            {{ mode === "directory" ? t("knowledge.referenceFolder.external.currentFolder") : t("knowledge.referenceFolder.external.parentFolder") }}
          </span>
          <span class="reference-external-value">{{ headerPathLabel }}</span>
        </div>

        <div class="reference-external-target-card">
          <span class="reference-external-label">{{ t("knowledge.referenceFolder.external.targetPath") }}</span>
          <span class="reference-external-target-path">
            {{ activeSource === "unity" ? unityTargetPathLabel : feishuTargetPathLabel }}
          </span>
          <span
            v-if="activeSource === 'unity' && unityExistingPath && unityExistingPath === unityTargetPath"
            class="reference-external-meta"
          >
            {{ t("knowledge.referenceFolder.external.unityReuseHint", referencePathLabel(unityExistingPath)) }}
          </span>
        </div>
      </div>
    </section>

    <section v-if="activeSource === 'unity'" class="reference-external-card">
      <div class="reference-section-header">
        <div>
          <div class="reference-section-title">{{ t("knowledge.referenceImport.title") }}</div>
          <div class="reference-section-hint">{{ t("knowledge.referenceImport.subtitle") }}</div>
        </div>
        <div class="reference-section-actions">
          <BaseButton
            v-if="unityCanDelete"
            variant="danger"
            size="sm"
            @click="void deleteUnityImport()"
          >
            {{ t("common.delete") }}
          </BaseButton>
          <BaseButton
            v-if="unityStatus?.running"
            variant="danger"
            size="sm"
            :disabled="unityCancelPending"
            @click="void cancelUnityImport()"
          >
            {{ unityCancelPending ? t("knowledge.referenceImport.window.cancelling") : t("common.cancel") }}
          </BaseButton>
          <BaseButton
            v-else
            variant="primary"
            size="sm"
            :disabled="unityDisableInputs || disableSourceSwitch"
            @click="void startUnityImport()"
          >
            {{ unityActionLabel }}
          </BaseButton>
        </div>
      </div>

      <div v-if="unityHasForeignBinding" class="reference-inline-note">
        {{ t("knowledge.referenceFolder.external.unityExistingConflict", referencePathLabel(unityExistingPath)) }}
        <button
          v-if="selectDirectory && unityExistingPath"
          type="button"
          class="reference-inline-link"
          @click="void focusDirectory(unityExistingPath, true)"
        >
          {{ t("knowledge.referenceFolder.external.openExistingUnity") }}
        </button>
      </div>

      <div class="reference-settings-grid">
        <div class="reference-field-stack">
          <span class="reference-external-label">{{ t("knowledge.referenceImport.window.language") }}</span>
          <BaseDropdown
            v-model="unitySelectedLocale"
            size="md"
            :disabled="unityDisableInputs || unityHasForeignBinding || disableSourceSwitch"
            :options="unityLocaleOptions"
            :aria-label="t('knowledge.referenceImport.window.language')"
          />
        </div>

        <div class="reference-meta-grid">
          <div class="reference-meta-row">
            <span>{{ t("knowledge.referenceImport.projectVersion") }}</span>
            <span>{{ currentUnityVersionLabel }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.referenceImport.docsVersion") }}</span>
            <span>{{ currentUnityDocsVersionLabel }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.referenceImport.locale") }}</span>
            <span>{{ unityImportedLocaleLabel }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.referenceImport.importedAt") }}</span>
            <span>{{ unityImportedAtLabel }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.overview.documentsUnit") }}</span>
            <span>{{ unityStatus?.importedDocCount ?? 0 }}</span>
          </div>
        </div>
      </div>

      <div class="reference-status-card">
        <div class="reference-status-header">
          <span class="reference-status-title">{{ unityCurrentStage }}</span>
          <span class="reference-status-value">{{ unityProgressLabel }}</span>
        </div>
        <div class="reference-progress-track">
          <div class="reference-progress-fill" :style="{ width: `${(unityProgressRatio ?? 0) * 100}%` }" />
        </div>
        <div class="reference-status-message">{{ unitySummaryMessage }}</div>
        <div class="reference-meta-grid compact">
          <div class="reference-meta-row">
            <span>{{ t("knowledge.referenceImport.window.transferred") }}</span>
            <span>{{ `${formatBytes(unityStatus?.downloadedBytes)} / ${formatBytes(unityStatus?.totalBytes)}` }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.referenceImport.window.processed") }}</span>
            <span>
              {{
                unityStatus?.totalDocs == null
                  ? `${unityStatus?.processedDocs ?? 0}`
                  : `${unityStatus?.processedDocs ?? 0} / ${unityStatus?.totalDocs ?? 0}`
              }}
            </span>
          </div>
          <div class="reference-meta-row wide">
            <span>{{ t("knowledge.referenceImport.window.currentPath") }}</span>
            <span class="reference-mono">{{ unityStatus?.currentPath || "—" }}</span>
          </div>
        </div>
      </div>

      <div class="reference-stage-list">
        <div
          v-for="stage in UNITY_IMPORT_STAGE_ORDER"
          :key="stage"
          class="reference-stage-row"
          :class="{
            active: unityStatus?.stage === stage,
            done: unityStatus?.stage === 'ready' || (unityStatus?.stage && UNITY_IMPORT_STAGE_ORDER.indexOf(stage) < UNITY_IMPORT_STAGE_ORDER.indexOf(unityStatus.stage as typeof UNITY_IMPORT_STAGE_ORDER[number])),
          }"
        >
          <span>{{ unityStageLabel(stage) }}</span>
          <span>{{ unityStatus?.stage === stage ? unityProgressLabel : "—" }}</span>
        </div>
      </div>
    </section>

    <section v-else class="reference-external-card">
      <div class="reference-section-header">
        <div>
          <div class="reference-section-title">{{ t("knowledge.feishuReference.title") }}</div>
          <div class="reference-section-hint">{{ t("knowledge.feishuReference.subtitle") }}</div>
        </div>
        <div class="reference-section-actions">
          <BaseButton
            v-if="feishuCanDelete"
            variant="danger"
            size="sm"
            @click="void deleteFeishuImport()"
          >
            {{ t("common.delete") }}
          </BaseButton>
          <BaseButton
            v-if="feishuWaitingForAuthorization"
            variant="danger"
            size="sm"
            :disabled="feishuCancelAuthorizationPending"
            @click="void cancelFeishuAuthorization()"
          >
            {{
              feishuCancelAuthorizationPending
                ? t("knowledge.feishuReference.window.cancelAuthorizationPending")
                : t("knowledge.feishuReference.window.cancelAuthorization")
            }}
          </BaseButton>
          <BaseButton
            v-else-if="feishuStatus?.running"
            variant="danger"
            size="sm"
            :disabled="feishuCancelImportPending"
            @click="void cancelFeishuImport()"
          >
            {{ feishuCancelImportPending ? t("knowledge.referenceImport.window.cancelling") : t("common.cancel") }}
          </BaseButton>
          <BaseButton
            v-else
            variant="primary"
            size="sm"
            :disabled="feishuDisableInputs || disableSourceSwitch"
            @click="void startFeishuImport()"
          >
            {{ feishuActionLabel }}
          </BaseButton>
        </div>
      </div>

      <div class="reference-split-grid">
        <section class="reference-subsection">
          <div class="reference-subsection-title">{{ t("knowledge.feishuReference.window.connectionTitle") }}</div>
          <div class="reference-subsection-hint">{{ t("knowledge.feishuReference.window.connectionHint") }}</div>

          <div class="reference-field-stack">
            <span class="reference-external-label">{{ t("knowledge.feishuReference.window.authMode") }}</span>
            <BaseSegmented
              v-model="feishuAuthMode"
              size="sm"
              :disabled="feishuDisableInputs || disableSourceSwitch"
              :options="[
                { value: 'app_credentials', label: t('knowledge.feishuReference.auth.appCredentials') },
                { value: 'oauth', label: t('knowledge.feishuReference.auth.oauth') },
              ]"
            />
          </div>

          <div class="reference-form-grid">
            <label class="reference-field-stack">
              <span class="reference-external-label">{{ t("knowledge.feishuReference.window.appId") }}</span>
              <input
                v-model="feishuAppId"
                class="reference-input"
                :disabled="feishuDisableInputs"
                :placeholder="t('knowledge.feishuReference.window.appIdPlaceholder')"
              />
            </label>

            <label class="reference-field-stack">
              <span class="reference-external-label">{{ t("knowledge.feishuReference.window.appSecret") }}</span>
              <input
                :value="feishuAppSecret"
                class="reference-input"
                type="password"
                :disabled="feishuDisableInputs"
                :placeholder="t('knowledge.feishuReference.window.appSecretPlaceholder')"
                @input="onFeishuAppSecretInput"
              />
            </label>

            <label class="reference-field-stack reference-field-span">
              <span class="reference-external-label">{{ t("knowledge.feishuReference.window.openBaseUrl") }}</span>
              <input
                v-model="feishuOpenBaseUrl"
                class="reference-input"
                :disabled="feishuDisableInputs"
              />
            </label>
          </div>

          <div v-if="feishuAuthMode === 'oauth'" class="reference-field-stack">
            <span class="reference-external-label">{{ t("knowledge.feishuReference.window.persistenceMode") }}</span>
            <BaseSegmented
              v-model="feishuOauthPersistenceMode"
              size="sm"
              :disabled="feishuDisableInputs"
              :options="[
                { value: 'session', label: t('knowledge.feishuReference.window.persistenceSession') },
                { value: 'offline', label: t('knowledge.feishuReference.window.persistenceOffline') },
              ]"
            />
            <div class="reference-inline-note">
              {{
                feishuOauthPersistenceMode === 'offline'
                  ? t('knowledge.feishuReference.window.persistenceOfflineHint')
                  : t('knowledge.feishuReference.window.persistenceSessionHint')
              }}
            </div>
          </div>

          <div v-if="feishuAuthMode === 'oauth' && feishuStatus?.callbackUrls?.length" class="reference-callback-list">
            <div class="reference-inline-note">
              {{ t("knowledge.feishuReference.window.oauthAdminHint") }}
            </div>
            <div class="reference-callback-title">{{ t("knowledge.feishuReference.window.oauthRedirectHint") }}</div>
            <div
              v-for="callbackUrl in feishuStatus.callbackUrls"
              :key="callbackUrl"
              class="reference-callback-item reference-mono"
            >
              {{ callbackUrl }}
            </div>
          </div>

          <div class="reference-button-row">
            <BaseButton
              v-if="feishuWaitingForAuthorization"
              size="sm"
              :disabled="feishuCancelAuthorizationPending"
              @click="void cancelFeishuAuthorization()"
            >
              {{
                feishuCancelAuthorizationPending
                  ? t("knowledge.feishuReference.window.cancelAuthorizationPending")
                  : t("knowledge.feishuReference.window.cancelAuthorization")
              }}
            </BaseButton>
            <BaseButton
              v-else-if="feishuAuthMode === 'oauth'"
              size="sm"
              :disabled="!feishuCanAuthorize"
              @click="void startFeishuAuthorization()"
            >
              {{ feishuAuthorizePending ? t("knowledge.feishuReference.window.authorizing") : t("knowledge.feishuReference.window.authorize") }}
            </BaseButton>
            <BaseButton
              v-if="feishuShowTestConnection"
              size="sm"
              :disabled="!feishuCanTestConnection"
              @click="void testFeishuConnection()"
            >
              {{ feishuTestPending ? t("knowledge.feishuReference.window.testing") : t("knowledge.feishuReference.window.testConnection") }}
            </BaseButton>
          </div>

          <div v-if="feishuStatus?.missingScopes?.length" class="reference-inline-note">
            {{ t("knowledge.feishuReference.window.missingScopesHint", feishuStatus.missingScopes.join(", ")) }}
          </div>
        </section>

        <section class="reference-subsection">
          <div class="reference-subsection-title">{{ t("knowledge.feishuReference.window.scopeTitle") }}</div>
          <div class="reference-subsection-hint">{{ t("knowledge.feishuReference.window.scopeHint") }}</div>

          <div class="reference-field-stack">
            <span class="reference-external-label">{{ t("knowledge.feishuReference.window.space") }}</span>
            <BaseDropdown
              v-model="feishuSelectedSpaceId"
              size="md"
              :disabled="feishuDisableInputs || !feishuSpaceDropdownOptions.length"
              :options="feishuSpaceDropdownOptions"
              :placeholder="t('knowledge.feishuReference.window.selectSpacePlaceholder')"
              :aria-label="t('knowledge.feishuReference.window.space')"
            />
          </div>

          <div class="reference-tree-header">
            <div class="reference-tree-selection">
              <span class="reference-external-label">{{ t("knowledge.feishuReference.window.selectedRoot") }}</span>
              <span class="reference-tree-selection-value">{{ feishuSelectedScopeLabel }}</span>
            </div>
            <BaseButton
              size="sm"
              :disabled="feishuDisableInputs || !feishuSelectedSpaceId"
              @click="feishuSelectedRoots = []"
            >
              {{ t("knowledge.feishuReference.window.useSpaceRoot") }}
            </BaseButton>
          </div>

          <div class="reference-tree-shell">
            <div v-if="feishuNodeLoading" class="reference-tree-empty">{{ t("common.loading") }}</div>
            <div v-else-if="feishuNodeError" class="reference-tree-empty error">{{ feishuNodeError }}</div>
            <div v-else-if="!feishuSelectedSpaceId" class="reference-tree-empty">
              {{ t("knowledge.feishuReference.window.selectSpaceFirst") }}
            </div>
            <div v-else-if="!feishuVisibleRows.length" class="reference-tree-empty">
              {{ t("knowledge.feishuReference.window.emptyNodes") }}
            </div>
            <div v-else class="reference-tree-list">
              <div
                v-for="row in feishuVisibleRows"
                :key="row.key"
                class="reference-tree-row"
                :class="{
                  selected: feishuSelectedRootTokenSet.has(row.node.summary.nodeToken),
                  disabled: feishuDisableInputs,
                }"
                :style="{ paddingLeft: `${12 + row.node.depth * 16}px` }"
              >
                <button
                  type="button"
                  class="reference-tree-toggle"
                  :disabled="!row.canExpand"
                  @click="void toggleFeishuNode(row)"
                >
                  <span v-if="row.canExpand">{{ row.expanded ? "▾" : "▸" }}</span>
                </button>
                <button
                  type="button"
                  class="reference-tree-node"
                  :aria-pressed="feishuSelectedRootTokenSet.has(row.node.summary.nodeToken)"
                  :disabled="feishuDisableInputs"
                  @click="toggleFeishuRootSelection(row.node)"
                >
                  <span class="reference-tree-node-title">{{ row.node.summary.title || row.node.summary.nodeToken }}</span>
                  <span class="reference-tree-node-path">{{ row.node.pathLabel }}</span>
                </button>
              </div>
            </div>
          </div>
        </section>
      </div>

      <div class="reference-status-card">
        <div class="reference-status-header">
          <span class="reference-status-title">{{ feishuStageLabel(feishuStatus?.stage) }}</span>
          <span class="reference-status-value">{{ feishuProgressLabel }}</span>
        </div>
        <div class="reference-progress-track">
          <div class="reference-progress-fill" :style="{ width: `${(feishuProgressRatio ?? 0) * 100}%` }" />
        </div>
        <div class="reference-status-message">{{ feishuSummaryMessage }}</div>
        <div class="reference-meta-grid compact">
          <div class="reference-meta-row">
            <span>{{ t("knowledge.feishuReference.window.state") }}</span>
            <span>{{ feishuStateLabel(feishuStatus?.state) }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.feishuReference.window.selectedScope") }}</span>
            <span>{{ feishuSelectedScopeLabel }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.feishuReference.window.importedScope") }}</span>
            <span>{{ feishuImportedScopeLabel }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.referenceImport.importedAt") }}</span>
            <span>{{ feishuImportedAtLabel }}</span>
          </div>
          <div class="reference-meta-row">
            <span>{{ t("knowledge.overview.documentsUnit") }}</span>
            <span>{{ feishuStatus?.importedDocCount ?? 0 }}</span>
          </div>
          <div class="reference-meta-row wide">
            <span>{{ t("knowledge.feishuReference.window.currentItem") }}</span>
            <span class="reference-mono">
              {{ feishuStatus?.currentTitle || feishuStatus?.currentPath || "—" }}
            </span>
          </div>
        </div>
      </div>
    </section>
    </template>
  </div>
</template>

<style scoped>
.reference-external-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}

.reference-external-panel.mode-window {
  gap: 16px;
}

.reference-external-window-tabs {
  padding-bottom: 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 74%, transparent);
}

.reference-external-window-source-tabs {
  width: 100%;
}

.reference-external-window-source-tabs :deep(.base-segmented) {
  width: 100%;
}

.reference-external-window-source-tabs :deep(.base-segmented-item) {
  flex: 1 1 0;
  min-height: 36px;
  font-size: 13px;
}

.reference-external-window-meta {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.reference-external-window-meta-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}

.reference-external-window-meta-field {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.reference-external-window-meta-value {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

.reference-external-window-meta-hint,
.reference-external-window-summary,
.reference-external-window-detail {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.reference-external-window-config {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
  padding: 0;
}

.reference-external-window-config-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.reference-external-window-config-label,
.reference-external-window-stage-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-external-window-config-hint,
.reference-external-window-stage-caption {
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.reference-external-window-locale {
  width: 180px;
  flex-shrink: 0;
}

.reference-external-window-note,
.reference-external-window-block {
  margin-top: 0;
}

.reference-external-window-hero {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.reference-external-window-hero-copy {
  min-width: 0;
}

.reference-external-window-stage-title {
  font-size: 24px;
  line-height: 1.2;
}

.reference-external-window-stage-caption {
  margin-top: 4px;
}

.reference-external-window-stage-value {
  flex-shrink: 0;
  font-size: 28px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.reference-progress-track-window {
  margin-top: 0;
}

.reference-external-window-stage-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-external-window-stage-card {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  color: var(--text-secondary);
}

.reference-external-window-stage-card:first-child {
  border-top: none;
}

.reference-external-window-stage-card.is-complete,
.reference-external-window-stage-card.is-current {
  color: var(--text-color);
}

.reference-external-window-stage-card.is-current {
  color: var(--text-color);
}

.reference-external-window-stage-card.is-error {
  color: var(--status-danger-fg, var(--text-color));
}

.reference-external-window-stage-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.reference-external-window-stage-dot {
  width: 7px;
  height: 7px;
  flex-shrink: 0;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 60%, transparent);
}

.reference-external-window-stage-card.is-complete .reference-external-window-stage-dot,
.reference-external-window-stage-card.is-current .reference-external-window-stage-dot {
  background: color-mix(in srgb, var(--accent-color) 76%, white 24%);
}

.reference-external-window-stage-card.is-error .reference-external-window-stage-dot {
  background: var(--danger-color, #d9534f);
}

.reference-external-window-stage-name,
.reference-external-window-stage-status {
  font-size: 11px;
  line-height: 1.4;
}

.reference-external-window-stage-track {
  position: relative;
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 82%, var(--border-color) 18%);
  overflow: hidden;
}

.reference-external-window-stage-track-fill {
  position: absolute;
  inset: 0 auto 0 0;
  min-width: 0;
  height: 100%;
  border-radius: inherit;
  background: color-mix(in srgb, var(--accent-color) 78%, white 22%);
  transition: width 0.18s ease;
}

.reference-external-window-rows {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-external-window-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.reference-external-window-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.reference-external-window-path {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-external-window-path-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.reference-external-window-path-value {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

.reference-external-window-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-external-window-card {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 0;
  border: none;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
}

.reference-external-panel.mode-window .reference-external-window-card {
  padding-top: 16px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-external-window-card-header {
  align-items: flex-start;
}

.reference-external-window-scope-grid {
  display: grid;
  grid-template-columns: minmax(260px, 1fr) minmax(260px, 1fr);
  gap: 12px;
}

.reference-external-window-status-card {
  padding: 0;
  margin-top: 0;
  border: none;
  border-radius: 0;
  background: transparent;
}

.reference-external-card,
.reference-subsection,
.reference-status-card {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--bg-color) 10%);
}

.reference-external-card {
  padding: 14px;
}

.reference-external-topbar,
.reference-section-header,
.reference-tree-header,
.reference-status-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.reference-external-copy,
.reference-section-title,
.reference-tree-selection {
  min-width: 0;
}

.reference-external-title,
.reference-section-title,
.reference-subsection-title,
.reference-status-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-external-hint,
.reference-section-hint,
.reference-subsection-hint,
.reference-tree-node-path,
.reference-inline-note,
.reference-external-meta,
.reference-status-message,
.reference-tree-empty {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.reference-external-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.reference-external-grid.with-header {
  margin-top: 14px;
}

.reference-external-target-card {
  grid-column: 1 / -1;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 72px;
  padding: 12px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 76%, var(--panel-bg) 24%);
}

.reference-external-label {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-tertiary, var(--text-secondary));
}

.reference-external-field,
.reference-field-stack {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.reference-external-value,
.reference-external-target-path,
.reference-tree-selection-value,
.reference-status-value {
  font-size: 13px;
  color: var(--text-color);
  font-weight: 600;
}

.reference-section-actions,
.reference-button-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.reference-settings-grid {
  display: grid;
  grid-template-columns: minmax(200px, 240px) minmax(0, 1fr);
  gap: 12px;
  margin-top: 14px;
}

.reference-meta-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px 14px;
}

.reference-meta-grid.compact {
  margin-top: 12px;
}

.reference-meta-row {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.reference-meta-row span:last-child {
  text-align: right;
  color: var(--text-color);
}

.reference-meta-row.wide {
  grid-column: 1 / -1;
}

.reference-status-card {
  padding: 14px;
  margin-top: 14px;
}

.reference-progress-track {
  width: 100%;
  height: 8px;
  border-radius: 999px;
  overflow: hidden;
  background: color-mix(in srgb, var(--sidebar-bg) 82%, var(--panel-bg) 18%);
  border: 1px solid var(--border-color);
  margin-top: 10px;
}

.reference-progress-fill {
  height: 100%;
  background: color-mix(in srgb, var(--accent-color) 88%, white 12%);
}

.reference-stage-list {
  margin-top: 12px;
  border-top: 1px solid var(--border-color);
}

.reference-stage-row {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 9px 0;
  font-size: 12px;
  color: var(--text-secondary);
  border-top: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
}

.reference-stage-row:first-child {
  border-top: none;
}

.reference-stage-row.active span,
.reference-stage-row.done span {
  color: var(--text-color);
}

.reference-inline-note {
  margin-top: 12px;
}

.reference-inline-link {
  margin-left: 8px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent-color);
  cursor: pointer;
  font: inherit;
}

.reference-inline-link:hover {
  text-decoration: underline;
}

.reference-split-grid {
  display: grid;
  grid-template-columns: minmax(320px, 0.85fr) minmax(360px, 1.15fr);
  gap: 12px;
  margin-top: 14px;
}

.reference-subsection {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px;
  min-width: 0;
}

.reference-form-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.reference-field-span {
  grid-column: 1 / -1;
}

.reference-input {
  width: 100%;
  min-height: 34px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
  color: var(--text-color);
  padding: 0 12px;
  font-size: 13px;
  box-sizing: border-box;
}

.reference-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.reference-callback-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.reference-callback-title {
  font-size: 12px;
  color: var(--text-secondary);
}

.reference-callback-item {
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 80%, var(--panel-bg) 20%);
  font-size: 12px;
}

.reference-tree-shell {
  min-height: 260px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 82%, var(--panel-bg) 18%);
  overflow: auto;
}

.reference-tree-empty {
  min-height: 200px;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 16px;
}

.reference-tree-empty.error {
  color: var(--status-danger-fg);
}

.reference-tree-list {
  padding: 8px 0;
}

.reference-tree-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 34px;
  padding-right: 10px;
  transition: background 0.12s ease;
}

.reference-tree-row:hover {
  background: var(--hover-bg);
}

.reference-tree-row.selected,
.reference-tree-row.selected:hover {
  background: var(--active-bg);
}

.reference-tree-row.disabled {
  opacity: 0.72;
}

.reference-tree-toggle {
  width: 20px;
  height: 20px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  flex-shrink: 0;
}

.reference-tree-toggle:disabled {
  opacity: 0.35;
  cursor: default;
}

.reference-tree-node {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  min-width: 0;
  flex: 1;
  padding: 4px 0;
  border: none;
  background: transparent;
  color: var(--text-color);
  cursor: pointer;
  text-align: left;
}

.reference-tree-node:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.reference-tree-node-title {
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
}

.reference-tree-row.selected .reference-tree-node-title {
  color: var(--text-color);
  font-weight: 600;
}

.reference-mono {
  font-family: var(--font-mono-identifier);
}

@media (max-width: 1100px) {
  .reference-external-window-meta-grid,
  .reference-external-window-scope-grid,
  .reference-external-window-stage-grid,
  .reference-split-grid,
  .reference-settings-grid,
  .reference-external-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .reference-meta-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
