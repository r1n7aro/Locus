import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRenderer, defineComponent, nextTick, reactive } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { useKnowledgeState } from "../composables/useKnowledgeState";
import type {
  FeishuReferenceImportStatus,
  KnowledgeChangedEvent,
  KnowledgeDocumentSummary,
  KnowledgeSearchResult,
  UnityReferenceImportStatus,
} from "../types";

const notificationStoreMocks = vi.hoisted(() => ({
  addNotice: vi.fn(),
}));

const knowledgeDownloadWindowMocks = vi.hoisted(() => ({
  openKnowledgeDownloadProgressWindow: vi.fn(),
}));

const feishuReferenceImportWindowMocks = vi.hoisted(() => ({
  openFeishuReferenceImportProgressWindow: vi.fn(),
}));

const unityReferenceImportWindowMocks = vi.hoisted(() => ({
  openUnityReferenceImportProgressWindow: vi.fn(),
}));

const tauriEventMocks = vi.hoisted(() => ({
  listen: vi.fn(),
}));

const knowledgeMocks = vi.hoisted(() => ({
  createSkillScaffold: vi.fn(),
  deleteSkillPackage: vi.fn(),
  exportSkillPackage: vi.fn(),
  importSkillPackage: vi.fn(),
  knowledgeActivateEmbedding: vi.fn(),
  knowledgeCreate: vi.fn(),
  knowledgeDelete: vi.fn(),
  knowledgeDeleteFeishuReferenceDocs: vi.fn(),
  knowledgeDeleteUnityReferenceDocs: vi.fn(),
  knowledgeDownloadLocalEmbeddingModel: vi.fn(),
  knowledgeEdit: vi.fn(),
  knowledgeList: vi.fn(),
  knowledgeListPage: vi.fn(),
  knowledgeListDirectoryDocuments: vi.fn(),
  knowledgeListDirectoryDocumentsPage: vi.fn(),
  knowledgeListDirectories: vi.fn(),
  knowledgeListExternalReferenceDirectories: vi.fn(),
  knowledgeListUnityManagedDirectoryStats: vi.fn(),
  knowledgeMove: vi.fn(),
  knowledgeQuery: vi.fn(),
  knowledgeRead: vi.fn(),
  knowledgeRevealTarget: vi.fn(),
  knowledgeGetEmbeddingConfig: vi.fn(),
  knowledgeGetEmbeddingStatus: vi.fn(),
  knowledgeGetFeishuReferenceImportStatus: vi.fn(),
  knowledgeGetGeneralConfig: vi.fn(),
  knowledgeGetLexicalRebuildStatus: vi.fn(),
  knowledgeGetLocalEmbeddingModelCatalog: vi.fn(),
  knowledgeGetOverview: vi.fn(),
  knowledgeGetUnityReferenceImportStatus: vi.fn(),
  knowledgeInspectLocalEmbeddingModelDirectory: vi.fn(),
  knowledgeImportFeishuReferenceDocs: vi.fn(),
  knowledgeImportUnityReferenceDocs: vi.fn(),
  knowledgeRebuildLexicalIndex: vi.fn(),
  knowledgeDeactivateEmbedding: vi.fn(),
  knowledgeSaveEmbeddingConfig: vi.fn(),
  knowledgeSaveGeneralConfig: vi.fn(),
  setSkillConfig: vi.fn(),
}));

vi.mock("../services/knowledge", () => knowledgeMocks);
vi.mock(
  "../services/knowledgeDownloadWindow",
  () => knowledgeDownloadWindowMocks,
);
vi.mock(
  "../services/feishuReferenceImportWindow",
  () => feishuReferenceImportWindowMocks,
);
vi.mock(
  "../services/unityReferenceImportWindow",
  () => unityReferenceImportWindowMocks,
);
vi.mock("../services/errors", () => ({
  normalizeAppError: (error: unknown) => {
    if (typeof error === "object" && error !== null) {
      const payload = error as Record<string, unknown>;
      return {
        message: typeof payload.message === "string" ? payload.message : String(error),
        code: typeof payload.code === "string" ? payload.code : "test",
      };
    }
    return {
      message: String(error),
      code: "test",
    };
  },
}));
vi.mock("../i18n", () => ({
  t: (key: string) => key,
}));
vi.mock("../composables/warmupCache", () => ({
  getWarmup: () => null,
}));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => notificationStoreMocks,
}));
vi.mock("@tauri-apps/api/event", () => tauriEventMocks);
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isMaximized: async () => false,
    onResized: async () => () => undefined,
    setAlwaysOnTop: async () => undefined,
    minimize: async () => undefined,
    toggleMaximize: async () => undefined,
    close: async () => undefined,
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));

function createUnityReferenceImportStatus(
  overrides: Partial<UnityReferenceImportStatus> = {},
): UnityReferenceImportStatus {
  return {
    state: "missing",
    stage: "idle",
    running: false,
    projectVersion: "2022.3.21f1",
    docsVersion: "2022.3",
    selectedLocale: null,
    importedProjectVersion: null,
    importedDocsVersion: null,
    importedLocale: null,
    importedAt: null,
    importedDocCount: 0,
    managedPath: "reference/unity-official-docs",
    progress: null,
    downloadedBytes: null,
    totalBytes: null,
    processedDocs: 0,
    totalDocs: null,
    currentPath: null,
    sourceUrl: null,
    message: "",
    error: null,
    lastOutcome: null,
    ...overrides,
  };
}

function createFeishuReferenceImportStatus(
  overrides: Partial<FeishuReferenceImportStatus> = {},
): FeishuReferenceImportStatus {
  return {
    state: "missing_config",
    stage: "idle",
    running: false,
    authMode: "app_credentials",
    oauthPersistenceMode: "session",
    appId: "",
    appSecretConfigured: false,
    authorized: false,
    openBaseUrl: "https://open.feishu.cn",
    callbackUrls: ["http://127.0.0.1:39241/oauth/feishu/reference/callback"],
    requiredScopes: [],
    grantedScopes: [],
    missingScopes: [],
    accessTokenExpiresAt: null,
    refreshTokenExpiresAt: null,
    canRefresh: false,
    spaceId: null,
    spaceName: null,
    rootNodeToken: null,
    rootNodeTitle: null,
    importedSpaceId: null,
    importedSpaceName: null,
    importedRootNodeToken: null,
    importedRootNodeTitle: null,
    importedAt: null,
    importedDocCount: 0,
    managedPath: "reference/feishu-knowledge-base",
    progress: null,
    processedDocs: 0,
    totalDocs: null,
    currentTitle: null,
    currentPath: null,
    message: "",
    error: null,
    lastOutcome: null,
    ...overrides,
  };
}

async function flushPromises(rounds = 4) {
  for (let index = 0; index < rounds; index += 1) {
    await Promise.resolve();
  }
}

type KnowledgeStateProps = {
  workingDir: string;
  selectedModelId: string;
  modelDefaults: any;
};

type TauriEventHandler<T> = (event: { payload: T }) => void;

let tauriEventHandlers: Record<string, TauriEventHandler<any>[]> = {};

function emitTauriEvent<T>(eventName: string, payload: T) {
  for (const handler of tauriEventHandlers[eventName] ?? []) {
    handler({ payload });
  }
}

const testRenderer = createRenderer<any, any>({
  patchProp() {},
  insert(child, parent) {
    parent.children ??= [];
    parent.children.push(child);
  },
  remove() {},
  createElement(type) {
    return { type, children: [] };
  },
  createText(text) {
    return { text };
  },
  createComment(text) {
    return { comment: text };
  },
  setText(node, text) {
    node.text = text;
  },
  setElementText(node, text) {
    node.text = text;
  },
  parentNode() {
    return null;
  },
  nextSibling() {
    return null;
  },
});

function mountKnowledgeState(props: KnowledgeStateProps) {
  let state: ReturnType<typeof useKnowledgeState> | null = null;
  const Root = defineComponent({
    setup() {
      state = useKnowledgeState(props);
      return () => null;
    },
  });
  const app = testRenderer.createApp(Root);
  app.mount({ children: [] });
  if (!state) throw new Error("useKnowledgeState did not mount");
  const mountedState = state as ReturnType<typeof useKnowledgeState>;
  return {
    state: mountedState,
    unmount: () => app.unmount(),
  };
}

