import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { getWarmup } from "./warmupCache";
import {
  knowledgeActivateEmbedding,
  knowledgeCreate,
  knowledgeDeactivateEmbedding,
  knowledgeDelete,
  knowledgeDeleteExternalReferenceDirectory,
  knowledgeDeleteFeishuReferenceDocs,
  knowledgeDeleteUnityReferenceDocs,
  knowledgeDownloadLocalEmbeddingModel,
  knowledgeGetEmbeddingConfig,
  knowledgeGetEmbeddingStatus,
  knowledgeGetFeishuReferenceImportStatus,
  knowledgeGetGeneralConfig,
  knowledgeGetLexicalRebuildStatus,
  knowledgeGetLocalEmbeddingModelCatalog,
  knowledgeGetOverview,
  knowledgeGetUnityReferenceImportStatus,
  knowledgeInspectLocalEmbeddingModelDirectory,
  knowledgeEdit,
  knowledgeList,
  knowledgeListDirectoryDocuments,
  knowledgeListDirectoryDocumentsPage,
  knowledgeListExternalReferenceDirectories,
  knowledgeListUnityManagedDirectoryStats,
  knowledgeListDirectories,
  knowledgeMove,
  knowledgeQuery,
  knowledgeRebuildLexicalIndex,
  knowledgeRevealTarget,
  knowledgeRead,
  knowledgeSaveEmbeddingConfig,
  knowledgeSaveGeneralConfig,
} from "../services/knowledge";
import type {
  EmbeddingConfig,
  EmbeddingLocalModelCatalog,
  EmbeddingLocalModelDirectoryInspection,
  EmbeddingStatus,
  FeishuReferenceImportStatus,
  KnowledgeCatalogStats,
  KnowledgeDirectoryConfig,
  KnowledgeDirectoryConfigRecord,
  KnowledgeDocument,
  KnowledgeDocumentListInput,
  KnowledgeDocumentPatch,
  KnowledgeDocumentSection,
  KnowledgeDocumentSummary,
  KnowledgeFolderDisplayStats,
  KnowledgeGeneralConfig,
  KnowledgeChangedEvent,
  KnowledgeRetrievalOverview,
  KnowledgeDocumentType,
  KnowledgeExternalSource,
  KnowledgeSearchMatchKind,
  KnowledgeSearchSelectionContext,
  KnowledgeSearchResult,
  KnowledgeStorageSource,
  LexicalRebuildStatus,
  ModelDefaults,
  UnityReferenceImportStatus,
} from "../types";
import { normalizeAppError } from "../services/errors";
import { useNotificationStore } from "../stores/notification";
import { useUiStore } from "../stores/ui";
import { acquireSelectionLock } from "./useSelectionLock";
import { openKnowledgeDownloadProgressWindow } from "../services/knowledgeDownloadWindow";
import { openFeishuReferenceImportProgressWindow } from "../services/feishuReferenceImportWindow";
import { openUnityReferenceImportProgressWindow } from "../services/unityReferenceImportWindow";
import { KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT } from "../services/knowledgeLexicalProgressWindow";
import {
  buildKnowledgeCreateDefaults,
  defaultSummaryEnabledForType,
} from "../components/knowledge/knowledgeEditMode";
import { pruneKnowledgeDeleteTargets } from "../components/knowledge/knowledgeExplorerSelection";
import { t } from "../i18n";

interface KnowledgeProps {
  workingDir: string;
  selectedModelId: string;
  modelDefaults: ModelDefaults;
}

export type KnowledgeViewMode = "browse" | "search";

export type ExplorerNode =
  | {
      kind: "folder";
      path: string;
      relativePath: string;
      name: string;
      depth: number;
      children: ExplorerNode[];
    }
  | {
      kind: "document";
      path: string;
      name: string;
      depth: number;
      document: KnowledgeDocumentSummary;
    };

type FolderNode = Extract<ExplorerNode, { kind: "folder" }>;

const DEFAULT_TYPE_ORDER: KnowledgeDocumentType[] = [
  "design",
  "memory",
  "skill",
  "reference",
];
const TYPE_PREFIX_RE = /^(design|memory|skill|reference)\//;
const FEISHU_REFERENCE_MANAGED_DIR = "feishu-knowledge-base";
const UNITY_REFERENCE_MANAGED_DIR = "unity-official-docs";
const ROOT_DIRECTORY_PAGE_KEY = "__root__";
const REFERENCE_DOCUMENT_PAGE_SIZE = 160;

function emptyTypeStats(): Record<KnowledgeDocumentType, number> {
  return {
    design: 0,
    memory: 0,
    skill: 0,
    reference: 0,
  };
}

function emptyStorageSourceStats(): Record<KnowledgeStorageSource, number> {
  return {
    project: 0,
    app: 0,
  };
}

function emptyDirectoryGroups(): Record<KnowledgeDocumentType, string[]> {
  return {
    design: [],
    memory: [],
    skill: [],
    reference: [],
  };
}

function emptyRootDirectoryConfigs(): Record<
  KnowledgeDocumentType,
  Record<string, KnowledgeDirectoryConfigRecord>
> {
  return {
    design: {},
    memory: {},
    skill: {},
    reference: {},
  };
}

function emptyLoadedDirectoryDocumentPaths(): Record<
  KnowledgeDocumentType,
  Set<string>
> {
  return {
    design: new Set<string>(),
    memory: new Set<string>(),
    skill: new Set<string>(),
    reference: new Set<string>(),
  };
}

function emptyDirectoryPageCursorState(): Record<
  KnowledgeDocumentType,
  Record<string, string | null>
> {
  return {
    design: {},
    memory: {},
    skill: {},
    reference: {},
  };
}

function emptyDirectoryPageLoadingState(): Record<
  KnowledgeDocumentType,
  Record<string, boolean>
> {
  return {
    design: {},
    memory: {},
    skill: {},
    reference: {},
  };
}

function slugifyKnowledgePath(title: string): string {
  const slug = title
    .trim()
    .toLowerCase()
    .replace(/['"]/g, "")
    .replace(/[^a-z0-9\u4e00-\u9fff]+/gi, "-")
    .replace(/^-+|-+$/g, "");
  return slug || `document-${Date.now().toString(36)}`;
}

function deriveSearchMode(
  results: KnowledgeSearchResult[],
): KnowledgeSearchMatchKind | null {
  if (!results.length) return null;
  const kinds = new Set(results.map((result) => result.matchKind));
  if (kinds.has("hybrid") || (kinds.has("lexical") && kinds.has("semantic")))
    return "hybrid";
  if (kinds.has("semantic")) return "semantic";
  return "lexical";
}

function typeRootPath(type: KnowledgeDocumentType): string {
  return type;
}

function fullDocumentPath(type: KnowledgeDocumentType, path: string): string {
  return `${type}/${path.replace(/\\/g, "/").replace(/^\/+/, "")}`;
}

function buildCreatePath(
  type: KnowledgeDocumentType,
  name: string,
  parentDir = "",
): string {
  const normalizedParent = parentDir
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
  const normalizedName = name
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
  if (type === "skill") {
    const skillPath = normalizedName
      .replace(/\.md$/i, "")
      .replace(/^\/+|\/+$/g, "");
    return normalizedParent
      ? `${normalizedParent}/${skillPath}.md`
      : `${skillPath}.md`;
  }
  return normalizedParent
    ? `${normalizedParent}/${normalizedName}`
    : normalizedName;
}

function normalizeDirectorySelectionPath(path: string): string {
  return path
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
}

function directoryPageStateKey(path: string | null | undefined): string {
  const normalized = normalizeDirectorySelectionPath(path ?? "");
  return normalized || ROOT_DIRECTORY_PAGE_KEY;
}

function normalizeRelativeEntryName(name: string): string {
  return name
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
}

function isRootDirectoryPath(path: string): boolean {
  const normalized = normalizeDirectorySelectionPath(path);
  return !!normalized && !normalized.includes("/");
}

function joinRelativePath(parentDir: string, name: string): string {
  const normalizedParent = normalizeDirectorySelectionPath(parentDir);
  const normalizedName = normalizeRelativeEntryName(name);
  return normalizedParent
    ? `${normalizedParent}/${normalizedName}`
    : normalizedName;
}

function siblingRelativePath(path: string, name: string): string {
  const normalizedPath = normalizeDirectorySelectionPath(path);
  const segments = normalizedPath.split("/").filter(Boolean);
  return joinRelativePath(segments.slice(0, -1).join("/"), name);
}

function parentDirectoryPath(path: string): string {
  const normalizedPath = normalizeDirectorySelectionPath(path);
  const segments = normalizedPath.split("/").filter(Boolean);
  return segments.slice(0, -1).join("/");
}

function replaceRelativePathPrefix(
  path: string | null | undefined,
  sourcePath: string,
  targetPath: string,
): string | null {
  if (!path) return null;
  const normalizedPath = path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  const normalizedSource = normalizeDirectorySelectionPath(sourcePath);
  const normalizedTarget = normalizeDirectorySelectionPath(targetPath);
  if (normalizedPath === normalizedSource) return normalizedTarget;
  if (normalizedPath.startsWith(`${normalizedSource}/`)) {
    return `${normalizedTarget}${normalizedPath.slice(normalizedSource.length)}`;
  }
  return null;
}

function normalizeSelectionPath(path: string): string {
  return path.trim().replace(/\\/g, "/");
}

function normalizeWorkspacePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/g, "").toLowerCase();
}

function inferSelectionType(path: string): KnowledgeDocumentType | null {
  const normalized = normalizeSelectionPath(path);
  if (normalized.startsWith("design/")) return "design";
  if (normalized.startsWith("memory/")) return "memory";
  if (normalized.startsWith("skill/")) return "skill";
  if (normalized.startsWith("reference/")) return "reference";
  return null;
}

function relativeSelectionPath(path: string): string {
  return normalizeSelectionPath(path).replace(TYPE_PREFIX_RE, "");
}

function isUnityReferenceManagedPath(path: string | null | undefined): boolean {
  if (!path) return false;
  const normalized = path
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
  return (
    normalized === UNITY_REFERENCE_MANAGED_DIR ||
    normalized.startsWith(`${UNITY_REFERENCE_MANAGED_DIR}/`)
  );
}

function normalizeUnityImportWindowTargetPath(
  path: string | null | undefined,
): string {
  const normalized = normalizeDirectorySelectionPath(path ?? "");
  return normalized === UNITY_REFERENCE_MANAGED_DIR ? normalized : "";
}

function isFeishuReferenceManagedPath(
  path: string | null | undefined,
): boolean {
  if (!path) return false;
  const normalized = path
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
  return (
    normalized === FEISHU_REFERENCE_MANAGED_DIR ||
    normalized.startsWith(`${FEISHU_REFERENCE_MANAGED_DIR}/`)
  );
}

function compareNodes(left: ExplorerNode, right: ExplorerNode): number {
  if (left.kind === "folder" && right.kind !== "folder") return -1;
  if (left.kind !== "folder" && right.kind === "folder") return 1;
  return left.name.localeCompare(right.name, undefined, {
    sensitivity: "base",
    numeric: true,
  });
}

function ensureFolderNode(
  folderMap: Map<string, FolderNode>,
  type: KnowledgeDocumentType,
  relativePath: string,
): FolderNode {
  const normalized = relativePath
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
  const rootPath = typeRootPath(type);
  const existing = folderMap.get(normalized || rootPath);
  if (existing) return existing;

  if (!normalized) {
    const rootNode: FolderNode = {
      kind: "folder",
      path: rootPath,
      relativePath: "",
      name: type,
      depth: 0,
      children: [],
    };
    folderMap.set(rootPath, rootNode);
    return rootNode;
  }

  const segments = normalized.split("/").filter(Boolean);
  const parentRelativePath = segments.slice(0, -1).join("/");
  const parentNode = ensureFolderNode(folderMap, type, parentRelativePath);
  const fullPath = fullDocumentPath(type, normalized);
  const folderNode: FolderNode = {
    kind: "folder",
    path: fullPath,
    relativePath: normalized,
    name: segments[segments.length - 1] ?? normalized,
    depth: segments.length,
    children: [],
  };
  folderMap.set(normalized, folderNode);
  parentNode.children.push(folderNode);
  return folderNode;
}

function buildExplorerTree(
  documentsByType: Record<KnowledgeDocumentType, KnowledgeDocumentSummary[]>,
  directoryGroups: Record<KnowledgeDocumentType, string[]>,
): FolderNode[] {
  return DEFAULT_TYPE_ORDER.map((type) => {
    const folderMap = new Map<string, FolderNode>();
    const rootNode = ensureFolderNode(folderMap, type, "");

    for (const directory of directoryGroups[type]) {
      ensureFolderNode(folderMap, type, directory);
    }

    for (const document of documentsByType[type]) {
      const normalizedPath = document.path
        .replace(/\\/g, "/")
        .replace(/^\/+/, "");
      const segments = normalizedPath.split("/").filter(Boolean);
      const fileName = segments[segments.length - 1] ?? document.title;
      const parentRelativePath = segments.slice(0, -1).join("/");
      const parentNode = ensureFolderNode(folderMap, type, parentRelativePath);
      parentNode.children.push({
        kind: "document",
        path: fullDocumentPath(type, normalizedPath),
        name: fileName,
        depth: segments.length,
        document,
      });
    }

    const sortChildren = (node: FolderNode) => {
      node.children.sort(compareNodes);
      for (const child of node.children) {
        if (child.kind === "folder") sortChildren(child);
      }
    };
    sortChildren(rootNode);
    return rootNode;
  });
}