describe("useKnowledgeState", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.useRealTimers();
    tauriEventHandlers = {};
    tauriEventMocks.listen.mockImplementation(
      async (eventName: string, handler: TauriEventHandler<any>) => {
        tauriEventHandlers[eventName] ??= [];
        tauriEventHandlers[eventName].push(handler);
        return () => {
          tauriEventHandlers[eventName] = (
            tauriEventHandlers[eventName] ?? []
          ).filter((entry) => entry !== handler);
        };
      },
    );
    knowledgeDownloadWindowMocks.openKnowledgeDownloadProgressWindow.mockResolvedValue(
      undefined,
    );
    feishuReferenceImportWindowMocks.openFeishuReferenceImportProgressWindow.mockResolvedValue(
      undefined,
    );
    unityReferenceImportWindowMocks.openUnityReferenceImportProgressWindow.mockResolvedValue(
      undefined,
    );

    knowledgeMocks.knowledgeList.mockResolvedValue([
      {
        id: "design-1",
        type: "design",
        path: "combat/core-loop.md",
        title: "核心循环",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "摘要",
        createdAt: 1,
        updatedAt: 2,
        hasSummary: true,
      },
      {
        id: "memory-1",
        type: "memory",
        path: "project-understanding.md",
        title: "项目理解",
        injectMode: "full",
        summaryEnabled: false,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: true,
        explicitMaintenanceRules: true,
        lexicalSearch: "enabled",
        vectorSearch: "disabled",
        summary: null,
        createdAt: 1,
        updatedAt: 3,
        hasSummary: false,
      },
    ]);
    knowledgeMocks.knowledgeListPage.mockResolvedValue({
      items: [],
      nextCursor: null,
    });

    knowledgeMocks.knowledgeRead.mockImplementation(async (input: any) => {
      if (input.kind === "directory") {
        return {
          kind: "directory",
          directory: {
            version: 4,
            type: input.type ?? "design",
            path: input.path,
            configPath: `${input.path}.locus-meta`,
            exists: true,
            updatedAt: 2,
            summary: "维护战斗结构缓存",
            injectMode: "excerpt",
            inheritInjectMode: true,
            injectModeSource: { kind: "type_default", path: null },
            aiMaintained: false,
            inheritAiConfig: true,
            aiConfigSource: { kind: "type_default", path: null },
            explicitMaintenanceRules: false,
            lexicalSearch: "inherit",
            vectorSearch: "inherit",
            effectiveLexicalSearch: { enabled: true, source: "default" },
            effectiveVectorSearch: { enabled: true, source: "default" },
            inheritToChildren: true,
            allowCreateDocuments: true,
            allowCreateDirectories: true,
            allowMoveDocuments: true,
            allowMoveDirectories: true,
            maintenanceRules: "",
          },
        };
      }

      return {
        kind: "document",
        document: {
          id: "design-1",
          type: input.type ?? "design",
          path: input.path,
          title: "核心循环",
          injectMode: "excerpt",
          inheritInjectMode: false,
          injectModeSource: { kind: "self", path: null },
          summaryEnabled: true,
          commandEnabled: false,
          readOnly: false,
          aiMaintained: false,
          inheritAiConfig: false,
          aiConfigSource: { kind: "self", path: null },
          explicitMaintenanceRules: false,
          summary: "摘要",
          body: "正文",
          maintenanceRules: null,
          createdAt: 1,
          updatedAt: 2,
          hasSummary: true,
        },
      };
    });

    knowledgeMocks.knowledgeQuery.mockResolvedValue([]);
    knowledgeMocks.knowledgeRevealTarget.mockResolvedValue(undefined);
    knowledgeMocks.knowledgeListDirectoryDocuments.mockResolvedValue([]);
    knowledgeMocks.knowledgeListDirectoryDocumentsPage.mockResolvedValue({
      items: [],
      nextCursor: null,
    });
    knowledgeMocks.knowledgeListDirectories.mockResolvedValue(["combat"]);
    knowledgeMocks.knowledgeListExternalReferenceDirectories.mockResolvedValue(
      [],
    );
    knowledgeMocks.knowledgeListUnityManagedDirectoryStats.mockResolvedValue(
      [],
    );
    knowledgeMocks.knowledgeGetOverview.mockResolvedValue({
      totalDocumentCount: 2,
      fullText: {
        enabled: true,
        indexableItemCount: 2,
        indexedItemCount: 2,
        freshItemCount: 2,
        staleItemCount: 0,
        pendingItemCount: 0,
        chunkCount: 4,
        lastBuildAt: null,
      },
      semantic: {
        enabled: false,
        ready: false,
        backend: "fastembed",
        model: "Qwen/Qwen3-Embedding-4B",
        deviceRoute: "cpu",
        deviceName: "CPU",
        indexedItemCount: 0,
        embeddedChunkCount: 0,
        pendingItemCount: 2,
        coverageRatio: 0,
        stage: null,
        error: null,
      },
      performance: {
        dbBytes: 0,
        lexicalIndexBytes: 0,
        localModelBytes: 0,
        gpuMemoryBytes: 0,
        gpuDedicatedMemoryBytes: 0,
        totalBytes: 0,
        avgChunksPerItem: 2,
      },
    });
    knowledgeMocks.knowledgeGetGeneralConfig.mockResolvedValue({
      enabled: true,
      lexicalSearchEnabled: true,
      semanticSearchEnabled: false,
    });
    knowledgeMocks.knowledgeGetEmbeddingConfig.mockResolvedValue({
      enabled: false,
      embeddingMode: "local",
      devicePolicy: "cpu",
      localRuntime: "fastembed",
      localModel: "Qwen/Qwen3-Embedding-4B",
      localModelPath: "",
      localModelDownloadSource: "official",
      remoteEndpoint: "",
      remoteApiKey: "",
      remoteModel: "",
      remoteDimensions: 0,
      remoteMaxBatch: 0,
    });
    knowledgeMocks.knowledgeGetEmbeddingStatus.mockResolvedValue({
      enabled: false,
      ready: false,
      activating: false,
      modelDownloaded: false,
      modelDownloadProgress: null,
      indexProgress: null,
      error: null,
      stage: null,
      detail: null,
      currentFile: null,
      downloadedBytes: null,
      totalBytes: null,
      processedDocs: null,
      totalDocs: null,
      lastTestSummary: null,
      lastTestPassed: null,
    });
    knowledgeMocks.knowledgeGetLexicalRebuildStatus.mockResolvedValue({
      running: false,
      stage: null,
      detail: null,
      currentFile: null,
      processedDocs: null,
      totalDocs: null,
      error: null,
      startedAt: null,
      completedAt: null,
    });
    knowledgeMocks.knowledgeGetLocalEmbeddingModelCatalog.mockResolvedValue({
      managedDirectory: "F:/app-data/knowledge_models",
      presets: [
        {
          id: "Qwen/Qwen3-Embedding-4B",
          label: "Qwen/Qwen3-Embedding-4B",
          downloaded: false,
          dimensions: 2560,
        },
      ],
      availableModels: [],
    });
    knowledgeMocks.knowledgeGetFeishuReferenceImportStatus.mockResolvedValue(
      createFeishuReferenceImportStatus(),
    );
    knowledgeMocks.knowledgeGetUnityReferenceImportStatus.mockResolvedValue(
      createUnityReferenceImportStatus(),
    );
    knowledgeMocks.knowledgeInspectLocalEmbeddingModelDirectory.mockResolvedValue(
      {
        path: "",
        label: "",
        ready: false,
        modelFile: null,
        missingFiles: [],
      },
    );
    knowledgeMocks.knowledgeImportUnityReferenceDocs.mockResolvedValue(
      createUnityReferenceImportStatus({
        state: "running",
        stage: "resolving_source",
        running: true,
        message: "正在准备 Unity 文档导入",
      }),
    );
    knowledgeMocks.knowledgeImportFeishuReferenceDocs.mockResolvedValue(
      createFeishuReferenceImportStatus({
        state: "running",
        stage: "listing_nodes",
        running: true,
        message: "正在读取飞书知识空间结构。",
      }),
    );
    knowledgeMocks.knowledgeDeleteFeishuReferenceDocs.mockResolvedValue(
      createFeishuReferenceImportStatus(),
    );
    knowledgeMocks.knowledgeDeleteUnityReferenceDocs.mockResolvedValue(
      createUnityReferenceImportStatus(),
    );
    knowledgeMocks.knowledgeSaveGeneralConfig.mockImplementation(
      async (config: any) => config,
    );
    knowledgeMocks.knowledgeSaveEmbeddingConfig.mockImplementation(
      async (config: any) => config,
    );
    knowledgeMocks.setSkillConfig.mockResolvedValue(undefined);
    knowledgeMocks.knowledgeActivateEmbedding.mockResolvedValue(undefined);
    knowledgeMocks.knowledgeDeactivateEmbedding.mockResolvedValue(undefined);
    knowledgeMocks.knowledgeDownloadLocalEmbeddingModel.mockResolvedValue(
      undefined,
    );
    knowledgeMocks.knowledgeRebuildLexicalIndex.mockResolvedValue(2);
    knowledgeMocks.knowledgeCreate.mockImplementation(async (input: any) => {
      if (input.kind === "directory") {
        return {
          kind: "directory",
          type: input.type ?? "design",
          path: input.path,
          resultPath: input.path,
          directory: {
            version: 4,
            type: input.type ?? "design",
            path: input.path,
            configPath: `${input.path}.locus-meta`,
            exists: true,
            updatedAt: 2,
            summary: "",
            injectMode: "excerpt",
            inheritInjectMode: true,
            injectModeSource: { kind: "type_default", path: null },
            aiMaintained: false,
            inheritAiConfig: true,
            aiConfigSource: { kind: "type_default", path: null },
            explicitMaintenanceRules: false,
            lexicalSearch: "inherit",
            vectorSearch: "inherit",
            effectiveLexicalSearch: { enabled: true, source: "default" },
            effectiveVectorSearch: { enabled: true, source: "default" },
            inheritToChildren: true,
            allowCreateDocuments: true,
            allowCreateDirectories: true,
            allowMoveDocuments: true,
            allowMoveDirectories: true,
            maintenanceRules: "",
          },
        };
      }

      return {
        kind: "document",
        type: input.type ?? "design",
        path: input.path ?? "new-doc.md",
        resultPath: input.path ?? "new-doc.md",
        document: {
          id: "new-doc",
          type: input.type ?? "design",
          path: input.path ?? "new-doc.md",
          title: input.document?.title ?? "新文档",
          injectMode: "none",
          inheritInjectMode: input.document?.inheritInjectMode ?? false,
          injectModeSource: {
            kind: input.document?.inheritInjectMode ? "type_default" : "self",
            path: null,
          },
          summaryEnabled: input.document?.summaryEnabled ?? false,
          commandEnabled: input.document?.commandEnabled ?? false,
          readOnly: input.document?.readOnly ?? false,
          aiMaintained: input.document?.aiMaintained ?? false,
          inheritAiConfig: input.document?.inheritAiConfig ?? false,
          aiConfigSource: {
            kind: input.document?.inheritAiConfig ? "type_default" : "self",
            path: null,
          },
          explicitMaintenanceRules:
            input.document?.explicitMaintenanceRules ?? false,
          summary: input.document?.summary ?? null,
          body: input.document?.body ?? "",
          maintenanceRules: input.document?.maintenanceRules ?? null,
          skillEnabled: input.document?.skillEnabled ?? null,
          skillSurface: input.document?.skillSurface ?? null,
          commandTrigger: input.document?.commandTrigger ?? null,
          createdAt: 1,
          updatedAt: 2,
          hasSummary:
            !!input.document?.summaryEnabled && !!input.document?.summary,
        },
      };
    });
    knowledgeMocks.createSkillScaffold.mockImplementation(async (input: any) => {
      const dirName = (input.path ?? `${input.name}.md`)
        .replace(/\\/g, "/")
        .replace(/^skill\//, "")
        .replace(/\.md$/i, "");
      return {
        name: input.name,
        description: input.summary ?? "",
        argumentHint: input.argumentHint ?? "",
        dirName,
        source: "project",
        relPath: `skill/${dirName}.md`,
        updatedAt: 2,
        skillEnabled: true,
        skillSurface: "command",
        skillDescription: input.summary ?? null,
        commandTrigger: input.commandTrigger ?? `/${input.name}`,
        kind: "document",
        hasUnity: false,
        hasL0: true,
        hasL1: true,
        hasL2: true,
      };
    });
    knowledgeMocks.knowledgeEdit.mockImplementation(async (input: any) => {
      if (input.kind === "directory") {
        return {
          kind: "directory",
          type: input.type ?? "design",
          path: input.path,
          directory: {
            version: input.config?.version ?? 4,
            type: input.type ?? "design",
            path: input.path,
            configPath: `${input.path}.locus-meta`,
            exists: true,
            updatedAt: 3,
            summary: input.config?.summary ?? "",
            injectMode: input.config?.injectMode ?? "excerpt",
            inheritInjectMode: input.config?.inheritInjectMode ?? false,
            injectModeSource: {
              kind: input.config?.inheritInjectMode ? "type_default" : "self",
              path: null,
            },
            aiMaintained: input.config?.aiMaintained ?? false,
            inheritAiConfig: input.config?.inheritAiConfig ?? false,
            aiConfigSource: {
              kind: input.config?.inheritAiConfig ? "type_default" : "self",
              path: null,
            },
            explicitMaintenanceRules:
              input.config?.explicitMaintenanceRules ?? false,
            lexicalSearch: input.config?.lexicalSearch ?? "inherit",
            vectorSearch: input.config?.vectorSearch ?? "inherit",
            effectiveLexicalSearch: {
              enabled:
                (input.config?.lexicalSearch ?? "inherit") !== "disabled",
              source:
                (input.config?.lexicalSearch ?? "inherit") === "inherit"
                  ? "default"
                  : "self",
            },
            effectiveVectorSearch: {
              enabled: (input.config?.vectorSearch ?? "inherit") !== "disabled",
              source:
                (input.config?.vectorSearch ?? "inherit") === "inherit"
                  ? "default"
                  : "self",
            },
            inheritToChildren: input.config?.inheritToChildren ?? true,
            allowCreateDocuments: input.config?.allowCreateDocuments ?? true,
            allowCreateDirectories:
              input.config?.allowCreateDirectories ?? true,
            allowMoveDocuments: input.config?.allowMoveDocuments ?? true,
            allowMoveDirectories: input.config?.allowMoveDirectories ?? true,
            maintenanceRules: input.config?.maintenanceRules ?? "",
          },
        };
      }

      return {
        kind: "document",
        type: input.type ?? "design",
        path: input.path,
        resultPath: input.document?.newPath ?? input.path,
        document: {
          id: input.document?.id ?? "design-1",
          type: input.type ?? "design",
          path: input.document?.newPath ?? input.path,
          title: input.document?.title ?? "核心循环",
          injectMode: "excerpt",
          inheritInjectMode: input.document?.inheritInjectMode ?? false,
          injectModeSource: {
            kind: input.document?.inheritInjectMode ? "type_default" : "self",
            path: null,
          },
          summaryEnabled: true,
          commandEnabled: false,
          readOnly: input.document?.readOnly ?? false,
          aiMaintained: input.document?.aiMaintained ?? false,
          inheritAiConfig: input.document?.inheritAiConfig ?? false,
          aiConfigSource: {
            kind: input.document?.inheritAiConfig ? "type_default" : "self",
            path: null,
          },
          explicitMaintenanceRules:
            input.document?.explicitMaintenanceRules ?? false,
          summary: input.document?.summary ?? "摘要",
          body: input.document?.body ?? "正文",
          maintenanceRules: input.document?.maintenanceRules ?? null,
          createdAt: 1,
          updatedAt: 3,
          hasSummary: true,
        },
      };
    });
    knowledgeMocks.knowledgeMove.mockImplementation(async (input: any) => ({
      kind: input.kind,
      type: input.type ?? "design",
      path: input.path,
      resultPath: input.newPath,
      document:
        input.kind === "document"
          ? {
              id: "design-1",
              type: input.type ?? "design",
              path: input.newPath,
              title: "核心循环",
              injectMode: "excerpt",
              inheritInjectMode: false,
              injectModeSource: { kind: "self", path: null },
              summaryEnabled: true,
              commandEnabled: false,
              readOnly: false,
              aiMaintained: false,
              inheritAiConfig: false,
              aiConfigSource: { kind: "self", path: null },
              explicitMaintenanceRules: false,
              summary: "摘要",
              body: "正文",
              maintenanceRules: null,
              createdAt: 1,
              updatedAt: 3,
              hasSummary: true,
            }
          : null,
    }));
    knowledgeMocks.knowledgeDelete.mockImplementation(async (input: any) => ({
      kind: input.kind,
      type: input.type ?? "design",
      path: input.path,
      resultPath: input.path,
      document: null,
      directory: null,
    }));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("loads unified knowledge documents and computes catalog stats", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();

    expect(knowledgeMocks.knowledgeList).toHaveBeenCalled();
    expect(state.documents.value).toHaveLength(2);
    expect(state.catalogStats.value.total).toBe(2);
    expect(state.catalogStats.value.byType.design).toBe(1);
    expect(state.catalogStats.value.byType.memory).toBe(1);
    expect(state.catalogStats.value.summaryMissing).toBe(0);
    expect(state.currentExplorerRoot.value?.children[0]).toMatchObject({
      kind: "folder",
      name: "combat",
    });
  });

  it("reloads the active knowledge list when the workspace changes", async () => {
    let currentDocs: KnowledgeDocumentSummary[] = [
      {
        id: "design-a",
        type: "design",
        path: "repo-a.md",
        title: "Repo A",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "A",
        createdAt: 1,
        updatedAt: 2,
        hasSummary: true,
      },
    ];
    knowledgeMocks.knowledgeList.mockImplementation(async () => currentDocs);
    knowledgeMocks.knowledgeListDirectories.mockResolvedValue([]);
    const props = reactive({
      workingDir: "F:/repo-a",
      selectedModelId: "",
      modelDefaults: {} as any,
    });
    const state = useKnowledgeState(props);

    await state.refreshKnowledgeData({ includeOverview: false });
    expect(state.documents.value.map((doc) => doc.path)).toEqual([
      "repo-a.md",
    ]);

    currentDocs = [
      {
        id: "design-b",
        type: "design",
        path: "repo-b.md",
        title: "Repo B",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "B",
        createdAt: 3,
        updatedAt: 4,
        hasSummary: true,
      },
    ];
    props.workingDir = "F:/repo-b";
    await nextTick();
    await flushPromises(8);

    expect(state.documents.value.map((doc) => doc.path)).toEqual([
      "repo-b.md",
    ]);
    expect(knowledgeMocks.knowledgeList).toHaveBeenCalledTimes(2);
  });

  it("drops queued external knowledge changes after the workspace changes", async () => {
    vi.useFakeTimers();
    const props = reactive({
      workingDir: "F:/repo-a",
      selectedModelId: "",
      modelDefaults: {} as any,
    });
    const mounted = mountKnowledgeState(props);
    await flushPromises(8);
    knowledgeMocks.knowledgeList.mockClear();

    emitTauriEvent<KnowledgeChangedEvent>("knowledge-changed", {
      workingDir: "F:/repo-a",
      source: "knowledge_fs_watcher",
      changedAt: 1,
      docType: "design",
      path: "old-project.md",
      parentPath: "",
      targetKind: "document",
      changeKind: "content",
      subtree: false,
    });

    props.workingDir = "F:/repo-b";
    await nextTick();
    await flushPromises(4);
    vi.advanceTimersByTime(100);
    await flushPromises(8);

    expect(knowledgeMocks.knowledgeList).not.toHaveBeenCalledWith(
      expect.objectContaining({
        type: "design",
        pathPrefix: "old-project.md",
      }),
    );
    mounted.unmount();
  });

  it("refreshes the active skill list when installed plugins change", async () => {
    vi.useFakeTimers();
    const designDocs: KnowledgeDocumentSummary[] = [
      {
        id: "design-1",
        type: "design",
        path: "combat/core-loop.md",
        title: "核心循环",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "摘要",
        createdAt: 1,
        updatedAt: 2,
        hasSummary: true,
      },
    ];
    let skillDocs: KnowledgeDocumentSummary[] = [
      {
        id: "skill-old",
        type: "skill",
        path: "old-plugin/SKILL.md",
        title: "Old Plugin",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: true,
        readOnly: true,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "Old",
        createdAt: 1,
        updatedAt: 2,
        hasSummary: true,
      },
    ];
    knowledgeMocks.knowledgeList.mockImplementation(async (input: any = {}) =>
      input.type === "skill" ? skillDocs : designDocs,
    );

    const props = reactive({
      workingDir: "F:/repo",
      selectedModelId: "",
      modelDefaults: {} as any,
    });
    const mounted = mountKnowledgeState(props);
    await flushPromises(8);
    await mounted.state.selectType("skill");
    await flushPromises(8);

    knowledgeMocks.knowledgeList.mockClear();
    skillDocs = [
      {
        id: "skill-new",
        type: "skill",
        path: "new-plugin/SKILL.md",
        title: "New Plugin",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: true,
        readOnly: true,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "New",
        createdAt: 3,
        updatedAt: 4,
        hasSummary: true,
      },
    ];

    emitTauriEvent<void>("plugins-changed", undefined);
    vi.advanceTimersByTime(100);
    await flushPromises(8);

    expect(knowledgeMocks.knowledgeList).toHaveBeenCalledWith(
      expect.objectContaining({ type: "skill" }),
    );
    expect(
      mounted.state.documents.value
        .filter((document) => document.type === "skill")
        .map((document) => document.path),
    ).toEqual(["new-plugin/SKILL.md"]);
    mounted.unmount();
  });

  it("refreshes external document changes without showing selected document loading", async () => {
    vi.useFakeTimers();
    const props = reactive({
      workingDir: "F:/repo",
      selectedModelId: "",
      modelDefaults: {} as any,
    });
    const mounted = mountKnowledgeState(props);
    await flushPromises(8);

    await mounted.state.selectDocument(mounted.state.documents.value[0]!);
    knowledgeMocks.knowledgeRead.mockClear();

    const readDeferred: { resolve?: (value: any) => void } = {};
    knowledgeMocks.knowledgeRead.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          readDeferred.resolve = resolve;
        }),
    );

    emitTauriEvent<KnowledgeChangedEvent>("knowledge-changed", {
      workingDir: "F:/repo",
      source: "knowledge_fs_watcher",
      changedAt: 2,
      docType: "design",
      path: "combat/core-loop.md",
      parentPath: "combat",
      targetKind: "document",
      changeKind: "content",
      subtree: false,
    });
    vi.advanceTimersByTime(100);
    await flushPromises(8);

    expect(mounted.state.selectedDocumentLoading.value).toBe(false);
    expect(knowledgeMocks.knowledgeRead).toHaveBeenCalledWith({
      kind: "document",
      path: "combat/core-loop.md",
      type: "design",
      part: "full",
    });

    readDeferred.resolve?.({
      kind: "document",
      document: {
        id: "design-1",
        type: "design",
        path: "combat/core-loop.md",
        title: "核心循环",
        injectMode: "excerpt",
        inheritInjectMode: false,
        injectModeSource: { kind: "self", path: null },
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        inheritAiConfig: false,
        aiConfigSource: { kind: "self", path: null },
        explicitMaintenanceRules: false,
        summary: "摘要",
        body: "正文 v3",
        maintenanceRules: null,
        createdAt: 1,
        updatedAt: 4,
        hasSummary: true,
      },
    });
    await flushPromises(8);

    expect(mounted.state.selectedDocument.value?.body).toBe("正文 v3");
    mounted.unmount();
  });

  it("drops in-flight search results after the workspace changes", async () => {
    vi.useFakeTimers();
    const searchDeferred: {
      resolve?: (results: KnowledgeSearchResult[]) => void;
    } = {};
    knowledgeMocks.knowledgeQuery.mockImplementationOnce(
      () =>
        new Promise<KnowledgeSearchResult[]>((resolve) => {
          searchDeferred.resolve = resolve;
        }),
    );
    const props = reactive({
      workingDir: "F:/repo-a",
      selectedModelId: "",
      modelDefaults: {} as any,
    });
    const state = useKnowledgeState(props);

    state.searchQuery.value = "old project";
    await nextTick();
    vi.advanceTimersByTime(220);
    await flushPromises(4);
    expect(knowledgeMocks.knowledgeQuery).toHaveBeenCalledTimes(1);

    props.workingDir = "F:/repo-b";
    await nextTick();
    await flushPromises(4);
    searchDeferred.resolve!([
      {
        id: "old-result",
        type: "design",
        path: "old-project.md",
        title: "Old Project",
        injectMode: "excerpt",
        aiMaintained: false,
        snippet: "stale",
        matchKind: "lexical",
        score: 1,
      },
    ]);
    await flushPromises(8);

    expect(state.searchResults.value).toEqual([]);
    expect(state.searching.value).toBe(false);
  });

  it("drops stale reference status refresh results after the workspace changes", async () => {
    const statusDeferred: {
      resolve?: (status: FeishuReferenceImportStatus) => void;
    } = {};
    knowledgeMocks.knowledgeGetFeishuReferenceImportStatus.mockImplementationOnce(
      () =>
        new Promise<FeishuReferenceImportStatus>((resolve) => {
          statusDeferred.resolve = resolve;
        }),
    );
    const props = reactive({
      workingDir: "F:/repo-a",
      selectedModelId: "",
      modelDefaults: {} as any,
    });
    const state = useKnowledgeState(props);

    const pending = state.refreshFeishuReferenceImportStatus();
    props.workingDir = "F:/repo-b";
    await nextTick();
    await flushPromises(4);
    statusDeferred.resolve!(
      createFeishuReferenceImportStatus({
        state: "running",
        stage: "listing_nodes",
        running: true,
        message: "old workspace",
      }),
    );
    await pending;
    await flushPromises(4);

    expect(state.feishuReferenceImportStatus.value).toBeNull();
    expect(state.feishuReferenceImportPending.value).toBe(false);
  });

  it("loads local embedding model catalog alongside retrieval state", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();

    expect(
      knowledgeMocks.knowledgeGetLocalEmbeddingModelCatalog,
    ).toHaveBeenCalledTimes(1);
    expect(state.embeddingLocalModelCatalog.value?.managedDirectory).toBe(
      "F:/app-data/knowledge_models",
    );
    expect(state.embeddingLocalModelCatalog.value?.presets[0]).toMatchObject({
      id: "Qwen/Qwen3-Embedding-4B",
      downloaded: false,
    });
  });

  it("updates lexical rebuild status from the backend event", async () => {
    const mounted = mountKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );
    await flushPromises(8);
    knowledgeMocks.knowledgeGetLexicalRebuildStatus.mockClear();

    emitTauriEvent("knowledge-lexical-rebuild-status", {
      running: true,
      stage: "indexing",
      detail: "Indexing docs",
      currentFile: "design/a.md",
      progress: 0.42,
      processedDocs: 42,
      totalDocs: 100,
      error: null,
      startedAt: "2026-04-16T00:00:00Z",
      completedAt: null,
    });
    await nextTick();

    expect(mounted.state.lexicalRebuildStatus.value).toMatchObject({
      running: true,
      stage: "indexing",
      progress: 0.42,
    });
    expect(
      knowledgeMocks.knowledgeGetLexicalRebuildStatus,
    ).not.toHaveBeenCalled();

    mounted.unmount();
  });

  it("updates the embedding device policy through the shared config save path", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.setEmbeddingDevicePolicy("gpu_directml");

    expect(knowledgeMocks.knowledgeSaveEmbeddingConfig).toHaveBeenCalledWith(
      expect.objectContaining({
        enabled: false,
        embeddingMode: "local",
        devicePolicy: "gpu_directml",
      }),
    );
  });

  it("updates the embedding download source through the shared config save path", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.setEmbeddingDownloadSource("hf-mirror");

    expect(knowledgeMocks.knowledgeSaveEmbeddingConfig).toHaveBeenCalledWith(
      expect.objectContaining({
        enabled: false,
        embeddingMode: "local",
        localModelDownloadSource: "hf-mirror",
      }),
    );
  });

  it("surfaces embedding runtime status errors through the shared notification banner", async () => {
    knowledgeMocks.knowledgeGetEmbeddingStatus.mockResolvedValue({
      enabled: true,
      ready: false,
      activating: false,
      modelDownloaded: true,
      modelDownloadProgress: null,
      indexProgress: null,
      error: "DirectML init failed",
      stage: "error",
      detail: null,
      currentFile: null,
      downloadedBytes: null,
      totalBytes: null,
      processedDocs: null,
      totalDocs: null,
      lastTestSummary: null,
      lastTestPassed: null,
    });

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await nextTick();

    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "error",
      "knowledge.retrieval.runtimeInitFailed",
      expect.objectContaining({
        operation: "knowledge_embedding_runtime",
        replaceOperation: true,
      }),
    );
  });

  it("downloads a local embedding model without switching the active config", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.downloadSelectedLocalEmbeddingModel("BAAI/bge-large-zh-v1.5");

    expect(
      knowledgeDownloadWindowMocks.openKnowledgeDownloadProgressWindow,
    ).toHaveBeenCalledWith("BAAI/bge-large-zh-v1.5");
    expect(
      knowledgeMocks.knowledgeDownloadLocalEmbeddingModel,
    ).toHaveBeenCalledWith("BAAI/bge-large-zh-v1.5");
    expect(knowledgeMocks.knowledgeSaveEmbeddingConfig).not.toHaveBeenCalled();
  });

  it("treats a cancelled local embedding download as an info result", async () => {
    knowledgeMocks.knowledgeDownloadLocalEmbeddingModel.mockRejectedValueOnce({
      code: "knowledge.embedding_model_download_cancelled",
      message: "Model download cancelled",
    });

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.downloadSelectedLocalEmbeddingModel("Qwen/Qwen3-Embedding-0.6B");

    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "info",
      "knowledge.retrieval.modelDownloadCancelledNotice",
      expect.objectContaining({
        code: "knowledge.embedding_model_download_cancelled",
        operation: "knowledge_download_local_embedding_model",
        replaceOperation: true,
      }),
    );
    expect(notificationStoreMocks.addNotice).not.toHaveBeenCalledWith(
      "error",
      expect.any(String),
      expect.objectContaining({
        operation: "knowledge_download_local_embedding_model",
      }),
    );
    expect(state.error.value).toBe("");
  });

  it("loads unity reference import status with the overview data", async () => {
    knowledgeMocks.knowledgeGetUnityReferenceImportStatus.mockResolvedValueOnce(
      createUnityReferenceImportStatus({
        state: "ready",
        stage: "ready",
        importedProjectVersion: "2022.3.21f1",
        importedDocsVersion: "2022.3",
        importedLocale: "zh-CN",
        importedAt: 1710000000000,
        importedDocCount: 1248,
        message: "导入完成",
      }),
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();

    expect(
      knowledgeMocks.knowledgeGetUnityReferenceImportStatus,
    ).toHaveBeenCalledTimes(1);
    expect(state.unityReferenceImportStatus.value).toMatchObject({
      state: "ready",
      stage: "ready",
      importedDocsVersion: "2022.3",
      importedDocCount: 1248,
      importedLocale: "zh-CN",
    });
    expect(state.unityReferenceImportPending.value).toBe(false);
  });

  it("opens the unity reference import window without starting the job immediately", async () => {
    knowledgeMocks.knowledgeGetUnityReferenceImportStatus.mockResolvedValueOnce(
      createUnityReferenceImportStatus({
        state: "ready",
        stage: "ready",
        running: false,
        importedProjectVersion: "2022.3.21f1",
        importedDocsVersion: "2022.3",
        importedLocale: "zh-CN",
        importedDocCount: 1248,
        message: "导入完成",
      }),
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.importUnityReferenceDocs();

    expect(
      knowledgeMocks.knowledgeGetUnityReferenceImportStatus,
    ).toHaveBeenCalledTimes(1);
    expect(
      knowledgeMocks.knowledgeImportUnityReferenceDocs,
    ).not.toHaveBeenCalled();
    expect(
      unityReferenceImportWindowMocks.openUnityReferenceImportProgressWindow,
    ).toHaveBeenCalledWith();
    expect(state.unityReferenceImportStatus.value).toMatchObject({
      state: "ready",
      importedDocsVersion: "2022.3",
      importedDocCount: 1248,
    });
    expect(notificationStoreMocks.addNotice).not.toHaveBeenCalledWith(
      "info",
      "knowledge.referenceImport.startedNotice",
      expect.anything(),
    );
  });

  it("keeps unity imports on the managed window when a custom reference folder is selected", async () => {
    knowledgeMocks.knowledgeGetUnityReferenceImportStatus.mockResolvedValue(
      createUnityReferenceImportStatus(),
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.importUnityReferenceDocs("external/unity-manual");

    expect(
      knowledgeMocks.knowledgeGetUnityReferenceImportStatus,
    ).toHaveBeenCalledTimes(1);
    expect(
      unityReferenceImportWindowMocks.openUnityReferenceImportProgressWindow,
    ).toHaveBeenCalledWith();
  });

  it("opens the feishu reference import window for a selected reference folder", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.importFeishuReferenceDocs("external/feishu-wiki");

    expect(
      knowledgeMocks.knowledgeGetFeishuReferenceImportStatus,
    ).not.toHaveBeenCalled();
    expect(
      feishuReferenceImportWindowMocks.openFeishuReferenceImportProgressWindow,
    ).toHaveBeenCalledWith({
      targetPath: "external/feishu-wiki",
    });
  });

  it("treats a cancelled reimport as cancellation instead of success", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    state.unityReferenceImportStatus.value = createUnityReferenceImportStatus({
      state: "running",
      stage: "converting",
      running: true,
      importedDocsVersion: "2022.3",
      importedDocCount: 1248,
    });
    knowledgeMocks.knowledgeGetUnityReferenceImportStatus.mockResolvedValueOnce(
      createUnityReferenceImportStatus({
        state: "ready",
        stage: "ready",
        running: false,
        importedProjectVersion: "2022.3.21f1",
        importedDocsVersion: "2022.3",
        importedLocale: "zh-CN",
        importedDocCount: 1248,
        message: "已取消 Unity 文档导入。",
        lastOutcome: "cancelled",
      }),
    );

    await state.refreshUnityReferenceImportStatus(true);

    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "info",
      "knowledge.referenceImport.cancelledNotice",
      expect.objectContaining({
        operation: "knowledge_import_unity_reference_docs",
        replaceOperation: true,
      }),
    );
    expect(notificationStoreMocks.addNotice).not.toHaveBeenCalledWith(
      "success",
      "knowledge.referenceImport.completedNotice",
      expect.anything(),
    );
  });

  it("deletes imported unity reference docs through the shared service", async () => {
    knowledgeMocks.knowledgeList
      .mockResolvedValueOnce([
        {
          id: "reference-1",
          type: "reference",
          path: "unity-official-docs/script-reference/Transform.md",
          title: "Transform",
          externalSource: {
            provider: "unity",
            sourceId: "unity-2022.3",
            syncEnabled: true,
          },
          injectMode: "none",
          summaryEnabled: false,
          commandEnabled: false,
          readOnly: true,
          aiMaintained: false,
          explicitMaintenanceRules: true,
          summary: null,
          createdAt: 1,
          updatedAt: 2,
          hasSummary: false,
        },
      ])
      .mockResolvedValueOnce([]);
    knowledgeMocks.knowledgeListDirectories
      .mockResolvedValueOnce([
        "unity-official-docs",
        "unity-official-docs/script-reference",
      ])
      .mockResolvedValueOnce([]);
    knowledgeMocks.knowledgeGetUnityReferenceImportStatus.mockResolvedValueOnce(
      createUnityReferenceImportStatus({
        state: "missing_current_version",
        stage: "idle",
        running: false,
        importedProjectVersion: null,
        importedDocsVersion: null,
        importedLocale: null,
        importedDocCount: 0,
        importedAt: null,
        message: "已删除",
      }),
    );
    knowledgeMocks.knowledgeDeleteUnityReferenceDocs.mockResolvedValueOnce(
      createUnityReferenceImportStatus({
        state: "missing_current_version",
        stage: "idle",
        running: false,
        importedProjectVersion: null,
        importedDocsVersion: null,
        importedLocale: null,
        importedDocCount: 0,
        importedAt: null,
        message: "已删除",
      }),
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.deleteUnityReferenceDocs();

    expect(
      knowledgeMocks.knowledgeDeleteUnityReferenceDocs,
    ).toHaveBeenCalledTimes(1);
    expect(state.documents.value).toHaveLength(0);
    expect(state.unityReferenceImportStatus.value).toMatchObject({
      state: "missing",
      importedDocsVersion: null,
      importedDocCount: 0,
    });
    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "success",
      "knowledge.referenceImport.deletedNotice",
      expect.objectContaining({
        operation: "knowledge_delete_unity_reference_docs",
        replaceOperation: true,
      }),
    );
  });

  it("continues downloading when the progress window fails to open", async () => {
    knowledgeDownloadWindowMocks.openKnowledgeDownloadProgressWindow.mockRejectedValue(
      new Error("window failed"),
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.downloadSelectedLocalEmbeddingModel("BAAI/bge-m3");

    expect(
      knowledgeMocks.knowledgeDownloadLocalEmbeddingModel,
    ).toHaveBeenCalledWith("BAAI/bge-m3");
  });

  it("reads the selected document with its type-aware path", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectDocument(state.documents.value[0]!);

    expect(knowledgeMocks.knowledgeRead).toHaveBeenCalledWith({
      kind: "document",
      path: "combat/core-loop.md",
      type: "design",
      part: "full",
    });
    expect(state.selectedDocument.value?.body).toBe("正文");
  });

  it("does not reread a selected document after deleting it from the explorer", async () => {
    let deleted = false;
    const designDoc: KnowledgeDocumentSummary = {
      id: "design-1",
      type: "design",
      path: "combat/core-loop.md",
      title: "核心循环",
      injectMode: "excerpt",
      summaryEnabled: true,
      commandEnabled: false,
      readOnly: false,
      aiMaintained: false,
      explicitMaintenanceRules: false,
      summary: "摘要",
      createdAt: 1,
      updatedAt: 2,
      hasSummary: true,
    };
    const memoryDoc: KnowledgeDocumentSummary = {
      id: "memory-1",
      type: "memory",
      path: "project-understanding.md",
      title: "项目理解",
      injectMode: "full",
      summaryEnabled: false,
      commandEnabled: false,
      readOnly: false,
      aiMaintained: true,
      explicitMaintenanceRules: true,
      summary: null,
      createdAt: 1,
      updatedAt: 3,
      hasSummary: false,
    };

    knowledgeMocks.knowledgeList.mockImplementation(async (input: any = {}) => {
      const docs = deleted ? [memoryDoc] : [designDoc, memoryDoc];
      return docs.filter((doc) => !input.type || doc.type === input.type);
    });
    knowledgeMocks.knowledgeDelete.mockImplementation(async (input: any) => {
      deleted = true;
      return {
        kind: input.kind,
        type: input.type ?? "design",
        path: input.path,
        resultPath: input.path,
        document: null,
        directory: null,
      };
    });

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectDocument(designDoc);
    knowledgeMocks.knowledgeRead.mockClear();

    const folderNode = state.currentExplorerRoot.value?.children[0];
    expect(folderNode?.kind).toBe("folder");
    const documentNode =
      folderNode?.kind === "folder" ? folderNode.children[0] : null;
    expect(documentNode?.kind).toBe("document");

    await state.deleteExplorerNode(documentNode!);

    expect(knowledgeMocks.knowledgeDelete).toHaveBeenCalledWith({
      kind: "document",
      type: "design",
      path: "combat/core-loop.md",
    });
    expect(
      knowledgeMocks.knowledgeRead.mock.calls.some(([input]) => {
        const request = input as { kind?: string; path?: string; type?: string };
        return (
          request.kind === "document" &&
          request.type === "design" &&
          request.path === "combat/core-loop.md"
        );
      }),
    ).toBe(false);
    expect(state.selectedDocument.value).toBeNull();
    expect(state.selectedPath.value).toBeNull();
  });

  it("keeps the search state after selecting a search result", async () => {
    vi.useFakeTimers();
    try {
      knowledgeMocks.knowledgeQuery.mockResolvedValueOnce([
        {
          id: "design-1",
          type: "design",
          path: "combat/core-loop.md",
          title: "核心循环",
          injectMode: "excerpt",
          aiMaintained: false,
          snippet: "核心循环",
          matchKind: "lexical",
          matchedSection: "body",
          score: 0.92,
          estimatedTokens: 42,
          updatedAt: 2,
        },
      ]);

      const state = useKnowledgeState(
        reactive({
          workingDir: "F:/repo",
          selectedModelId: "",
          modelDefaults: {} as any,
        }),
      );

      await state.refreshKnowledgeData();
      state.searchQuery.value = "核心";
      await nextTick();
      await vi.advanceTimersByTimeAsync(220);

      expect(state.searchResults.value).toHaveLength(1);

      await state.selectSearchResult(state.searchResults.value[0]!);

      expect(state.searchQuery.value).toBe("核心");
      expect(state.searchResults.value).toHaveLength(1);
      expect(state.viewMode.value).toBe("search");
      expect(state.selectedSearchContext.value).toEqual({
        query: "核心",
        result: expect.objectContaining({
          id: "design-1",
          matchKind: "lexical",
          matchedSection: "body",
          snippet: "核心循环",
        }),
      });
      expect(state.selectedDocument.value?.path).toBe("combat/core-loop.md");
      expect(state.selectedPath.value).toBe("design/combat/core-loop.md");

      state.clearSearch();
      expect(state.selectedSearchContext.value).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("reads the selected folder config from the active type tree", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectDirectory("combat");

    expect(knowledgeMocks.knowledgeRead).toHaveBeenCalledWith({
      kind: "directory",
      path: "combat",
      type: "design",
    });
    expect(state.selectedDirectoryConfig.value?.configPath).toBe(
      "combat.locus-meta",
    );
    expect(state.selectedPath.value).toBe("design/combat");
  });

  it("shows directory-only memory cache folders in the explorer tree", async () => {
    knowledgeMocks.knowledgeListDirectories.mockImplementation(
      async (type: string) =>
        type === "memory" ? ["unity-project-understanding"] : [],
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectType("memory");

    expect(state.currentExplorerRoot.value?.children).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          kind: "folder",
          name: "unity-project-understanding",
        }),
      ]),
    );
  });

  it("reloads an inactive memory type after an external knowledge change", async () => {
    const designDoc = {
      id: "design-1",
      type: "design" as const,
      path: "combat/core-loop.md",
      title: "核心循环",
      injectMode: "excerpt" as const,
      summaryEnabled: true,
      commandEnabled: false,
      readOnly: false,
      aiMaintained: false,
      explicitMaintenanceRules: false,
      summary: "摘要",
      createdAt: 1,
      updatedAt: 2,
      hasSummary: true,
    };
    const initialMemoryDoc = {
      id: "memory-1",
      type: "memory" as const,
      path: "project-understanding.md",
      title: "项目理解",
      injectMode: "full" as const,
      summaryEnabled: false,
      commandEnabled: false,
      readOnly: false,
      aiMaintained: true,
      explicitMaintenanceRules: true,
      summary: null,
      createdAt: 1,
      updatedAt: 3,
      hasSummary: false,
    };
    const nextMemoryDoc = {
      id: "memory-2",
      type: "memory" as const,
      path: "project-memory.md",
      title: "项目记忆",
      injectMode: "full" as const,
      summaryEnabled: false,
      commandEnabled: false,
      readOnly: false,
      aiMaintained: true,
      explicitMaintenanceRules: true,
      summary: null,
      createdAt: 4,
      updatedAt: 5,
      hasSummary: false,
    };

    knowledgeMocks.knowledgeList.mockImplementation(
      async (input?: { type?: string }) => {
        if (input?.type === "design") return [designDoc];
        if (input?.type === "memory") return [initialMemoryDoc];
        return [designDoc, initialMemoryDoc];
      },
    );
    knowledgeMocks.knowledgeListDirectories.mockResolvedValue([]);
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectType("memory");
    expect(state.documents.value).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "memory",
          path: "project-understanding.md",
        }),
      ]),
    );

    await state.selectType("design");

    knowledgeMocks.knowledgeList.mockImplementation(
      async (input?: { type?: string }) => {
        if (input?.type === "design") return [designDoc];
        if (input?.type === "memory") return [nextMemoryDoc];
        return [designDoc, nextMemoryDoc];
      },
    );

    state.markKnowledgeDataDirty();
    await state.refreshKnowledgeData();
    await flushPromises();

    const memoryCallCountBeforeSwitch = knowledgeMocks.knowledgeList.mock.calls.filter(
      ([input]) => input?.type === "memory",
    ).length;

    await state.selectType("memory");

    expect(
      knowledgeMocks.knowledgeList.mock.calls.filter(
        ([input]) => input?.type === "memory",
      ).length,
    ).toBeGreaterThan(memoryCallCountBeforeSwitch);
    expect(state.documents.value).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "memory",
          path: "project-memory.md",
        }),
      ]),
    );
  });

  it("loads reference documents for an expanded folder when the full list does not include them", async () => {
    knowledgeMocks.knowledgeListDirectories.mockImplementation(
      async (type: string) => {
        if (type !== "reference") return [];
        return [
          "unity-official-docs",
          "unity-official-docs/script-reference",
          "unity-official-docs/script-reference/Transform",
        ];
      },
    );
    knowledgeMocks.knowledgeListDirectoryDocumentsPage.mockImplementation(
      async (type: string, path: string) => {
        if (
          type !== "reference" ||
          path !== "unity-official-docs/script-reference/Transform"
        ) {
          return {
            items: [],
            nextCursor: null,
          };
        }
        return {
          items: [
            {
              id: "reference-transform",
              type: "reference",
              path: "unity-official-docs/script-reference/Transform/Transform.md",
              title: "Transform",
              externalSource: {
                provider: "unity",
                sourceId: "unity-2022.3",
                syncEnabled: true,
              },
              injectMode: "none",
              summaryEnabled: false,
              commandEnabled: false,
              readOnly: true,
              aiMaintained: false,
              explicitMaintenanceRules: false,
              summary: null,
              createdAt: 1,
              updatedAt: 2,
              hasSummary: false,
            },
          ],
          nextCursor: null,
        };
      },
    );
    knowledgeMocks.knowledgeListUnityManagedDirectoryStats.mockResolvedValueOnce(
      [
        {
          path: "unity-official-docs",
          directChildCount: 2,
          descendantDocumentCount: 2429,
        },
        {
          path: "unity-official-docs/script-reference/Transform",
          directChildCount: 1,
          descendantDocumentCount: 1,
        },
      ],
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectType("reference");
    await state.togglePath("reference/unity-official-docs");
    await state.togglePath("reference/unity-official-docs/script-reference");
    await state.togglePath(
      "reference/unity-official-docs/script-reference/Transform",
    );

    const unityRoot = state.currentExplorerRoot.value?.children.find(
      (node) => node.kind === "folder" && node.name === "unity-official-docs",
    );
    const scriptReferenceFolder =
      unityRoot?.kind === "folder"
        ? unityRoot.children.find(
            (node) =>
              node.kind === "folder" && node.name === "script-reference",
          )
        : null;
    const transformFolder =
      scriptReferenceFolder?.kind === "folder"
        ? scriptReferenceFolder.children.find(
            (node) => node.kind === "folder" && node.name === "Transform",
          )
        : null;

    expect(
      knowledgeMocks.knowledgeListDirectoryDocumentsPage,
    ).toHaveBeenCalledWith(
      "reference",
      "unity-official-docs/script-reference/Transform",
      expect.any(Object),
    );
    expect(state.referenceManagedDirectoryStats.value).toMatchObject({
      "reference/unity-official-docs": {
        directChildCount: 2,
        descendantDocumentCount: 2429,
      },
    });
    expect(state.documents.value).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          path: "unity-official-docs/script-reference/Transform/Transform.md",
        }),
      ]),
    );
    expect(transformFolder).toEqual(
      expect.objectContaining({
        kind: "folder",
        children: expect.arrayContaining([
          expect.objectContaining({
            kind: "document",
            name: "Transform.md",
          }),
        ]),
      }),
    );
  });

  it("loads design documents for an expanded folder when root data is stale", async () => {
    knowledgeMocks.knowledgeList.mockImplementation(async (input?: { type?: string }) => {
      if (input?.type === "design") return [];
      return [];
    });
    knowledgeMocks.knowledgeListDirectories.mockImplementation(
      async (type: string) => (type === "design" ? ["system"] : []),
    );
    knowledgeMocks.knowledgeListDirectoryDocumentsPage.mockImplementation(
      async (type: string, path: string) => {
        if (type !== "design" || path !== "system") {
          return {
            items: [],
            nextCursor: null,
          };
        }
        return {
          items: [
            {
              id: "design-main-loop",
              type: "design",
              path: "system/主要玩法.md",
              title: "主要玩法",
              injectMode: "excerpt",
              summaryEnabled: true,
              commandEnabled: false,
              readOnly: false,
              aiMaintained: false,
              explicitMaintenanceRules: false,
              summary: "摘要",
              createdAt: 1,
              updatedAt: 2,
              hasSummary: true,
            },
          ],
          nextCursor: null,
        };
      },
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();

    const systemFolderBefore = state.currentExplorerRoot.value?.children.find(
      (node) => node.kind === "folder" && node.name === "system",
    );
    expect(systemFolderBefore).toEqual(
      expect.objectContaining({
        kind: "folder",
        children: [],
      }),
    );

    await state.togglePath("design/system");
    await nextTick();

    expect(
      knowledgeMocks.knowledgeListDirectoryDocumentsPage,
    ).toHaveBeenCalledWith("design", "system", expect.any(Object));
    expect(state.documents.value).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          path: "system/主要玩法.md",
        }),
      ]),
    );

    const systemFolderAfter = state.currentExplorerRoot.value?.children.find(
      (node) => node.kind === "folder" && node.name === "system",
    );
    expect(systemFolderAfter).toEqual(
      expect.objectContaining({
        kind: "folder",
        children: expect.arrayContaining([
          expect.objectContaining({
            kind: "document",
            name: "主要玩法.md",
          }),
        ]),
      }),
    );
  });

  it("updates sections without re-reading the active document", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectDocument(state.documents.value[0]!);
    const readCallsBeforeUpdate =
      knowledgeMocks.knowledgeRead.mock.calls.length;

    knowledgeMocks.knowledgeEdit.mockResolvedValueOnce({
      kind: "document",
      type: "design",
      path: "combat/core-loop.md",
      resultPath: "combat/core-loop.md",
      document: {
        id: "design-1",
        type: "design",
        path: "combat/core-loop.md",
        title: "核心循环",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "摘要",
        body: "正文 v2",
        maintenanceRules: null,
        createdAt: 1,
        updatedAt: 3,
        hasSummary: true,
      },
    });

    await state.updateSection(
      "design-1",
      "combat/core-loop.md",
      "body",
      "正文 v2",
    );

    expect(knowledgeMocks.knowledgeEdit).toHaveBeenCalledWith({
      kind: "document",
      type: "design",
      path: "combat/core-loop.md",
      document: {
        id: "design-1",
        summary: undefined,
        maintenanceRules: undefined,
        body: "正文 v2",
      },
    });
    expect(knowledgeMocks.knowledgeRead.mock.calls).toHaveLength(
      readCallsBeforeUpdate,
    );
    expect(state.selectedDocument.value?.body).toBe("正文 v2");
  });

  it("creates a new design document in proposal mode", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.createDocument("新文档");

    expect(knowledgeMocks.knowledgeCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "document",
        path: "新文档.md",
        document: expect.objectContaining({
          title: "新文档",
          body: "",
          inheritInjectMode: true,
          summaryEnabled: false,
          readOnly: false,
          inheritAiConfig: true,
        }),
      }),
    );
  });

  it("creates a new memory document with inherited category defaults", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    state.selectType("memory");
    await state.createDocument("项目记忆");

    expect(knowledgeMocks.knowledgeCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "document",
        type: "memory",
        path: "项目记忆.md",
        document: expect.objectContaining({
          title: "项目记忆",
          body: "",
          inheritInjectMode: true,
          summaryEnabled: false,
          readOnly: false,
          inheritAiConfig: true,
        }),
      }),
    );
  });

  it("appends .md when creating a document from the explorer name", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.createDocumentAt("systems", "core-loop");

    expect(knowledgeMocks.knowledgeCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "document",
        type: "design",
        path: "systems/core-loop.md",
      }),
    );

    knowledgeMocks.knowledgeCreate.mockClear();

    await state.createDocumentAt("systems", "combat.md");

    expect(knowledgeMocks.knowledgeCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "document",
        type: "design",
        path: "systems/combat.md",
      }),
    );
  });

  it("creates a new skill as a unified single-file document", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    state.selectType("skill");
    await state.createDocument("Create Skill");

    expect(knowledgeMocks.createSkillScaffold).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "create-skill",
        path: "create-skill.md",
        commandTrigger: "/create-skill",
      }),
    );
  });

  it("creates a nested skill with directory-style path", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    state.selectType("skill");
    await state.createDocumentAt("unity", "asset-audit");

    expect(knowledgeMocks.createSkillScaffold).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "asset-audit",
        path: "unity/asset-audit.md",
        commandTrigger: "/asset-audit",
      }),
    );
  });

  it("reports folder creation failures through the error banner", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    knowledgeMocks.knowledgeCreate.mockRejectedValueOnce(
      new Error("already exists"),
    );

    await state.createFolder("combat", "core-loop");

    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "error",
      expect.stringContaining("knowledge_create.directory"),
      expect.objectContaining({
        operation: "knowledge_create.directory",
        replaceOperation: true,
      }),
    );
  });

  it("moves a document into another directory", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    const folderNode = state.currentExplorerRoot.value?.children[0];
    expect(folderNode?.kind).toBe("folder");
    const documentNode =
      folderNode?.kind === "folder" ? folderNode.children[0] : null;
    expect(documentNode?.kind).toBe("document");

    await state.moveExplorerNode(documentNode!, "systems");

    expect(knowledgeMocks.knowledgeMove).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "document",
        type: "design",
        path: "combat/core-loop.md",
        newPath: "systems/core-loop.md",
      }),
    );
  });

  it("moves a folder into another directory", async () => {
    knowledgeMocks.knowledgeList.mockResolvedValue([
      {
        id: "design-1",
        type: "design",
        path: "combat/core-loop.md",
        title: "核心循环",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "摘要",
        createdAt: 1,
        updatedAt: 2,
        hasSummary: true,
      },
    ]);
    knowledgeMocks.knowledgeListDirectories.mockImplementation(
      async (type: string) => (type === "design" ? ["combat", "systems"] : []),
    );

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    const folderNode = state.currentExplorerRoot.value?.children[0];
    expect(folderNode?.kind).toBe("folder");

    await state.moveExplorerNode(folderNode!, "systems");

    expect(knowledgeMocks.knowledgeMove).toHaveBeenCalledWith({
      kind: "directory",
      type: "design",
      path: "combat",
      newPath: "systems/combat",
    });
  });

  it("represents package skills as package branches with selectable package info", async () => {
    const rootSkill: KnowledgeDocumentSummary = {
      id: "skill-package-root",
      type: "skill",
      path: "com.feishu.cli/SKILL.md",
      title: "Feishu CLI",
      injectMode: "excerpt",
      summaryEnabled: true,
      commandEnabled: true,
      readOnly: true,
      aiMaintained: false,
      storageSource: "app",
      explicitMaintenanceRules: false,
      externalSource: {
        provider: "package",
        sourceId: "com.feishu.cli",
        locator: "skills/com.feishu.cli",
      },
      skillEnabled: true,
      skillSurface: "both",
      commandTrigger: "/feishu",
      argumentHint: "[resource]",
      summary: "Use Feishu safely.",
      createdAt: 1,
      updatedAt: 2,
      hasSummary: true,
    };
    const childSkill: KnowledgeDocumentSummary = {
      ...rootSkill,
      id: "skill-package-child",
      path: "com.feishu.cli/reference/auth.md",
      title: "Auth",
      commandTrigger: null,
      argumentHint: null,
      summary: "Authentication notes.",
    };
    knowledgeMocks.knowledgeList.mockImplementation(async (input: any = {}) =>
      input.type === "skill" ? [childSkill, rootSkill] : [],
    );
    knowledgeMocks.knowledgeListDirectories.mockResolvedValue([]);

    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText,
      },
    });

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.selectType("skill");
    const packageNode = state.visibleExplorerTree.value[0];
    expect(packageNode).toMatchObject({
      kind: "package",
      name: "com.feishu.cli",
      path: "skill/com.feishu.cli",
    });
    expect(
      packageNode?.kind === "package"
        ? packageNode.children.some(
            (child) =>
              child.kind === "document" &&
              child.document.path === "com.feishu.cli/SKILL.md",
          )
        : false,
    ).toBe(true);
    expect(
      packageNode?.kind === "package"
        ? packageNode.children.some(
            (child) =>
              child.kind === "folder" &&
              child.relativePath === "com.feishu.cli/reference",
          )
        : false,
    ).toBe(true);

    if (packageNode?.kind !== "package") throw new Error("missing package node");
    await state.selectPackage(packageNode.document);

    expect(state.selectedPackageDocument.value?.path).toBe(
      "com.feishu.cli/SKILL.md",
    );
    expect(state.selectedPath.value).toBe("skill/com.feishu.cli");

    await state.updatePackageConfig({
      injectMode: "path",
      skillEnabled: true,
      skillSurface: "auto",
      commandTrigger: "/lark",
    });

    expect(knowledgeMocks.setSkillConfig).toHaveBeenCalledWith(
      "skill/com.feishu.cli",
      "app",
      {
        enabled: true,
        surface: "auto",
        description: "Use Feishu safely.",
        commandTrigger: "/lark",
        injectMode: "path",
      },
    );

    await state.copyExplorerRelativePath(packageNode);
    expect(writeText).toHaveBeenCalledWith("skill/com.feishu.cli");

    await state.openExplorerInFileSystem(packageNode);
    expect(knowledgeMocks.knowledgeRevealTarget).toHaveBeenCalledWith({
      kind: "directory",
      docType: "skill",
      path: "com.feishu.cli",
    });

    await state.deleteExplorerNode(packageNode);
    expect(knowledgeMocks.deleteSkillPackage).toHaveBeenCalledWith(
      "com.feishu.cli",
    );
  });

  it("renames a folder from the explorer and keeps the directory selection", async () => {
    const designDocs: KnowledgeDocumentSummary[] = [
      {
        id: "design-1",
        type: "design",
        path: "combat/core-loop.md",
        title: "核心循环",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "摘要",
        createdAt: 1,
        updatedAt: 2,
        hasSummary: true,
      },
    ];
    const designDirectories = ["combat"];
    knowledgeMocks.knowledgeList.mockImplementation(async () => designDocs);
    knowledgeMocks.knowledgeListDirectories.mockImplementation(
      async (type: string) => (type === "design" ? designDirectories : []),
    );
    knowledgeMocks.knowledgeMove.mockImplementation(async (input: any) => {
      if (
        input.kind === "directory" &&
        input.path === "combat" &&
        input.newPath === "systems"
      ) {
        designDocs[0] = {
          ...designDocs[0],
          path: "systems/core-loop.md",
        };
        designDirectories.splice(0, designDirectories.length, "systems");
      }
      return {
        kind: input.kind,
        type: input.type ?? "design",
        path: input.path,
        resultPath: input.newPath,
        document: null,
      };
    });

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectDirectory("combat");
    await state.renameExplorerFolder("combat", "systems");

    expect(knowledgeMocks.knowledgeMove).toHaveBeenCalledWith({
      kind: "directory",
      type: "design",
      path: "combat",
      newPath: "systems",
    });
    expect(state.selectedDirectoryPath.value).toBe("systems");
    expect(state.selectedDirectoryConfig.value?.path).toBe("systems");
  });

  it("renames a document from the explorer and keeps the document selection", async () => {
    const designDocs: KnowledgeDocumentSummary[] = [
      {
        id: "design-1",
        type: "design",
        path: "combat/core-loop.md",
        title: "核心循环",
        injectMode: "excerpt",
        summaryEnabled: true,
        commandEnabled: false,
        readOnly: false,
        aiMaintained: false,
        explicitMaintenanceRules: false,
        summary: "摘要",
        createdAt: 1,
        updatedAt: 2,
        hasSummary: true,
      },
    ];
    knowledgeMocks.knowledgeList.mockImplementation(async () => designDocs);
    knowledgeMocks.knowledgeListDirectories.mockResolvedValue(["combat"]);
    knowledgeMocks.knowledgeMove.mockImplementation(async (input: any) => {
      if (
        input.kind === "document" &&
        input.path === "combat/core-loop.md" &&
        input.newPath === "combat/systems-loop.md"
      ) {
        designDocs[0] = {
          ...designDocs[0],
          path: "combat/systems-loop.md",
          title: "系统循环",
        };
      }
      return {
        kind: input.kind,
        type: input.type ?? "design",
        path: input.path,
        resultPath: input.newPath,
        document:
          input.kind === "document"
            ? {
                id: "design-1",
                type: input.type ?? "design",
                path: input.newPath,
                title: "系统循环",
                injectMode: "excerpt",
                inheritInjectMode: false,
                injectModeSource: { kind: "self", path: null },
                summaryEnabled: true,
                commandEnabled: false,
                readOnly: false,
                aiMaintained: false,
                inheritAiConfig: false,
                aiConfigSource: { kind: "self", path: null },
                explicitMaintenanceRules: false,
                summary: "摘要",
                body: "正文",
                maintenanceRules: null,
                createdAt: 1,
                updatedAt: 3,
                hasSummary: true,
              }
            : null,
      };
    });

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectDocument({
      ...designDocs[0],
      explicitMaintenanceRules: false,
    });
    await state.renameExplorerDocument(
      "combat/core-loop.md",
      "systems-loop.md",
      "design",
    );

    expect(knowledgeMocks.knowledgeMove).toHaveBeenCalledWith({
      kind: "document",
      type: "design",
      path: "combat/core-loop.md",
      newPath: "combat/systems-loop.md",
    });
    expect(state.selectedDocument.value?.path).toBe("combat/systems-loop.md");
  });

  it("copies the explorer node path relative to the knowledge root", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText,
      },
    });

    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    const documentNode = state.currentExplorerRoot.value?.children[0];
    expect(documentNode?.kind).toBe("folder");
    const nestedDocumentNode =
      documentNode?.kind === "folder" ? documentNode.children[0] : null;
    expect(nestedDocumentNode?.kind).toBe("document");

    await state.copyExplorerRelativePath(nestedDocumentNode!);

    expect(writeText).toHaveBeenCalledWith("design/combat/core-loop.md");
    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "success",
      "knowledge.explorer.relativePathCopied",
      expect.objectContaining({
        operation: "knowledgeCopyRelativePath",
      }),
    );
  });

  it("reveals the explorer node in the system file manager", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    const folderNode = state.currentExplorerRoot.value?.children[0];
    expect(folderNode?.kind).toBe("folder");
    const nestedDocumentNode =
      folderNode?.kind === "folder" ? folderNode.children[0] : null;
    expect(nestedDocumentNode?.kind).toBe("document");

    await state.openExplorerInFileSystem(nestedDocumentNode!);

    expect(knowledgeMocks.knowledgeRevealTarget).toHaveBeenCalledWith({
      kind: "document",
      docType: "design",
      path: "combat/core-loop.md",
    });
  });

  it("saves the selected folder config through the directory config service", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    await state.selectDirectory("combat");
    await state.saveDirectoryConfig("combat", {
      version: 4,
      summary: "维护战斗与角色组织",
      injectMode: "path",
      aiMaintained: true,
      explicitMaintenanceRules: true,
      lexicalSearch: "enabled",
      vectorSearch: "disabled",
      inheritToChildren: true,
      allowCreateDocuments: true,
      allowCreateDirectories: true,
      allowMoveDocuments: true,
      allowMoveDirectories: true,
      maintenanceRules: "- 只记录稳定结构事实",
    });

    expect(knowledgeMocks.knowledgeEdit).toHaveBeenCalledWith({
      kind: "directory",
      type: "design",
      path: "combat",
      config: expect.objectContaining({
        injectMode: "path",
        aiMaintained: true,
        explicitMaintenanceRules: true,
        maintenanceRules: "- 只记录稳定结构事实",
      }),
    });
    expect(state.selectedDirectoryConfig.value?.summary).toBe(
      "维护战斗与角色组织",
    );
    expect(state.selectedDirectoryConfig.value?.injectMode).toBe("path");
  });

  it("deletes a folder from the explorer tree", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    const folderNode = state.currentExplorerRoot.value?.children[0];
    expect(folderNode?.kind).toBe("folder");

    await state.deleteExplorerNode(folderNode!);

    expect(knowledgeMocks.knowledgeDelete).toHaveBeenCalledWith({
      kind: "directory",
      type: "design",
      path: "combat",
    });
  });

  it("prunes nested document deletes when a selected folder already covers them", async () => {
    const state = useKnowledgeState(
      reactive({
        workingDir: "F:/repo",
        selectedModelId: "",
        modelDefaults: {} as any,
      }),
    );

    await state.refreshKnowledgeData();
    const folderNode = state.currentExplorerRoot.value?.children[0];
    expect(folderNode?.kind).toBe("folder");
    const documentNode =
      folderNode?.kind === "folder" ? folderNode.children[0] : null;
    expect(documentNode?.kind).toBe("document");

    await state.deleteExplorerNodes([folderNode!, documentNode!]);

    expect(knowledgeMocks.knowledgeDelete).toHaveBeenCalledTimes(1);
    expect(knowledgeMocks.knowledgeDelete).toHaveBeenCalledWith({
      kind: "directory",
      type: "design",
      path: "combat",
    });
  });
});