export function useKnowledgeState(props: KnowledgeProps) {
  const hasWorkspace = computed(() => !!props.workingDir.trim());
  const notificationStore = useNotificationStore();
  const uiStore = useUiStore();

  const error = ref("");
  const sidebarWidth = ref(272);

  const documents = ref<KnowledgeDocumentSummary[]>([]);
  const directoryGroups = ref<Record<KnowledgeDocumentType, string[]>>(
    emptyDirectoryGroups(),
  );
  const rootDirectoryConfigs = ref(emptyRootDirectoryConfigs());
  const referenceExternalDirectorySources = ref<
    Record<string, KnowledgeExternalSource[]>
  >({});
  const referenceManagedDirectoryStats = ref<
    Record<string, KnowledgeFolderDisplayStats>
  >({});
  const loading = ref(false);
  const activeType = ref<KnowledgeDocumentType>("design");
  const expandedPaths = ref<Set<string>>(new Set(DEFAULT_TYPE_ORDER));
  const selectedDocumentId = ref<string | null>(null);
  const selectedDocument = ref<KnowledgeDocument | null>(null);
  const selectedDocumentLoading = ref(false);
  const selectedDirectoryPath = ref<string | null>(null);
  const selectedDirectoryConfig = ref<KnowledgeDirectoryConfigRecord | null>(
    null,
  );
  const selectedDirectoryLoading = ref(false);
  const savingDocument = ref(false);
  const creatingDocument = ref(false);
  const deletingDocument = ref(false);

  const searchQuery = ref("");
  const searchResults = ref<KnowledgeSearchResult[]>([]);
  const searching = ref(false);
  const searchLatencyMs = ref<number | null>(null);
  const searchMode = ref<KnowledgeSearchMatchKind | null>(null);
  const recentQueryTokens = ref<number | null>(null);
  const selectedSearchContext = ref<KnowledgeSearchSelectionContext | null>(
    null,
  );

  const overviewLoading = ref(false);
  const retrievalOverview = ref<KnowledgeRetrievalOverview | null>(null);
  const generalConfig = ref<KnowledgeGeneralConfig | null>(null);
  const embeddingConfig = ref<EmbeddingConfig | null>(null);
  const embeddingLocalModelCatalog = ref<EmbeddingLocalModelCatalog | null>(
    null,
  );
  const embeddingLocalDirectoryInspection =
    ref<EmbeddingLocalModelDirectoryInspection | null>(null);
  const embeddingStatus = ref<EmbeddingStatus | null>(null);
  const lexicalRebuildStatus = ref<LexicalRebuildStatus | null>(null);
  const feishuReferenceImportStatus = ref<FeishuReferenceImportStatus | null>(
    null,
  );
  const unityReferenceImportStatus = ref<UnityReferenceImportStatus | null>(
    null,
  );
  const retrievalActionPending = ref(false);
  const feishuReferenceImportPending = ref(false);
  const feishuReferenceDeletePending = ref(false);
  const unityReferenceImportPending = ref(false);
  const unityReferenceDeletePending = ref(false);
  const pendingSelectionPath = ref<string | null>(null);

  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let externalRefreshTimer: ReturnType<typeof setTimeout> | null = null;
  type WorkspaceRequestSnapshot = {
    workspaceKey: string;
    requestVersion: number;
  };
  type QueuedKnowledgeChange = {
    change: KnowledgeChangedEvent;
    request: WorkspaceRequestSnapshot;
  };
  let pendingExternalChanges: QueuedKnowledgeChange[] = [];
  let retrievalStatusPollTimer: ReturnType<typeof setTimeout> | null = null;
  let feishuReferenceStatusPollTimer: ReturnType<typeof setTimeout> | null =
    null;
  let unityReferenceStatusPollTimer: ReturnType<typeof setTimeout> | null =
    null;
  let searchSeq = 0;
  let mutationQueue = Promise.resolve();
  let pendingSaveCount = 0;
  let releaseSelectionLock: (() => void) | null = null;
  let knowledgeChangedUnlisten: UnlistenFn | null = null;
  let lexicalRebuildStatusUnlisten: UnlistenFn | null = null;
  let destroyed = false;
  let loadedDirectoryDocumentPaths = emptyLoadedDirectoryDocumentPaths();
  let loadedDocumentTypes = new Set<KnowledgeDocumentType>();
  let loadedDirectoryTypes = new Set<KnowledgeDocumentType>();
  let dirtyDocumentTypes = new Set<KnowledgeDocumentType>();
  let dirtyDirectoryTypes = new Set<KnowledgeDocumentType>();
  const directoryDocumentNextCursor = ref(emptyDirectoryPageCursorState());
  const directoryDocumentLoading = ref(emptyDirectoryPageLoadingState());
  let workspaceRequestVersion = 0;

  function currentWorkspaceKey(): string {
    return normalizeWorkspacePath(props.workingDir);
  }

  function isCurrentWorkspaceRequest(
    request: WorkspaceRequestSnapshot,
  ): boolean {
    return (
      !destroyed &&
      !!request.workspaceKey &&
      request.workspaceKey === currentWorkspaceKey() &&
      request.requestVersion === workspaceRequestVersion
    );
  }

  function captureWorkspaceRequest(): WorkspaceRequestSnapshot {
    return {
      workspaceKey: currentWorkspaceKey(),
      requestVersion: workspaceRequestVersion,
    };
  }

  function isCurrentWorkspaceChange(
    change: KnowledgeChangedEvent,
    request: WorkspaceRequestSnapshot,
  ): boolean {
    const eventWorkspaceKey = normalizeWorkspacePath(change.workingDir);
    return (
      !!eventWorkspaceKey &&
      eventWorkspaceKey === request.workspaceKey &&
      isCurrentWorkspaceRequest(request)
    );
  }

  function clearExternalRefreshQueue() {
    if (externalRefreshTimer) {
      clearTimeout(externalRefreshTimer);
      externalRefreshTimer = null;
    }
    pendingExternalChanges = [];
  }

  function clearSearchTimer() {
    if (!searchTimer) return;
    clearTimeout(searchTimer);
    searchTimer = null;
  }

  function resetSearchRuntimeState() {
    clearSearchTimer();
    searchSeq += 1;
    selectedSearchContext.value = null;
    searchResults.value = [];
    searching.value = false;
    searchLatencyMs.value = null;
    searchMode.value = null;
    recentQueryTokens.value = null;
  }

  const documentsByType = computed<
    Record<KnowledgeDocumentType, KnowledgeDocumentSummary[]>
  >(() => {
    const grouped: Record<KnowledgeDocumentType, KnowledgeDocumentSummary[]> = {
      design: [],
      memory: [],
      skill: [],
      reference: [],
    };
    for (const doc of documents.value) {
      grouped[doc.type].push(doc);
    }
    for (const type of DEFAULT_TYPE_ORDER) {
      grouped[type].sort((left, right) => {
        const pathDelta = left.path.localeCompare(right.path, undefined, {
          sensitivity: "base",
        });
        if (pathDelta !== 0) return pathDelta;
        return left.title.localeCompare(right.title, undefined, {
          sensitivity: "base",
        });
      });
    }
    return grouped;
  });

  const explorerTree = computed(() =>
    buildExplorerTree(documentsByType.value, directoryGroups.value),
  );

  const currentExplorerRoot = computed(
    () =>
      explorerTree.value.find(
        (node) => node.path === typeRootPath(activeType.value),
      ) ?? null,
  );
  const visibleExplorerTree = computed(
    () => currentExplorerRoot.value?.children ?? [],
  );
  const activeDirectoryCount = computed(
    () => directoryGroups.value[activeType.value].length,
  );
  const selectedDocumentSummary = computed(
    () =>
      documents.value.find((doc) => doc.id === selectedDocumentId.value) ??
      null,
  );
  const selectedPath = computed(() =>
    selectedDocumentSummary.value
      ? fullDocumentPath(
          selectedDocumentSummary.value.type,
          selectedDocumentSummary.value.path,
        )
      : selectedDocument.value
        ? fullDocumentPath(
            selectedDocument.value.type,
            selectedDocument.value.path,
          )
        : selectedDirectoryPath.value
          ? fullDocumentPath(activeType.value, selectedDirectoryPath.value)
          : null,
  );
  const viewMode = computed<KnowledgeViewMode>(() =>
    searchQuery.value.trim() ? "search" : "browse",
  );

  const catalogStats = computed<KnowledgeCatalogStats>(() => {
    const byType = emptyTypeStats();
    const byStorageSource = emptyStorageSourceStats();
    let commandEnabled = 0;
    let aiMaintained = 0;
    let fullInjectable = 0;
    let summaryMissing = 0;
    let external = 0;

    for (const doc of documents.value) {
      byType[doc.type] += 1;
      byStorageSource[doc.storageSource ?? "project"] += 1;
      if (doc.commandEnabled) commandEnabled += 1;
      if (doc.aiMaintained) aiMaintained += 1;
      if (
        (doc.type === "design" || doc.type === "memory") &&
        doc.injectMode === "full"
      ) {
        fullInjectable += 1;
      }
      if (doc.summaryEnabled && !doc.hasSummary) summaryMissing += 1;
      if (doc.externalSource) external += 1;
    }

    return {
      total: documents.value.length,
      byType,
      byStorageSource,
      commandEnabled,
      aiMaintained,
      fullInjectable,
      summaryMissing,
      external,
    };
  });

  const overview = computed(() => catalogStats.value);
  let lastNotifiedEmbeddingRuntimeError = "";

  function notifyError(action: string, cause: unknown) {
    const err = normalizeAppError(cause);
    error.value = err.message;
    notificationStore.addNotice("error", `${action}: ${err.message}`, {
      code: err.code,
      operation: action,
      replaceOperation: true,
    });
  }

  function notifyEmbeddingRuntimeError(cause: unknown) {
    const err = normalizeAppError(cause);
    error.value = err.message;
    notificationStore.addNotice(
      "error",
      t("knowledge.retrieval.runtimeInitFailed", err.message),
      {
        code: err.code,
        operation: "knowledge_embedding_runtime",
        replaceOperation: true,
      },
    );
  }

  function scheduleRetrievalStatusPoll(delay = 450) {
    if (retrievalStatusPollTimer) clearTimeout(retrievalStatusPollTimer);
    retrievalStatusPollTimer = setTimeout(() => {
      retrievalStatusPollTimer = null;
      void refreshRetrievalRuntimeStatus();
    }, delay);
  }

  function stopRetrievalStatusPoll() {
    if (!retrievalStatusPollTimer) return;
    clearTimeout(retrievalStatusPollTimer);
    retrievalStatusPollTimer = null;
  }

  function scheduleFeishuReferenceStatusPoll(delay = 900) {
    if (feishuReferenceStatusPollTimer)
      clearTimeout(feishuReferenceStatusPollTimer);
    feishuReferenceStatusPollTimer = setTimeout(() => {
      feishuReferenceStatusPollTimer = null;
      void refreshFeishuReferenceImportStatus(true);
    }, delay);
  }

  function stopFeishuReferenceStatusPoll() {
    if (!feishuReferenceStatusPollTimer) return;
    clearTimeout(feishuReferenceStatusPollTimer);
    feishuReferenceStatusPollTimer = null;
  }

  function scheduleUnityReferenceStatusPoll(delay = 900) {
    if (unityReferenceStatusPollTimer)
      clearTimeout(unityReferenceStatusPollTimer);
    unityReferenceStatusPollTimer = setTimeout(() => {
      unityReferenceStatusPollTimer = null;
      void refreshUnityReferenceImportStatus(true);
    }, delay);
  }

  function stopUnityReferenceStatusPoll() {
    if (!unityReferenceStatusPollTimer) return;
    clearTimeout(unityReferenceStatusPollTimer);
    unityReferenceStatusPollTimer = null;
  }

  async function refreshRetrievalRuntimeStatus() {
    if (!hasWorkspace.value) return;
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    try {
      const nextEmbeddingStatus = await knowledgeGetEmbeddingStatus();
      if (!isCurrentWorkspaceRequest(request)) return;
      embeddingStatus.value = nextEmbeddingStatus;
      if (retrievalActionPending.value || nextEmbeddingStatus.activating) {
        scheduleRetrievalStatusPoll();
      }
    } catch {
      if (!isCurrentWorkspaceRequest(request)) return;
      stopRetrievalStatusPoll();
    }
  }

  function handleLexicalRebuildStatus(nextStatus: LexicalRebuildStatus) {
    if (!hasWorkspace.value) return;
    lexicalRebuildStatus.value = nextStatus;
    if (nextStatus.running) return;
    if (nextStatus.stage === "completed" || nextStatus.stage === "error") {
      if (!embeddingStatus.value?.activating) {
        stopRetrievalStatusPoll();
      }
      void loadOverview();
    }
  }

  async function refreshFeishuReferenceImportStatus(handleTransition = false) {
    if (!hasWorkspace.value) return;
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    const previous = feishuReferenceImportStatus.value;
    try {
      const nextStatus = await knowledgeGetFeishuReferenceImportStatus();
      if (!isCurrentWorkspaceRequest(request)) return;
      feishuReferenceImportStatus.value = nextStatus;
      feishuReferenceImportPending.value = nextStatus.running;
      if (nextStatus.running || nextStatus.stage === "authorizing") {
        scheduleFeishuReferenceStatusPoll();
      } else {
        stopFeishuReferenceStatusPoll();
      }

      if (handleTransition && previous?.running && !nextStatus.running) {
        if (nextStatus.lastOutcome === "cancelled") {
          notificationStore.addNotice(
            "info",
            t("knowledge.feishuReference.cancelledNotice"),
            {
              operation: "knowledge_import_feishu_reference_docs",
              replaceOperation: true,
            },
          );
          return;
        }
        if (nextStatus.state === "ready") {
          notificationStore.addNotice(
            "success",
            t(
              "knowledge.feishuReference.completedNotice",
              nextStatus.importedDocCount,
            ),
            {
              operation: "knowledge_import_feishu_reference_docs",
              replaceOperation: true,
            },
          );
          await refreshKnowledgeData();
          return;
        }
        if (nextStatus.state === "error") {
          notificationStore.addNotice(
            "error",
            nextStatus.error?.trim() ||
              nextStatus.message ||
              t("knowledge.feishuReference.errorNotice"),
            {
              operation: "knowledge_import_feishu_reference_docs",
              replaceOperation: true,
            },
          );
        }
      }
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) return;
      feishuReferenceImportPending.value = false;
      stopFeishuReferenceStatusPoll();
      if (handleTransition) {
        notifyError("knowledge_get_feishu_reference_import_status", cause);
      }
    }
  }

  async function refreshUnityReferenceImportStatus(handleTransition = false) {
    if (!hasWorkspace.value) return;
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    const previous = unityReferenceImportStatus.value;
    try {
      const nextStatus = await knowledgeGetUnityReferenceImportStatus();
      if (!isCurrentWorkspaceRequest(request)) return;
      unityReferenceImportStatus.value = nextStatus;
      unityReferenceImportPending.value = nextStatus.running;
      if (nextStatus.running) {
        scheduleUnityReferenceStatusPoll();
      } else {
        stopUnityReferenceStatusPoll();
      }

      if (handleTransition && previous?.running && !nextStatus.running) {
        if (nextStatus.lastOutcome === "cancelled") {
          notificationStore.addNotice(
            "info",
            t("knowledge.referenceImport.cancelledNotice"),
            {
              operation: "knowledge_import_unity_reference_docs",
              replaceOperation: true,
            },
          );
          return;
        }
        if (nextStatus.state === "ready") {
          notificationStore.addNotice(
            "success",
            t(
              "knowledge.referenceImport.completedNotice",
              nextStatus.importedDocCount,
            ),
            {
              operation: "knowledge_import_unity_reference_docs",
              replaceOperation: true,
            },
          );
          await refreshKnowledgeData();
          return;
        }
        if (nextStatus.state === "error") {
          notificationStore.addNotice(
            "error",
            nextStatus.error?.trim() ||
              nextStatus.message ||
              t("knowledge.referenceImport.errorNotice"),
            {
              operation: "knowledge_import_unity_reference_docs",
              replaceOperation: true,
            },
          );
        }
      }
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) return;
      unityReferenceImportPending.value = false;
      stopUnityReferenceStatusPoll();
      if (handleTransition) {
        notifyError("knowledge_get_unity_reference_import_status", cause);
      }
    }
  }

  function consumePendingUiSelection() {
    const pending = uiStore.pendingKnowledgeSelection;
    if (!pending) return;

    const type = inferSelectionType(pending.path) ?? pending.dashboard;
    if (!type) {
      uiStore.clearPendingKnowledgeSelection(pending.id);
      return;
    }

    activeType.value = type;
    pendingSelectionPath.value = relativeSelectionPath(
      inferSelectionType(pending.path)
        ? pending.path
        : `${type}/${pending.path}`,
    );
    uiStore.clearPendingKnowledgeSelection(pending.id);
  }

  function enqueueMutation<T>(work: () => Promise<T>): Promise<T> {
    const scheduled = mutationQueue.then(work, work);
    mutationQueue = scheduled.then(
      () => undefined,
      () => undefined,
    );
    return scheduled;
  }

  function beginSave() {
    pendingSaveCount += 1;
    savingDocument.value = pendingSaveCount > 0;
  }

  function endSave() {
    pendingSaveCount = Math.max(0, pendingSaveCount - 1);
    savingDocument.value = pendingSaveCount > 0;
  }

  function beginExplorerDrag() {
    releaseSelectionLock?.();
    releaseSelectionLock = acquireSelectionLock();
  }

  function endExplorerDrag() {
    releaseSelectionLock?.();
    releaseSelectionLock = null;
  }

  function mergeDocuments(nextDocuments: KnowledgeDocumentSummary[]) {
    const merged = new Map<string, KnowledgeDocumentSummary>();
    for (const document of documents.value) {
      merged.set(`${document.type}:${document.path}`, document);
    }
    for (const document of nextDocuments) {
      merged.set(`${document.type}:${document.path}`, document);
    }
    documents.value = Array.from(merged.values());
  }

  function replaceTypeDocuments(
    type: KnowledgeDocumentType,
    nextDocuments: KnowledgeDocumentSummary[],
  ) {
    const merged = new Map<string, KnowledgeDocumentSummary>();
    for (const document of documents.value) {
      if (document.type === type) continue;
      merged.set(`${document.type}:${document.path}`, document);
    }
    for (const document of nextDocuments) {
      merged.set(`${document.type}:${document.path}`, document);
    }
    documents.value = Array.from(merged.values());
  }

  function isDocumentInDirectory(
    document: KnowledgeDocumentSummary,
    directoryPath: string,
  ): boolean {
    const documentParent = parentDirectoryPath(document.path) ?? "";
    return documentParent === directoryPath;
  }

  function pathMatchesSubtree(path: string, prefix: string): boolean {
    if (!prefix) return true;
    return path === prefix || path.startsWith(`${prefix}/`);
  }

  function replaceDocumentPath(
    type: KnowledgeDocumentType,
    path: string,
    nextDocuments: KnowledgeDocumentSummary[],
  ) {
    const merged = new Map<string, KnowledgeDocumentSummary>();
    for (const document of documents.value) {
      if (document.type === type && document.path === path) continue;
      merged.set(`${document.type}:${document.path}`, document);
    }
    for (const document of nextDocuments) {
      merged.set(`${document.type}:${document.path}`, document);
    }
    documents.value = Array.from(merged.values());
  }

  function replaceDirectoryDocuments(
    type: KnowledgeDocumentType,
    directoryPath: string,
    nextDocuments: KnowledgeDocumentSummary[],
  ) {
    const normalizedDirectory = normalizeDirectorySelectionPath(directoryPath);
    const merged = new Map<string, KnowledgeDocumentSummary>();
    for (const document of documents.value) {
      if (
        document.type === type &&
        isDocumentInDirectory(document, normalizedDirectory)
      ) {
        continue;
      }
      merged.set(`${document.type}:${document.path}`, document);
    }
    for (const document of nextDocuments) {
      merged.set(`${document.type}:${document.path}`, document);
    }
    documents.value = Array.from(merged.values());
  }

  function replaceDocumentSubtree(
    type: KnowledgeDocumentType,
    pathPrefix: string,
    nextDocuments: KnowledgeDocumentSummary[],
  ) {
    const normalizedPrefix = pathPrefix.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
    const merged = new Map<string, KnowledgeDocumentSummary>();
    for (const document of documents.value) {
      if (
        document.type === type &&
        pathMatchesSubtree(document.path, normalizedPrefix)
      ) {
        continue;
      }
      merged.set(`${document.type}:${document.path}`, document);
    }
    for (const document of nextDocuments) {
      merged.set(`${document.type}:${document.path}`, document);
    }
    documents.value = Array.from(merged.values());
  }

  function getDirectoryPageCursor(
    type: KnowledgeDocumentType,
    path: string | null | undefined,
  ): string | null {
    const key = directoryPageStateKey(path);
    return directoryDocumentNextCursor.value[type][key] ?? null;
  }

  function setDirectoryPageCursor(
    type: KnowledgeDocumentType,
    path: string | null | undefined,
    cursor: string | null | undefined,
  ) {
    const key = directoryPageStateKey(path);
    directoryDocumentNextCursor.value = {
      ...directoryDocumentNextCursor.value,
      [type]: {
        ...directoryDocumentNextCursor.value[type],
        [key]: cursor ?? null,
      },
    };
  }

  function isDirectoryPageLoading(
    type: KnowledgeDocumentType,
    path: string | null | undefined,
  ): boolean {
    const key = directoryPageStateKey(path);
    return !!directoryDocumentLoading.value[type][key];
  }

  function setDirectoryPageLoading(
    type: KnowledgeDocumentType,
    path: string | null | undefined,
    loadingValue: boolean,
  ) {
    const key = directoryPageStateKey(path);
    directoryDocumentLoading.value = {
      ...directoryDocumentLoading.value,
      [type]: {
        ...directoryDocumentLoading.value[type],
        [key]: loadingValue,
      },
    };
  }

  function resetTypePagination(type: KnowledgeDocumentType) {
    directoryDocumentNextCursor.value = {
      ...directoryDocumentNextCursor.value,
      [type]: {},
    };
    directoryDocumentLoading.value = {
      ...directoryDocumentLoading.value,
      [type]: {},
    };
  }

  function markKnowledgeTypeDirty(
    type: KnowledgeDocumentType,
    options?: { documents?: boolean; directories?: boolean },
  ) {
    if (options?.documents !== false) {
      dirtyDocumentTypes.add(type);
    }
    if (options?.directories !== false) {
      dirtyDirectoryTypes.add(type);
    }
  }

  function clearSelectedDocumentState() {
    selectedDocument.value = null;
    selectedDocumentId.value = null;
    selectedDocumentLoading.value = false;
  }

  function clearSelectedDirectoryState() {
    selectedDirectoryPath.value = null;
    selectedDirectoryConfig.value = null;
    selectedDirectoryLoading.value = false;
  }

  async function refreshExternalDocumentPath(
    type: KnowledgeDocumentType,
    path: string,
    request: WorkspaceRequestSnapshot,
  ) {
    if (!isCurrentWorkspaceRequest(request)) return;
    const exactDocuments = (await knowledgeList({
      type,
      pathPrefix: path,
    })).filter((document) => document.path === path);
    if (!isCurrentWorkspaceRequest(request)) return;
    replaceDocumentPath(type, path, exactDocuments);
    loadedDocumentTypes.add(type);
    dirtyDocumentTypes.delete(type);

    if (selectedDocument.value?.type === type && selectedDocument.value.path === path) {
      if (exactDocuments.length === 0) {
        clearSelectedDocumentState();
      } else {
        await loadSelectedDocument(exactDocuments[0]);
      }
    }
  }

  async function refreshExternalDirectoryDocuments(
    type: KnowledgeDocumentType,
    directoryPath: string,
    request: WorkspaceRequestSnapshot,
  ) {
    if (!isCurrentWorkspaceRequest(request)) return;
    const normalizedDirectory = normalizeDirectorySelectionPath(directoryPath);
    const nextDocuments = await knowledgeListDirectoryDocuments(
      type,
      normalizedDirectory,
    );
    if (!isCurrentWorkspaceRequest(request)) return;
    replaceDirectoryDocuments(type, normalizedDirectory, nextDocuments);
    loadedDirectoryDocumentPaths[type].add(normalizedDirectory);
    loadedDocumentTypes.add(type);
    dirtyDocumentTypes.delete(type);
    setDirectoryPageCursor(type, normalizedDirectory, null);
  }

  async function refreshExternalSubtree(
    type: KnowledgeDocumentType,
    pathPrefix: string,
    request: WorkspaceRequestSnapshot,
  ) {
    if (!isCurrentWorkspaceRequest(request)) return;
    const normalizedPrefix = pathPrefix
      .trim()
      .replace(/\\/g, "/")
      .replace(/^\/+|\/+$/g, "");
    const nextDocuments = (await knowledgeList({
      type,
      pathPrefix: normalizedPrefix,
    })).filter((document) => pathMatchesSubtree(document.path, normalizedPrefix));
    if (!isCurrentWorkspaceRequest(request)) return;
    replaceDocumentSubtree(type, normalizedPrefix, nextDocuments);
    loadedDocumentTypes.add(type);
    dirtyDocumentTypes.delete(type);
  }

  async function applyExternalKnowledgeChange(
    change: KnowledgeChangedEvent,
    request: WorkspaceRequestSnapshot,
  ) {
    if (!isCurrentWorkspaceChange(change, request)) return;
    const type = change.docType;
    const targetKind = change.targetKind;
    const path = (change.path ?? "")
      .trim()
      .replace(/\\/g, "/")
      .replace(/^\/+|\/+$/g, "");
    const parentPath = normalizeDirectorySelectionPath(
      change.parentPath ?? parentDirectoryPath(path) ?? "",
    );
    const isStructureChange = change.changeKind === "structure";
    const isConfigChange = change.changeKind === "config";
    const affectsSubtree = !!change.subtree || isConfigChange;

    if (!type || !targetKind) {
      markKnowledgeDataDirty();
      await refreshKnowledgeData();
      return;
    }

    markKnowledgeTypeDirty(type, {
      directories: isStructureChange || isConfigChange || targetKind === "directory",
    });

    if (type !== activeType.value) {
      return;
    }

    if (targetKind === "document" && path) {
      if (isStructureChange && type === "reference") {
        await refreshExternalDirectoryDocuments(type, parentPath, request);
        if (!isCurrentWorkspaceRequest(request)) return;
        if (selectedDocument.value?.type === type && selectedDocument.value.path === path) {
          const selectedMatch = documents.value.find(
            (document) => document.type === type && document.path === path,
          );
          if (selectedMatch) {
            await loadSelectedDocument(selectedMatch);
          } else {
            clearSelectedDocumentState();
          }
        }
      } else {
        await refreshExternalDocumentPath(type, path, request);
      }
      if (!isCurrentWorkspaceRequest(request)) return;

      if (
        selectedDirectoryPath.value &&
        normalizeDirectorySelectionPath(selectedDirectoryPath.value) === parentPath &&
        (isStructureChange || isConfigChange)
      ) {
        await loadSelectedDirectoryConfig(selectedDirectoryPath.value);
        if (!isCurrentWorkspaceRequest(request)) return;
      }

      if (isStructureChange) {
        await loadDirectories(type);
        if (!isCurrentWorkspaceRequest(request)) return;
        dirtyDirectoryTypes.delete(type);
      }
      return;
    }

    if (targetKind === "directory" && path) {
      if (affectsSubtree) {
        await refreshExternalSubtree(type, path, request);
      } else {
        await refreshExternalDirectoryDocuments(type, path, request);
      }
      if (!isCurrentWorkspaceRequest(request)) return;
      await loadDirectories(type);
      if (!isCurrentWorkspaceRequest(request)) return;
      dirtyDirectoryTypes.delete(type);

      const normalizedSelectedDirectory = selectedDirectoryPath.value
        ? normalizeDirectorySelectionPath(selectedDirectoryPath.value)
        : "";
      if (
        normalizedSelectedDirectory &&
        pathMatchesSubtree(normalizedSelectedDirectory, path)
      ) {
        if (directoryGroups.value[type].includes(normalizedSelectedDirectory)) {
          await loadSelectedDirectoryConfig(selectedDirectoryPath.value);
          if (!isCurrentWorkspaceRequest(request)) return;
        } else {
          clearSelectedDirectoryState();
        }
      }

      if (
        selectedDocument.value?.type === type &&
        pathMatchesSubtree(selectedDocument.value.path, path)
      ) {
        const selectedMatch = documents.value.find(
          (document) =>
            document.type === type && document.path === selectedDocument.value?.path,
        );
        if (selectedMatch) {
          await loadSelectedDocument(selectedMatch);
          if (!isCurrentWorkspaceRequest(request)) return;
        } else {
          clearSelectedDocumentState();
        }
      } else if (
        selectedDocumentSummary.value?.type === type &&
        pathMatchesSubtree(selectedDocumentSummary.value.path, path)
      ) {
        await loadSelectedDocument(selectedDocumentSummary.value);
        if (!isCurrentWorkspaceRequest(request)) return;
      }
      return;
    }

    markKnowledgeDataDirty();
    await refreshKnowledgeData();
  }

  async function flushExternalRefreshQueue() {
    const queued = pendingExternalChanges;
    pendingExternalChanges = [];
    if (queued.length === 0) {
      await refreshKnowledgeData();
      return;
    }
    for (const item of queued) {
      if (!isCurrentWorkspaceChange(item.change, item.request)) continue;
      await applyExternalKnowledgeChange(item.change, item.request);
    }
  }

  function scheduleExternalRefresh(change?: KnowledgeChangedEvent) {
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    if (change) {
      if (!isCurrentWorkspaceChange(change, request)) return;
      pendingExternalChanges.push({ change, request });
    } else {
      markKnowledgeDataDirty();
    }
    if (externalRefreshTimer) {
      clearTimeout(externalRefreshTimer);
      externalRefreshTimer = null;
    }
    externalRefreshTimer = setTimeout(() => {
      externalRefreshTimer = null;
      void flushExternalRefreshQueue();
    }, 80);
  }

  function markKnowledgeDataDirty() {
    clearExternalRefreshQueue();
    dirtyDocumentTypes = new Set(DEFAULT_TYPE_ORDER);
    dirtyDirectoryTypes = new Set(DEFAULT_TYPE_ORDER);
  }

  function resetWorkspaceState() {
    workspaceRequestVersion += 1;
    clearExternalRefreshQueue();
    stopRetrievalStatusPoll();
    stopFeishuReferenceStatusPoll();
    stopUnityReferenceStatusPoll();
    documents.value = [];
    directoryGroups.value = emptyDirectoryGroups();
    rootDirectoryConfigs.value = emptyRootDirectoryConfigs();
    referenceExternalDirectorySources.value = {};
    referenceManagedDirectoryStats.value = {};
    loading.value = false;
    error.value = "";
    activeType.value = "design";
    expandedPaths.value = new Set(DEFAULT_TYPE_ORDER);
    selectedDocumentId.value = null;
    selectedDocument.value = null;
    selectedDocumentLoading.value = false;
    selectedDirectoryPath.value = null;
    selectedDirectoryConfig.value = null;
    selectedDirectoryLoading.value = false;
    savingDocument.value = false;
    pendingSaveCount = 0;
    creatingDocument.value = false;
    deletingDocument.value = false;
    searchQuery.value = "";
    resetSearchRuntimeState();
    overviewLoading.value = false;
    retrievalOverview.value = null;
    generalConfig.value = null;
    embeddingConfig.value = null;
    embeddingLocalModelCatalog.value = null;
    embeddingLocalDirectoryInspection.value = null;
    embeddingStatus.value = null;
    lexicalRebuildStatus.value = null;
    feishuReferenceImportStatus.value = null;
    unityReferenceImportStatus.value = null;
    retrievalActionPending.value = false;
    feishuReferenceImportPending.value = false;
    feishuReferenceDeletePending.value = false;
    unityReferenceImportPending.value = false;
    unityReferenceDeletePending.value = false;
    pendingSelectionPath.value = null;
    loadedDirectoryDocumentPaths = emptyLoadedDirectoryDocumentPaths();
    loadedDocumentTypes = new Set<KnowledgeDocumentType>();
    loadedDirectoryTypes = new Set<KnowledgeDocumentType>();
    dirtyDocumentTypes = new Set<KnowledgeDocumentType>();
    dirtyDirectoryTypes = new Set<KnowledgeDocumentType>();
    directoryDocumentNextCursor.value = emptyDirectoryPageCursorState();
    directoryDocumentLoading.value = emptyDirectoryPageLoadingState();
  }

  async function loadDirectoryDocumentPage(
    type: KnowledgeDocumentType,
    directoryPath: string,
    options?: {
      reset?: boolean;
      limit?: number;
      replaceTypeDocuments?: boolean;
      replaceDirectoryDocuments?: boolean;
      silent?: boolean;
    },
  ) {
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    const normalizedPath = normalizeDirectorySelectionPath(directoryPath);
    const reset = options?.reset ?? false;
    if (!reset && isDirectoryPageLoading(type, normalizedPath)) return;
    const currentCursor = reset
      ? null
      : getDirectoryPageCursor(type, normalizedPath);
    if (
      !reset &&
      currentCursor === null &&
      loadedDirectoryDocumentPaths[type].has(normalizedPath)
    ) {
      return;
    }

    setDirectoryPageLoading(type, normalizedPath, true);
    try {
      const page = await knowledgeListDirectoryDocumentsPage(
        type,
        normalizedPath,
        {
          cursor: currentCursor,
          limit: options?.limit ?? REFERENCE_DOCUMENT_PAGE_SIZE,
        },
      );
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      if (reset && options?.replaceTypeDocuments) {
        replaceTypeDocuments(type, page.items);
      } else if (reset && options?.replaceDirectoryDocuments) {
        replaceDirectoryDocuments(type, normalizedPath, page.items);
      } else if (page.items.length) {
        mergeDocuments(page.items);
      } else if (reset && options?.replaceTypeDocuments) {
        replaceTypeDocuments(type, []);
      } else if (reset && options?.replaceDirectoryDocuments) {
        replaceDirectoryDocuments(type, normalizedPath, []);
      }
      loadedDirectoryDocumentPaths[type].add(normalizedPath);
      setDirectoryPageCursor(type, normalizedPath, page.nextCursor ?? null);
      loadedDocumentTypes.add(type);
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      if (!options?.silent) {
        notifyError("knowledge_list_directory_documents_page", cause);
      }
      throw cause;
    } finally {
      if (isCurrentWorkspaceRequest(request)) {
        setDirectoryPageLoading(type, normalizedPath, false);
      }
    }
  }

  async function loadDocuments(input: KnowledgeDocumentListInput = {}) {
    if (!hasWorkspace.value) {
      resetWorkspaceState();
      return;
    }
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    loading.value = true;
    error.value = "";
    const normalizedPrefix = input.pathPrefix?.trim() ?? "";
    const hasTypeFilter = !!input.type;
    const hasPrefixFilter = !!normalizedPrefix;
    const isFullLoad = !hasTypeFilter && !hasPrefixFilter;
    const isTypeRootLoad = hasTypeFilter && !hasPrefixFilter;
    const isPagedReferenceRootLoad =
      isTypeRootLoad && input.type === "reference";
    try {
      if (isPagedReferenceRootLoad) {
        resetTypePagination("reference");
        loadedDirectoryDocumentPaths.reference = new Set<string>();
        await loadDirectoryDocumentPage("reference", "", {
          reset: true,
          replaceTypeDocuments: true,
          silent: true,
        });
      } else {
        const docs = await knowledgeList(input);
        if (!isCurrentWorkspaceRequest(request)) {
          return;
        }
        if (isFullLoad) {
          documents.value = docs;
          loadedDirectoryDocumentPaths = emptyLoadedDirectoryDocumentPaths();
          loadedDocumentTypes = new Set(DEFAULT_TYPE_ORDER);
          dirtyDocumentTypes = new Set<KnowledgeDocumentType>();
          directoryDocumentNextCursor.value = emptyDirectoryPageCursorState();
          directoryDocumentLoading.value = emptyDirectoryPageLoadingState();
        } else if (isTypeRootLoad && input.type) {
          replaceTypeDocuments(input.type, docs);
          loadedDirectoryDocumentPaths[input.type] = new Set<string>();
          resetTypePagination(input.type);
          loadedDocumentTypes.add(input.type);
          dirtyDocumentTypes.delete(input.type);
        } else {
          mergeDocuments(docs);
        }
      }
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      if (selectedDocumentId.value) {
        const stillExists = documents.value.some(
          (doc) => doc.id === selectedDocumentId.value,
        );
        if (!stillExists && !isPagedReferenceRootLoad) {
          selectedDocument.value = null;
          selectedDocumentId.value = null;
        }
      }
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      if (isFullLoad) {
        documents.value = [];
        loadedDirectoryDocumentPaths = emptyLoadedDirectoryDocumentPaths();
        loadedDocumentTypes = new Set<KnowledgeDocumentType>();
        directoryDocumentNextCursor.value = emptyDirectoryPageCursorState();
        directoryDocumentLoading.value = emptyDirectoryPageLoadingState();
      } else if (isTypeRootLoad && input.type) {
        documents.value = documents.value.filter(
          (document) => document.type !== input.type,
        );
        loadedDirectoryDocumentPaths[input.type] = new Set<string>();
        loadedDocumentTypes.delete(input.type);
        resetTypePagination(input.type);
      }
      notifyError("knowledge_list", cause);
    } finally {
      if (isCurrentWorkspaceRequest(request)) {
        loading.value = false;
      }
    }
  }

  async function ensureDirectoryDocumentsLoaded(
    type: KnowledgeDocumentType,
    directoryPath: string,
  ) {
    const normalizedPath = normalizeDirectorySelectionPath(directoryPath);
    if (!normalizedPath) return;
    if (loadedDirectoryDocumentPaths[type].has(normalizedPath)) return;
    try {
      await loadDirectoryDocumentPage(type, normalizedPath, {
        reset: true,
      });
    } catch {
      return;
    }
  }

  function hasLoadedDirectoryDocuments(
    type: KnowledgeDocumentType,
    directoryPath: string,
  ): boolean {
    const normalizedPath = normalizeDirectorySelectionPath(directoryPath);
    if (!normalizedPath) return false;
    return loadedDirectoryDocumentPaths[type].has(normalizedPath);
  }

  function hasMoreRootDocuments(type: KnowledgeDocumentType): boolean {
    return getDirectoryPageCursor(type, "") !== null;
  }

  function hasMoreDirectoryDocuments(
    type: KnowledgeDocumentType,
    directoryPath: string,
  ): boolean {
    return getDirectoryPageCursor(type, directoryPath) !== null;
  }

  function isRootDocumentsLoading(type: KnowledgeDocumentType): boolean {
    return isDirectoryPageLoading(type, "");
  }

  function isDirectoryDocumentsLoading(
    type: KnowledgeDocumentType,
    directoryPath: string,
  ): boolean {
    return isDirectoryPageLoading(type, directoryPath);
  }

  async function loadMoreRootDocuments(
    type: KnowledgeDocumentType = activeType.value,
  ) {
    if (getDirectoryPageCursor(type, "") === null) return;
    try {
      await loadDirectoryDocumentPage(type, "", {});
    } catch {
      return;
    }
  }

  async function loadMoreDirectoryDocuments(
    type: KnowledgeDocumentType,
    directoryPath: string,
  ) {
    if (getDirectoryPageCursor(type, directoryPath) === null) return;
    try {
      await loadDirectoryDocumentPage(type, directoryPath, {});
    } catch {
      return;
    }
  }

  async function loadDirectories(
    type: KnowledgeDocumentType = activeType.value,
  ) {
    if (!hasWorkspace.value) {
      directoryGroups.value = emptyDirectoryGroups();
      rootDirectoryConfigs.value = emptyRootDirectoryConfigs();
      referenceExternalDirectorySources.value = {};
      referenceManagedDirectoryStats.value = {};
      return;
    }

    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    try {
      const [paths, externalBindings, unityManagedStats] = await Promise.all([
        knowledgeListDirectories(type),
        type === "reference"
          ? knowledgeListExternalReferenceDirectories()
          : Promise.resolve([]),
        type === "reference"
          ? knowledgeListUnityManagedDirectoryStats()
          : Promise.resolve([]),
      ]);
      const nextPaths = [...paths].sort((left, right) =>
        left.localeCompare(right, undefined, { sensitivity: "base" }),
      );
      const rootPaths = nextPaths.filter(isRootDirectoryPath);
      const settled = await Promise.allSettled(
        rootPaths.map(async (path) => {
          const result = await knowledgeRead({
            kind: "directory",
            path,
            type,
          });
          return result.directory;
        }),
      );
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      const configs = settled.reduce<
        Record<string, KnowledgeDirectoryConfigRecord>
      >((acc, entry) => {
        if (entry.status !== "fulfilled" || !entry.value) return acc;
        acc[entry.value.path] = entry.value;
        return acc;
      }, {});
      directoryGroups.value = {
        ...directoryGroups.value,
        [type]: nextPaths,
      };
      rootDirectoryConfigs.value = {
        ...rootDirectoryConfigs.value,
        [type]: configs,
      };
      loadedDirectoryTypes.add(type);
      dirtyDirectoryTypes.delete(type);
      if (type === "reference") {
        referenceExternalDirectorySources.value = externalBindings.reduce<
          Record<string, KnowledgeExternalSource[]>
        >((acc, binding) => {
          acc[binding.path] = binding.externalSources;
          return acc;
        }, {});
        referenceManagedDirectoryStats.value = unityManagedStats.reduce<
          Record<string, KnowledgeFolderDisplayStats>
        >((acc, stat) => {
          acc[fullDocumentPath("reference", stat.path)] = {
            directChildCount: stat.directChildCount,
            descendantDocumentCount: stat.descendantDocumentCount,
          };
          return acc;
        }, {});
      }
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      directoryGroups.value = {
        ...directoryGroups.value,
        [type]: [],
      };
      rootDirectoryConfigs.value = {
        ...rootDirectoryConfigs.value,
        [type]: {},
      };
      loadedDirectoryTypes.delete(type);
      if (type === "reference") {
        referenceExternalDirectorySources.value = {};
        referenceManagedDirectoryStats.value = {};
      }
      notifyError("knowledge_list_directories", cause);
    }
  }

  async function loadOverview() {
    if (!hasWorkspace.value) {
      retrievalOverview.value = null;
      generalConfig.value = null;
      embeddingConfig.value = null;
      embeddingLocalModelCatalog.value = null;
      embeddingLocalDirectoryInspection.value = null;
      embeddingStatus.value = null;
      lexicalRebuildStatus.value = null;
      feishuReferenceImportStatus.value = null;
      overviewLoading.value = false;
      return;
    }

    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    overviewLoading.value = true;
    try {
      const [
        nextOverview,
        nextGeneralConfig,
        nextEmbeddingConfig,
        nextEmbeddingStatus,
        nextLexicalStatus,
        nextEmbeddingLocalCatalog,
        nextFeishuReferenceImportStatus,
        nextUnityReferenceImportStatus,
      ] = await Promise.all([
        knowledgeGetOverview(),
        knowledgeGetGeneralConfig(),
        knowledgeGetEmbeddingConfig(),
        knowledgeGetEmbeddingStatus(),
        knowledgeGetLexicalRebuildStatus(),
        knowledgeGetLocalEmbeddingModelCatalog(),
        knowledgeGetFeishuReferenceImportStatus(),
        knowledgeGetUnityReferenceImportStatus(),
      ]);
      const nextDirectoryInspection = nextEmbeddingConfig.localModelPath.trim()
        ? await knowledgeInspectLocalEmbeddingModelDirectory(
            nextEmbeddingConfig.localModelPath,
          )
        : embeddingLocalDirectoryInspection.value;
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      retrievalOverview.value = nextOverview;
      generalConfig.value = nextGeneralConfig;
      embeddingConfig.value = nextEmbeddingConfig;
      embeddingLocalModelCatalog.value = nextEmbeddingLocalCatalog;
      embeddingLocalDirectoryInspection.value = nextDirectoryInspection;
      embeddingStatus.value = nextEmbeddingStatus;
      lexicalRebuildStatus.value = nextLexicalStatus;
      feishuReferenceImportStatus.value = nextFeishuReferenceImportStatus;
      feishuReferenceImportPending.value =
        nextFeishuReferenceImportStatus.running;
      unityReferenceImportStatus.value = nextUnityReferenceImportStatus;
      unityReferenceImportPending.value =
        nextUnityReferenceImportStatus.running;
      if (nextEmbeddingStatus.activating) {
        scheduleRetrievalStatusPoll();
      } else {
        stopRetrievalStatusPoll();
      }
      if (
        nextFeishuReferenceImportStatus.running ||
        nextFeishuReferenceImportStatus.stage === "authorizing"
      ) {
        scheduleFeishuReferenceStatusPoll();
      } else {
        stopFeishuReferenceStatusPoll();
      }
      if (nextUnityReferenceImportStatus.running) {
        scheduleUnityReferenceStatusPoll();
      } else {
        stopUnityReferenceStatusPoll();
      }
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      notifyError("knowledge_get_overview", cause);
      retrievalOverview.value = null;
      stopRetrievalStatusPoll();
      stopUnityReferenceStatusPoll();
    } finally {
      if (isCurrentWorkspaceRequest(request)) {
        overviewLoading.value = false;
      }
    }
  }

  async function loadSelectedDocument(
    target?: KnowledgeDocumentSummary | KnowledgeDocument | null,
  ) {
    if (!hasWorkspace.value) return;
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    const refTarget = target ?? selectedDocumentSummary.value;
    if (!refTarget) {
      selectedDocument.value = null;
      selectedDocumentId.value = null;
      return;
    }

    selectedDocumentLoading.value = true;
    error.value = "";
    try {
      const result = await knowledgeRead({
        kind: "document",
        path: refTarget.path,
        type: refTarget.type,
        part: "full",
      });
      const doc = result.document;
      if (!doc) throw new Error("knowledge_read returned no document");
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      selectedDocument.value = doc;
      selectedDocumentId.value = doc.id;
      activeType.value = doc.type;
      mergeDocuments([doc]);
      if (doc.type === "reference") {
        const parentPath = parentDirectoryPath(doc.path);
        if (parentPath) {
          await ensureDirectoryDocumentsLoaded(doc.type, parentPath);
        }
      }
      expandAncestors(fullDocumentPath(doc.type, doc.path));
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      notifyError("knowledge_read", cause);
    } finally {
      if (isCurrentWorkspaceRequest(request)) {
        selectedDocumentLoading.value = false;
      }
    }
  }

  async function loadSelectedDirectoryConfig(targetPath?: string | null) {
    if (!hasWorkspace.value) return;
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    const refPath = normalizeDirectorySelectionPath(
      targetPath ?? selectedDirectoryPath.value ?? "",
    );
    if (!refPath) {
      selectedDirectoryPath.value = null;
      selectedDirectoryConfig.value = null;
      return;
    }

    selectedDirectoryLoading.value = true;
    error.value = "";
    try {
      const result = await knowledgeRead({
        kind: "directory",
        path: refPath,
        type: activeType.value,
      });
      const config = result.directory;
      if (!config)
        throw new Error("knowledge_read returned no directory config");
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      selectedDirectoryPath.value = config.path;
      selectedDirectoryConfig.value = config;
      expandAncestors(fullDocumentPath(config.type, config.path));
    } catch (cause) {
      if (!isCurrentWorkspaceRequest(request)) {
        return;
      }
      notifyError("knowledge_read.directory", cause);
    } finally {
      if (isCurrentWorkspaceRequest(request)) {
        selectedDirectoryLoading.value = false;
      }
    }
  }

  async function ensureTypeDataLoaded(
    type: KnowledgeDocumentType,
    options?: { force?: boolean; includeOverview?: boolean },
  ) {
    const force = options?.force ?? false;
    const shouldLoadDocuments =
      force || dirtyDocumentTypes.has(type) || !loadedDocumentTypes.has(type);
    const shouldLoadDirectories =
      force || dirtyDirectoryTypes.has(type) || !loadedDirectoryTypes.has(type);
    await Promise.all([
      shouldLoadDocuments ? loadDocuments({ type }) : Promise.resolve(),
      shouldLoadDirectories ? loadDirectories(type) : Promise.resolve(),
      options?.includeOverview ? loadOverview() : Promise.resolve(),
    ]);
  }

  async function refreshKnowledgeData(options?: {
    force?: boolean;
    includeOverview?: boolean;
  }) {
    if (!hasWorkspace.value) {
      resetWorkspaceState();
      return;
    }

    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    const force = options?.force ?? true;
    const includeOverview = options?.includeOverview ?? true;
    consumePendingUiSelection();
    await ensureTypeDataLoaded(activeType.value, {
      force,
      includeOverview,
    });
    if (!isCurrentWorkspaceRequest(request)) {
      return;
    }
    if (pendingSelectionPath.value) {
      const pendingPath = pendingSelectionPath.value;
      const next = documents.value.find((doc) => doc.path === pendingPath);
      if (next) {
        pendingSelectionPath.value = null;
        await loadSelectedDocument(next);
      } else {
        await loadSelectedDocument({
          id: pendingPath,
          path: pendingPath,
          title: pendingPath.split("/").pop() ?? pendingPath,
          type: activeType.value,
          injectMode: "excerpt",
          summaryEnabled: defaultSummaryEnabledForType(activeType.value),
          commandEnabled: false,
          readOnly: activeType.value === "reference",
          aiMaintained: false,
          explicitMaintenanceRules: false,
          summary: null,
          createdAt: 0,
          updatedAt: 0,
          hasSummary: false,
        });
        pendingSelectionPath.value = null;
      }
    } else if (selectedDocumentId.value) {
      await loadSelectedDocument();
    } else if (selectedDirectoryPath.value) {
      const stillExists = directoryGroups.value[activeType.value].includes(
        selectedDirectoryPath.value,
      );
      if (stillExists) {
        await loadSelectedDirectoryConfig();
      } else {
        selectedDirectoryPath.value = null;
        selectedDirectoryConfig.value = null;
      }
    }
  }

  function isPathExpanded(path: string): boolean {
    return expandedPaths.value.has(path);
  }

  async function togglePath(path: string) {
    const expanding = !expandedPaths.value.has(path);
    const next = new Set(expandedPaths.value);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    expandedPaths.value = next;
    if (!expanding) return;
    const type = inferSelectionType(path);
    if (!type) return;
    const relativePath = relativeSelectionPath(path);
    if (!relativePath || relativePath === path) return;
    // Root warmup data can be partial, so folder expansion should hydrate child
    // documents for every knowledge type, not only managed references.
    await ensureDirectoryDocumentsLoaded(type, relativePath);
  }

  function expandPath(path: string) {
    if (expandedPaths.value.has(path)) return;
    const next = new Set(expandedPaths.value);
    next.add(path);
    expandedPaths.value = next;
  }

  function expandAncestors(path: string) {
    const normalized = path.replace(/\\/g, "/");
    const parts = normalized.split("/");
    let current = "";
    for (let index = 0; index < parts.length - 1; index += 1) {
      current = current ? `${current}/${parts[index]}` : parts[index]!;
      expandPath(current);
    }
  }

  function collapseBranch(path: string) {
    const prefix = `${path}/`;
    expandedPaths.value = new Set(
      Array.from(expandedPaths.value).filter(
        (entry) => entry !== path && !entry.startsWith(prefix),
      ),
    );
  }

  function pruneUnityReferenceManagedEntries() {
    documents.value = documents.value.filter(
      (doc) =>
        !(doc.type === "reference" && isUnityReferenceManagedPath(doc.path)),
    );
    directoryGroups.value = {
      ...directoryGroups.value,
      reference: directoryGroups.value.reference.filter(
        (path) => !isUnityReferenceManagedPath(path),
      ),
    };
    referenceManagedDirectoryStats.value = Object.fromEntries(
      Object.entries(referenceManagedDirectoryStats.value).filter(
        ([path]) => !isUnityReferenceManagedPath(relativeSelectionPath(path)),
      ),
    );

    if (
      selectedDocument.value?.type === "reference" &&
      isUnityReferenceManagedPath(selectedDocument.value.path)
    ) {
      selectedDocument.value = null;
      selectedDocumentId.value = null;
    }

    if (isUnityReferenceManagedPath(selectedDirectoryPath.value)) {
      selectedDirectoryPath.value = null;
      selectedDirectoryConfig.value = null;
      selectedDirectoryLoading.value = false;
    }

    if (isUnityReferenceManagedPath(pendingSelectionPath.value)) {
      pendingSelectionPath.value = null;
    }

    collapseBranch(fullDocumentPath("reference", UNITY_REFERENCE_MANAGED_DIR));
  }

  function pruneFeishuReferenceManagedEntries() {
    documents.value = documents.value.filter(
      (doc) =>
        !(doc.type === "reference" && isFeishuReferenceManagedPath(doc.path)),
    );
    directoryGroups.value = {
      ...directoryGroups.value,
      reference: directoryGroups.value.reference.filter(
        (path) => !isFeishuReferenceManagedPath(path),
      ),
    };

    if (
      selectedDocument.value?.type === "reference" &&
      isFeishuReferenceManagedPath(selectedDocument.value.path)
    ) {
      selectedDocument.value = null;
      selectedDocumentId.value = null;
    }

    if (isFeishuReferenceManagedPath(selectedDirectoryPath.value)) {
      selectedDirectoryPath.value = null;
      selectedDirectoryConfig.value = null;
      selectedDirectoryLoading.value = false;
    }

    if (isFeishuReferenceManagedPath(pendingSelectionPath.value)) {
      pendingSelectionPath.value = null;
    }

    collapseBranch(fullDocumentPath("reference", FEISHU_REFERENCE_MANAGED_DIR));
  }

  async function selectType(type: KnowledgeDocumentType) {
    activeType.value = type;
    clearSelection();
    await ensureTypeDataLoaded(type);
  }

  function buildSearchSelectionContext(
    result: KnowledgeSearchResult,
  ): KnowledgeSearchSelectionContext {
    return {
      query: searchQuery.value.trim(),
      result: { ...result },
    };
  }

  async function selectDocument(
    summary: KnowledgeDocumentSummary,
    options?: { searchContext?: KnowledgeSearchSelectionContext | null },
  ) {
    activeType.value = summary.type;
    selectedSearchContext.value = options?.searchContext ?? null;
    selectedDirectoryPath.value = null;
    selectedDirectoryConfig.value = null;
    selectedDirectoryLoading.value = false;
    selectedDocumentId.value = summary.id;
    pendingSelectionPath.value = summary.path;
    await loadSelectedDocument(summary);
  }

  async function selectDirectory(path: string) {
    const normalized = normalizeDirectorySelectionPath(path);
    if (!normalized) return;
    selectedSearchContext.value = null;
    selectedDocumentId.value = null;
    selectedDocument.value = null;
    selectedDocumentLoading.value = false;
    pendingSelectionPath.value = null;
    selectedDirectoryPath.value = normalized;
    await loadSelectedDirectoryConfig(normalized);
  }

  async function selectSearchResult(result: KnowledgeSearchResult) {
    const searchContext = buildSearchSelectionContext(result);
    const matched = documents.value.find(
      (doc) => doc.id === result.id || doc.path === result.path,
    );
    if (matched) {
      await selectDocument(matched, { searchContext });
      return;
    }
    activeType.value = result.type;
    selectedSearchContext.value = searchContext;
    selectedDocumentId.value = result.id;
    pendingSelectionPath.value = result.path;
    await Promise.all([
      ensureTypeDataLoaded(result.type),
      loadSelectedDocument({
        id: result.id,
        path: result.path,
        title: result.title,
        type: result.type,
        storageSource: result.storageSource ?? "project",
        injectMode: result.injectMode,
        summaryEnabled: defaultSummaryEnabledForType(result.type),
        commandEnabled: false,
        readOnly: false,
        aiMaintained: result.aiMaintained,
        explicitMaintenanceRules: false,
        summary: null,
        createdAt: 0,
        updatedAt: result.updatedAt ?? 0,
        hasSummary: false,
      }),
    ]);
  }

  function clearSelection() {
    selectedSearchContext.value = null;
    selectedDocumentId.value = null;
    selectedDocument.value = null;
    selectedDocumentLoading.value = false;
    selectedDirectoryPath.value = null;
    selectedDirectoryConfig.value = null;
    selectedDirectoryLoading.value = false;
  }

  async function searchKnowledge() {
    if (!hasWorkspace.value) {
      resetSearchRuntimeState();
      return;
    }
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    const query = searchQuery.value.trim();
    if (!query) {
      resetSearchRuntimeState();
      return;
    }

    const seq = ++searchSeq;
    const startedAt = Date.now();
    searching.value = true;
    try {
      const results = await knowledgeQuery({
        query,
        limit: 30,
      });
      if (seq !== searchSeq || !isCurrentWorkspaceRequest(request)) return;
      searchResults.value = results;
      searchLatencyMs.value = Math.max(1, Date.now() - startedAt);
      searchMode.value = deriveSearchMode(results);
      recentQueryTokens.value = results
        .slice(0, 3)
        .reduce((total, result) => total + (result.estimatedTokens ?? 0), 0);
    } catch (cause) {
      if (seq !== searchSeq || !isCurrentWorkspaceRequest(request)) return;
      notifyError("knowledge_query", cause);
      searchResults.value = [];
      searchLatencyMs.value = null;
      searchMode.value = null;
      recentQueryTokens.value = null;
    } finally {
      if (seq === searchSeq && isCurrentWorkspaceRequest(request)) {
        searching.value = false;
      }
    }
  }

  function scheduleSearch() {
    clearSearchTimer();
    if (!searchQuery.value.trim()) {
      resetSearchRuntimeState();
      return;
    }
    const request = captureWorkspaceRequest();
    if (!request.workspaceKey) return;
    searchTimer = setTimeout(() => {
      searchTimer = null;
      if (!isCurrentWorkspaceRequest(request)) return;
      void searchKnowledge();
    }, 220);
  }

  function clearSearch() {
    resetSearchRuntimeState();
    selectedSearchContext.value = null;
    searchQuery.value = "";
  }

  async function refreshRetrievalState() {
    await loadOverview();
  }

  async function saveEmbeddingConfigPatch(
    patch: Partial<EmbeddingConfig>,
    options?: { onError?: (cause: unknown) => void },
  ) {
    if (!embeddingConfig.value) return;
    retrievalActionPending.value = true;
    error.value = "";
    scheduleRetrievalStatusPoll(120);
    try {
      embeddingConfig.value = await knowledgeSaveEmbeddingConfig({
        ...embeddingConfig.value,
        ...patch,
      });
      await loadOverview();
    } catch (cause) {
      (
        options?.onError ??
        ((errorCause: unknown) =>
          notifyError("knowledge_save_embedding_config", errorCause))
      )(cause);
    } finally {
      retrievalActionPending.value = false;
      if (!embeddingStatus.value?.activating) {
        stopRetrievalStatusPoll();
      }
    }
  }

  async function setEmbeddingDevicePolicy(devicePolicy: string) {
    if (
      !embeddingConfig.value ||
      embeddingConfig.value.embeddingMode === "remote"
    )
      return;
    const runtimeRoute = embeddingStatus.value?.ready
      ? retrievalOverview.value?.semantic.deviceRoute?.trim().toLowerCase() ||
        ""
      : "";
    const runtimeDevicePolicy =
      runtimeRoute === "directml"
        ? "gpu_directml"
        : runtimeRoute === "cuda"
          ? "gpu_cuda"
          : runtimeRoute === "cpu" || runtimeRoute.startsWith("cpu")
            ? "cpu_fastembed"
            : "";
    if (
      embeddingConfig.value.devicePolicy === devicePolicy &&
      runtimeDevicePolicy === devicePolicy
    ) {
      return;
    }
    await saveEmbeddingConfigPatch(
      { devicePolicy },
      { onError: notifyEmbeddingRuntimeError },
    );
  }

  async function setEmbeddingDownloadSource(downloadSource: string) {
    if (
      !embeddingConfig.value ||
      embeddingConfig.value.embeddingMode === "remote"
    )
      return;
    const normalizedSource = downloadSource.trim().toLowerCase().replace(/_/g, "-") === "hf-mirror"
      ? "hf-mirror"
      : "official";
    if (
      embeddingConfig.value.localModelDownloadSource === normalizedSource
    ) {
      return;
    }
    await saveEmbeddingConfigPatch({
      localModelDownloadSource: normalizedSource,
    });
  }

  async function saveGeneralConfigPatch(
    patch: Partial<KnowledgeGeneralConfig>,
  ) {
    if (!generalConfig.value) return;
    retrievalActionPending.value = true;
    error.value = "";
    try {
      generalConfig.value = await knowledgeSaveGeneralConfig({
        ...generalConfig.value,
        ...patch,
      });
      await loadOverview();
    } catch (cause) {
      notifyError("knowledge_save_general_config", cause);
    } finally {
      retrievalActionPending.value = false;
    }
  }

  async function setSemanticSearchEnabled(enabled: boolean) {
    if (!generalConfig.value || !embeddingConfig.value) return;
    retrievalActionPending.value = true;
    error.value = "";
    scheduleRetrievalStatusPoll(120);
    try {
      generalConfig.value = await knowledgeSaveGeneralConfig({
        ...generalConfig.value,
        semanticSearchEnabled: enabled,
      });
      embeddingConfig.value = await knowledgeSaveEmbeddingConfig({
        ...embeddingConfig.value,
        enabled,
      });
      await loadOverview();
    } catch (cause) {
      notifyError("knowledge_save_embedding_config", cause);
    } finally {
      retrievalActionPending.value = false;
      if (!embeddingStatus.value?.activating) {
        stopRetrievalStatusPoll();
      }
    }
  }

  async function activateSemanticRuntime() {
    retrievalActionPending.value = true;
    error.value = "";
    scheduleRetrievalStatusPoll(120);
    try {
      await knowledgeActivateEmbedding();
      await loadOverview();
    } catch (cause) {
      notifyEmbeddingRuntimeError(cause);
    } finally {
      retrievalActionPending.value = false;
      if (!embeddingStatus.value?.activating) {
        stopRetrievalStatusPoll();
      }
    }
  }

  async function deactivateSemanticRuntime() {
    retrievalActionPending.value = true;
    error.value = "";
    try {
      await knowledgeDeactivateEmbedding();
      await loadOverview();
    } catch (cause) {
      notifyError("knowledge_deactivate_embedding", cause);
    } finally {
      retrievalActionPending.value = false;
    }
  }

  async function rebuildLexicalIndex() {
    retrievalActionPending.value = true;
    error.value = "";
    try {
      await knowledgeRebuildLexicalIndex();
      await loadOverview();
    } catch (cause) {
      notifyError("knowledge_rebuild_lexical_index", cause);
    } finally {
      retrievalActionPending.value = false;
      if (!embeddingStatus.value?.activating) {
        stopRetrievalStatusPoll();
      }
    }
  }

  async function setLocalEmbeddingModelPreset(modelId: string) {
    if (
      !embeddingConfig.value ||
      embeddingConfig.value.embeddingMode === "remote"
    )
      return;
    if (
      embeddingConfig.value.localModel === modelId &&
      !embeddingConfig.value.localModelPath.trim()
    ) {
      return;
    }
    await saveEmbeddingConfigPatch({
      embeddingMode: "local",
      localModel: modelId,
      localModelPath: "",
    });
  }

  async function selectLocalEmbeddingModelOption(optionValue: string) {
    if (!optionValue) return;
    if (optionValue.startsWith("preset:")) {
      await setLocalEmbeddingModelPreset(optionValue.slice("preset:".length));
      return;
    }
    if (!optionValue.startsWith("directory:")) return;
    const targetPath = optionValue.slice("directory:".length);
    const selectedModel =
      embeddingLocalModelCatalog.value?.availableModels.find(
        (model) => model.localModelPath === targetPath,
      );
    if (!selectedModel) return;
    await saveEmbeddingConfigPatch({
      embeddingMode: "local",
      localModel: selectedModel.modelId,
      localModelPath: selectedModel.localModelPath,
    });
  }

  async function browseLocalEmbeddingModelDirectory() {
    if (!embeddingConfig.value) return;
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath:
          embeddingConfig.value.localModelPath || props.workingDir || undefined,
      });
      if (!selected || typeof selected !== "string") return;
      const inspection =
        await knowledgeInspectLocalEmbeddingModelDirectory(selected);
      embeddingLocalDirectoryInspection.value = inspection;
      await saveEmbeddingConfigPatch({
        embeddingMode: "local",
        localModel: inspection.label,
        localModelPath: inspection.path,
      });
    } catch (cause) {
      notifyError("knowledge_pick_local_embedding_directory", cause);
    }
  }

  async function downloadSelectedLocalEmbeddingModel(modelId?: string) {
    const targetModelId = (
      modelId ??
      embeddingConfig.value?.localModel ??
      ""
    ).trim();
    if (!targetModelId) return;

    retrievalActionPending.value = true;
    error.value = "";
    scheduleRetrievalStatusPoll(120);
    try {
      try {
        await openKnowledgeDownloadProgressWindow(targetModelId);
      } catch (cause) {
        console.error(
          "Failed to open knowledge download progress window:",
          cause,
        );
      }
      await knowledgeDownloadLocalEmbeddingModel(targetModelId);
      await loadOverview();
      notificationStore.addNotice(
        "success",
        t("knowledge.retrieval.modelDownloadedNotice", targetModelId),
        {
          operation: "knowledge_download_local_embedding_model",
          replaceOperation: true,
        },
      );
    } catch (cause) {
      const err = normalizeAppError(cause);
      if (err.code === "knowledge.embedding_model_download_cancelled") {
        error.value = "";
        notificationStore.addNotice(
          "info",
          t("knowledge.retrieval.modelDownloadCancelledNotice"),
          {
            code: err.code,
            operation: "knowledge_download_local_embedding_model",
            replaceOperation: true,
          },
        );
        await loadOverview();
        return;
      }
      error.value = err.message;
      notificationStore.addNotice(
        "error",
        t("knowledge.retrieval.modelDownloadFailed", err.message),
        {
          code: err.code,
          operation: "knowledge_download_local_embedding_model",
          replaceOperation: true,
        },
      );
    } finally {
      retrievalActionPending.value = false;
      if (!embeddingStatus.value?.activating) {
        stopRetrievalStatusPoll();
      }
    }
  }

  async function importFeishuReferenceDocs(targetPath?: string | null) {
    if (!hasWorkspace.value) return;
    error.value = "";
    try {
      const normalizedTargetPath = normalizeDirectorySelectionPath(
        targetPath ?? "",
      );
      if (normalizedTargetPath) {
        await openFeishuReferenceImportProgressWindow({
          targetPath: normalizedTargetPath,
        });
        return;
      }
      const status = await knowledgeGetFeishuReferenceImportStatus();
      feishuReferenceImportStatus.value = status;
      feishuReferenceImportPending.value = status.running;
      if (status.running || status.stage === "authorizing") {
        scheduleFeishuReferenceStatusPoll(280);
      } else {
        stopFeishuReferenceStatusPoll();
      }
      await openFeishuReferenceImportProgressWindow();
    } catch (cause) {
      notifyError("knowledge_open_feishu_reference_import_window", cause);
    }
  }

  async function deleteFeishuReferenceDocs(targetPath?: string | null) {
    if (
      !hasWorkspace.value ||
      feishuReferenceImportPending.value ||
      feishuReferenceDeletePending.value
    ) {
      return;
    }

    feishuReferenceDeletePending.value = true;
    error.value = "";
    try {
      const normalizedTargetPath = normalizeDirectorySelectionPath(
        targetPath ?? "",
      );
      const status = await knowledgeDeleteFeishuReferenceDocs(
        normalizedTargetPath || undefined,
      );
      feishuReferenceImportStatus.value = status;
      feishuReferenceImportPending.value = status.running;
      if (!normalizedTargetPath) {
        pruneFeishuReferenceManagedEntries();
      }
      await refreshKnowledgeData();
      notificationStore.addNotice(
        "success",
        t("knowledge.feishuReference.deletedNotice"),
        {
          operation: "knowledge_delete_feishu_reference_docs",
          replaceOperation: true,
        },
      );
    } catch (cause) {
      notifyError("knowledge_delete_feishu_reference_docs", cause);
    } finally {
      feishuReferenceDeletePending.value = false;
    }
  }

  async function importUnityReferenceDocs(targetPath?: string | null) {
    if (!hasWorkspace.value) return;
    error.value = "";
    try {
      const normalizedTargetPath =
        normalizeUnityImportWindowTargetPath(targetPath);
      if (normalizedTargetPath) {
        await openUnityReferenceImportProgressWindow({
          targetPath: normalizedTargetPath,
        });
        return;
      }
      const status = await knowledgeGetUnityReferenceImportStatus();
      unityReferenceImportStatus.value = status;
      unityReferenceImportPending.value = status.running;
      if (status.running) {
        scheduleUnityReferenceStatusPoll(280);
      } else {
        stopUnityReferenceStatusPoll();
      }
      await openUnityReferenceImportProgressWindow();
    } catch (cause) {
      notifyError("knowledge_open_unity_reference_import_window", cause);
    }
  }

  async function deleteUnityReferenceDocs(targetPath?: string | null) {
    if (
      !hasWorkspace.value ||
      unityReferenceImportPending.value ||
      unityReferenceDeletePending.value
    ) {
      return;
    }

    unityReferenceDeletePending.value = true;
    error.value = "";
    try {
      const normalizedTargetPath = normalizeDirectorySelectionPath(
        targetPath ?? "",
      );
      const status = await knowledgeDeleteUnityReferenceDocs(
        normalizedTargetPath || undefined,
      );
      unityReferenceImportStatus.value = status;
      unityReferenceImportPending.value = status.running;
      if (!normalizedTargetPath) {
        pruneUnityReferenceManagedEntries();
      }
      await refreshKnowledgeData();
      notificationStore.addNotice(
        "success",
        t("knowledge.referenceImport.deletedNotice"),
        {
          operation: "knowledge_delete_unity_reference_docs",
          replaceOperation: true,
        },
      );
    } catch (cause) {
      notifyError("knowledge_delete_unity_reference_docs", cause);
    } finally {
      unityReferenceDeletePending.value = false;
    }
  }

  async function createDocument(title: string, parentDir = "") {
    if (!hasWorkspace.value) return;
    const trimmed = title.trim();
    if (!trimmed) return;
    creatingDocument.value = true;
    error.value = "";
    try {
      const relativeDir = parentDir
        .trim()
        .replace(/\\/g, "/")
        .replace(/^\/+|\/+$/g, "");
      const slug = slugifyKnowledgePath(trimmed);
      const filePath = buildCreatePath(activeType.value, slug, relativeDir);
      const defaults = buildKnowledgeCreateDefaults(activeType.value);
      const result = await enqueueMutation(() =>
        knowledgeCreate({
          kind: "document",
          type: activeType.value,
          path: filePath,
          document: {
            title: trimmed.replace(/\.md$/i, ""),
            body: "",
            inheritInjectMode: defaults.inheritInjectMode,
            summaryEnabled: defaults.summaryEnabled,
            skillEnabled: activeType.value === "skill" ? true : undefined,
            skillSurface: activeType.value === "skill" ? "command" : undefined,
            commandTrigger:
              activeType.value === "skill" ? `/${slug}` : undefined,
            readOnly: defaults.readOnly,
            inheritAiConfig: defaults.inheritAiConfig,
          },
        }),
      );
      const doc = result.document;
      if (!doc) throw new Error("knowledge_create returned no document");
      await refreshKnowledgeData();
      await selectDocument(doc);
      expandAncestors(fullDocumentPath(doc.type, doc.path));
    } catch (cause) {
      notifyError("knowledge_create.document", cause);
    } finally {
      creatingDocument.value = false;
    }
  }

  async function createDocumentAt(parentDir: string, name: string) {
    if (!hasWorkspace.value) return;
    const trimmed = name.trim();
    if (!trimmed) return;
    const normalizedParent = parentDir
      .trim()
      .replace(/\\/g, "/")
      .replace(/^\/+|\/+$/g, "");
    const slug =
      activeType.value === "skill"
        ? slugifyKnowledgePath(
            trimmed.replace(/\/SKILL\.md$/i, "").replace(/\.md$/i, ""),
          )
        : trimmed;
    const pathName = buildCreatePath(activeType.value, slug, normalizedParent);
    creatingDocument.value = true;
    error.value = "";
    try {
      const defaults = buildKnowledgeCreateDefaults(activeType.value);
      const result = await enqueueMutation(() =>
        knowledgeCreate({
          kind: "document",
          type: activeType.value,
          path: pathName,
          document: {
            title: trimmed.replace(/\.md$/i, ""),
            body: "",
            inheritInjectMode: defaults.inheritInjectMode,
            summaryEnabled: defaults.summaryEnabled,
            skillEnabled: activeType.value === "skill" ? true : undefined,
            skillSurface: activeType.value === "skill" ? "command" : undefined,
            commandTrigger:
              activeType.value === "skill" ? `/${slug}` : undefined,
            readOnly: defaults.readOnly,
            inheritAiConfig: defaults.inheritAiConfig,
          },
        }),
      );
      const doc = result.document;
      if (!doc) throw new Error("knowledge_create returned no document");
      await refreshKnowledgeData();
      await selectDocument(doc);
      expandAncestors(fullDocumentPath(doc.type, doc.path));
    } catch (cause) {
      notifyError("knowledge_create.document", cause);
    } finally {
      creatingDocument.value = false;
    }
  }

  async function createFolder(parentDir: string, name: string) {
    if (!hasWorkspace.value) return;
    const trimmed = name.trim();
    if (!trimmed) return;
    creatingDocument.value = true;
    error.value = "";
    try {
      const normalizedParent = parentDir
        .trim()
        .replace(/\\/g, "/")
        .replace(/^\/+|\/+$/g, "");
      const path = normalizedParent
        ? `${normalizedParent}/${trimmed}`
        : trimmed;
      await enqueueMutation(() =>
        knowledgeCreate({
          kind: "directory",
          type: activeType.value,
          path,
        }),
      );
      await refreshKnowledgeData();
      const fullPath = fullDocumentPath(activeType.value, path);
      expandAncestors(fullPath);
      expandPath(fullPath);
    } catch (cause) {
      notifyError("knowledge_create.directory", cause);
    } finally {
      creatingDocument.value = false;
    }
  }

  async function updateSection(
    id: string,
    path: string,
    section: KnowledgeDocumentSection,
    content: string,
  ) {
    if (!hasWorkspace.value) return;
    beginSave();
    error.value = "";
    try {
      const updated = await enqueueMutation(async () => {
        const result = await knowledgeEdit({
          kind: "document",
          type: activeType.value,
          path,
          document: {
            id,
            summary: section === "summary" ? content : undefined,
            maintenanceRules:
              section === "maintenanceRules" ? content : undefined,
            body: section === "body" ? content : undefined,
          },
        });
        return result.document;
      });
      if (!updated) throw new Error("knowledge_edit returned no document");
      selectedDocument.value =
        updated.id === selectedDocumentId.value
          ? updated
          : selectedDocument.value;
      await Promise.all([
        loadDocuments({ type: activeType.value }),
        loadOverview(),
      ]);
    } catch (cause) {
      notifyError("knowledge_edit.document.section", cause);
    } finally {
      endSave();
    }
  }

  async function updateMeta(
    id: string,
    path: string,
    meta: KnowledgeDocumentPatch,
  ) {
    if (!hasWorkspace.value) return;
    beginSave();
    error.value = "";
    try {
      const updated = await enqueueMutation(async () => {
        const result = await knowledgeEdit({
          kind: "document",
          type: activeType.value,
          path,
          document: {
            id,
            ...meta,
          },
        });
        return result.document;
      });
      if (!updated) throw new Error("knowledge_edit returned no document");
      selectedDocument.value =
        updated.id === selectedDocumentId.value
          ? updated
          : selectedDocument.value;
      await refreshKnowledgeData();
    } catch (cause) {
      notifyError("knowledge_edit.document.meta", cause);
    } finally {
      endSave();
    }
  }

  async function saveDirectoryConfig(
    path: string,
    config: KnowledgeDirectoryConfig,
  ) {
    if (!hasWorkspace.value) return;
    beginSave();
    error.value = "";
    try {
      const updated = await enqueueMutation(async () => {
        const result = await knowledgeEdit({
          kind: "directory",
          type: activeType.value,
          path,
          config,
        });
        return result.directory;
      });
      if (!updated)
        throw new Error("knowledge_edit returned no directory config");
      selectedDirectoryPath.value = updated.path;
      selectedDirectoryConfig.value = updated;
      if (isRootDirectoryPath(updated.path)) {
        rootDirectoryConfigs.value = {
          ...rootDirectoryConfigs.value,
          [updated.type]: {
            ...rootDirectoryConfigs.value[updated.type],
            [updated.path]: updated,
          },
        };
      }
      await loadOverview();
    } catch (cause) {
      notifyError("knowledge_edit.directory", cause);
    } finally {
      endSave();
    }
  }

  async function deleteDocument(path: string, type: KnowledgeDocumentType) {
    if (!hasWorkspace.value) return;
    deletingDocument.value = true;
    error.value = "";
    try {
      await enqueueMutation(() =>
        knowledgeDelete({
          kind: "document",
          type,
          path,
        }),
      );
      clearSelection();
      await refreshKnowledgeData();
    } catch (cause) {
      notifyError("knowledge_delete.document", cause);
    } finally {
      deletingDocument.value = false;
    }
  }

  function externalSourcesForReferenceDirectory(
    path: string,
  ): KnowledgeExternalSource[] {
    return (
      referenceExternalDirectorySources.value[
        normalizeDirectorySelectionPath(path)
      ] ?? []
    );
  }

  function selectionAffectedByDirectory(path: string): boolean {
    const normalizedPath = normalizeDirectorySelectionPath(path);
    const selectedDocumentPath = selectedDocument.value?.path ?? null;
    const selectedDirectoryValue = selectedDirectoryPath.value;
    return (
      (!!selectedDocumentPath &&
        (selectedDocumentPath === normalizedPath ||
          selectedDocumentPath.startsWith(`${normalizedPath}/`))) ||
      (!!selectedDirectoryValue &&
        (selectedDirectoryValue === normalizedPath ||
          selectedDirectoryValue.startsWith(`${normalizedPath}/`)))
    );
  }

  function externalProviderForDirectory(
    sources: KnowledgeExternalSource[],
  ): KnowledgeExternalSource["provider"] | null {
    return sources.find((source) => !!source?.provider)?.provider ?? null;
  }

  async function deleteManagedReferenceDirectory(
    path: string,
    sources: KnowledgeExternalSource[],
  ) {
    const provider = externalProviderForDirectory(sources);
    if (provider === "feishu") {
      await deleteFeishuReferenceDocs(path);
      return;
    }
    if (provider === "unity") {
      await deleteUnityReferenceDocs(path);
      return;
    }
    await knowledgeDeleteExternalReferenceDirectory(path);
    await refreshKnowledgeData();
  }

  async function deleteExplorerNode(node: ExplorerNode) {
    if (!hasWorkspace.value) return;
    if (node.kind === "folder") {
      if (!node.relativePath) return;
      try {
        const externalSources =
          activeType.value === "reference"
            ? externalSourcesForReferenceDirectory(node.relativePath)
            : [];
        if (externalSources.length > 0) {
          await deleteManagedReferenceDirectory(
            node.relativePath,
            externalSources,
          );
        } else {
          await enqueueMutation(() =>
            knowledgeDelete({
              kind: "directory",
              type: activeType.value,
              path: node.relativePath,
            }),
          );
          await refreshKnowledgeData();
        }
        if (selectionAffectedByDirectory(node.relativePath)) {
          clearSelection();
        }
        collapseBranch(node.path);
      } catch (cause) {
        notifyError("knowledge_delete.directory", cause);
      }
      return;
    }
    await deleteDocument(node.document.path, node.document.type);
  }

  async function deleteExplorerNodes(nodes: ExplorerNode[]) {
    if (!hasWorkspace.value || nodes.length === 0) return;

    const prunedTargets = pruneKnowledgeDeleteTargets(
      nodes.map((node) => ({
        kind: node.kind,
        path: node.kind === "folder" ? node.relativePath : node.document.path,
      })),
    );
    const prunedNodes = prunedTargets
      .map((target) =>
        nodes.find((node) =>
          target.kind === "folder"
            ? node.kind === "folder" && node.relativePath === target.path
            : node.kind === "document" && node.document.path === target.path,
        ),
      )
      .filter((node): node is ExplorerNode => !!node);
    const containsManagedExternalFolder = prunedNodes.some(
      (node) =>
        node.kind === "folder" &&
        activeType.value === "reference" &&
        externalSourcesForReferenceDirectory(node.relativePath).length > 0,
    );
    if (containsManagedExternalFolder) {
      for (const node of prunedNodes) {
        await deleteExplorerNode(node);
      }
      return;
    }

    const selectedDocumentPath = selectedDocument.value?.path ?? null;
    const selectedDirectoryValue = selectedDirectoryPath.value;

    const affectsSelectedDocument =
      !!selectedDocumentPath &&
      prunedTargets.some((target) => {
        if (target.kind === "document")
          return target.path === selectedDocumentPath;
        return (
          selectedDocumentPath === target.path ||
          selectedDocumentPath.startsWith(`${target.path}/`)
        );
      });
    const affectsSelectedDirectory =
      !!selectedDirectoryValue &&
      prunedTargets.some((target) => {
        if (target.kind === "document") return false;
        return (
          selectedDirectoryValue === target.path ||
          selectedDirectoryValue.startsWith(`${target.path}/`)
        );
      });

    deletingDocument.value = true;
    error.value = "";
    try {
      await enqueueMutation(async () => {
        for (const target of prunedTargets) {
          await knowledgeDelete({
            kind: target.kind === "folder" ? "directory" : "document",
            type: activeType.value,
            path: target.path,
          });
        }
      });
      if (affectsSelectedDocument || affectsSelectedDirectory) {
        clearSelection();
      }
      for (const node of nodes) {
        if (node.kind === "folder") {
          collapseBranch(node.path);
        }
      }
      await refreshKnowledgeData();
    } catch (cause) {
      notifyError("knowledge_delete.selection", cause);
    } finally {
      deletingDocument.value = false;
    }
  }

  function syncSelectedDocumentPath(sourcePath: string, targetPath: string) {
    if (pendingSelectionPath.value === sourcePath) {
      pendingSelectionPath.value = targetPath;
    } else if (
      selectedDocument.value?.path === sourcePath ||
      selectedDocumentSummary.value?.path === sourcePath
    ) {
      pendingSelectionPath.value = targetPath;
    }
    if (selectedDocument.value?.path === sourcePath) {
      selectedDocument.value = {
        ...selectedDocument.value,
        path: targetPath,
      };
    }
  }

  function syncDirectoryMoveSelection(sourcePath: string, targetPath: string) {
    const nextDirectoryPath = replaceRelativePathPrefix(
      selectedDirectoryPath.value,
      sourcePath,
      targetPath,
    );
    if (nextDirectoryPath) {
      selectedDirectoryPath.value = nextDirectoryPath;
      if (selectedDirectoryConfig.value) {
        selectedDirectoryConfig.value = {
          ...selectedDirectoryConfig.value,
          path: nextDirectoryPath,
        };
      }
    }

    const nextDocumentPath =
      replaceRelativePathPrefix(
        pendingSelectionPath.value,
        sourcePath,
        targetPath,
      ) ??
      replaceRelativePathPrefix(
        selectedDocument.value?.path,
        sourcePath,
        targetPath,
      ) ??
      replaceRelativePathPrefix(
        selectedDocumentSummary.value?.path,
        sourcePath,
        targetPath,
      );
    if (nextDocumentPath) {
      pendingSelectionPath.value = nextDocumentPath;
      if (selectedDocument.value) {
        selectedDocument.value = {
          ...selectedDocument.value,
          path: nextDocumentPath,
        };
      }
    }
  }

  async function moveDirectoryPath(sourcePath: string, targetPath: string) {
    const normalizedSourcePath = normalizeDirectorySelectionPath(sourcePath);
    const normalizedTargetPath = normalizeDirectorySelectionPath(targetPath);
    if (
      !normalizedSourcePath ||
      !normalizedTargetPath ||
      normalizedSourcePath === normalizedTargetPath
    ) {
      return;
    }

    error.value = "";
    try {
      await enqueueMutation(() =>
        knowledgeMove({
          kind: "directory",
          type: activeType.value,
          path: normalizedSourcePath,
          newPath: normalizedTargetPath,
        }),
      );
      syncDirectoryMoveSelection(normalizedSourcePath, normalizedTargetPath);
      collapseBranch(fullDocumentPath(activeType.value, normalizedSourcePath));
      await refreshKnowledgeData();
      const fullPath = fullDocumentPath(activeType.value, normalizedTargetPath);
      expandAncestors(fullPath);
      expandPath(fullPath);
    } catch (cause) {
      notifyError("knowledge_move.directory", cause);
    }
  }

  async function renameExplorerFolder(path: string, name: string) {
    if (!hasWorkspace.value) return;
    const normalizedName = normalizeRelativeEntryName(name);
    if (!normalizedName) return;
    if (normalizedName.includes("/")) {
      notifyError(
        "knowledge_move.directory",
        new Error("Folder name cannot contain path separators"),
      );
      return;
    }
    await moveDirectoryPath(path, siblingRelativePath(path, normalizedName));
  }

  async function renameExplorerDocument(
    path: string,
    name: string,
    docType: KnowledgeDocumentType,
  ) {
    if (!hasWorkspace.value) return;
    const normalizedName = normalizeRelativeEntryName(name);
    if (!normalizedName) return;
    if (normalizedName.includes("/")) {
      notifyError(
        "knowledge_move.document",
        new Error("Document name cannot contain path separators"),
      );
      return;
    }
    await moveDocumentPath(
      path,
      siblingRelativePath(path, normalizedName),
      docType,
    );
  }

  async function copyExplorerRelativePath(node: ExplorerNode) {
    const relativePath =
      node.kind === "folder"
        ? fullDocumentPath(activeType.value, node.relativePath)
        : fullDocumentPath(node.document.type, node.document.path);
    if (!relativePath.trim()) return;

    try {
      await navigator.clipboard.writeText(relativePath);
      notificationStore.addNotice(
        "success",
        t("knowledge.explorer.relativePathCopied"),
        {
          operation: "knowledgeCopyRelativePath",
          replaceOperation: true,
        },
      );
    } catch (cause) {
      const err = normalizeAppError(cause);
      notificationStore.addNotice(
        "error",
        t("knowledge.explorer.copyRelativePathFailed", err.message),
        {
          code: err.code,
          operation: "knowledgeCopyRelativePath",
          replaceOperation: true,
        },
      );
    }
  }

  async function openExplorerInFileSystem(node: ExplorerNode) {
    if (!hasWorkspace.value) return;

    try {
      await knowledgeRevealTarget({
        kind: node.kind === "folder" ? "directory" : "document",
        docType: node.kind === "folder" ? activeType.value : node.document.type,
        path: node.kind === "folder" ? node.relativePath : node.document.path,
      });
    } catch (cause) {
      notifyError("knowledge_reveal_target", cause);
    }
  }

  async function moveDocumentPath(
    path: string,
    nextPath: string,
    docType: KnowledgeDocumentType,
  ) {
    const normalizedPath = path.replace(/\\/g, "/").replace(/^\/+/, "");
    const normalizedNextPath = nextPath.replace(/\\/g, "/").replace(/^\/+/, "");
    if (
      !normalizedPath ||
      !normalizedNextPath ||
      normalizedPath === normalizedNextPath
    )
      return;

    beginSave();
    error.value = "";
    try {
      syncSelectedDocumentPath(normalizedPath, normalizedNextPath);
      await enqueueMutation(() =>
        knowledgeMove({
          kind: "document",
          type: docType,
          path: normalizedPath,
          newPath: normalizedNextPath,
        }),
      );
      await refreshKnowledgeData();
      expandAncestors(fullDocumentPath(docType, normalizedNextPath));
    } catch (cause) {
      notifyError("knowledge_move.document", cause);
    } finally {
      endSave();
    }
  }

  async function moveExplorerNode(node: ExplorerNode, targetDir: string) {
    if (!hasWorkspace.value) return;
    const normalizedTargetDir = targetDir
      .trim()
      .replace(/\\/g, "/")
      .replace(/^\/+|\/+$/g, "");

    if (node.kind === "folder") {
      const nextPath = joinRelativePath(normalizedTargetDir, node.name);
      await moveDirectoryPath(node.relativePath, nextPath);
      return;
    }

    const normalizedPath = node.document.path
      .replace(/\\/g, "/")
      .replace(/^\/+/, "");
    const fileName = normalizedPath.split("/").pop() ?? node.name;
    const nextPath = normalizedTargetDir
      ? `${normalizedTargetDir}/${fileName}`
      : fileName;
    await moveDocumentPath(node.document.path, nextPath, node.document.type);
  }

  async function selectDocumentByPath(path: string) {
    const normalized = path.replace(/\\/g, "/");
    const matched = documents.value.find(
      (doc) => fullDocumentPath(doc.type, doc.path) === normalized,
    );
    if (!matched) return;
    await selectDocument(matched);
  }

  onMounted(async () => {
    const cachedDocs = getWarmup<KnowledgeDocumentSummary[]>(
      "knowledge:documents",
    );
    if (cachedDocs) documents.value = cachedDocs;
    void refreshKnowledgeData({ force: false, includeOverview: false });

    const release = await listen<KnowledgeChangedEvent>(
      "knowledge-changed",
      (event) => {
        if (!hasWorkspace.value) return;
        const eventWorkingDir = normalizeWorkspacePath(
          event.payload.workingDir,
        );
        const currentWorkingDir = normalizeWorkspacePath(props.workingDir);
        if (!eventWorkingDir || eventWorkingDir !== currentWorkingDir) return;
        scheduleExternalRefresh(event.payload);
      },
    );
    const releaseLexicalRebuildStatus = await listen<LexicalRebuildStatus>(
      KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT,
      (event) => {
        handleLexicalRebuildStatus(event.payload);
      },
    );

    if (destroyed) {
      release();
      releaseLexicalRebuildStatus();
      return;
    }
    knowledgeChangedUnlisten = release;
    lexicalRebuildStatusUnlisten = releaseLexicalRebuildStatus;
  });

  onUnmounted(() => {
    destroyed = true;
    clearSearchTimer();
    clearExternalRefreshQueue();
    stopRetrievalStatusPoll();
    stopFeishuReferenceStatusPoll();
    stopUnityReferenceStatusPoll();
    knowledgeChangedUnlisten?.();
    lexicalRebuildStatusUnlisten?.();
    endExplorerDrag();
  });

  watch(
    () => props.workingDir,
    (workingDir, previousWorkingDir) => {
      const nextWorkspaceKey = normalizeWorkspacePath(workingDir);
      if (!nextWorkspaceKey) {
        resetWorkspaceState();
        return;
      }
      const previousWorkspaceKey = normalizeWorkspacePath(
        previousWorkingDir ?? "",
      );
      if (nextWorkspaceKey !== previousWorkspaceKey) {
        const preservedActiveType = activeType.value;
        resetWorkspaceState();
        activeType.value = preservedActiveType;
      }
      void refreshKnowledgeData({ force: true, includeOverview: false });
    },
  );

  watch(
    () => uiStore.pendingKnowledgeSelection?.id ?? null,
    (selectionId) => {
      if (!selectionId || !hasWorkspace.value) return;
      consumePendingUiSelection();
      if (pendingSelectionPath.value) {
        void refreshKnowledgeData();
      }
    },
  );

  watch(searchQuery, () => {
    scheduleSearch();
  });

  watch(
    [() => embeddingStatus.value?.error?.trim() ?? "", retrievalActionPending],
    ([nextError, pending]) => {
      if (!nextError) {
        lastNotifiedEmbeddingRuntimeError = "";
        return;
      }
      if (pending || nextError === lastNotifiedEmbeddingRuntimeError) return;
      lastNotifiedEmbeddingRuntimeError = nextError;
      error.value = nextError;
      notificationStore.addNotice(
        "error",
        t("knowledge.retrieval.runtimeInitFailed", nextError),
        {
          operation: "knowledge_embedding_runtime",
          replaceOperation: true,
        },
      );
    },
  );

  return {
    error,
    sidebarWidth,
    loading,
    overview,
    overviewLoading,
    documents,
    documentsByType,
    directoryGroups,
    rootDirectoryConfigs,
    referenceExternalDirectorySources,
    referenceManagedDirectoryStats,
    explorerTree,
    currentExplorerRoot,
    visibleExplorerTree,
    activeDirectoryCount,
    activeType,
    expandedPaths,
    selectedPath,
    selectedDocumentId,
    selectedDocument,
    selectedDocumentSummary,
    selectedDocumentLoading,
    selectedDirectoryPath,
    selectedDirectoryConfig,
    selectedDirectoryLoading,
    savingDocument,
    creatingDocument,
    deletingDocument,
    searchQuery,
    searchResults,
    searching,
    searchLatencyMs,
    searchMode,
    recentQueryTokens,
    selectedSearchContext,
    viewMode,
    catalogStats,
    retrievalOverview,
    generalConfig,
    embeddingConfig,
    embeddingLocalModelCatalog,
    embeddingLocalDirectoryInspection,
    embeddingStatus,
    lexicalRebuildStatus,
    feishuReferenceImportStatus,
    unityReferenceImportStatus,
    retrievalActionPending,
    feishuReferenceImportPending,
    feishuReferenceDeletePending,
    unityReferenceImportPending,
    unityReferenceDeletePending,

    isPathExpanded,
    togglePath,
    expandPath,
    expandAncestors,
    hasMoreRootDocuments,
    hasMoreDirectoryDocuments,
    hasLoadedDirectoryDocuments,
    isRootDocumentsLoading,
    isDirectoryDocumentsLoading,
    loadMoreRootDocuments,
    loadMoreDirectoryDocuments,
    selectType,
    selectDocument,
    selectDirectory,
    selectSearchResult,
    selectDocumentByPath,
    clearSelection,
    clearSearch,
    beginExplorerDrag,
    endExplorerDrag,
    markKnowledgeDataDirty,
    refreshKnowledgeData,
    refreshRetrievalState,
    saveGeneralConfigPatch,
    saveEmbeddingConfigPatch,
    setSemanticSearchEnabled,
    setEmbeddingDevicePolicy,
    setEmbeddingDownloadSource,
    selectLocalEmbeddingModelOption,
    setLocalEmbeddingModelPreset,
    browseLocalEmbeddingModelDirectory,
    downloadSelectedLocalEmbeddingModel,
    refreshFeishuReferenceImportStatus,
    refreshUnityReferenceImportStatus,
    importFeishuReferenceDocs,
    importUnityReferenceDocs,
    deleteFeishuReferenceDocs,
    deleteUnityReferenceDocs,
    activateSemanticRuntime,
    deactivateSemanticRuntime,
    rebuildLexicalIndex,
    createDocument,
    createDocumentAt,
    createFolder,
    updateSection,
    updateMeta,
    saveDirectoryConfig,
    deleteDocument,
    deleteExplorerNode,
    deleteExplorerNodes,
    renameExplorerFolder,
    renameExplorerDocument,
    copyExplorerRelativePath,
    openExplorerInFileSystem,
    moveExplorerNode,
  };
}
