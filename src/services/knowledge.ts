import { ipcInvoke } from "./ipc";
import type {
  KnowledgeConfigSource,
  KnowledgeDirectoryConfigRecord,
  KnowledgeExternalDirectoryBinding,
  KnowledgeDocument,
  KnowledgeManagedDirectoryStat,
  KnowledgeExternalSource,
  KnowledgeDocumentListPage,
  KnowledgeReadInput,
  KnowledgeDocumentListInput,
  KnowledgeDocumentQueryInput,
  KnowledgeGeneralConfig,
  KnowledgeRetrievalOverview,
  KnowledgeDocumentSummary,
  KnowledgeCreateInput,
  KnowledgeDeleteInput,
  KnowledgeEditInput,
  KnowledgeMoveInput,
  KnowledgeMutationResult,
  KnowledgeReadResult,
  KnowledgeSearchResult,
  EmbeddingConfig,
  EmbeddingLocalModelCatalog,
  EmbeddingLocalModelDirectoryInspection,
  EmbeddingRuntimeTestResult,
  EmbeddingStatus,
  EffectiveCapabilityState,
  FolderIndexRuleSetting,
  FeishuReferenceConfigInput,
  FeishuReferenceImportRequest,
  FeishuReferenceImportStatus,
  FeishuReferenceNodeSummary,
  FeishuReferenceOauthStartResult,
  FeishuSourceTestResult,
  LexicalRebuildStatus,
  UnityReferenceImportLocale,
  UnityReferenceImportStatus,
  SkillConfig,
  SkillManifest,
} from "../types";

interface KnowledgeReadPayload {
  id: string;
  type: KnowledgeDocument["type"];
  path: string;
  title: string;
  injectMode: KnowledgeDocument["injectMode"];
  inheritInjectMode?: boolean;
  injectModeSource?: KnowledgeConfigSource | null;
  summaryEnabled: boolean;
  commandEnabled: boolean;
  readOnly: boolean;
  aiMaintained: boolean;
  storageSource?: KnowledgeDocument["storageSource"];
  inheritAiConfig?: boolean;
  aiConfigSource?: KnowledgeConfigSource | null;
  explicitMaintenanceRules: boolean;
  externalSource?: KnowledgeDocument["externalSource"];
  skillEnabled?: KnowledgeDocument["skillEnabled"];
  skillSurface?: KnowledgeDocument["skillSurface"];
  commandTrigger?: KnowledgeDocument["commandTrigger"];
  argumentHint?: KnowledgeDocument["argumentHint"];
  summary?: string | null;
  body?: string;
  maintenanceRules?: string | null;
  createdAt: number;
  updatedAt: number;
  hasBodyContent?: boolean;
  part?: "full" | "summary" | "body";
  fileMetadata?: KnowledgeDocument["fileMetadata"];
}

interface KnowledgeQueryPayload {
  id: string;
  type: KnowledgeSearchResult["type"];
  path: string;
  title: string;
  storageSource?: KnowledgeSearchResult["storageSource"];
  injectMode: KnowledgeSearchResult["injectMode"];
  aiMaintained: KnowledgeSearchResult["aiMaintained"];
  score: number;
  snippet: string;
  matchedSection?: KnowledgeSearchResult["matchedSection"] | null;
  hasSummary: boolean;
  updatedAt: number;
  matchKind?: KnowledgeSearchResult["matchKind"];
  semanticScore?: number | null;
  semanticConfidence?: number | null;
  estimatedTokens?: number | null;
}

interface KnowledgeReadResultPayload {
  kind: "document" | "directory";
  document?: KnowledgeReadPayload | null;
  directory?: KnowledgeDirectoryConfigPayload | null;
}

interface KnowledgeMutationPayload {
  kind: "document" | "directory";
  type: KnowledgeDocument["type"];
  path: string;
  resultPath?: string | null;
  document?: KnowledgeReadPayload | null;
  directory?: KnowledgeDirectoryConfigPayload | null;
}

interface KnowledgeDirectoryConfigPayload {
  version: number;
  summary: string;
  injectMode?: KnowledgeDocument["injectMode"];
  inheritInjectMode?: boolean;
  aiMaintained: boolean;
  inheritAiConfig?: boolean;
  explicitMaintenanceRules: boolean;
  lexicalSearch?: FolderIndexRuleSetting;
  vectorSearch?: FolderIndexRuleSetting;
  inheritToChildren: boolean;
  allowCreateDocuments: boolean;
  allowCreateDirectories: boolean;
  allowMoveDocuments: boolean;
  allowMoveDirectories: boolean;
  maintenanceRules: string;
  type: KnowledgeDocument["type"];
  path: string;
  configPath: string;
  exists: boolean;
  readOnly?: boolean;
  updatedAt: number;
  injectModeSource?: KnowledgeConfigSource | null;
  aiConfigSource?: KnowledgeConfigSource | null;
  effectiveLexicalSearch?: EffectiveCapabilityState | null;
  effectiveVectorSearch?: EffectiveCapabilityState | null;
  externalSources?: KnowledgeExternalSource[] | null;
}

interface KnowledgeExternalDirectoryBindingPayload {
  path: string;
  externalSources?: KnowledgeExternalSource[] | null;
}

function normalizeEffectiveCapabilityState(
  payload?: EffectiveCapabilityState | null,
): EffectiveCapabilityState {
  return {
    enabled: payload?.enabled ?? true,
    source: payload?.source ?? "default",
    reasonCode: payload?.reasonCode ?? undefined,
    sourceDir: payload?.sourceDir ?? undefined,
  };
}

function resolveKnowledgeDocumentPath(
  path?: string,
  type?: KnowledgeDocument["type"],
): string {
  const normalized = (path ?? "").trim().replace(/\\/g, "/");
  if (!normalized) {
    throw new Error("knowledge_read requires path");
  }
  if (/^(design|memory|skill|reference)\//.test(normalized)) {
    return normalized;
  }
  if (!type) {
    throw new Error(
      "knowledge_read requires a document type when path is not type-prefixed",
    );
  }
  return `${type}/${normalized}`;
}

function resolveKnowledgeDirectoryPath(
  path: string,
  type?: KnowledgeDocument["type"],
): string {
  const normalized = (path ?? "")
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
  if (!normalized) {
    throw new Error("knowledge_read requires path");
  }
  if (/^(design|memory|skill|reference)\//.test(normalized)) {
    return normalized;
  }
  if (!type) {
    throw new Error(
      "knowledge_read requires a directory type when path is not type-prefixed",
    );
  }
  return `${type}/${normalized}`;
}

function normalizeDocument(payload: KnowledgeReadPayload): KnowledgeDocument {
  return {
    id: payload.id,
    type: payload.type,
    path: payload.path,
    title: payload.title,
    injectMode: payload.injectMode,
    inheritInjectMode: payload.inheritInjectMode ?? false,
    injectModeSource: payload.injectModeSource ?? { kind: "self", path: null },
    summaryEnabled: payload.summaryEnabled,
    commandEnabled: payload.commandEnabled,
    readOnly: payload.readOnly,
    aiMaintained: payload.aiMaintained,
    storageSource: payload.storageSource ?? "project",
    inheritAiConfig: payload.inheritAiConfig ?? false,
    aiConfigSource: payload.aiConfigSource ?? { kind: "self", path: null },
    explicitMaintenanceRules: payload.explicitMaintenanceRules ?? false,
    externalSource: payload.externalSource ?? null,
    skillEnabled: payload.skillEnabled ?? null,
    skillSurface: payload.skillSurface ?? null,
    commandTrigger: payload.commandTrigger ?? null,
    argumentHint: payload.argumentHint ?? null,
    summary: payload.summary ?? null,
    body: payload.body ?? "",
    maintenanceRules: payload.maintenanceRules ?? null,
    createdAt: payload.createdAt,
    updatedAt: payload.updatedAt,
    hasSummary: payload.summaryEnabled && !!payload.summary?.trim(),
    hasBodyContent: payload.hasBodyContent ?? !!payload.body?.trim(),
    fileMetadata: payload.fileMetadata ?? null,
  };
}

function normalizeDirectoryConfig(
  payload: KnowledgeDirectoryConfigPayload,
): KnowledgeDirectoryConfigRecord {
  return {
    version: payload.version,
    summary: payload.summary ?? "",
    injectMode: payload.injectMode ?? "excerpt",
    inheritInjectMode: payload.inheritInjectMode ?? false,
    aiMaintained: !!payload.aiMaintained,
    inheritAiConfig: payload.inheritAiConfig ?? false,
    explicitMaintenanceRules: !!payload.explicitMaintenanceRules,
    lexicalSearch: payload.lexicalSearch ?? "inherit",
    vectorSearch: payload.vectorSearch ?? "inherit",
    inheritToChildren: payload.inheritToChildren !== false,
    allowCreateDocuments: payload.allowCreateDocuments !== false,
    allowCreateDirectories: payload.allowCreateDirectories !== false,
    allowMoveDocuments: payload.allowMoveDocuments !== false,
    allowMoveDirectories: payload.allowMoveDirectories !== false,
    maintenanceRules: payload.maintenanceRules ?? "",
    type: payload.type,
    path: payload.path,
    configPath: payload.configPath,
    exists: !!payload.exists,
    readOnly: !!payload.readOnly,
    updatedAt: payload.updatedAt ?? 0,
    injectModeSource: payload.injectModeSource ?? { kind: "self", path: null },
    aiConfigSource: payload.aiConfigSource ?? { kind: "self", path: null },
    effectiveLexicalSearch: normalizeEffectiveCapabilityState(
      payload.effectiveLexicalSearch,
    ),
    effectiveVectorSearch: normalizeEffectiveCapabilityState(
      payload.effectiveVectorSearch,
    ),
    externalSources: Array.isArray(payload.externalSources)
      ? payload.externalSources.filter(Boolean)
      : [],
  };
}

function normalizeReadResult(
  payload: KnowledgeReadResultPayload,
): KnowledgeReadResult {
  return {
    kind: payload.kind,
    document: payload.document ? normalizeDocument(payload.document) : null,
    directory: payload.directory
      ? normalizeDirectoryConfig(payload.directory)
      : null,
  };
}

function normalizeExternalDirectoryBinding(
  payload: KnowledgeExternalDirectoryBindingPayload,
): KnowledgeExternalDirectoryBinding {
  return {
    path: payload.path,
    externalSources: Array.isArray(payload.externalSources)
      ? payload.externalSources.filter(Boolean)
      : [],
  };
}

function normalizeMutationResult(
  payload: KnowledgeMutationPayload,
): KnowledgeMutationResult {
  return {
    kind: payload.kind,
    type: payload.type,
    path: payload.path,
    resultPath: payload.resultPath ?? null,
    document: payload.document ? normalizeDocument(payload.document) : null,
    directory: payload.directory
      ? normalizeDirectoryConfig(payload.directory)
      : null,
  };
}

export function knowledgeList(
  input: KnowledgeDocumentListInput = {},
): Promise<KnowledgeDocumentSummary[]> {
  return ipcInvoke<KnowledgeDocumentSummary[]>("knowledge_list", {
    docType: input.type,
    pathPrefix: input.pathPrefix,
  });
}

export function knowledgeListPage(
  input: KnowledgeDocumentListInput = {},
): Promise<KnowledgeDocumentListPage> {
  return ipcInvoke<KnowledgeDocumentListPage>("knowledge_list_page", {
    docType: input.type,
    pathPrefix: input.pathPrefix,
    cursor: input.cursor,
    limit: input.limit,
  });
}

export function knowledgeListDirectories(
  type: KnowledgeDocument["type"],
): Promise<string[]> {
  return ipcInvoke<string[]>("knowledge_list_directories", { docType: type });
}

export function knowledgeListDirectoryDocuments(
  type: KnowledgeDocument["type"],
  path: string,
): Promise<KnowledgeDocumentSummary[]> {
  return ipcInvoke<KnowledgeDocumentSummary[]>(
    "knowledge_list_directory_documents",
    { docType: type, path },
  );
}

export function knowledgeListDirectoryDocumentsPage(
  type: KnowledgeDocument["type"],
  path: string,
  options: { cursor?: string | null; limit?: number } = {},
): Promise<KnowledgeDocumentListPage> {
  return ipcInvoke<KnowledgeDocumentListPage>(
    "knowledge_list_directory_documents_page",
    {
      docType: type,
      path,
      cursor: options.cursor,
      limit: options.limit,
    },
  );
}

export async function knowledgeListExternalReferenceDirectories(): Promise<
  KnowledgeExternalDirectoryBinding[]
> {
  const payload = await ipcInvoke<KnowledgeExternalDirectoryBindingPayload[]>(
    "knowledge_list_external_reference_directories",
  );
  return payload.map(normalizeExternalDirectoryBinding);
}

export function knowledgeListUnityManagedDirectoryStats(): Promise<
  KnowledgeManagedDirectoryStat[]
> {
  return ipcInvoke<KnowledgeManagedDirectoryStat[]>(
    "knowledge_list_unity_managed_directory_stats",
  );
}

export async function knowledgeQuery(
  input: KnowledgeDocumentQueryInput,
): Promise<KnowledgeSearchResult[]> {
  const results = await ipcInvoke<KnowledgeQueryPayload[]>("knowledge_query", {
    query: input.query,
    limit: input.limit,
    types: input.types,
    pathPrefix: input.pathPrefix,
  });

  return results.map((result) => ({
    id: result.id,
    type: result.type,
    path: result.path,
    title: result.title,
    storageSource: result.storageSource ?? "project",
    injectMode: result.injectMode,
    aiMaintained: result.aiMaintained,
    score: result.score,
    snippet: result.snippet,
    matchKind: result.matchKind ?? "lexical",
    matchedSection: result.matchedSection ?? null,
    semanticScore: result.semanticScore ?? null,
    semanticConfidence: result.semanticConfidence ?? null,
    estimatedTokens: result.estimatedTokens ?? undefined,
    updatedAt: result.updatedAt,
  }));
}

export function knowledgeGetGeneralConfig(): Promise<KnowledgeGeneralConfig> {
  return ipcInvoke<KnowledgeGeneralConfig>("knowledge_get_general_config");
}

export function knowledgeSaveGeneralConfig(
  config: KnowledgeGeneralConfig,
): Promise<KnowledgeGeneralConfig> {
  return ipcInvoke<KnowledgeGeneralConfig>("knowledge_save_general_config", {
    config,
  });
}

export function knowledgeGetEmbeddingConfig(): Promise<EmbeddingConfig> {
  return ipcInvoke<EmbeddingConfig>("knowledge_get_embedding_config");
}

export function knowledgeSaveEmbeddingConfig(
  config: EmbeddingConfig,
): Promise<EmbeddingConfig> {
  return ipcInvoke<EmbeddingConfig>("knowledge_save_embedding_config", {
    config,
  });
}

export function knowledgeActivateEmbedding(): Promise<void> {
  return ipcInvoke<void>("knowledge_activate_embedding");
}

export function knowledgeDeactivateEmbedding(): Promise<void> {
  return ipcInvoke<void>("knowledge_deactivate_embedding");
}

export function knowledgeGetEmbeddingStatus(): Promise<EmbeddingStatus> {
  return ipcInvoke<EmbeddingStatus>("knowledge_get_embedding_status");
}

export function knowledgeTestEmbeddingRuntime(): Promise<EmbeddingRuntimeTestResult> {
  return ipcInvoke<EmbeddingRuntimeTestResult>(
    "knowledge_test_embedding_runtime",
  );
}

export function knowledgeGetLocalEmbeddingModelCatalog(): Promise<EmbeddingLocalModelCatalog> {
  return ipcInvoke<EmbeddingLocalModelCatalog>(
    "knowledge_get_local_embedding_model_catalog",
  );
}

export function knowledgeDownloadLocalEmbeddingModel(
  modelId: string,
): Promise<void> {
  return ipcInvoke<void>("knowledge_download_local_embedding_model", {
    modelId,
  });
}

export function knowledgeCancelLocalEmbeddingModelDownload(): Promise<void> {
  return ipcInvoke<void>("knowledge_cancel_local_embedding_model_download");
}

export function knowledgeCloseDownloadProgressWindow(): Promise<void> {
  return ipcInvoke<void>("knowledge_close_download_progress_window");
}

export function knowledgeCloseUnityReferenceImportProgressWindow(): Promise<void> {
  return ipcInvoke<void>(
    "knowledge_close_unity_reference_import_progress_window",
  );
}

export function knowledgeCloseFeishuReferenceImportProgressWindow(): Promise<void> {
  return ipcInvoke<void>(
    "knowledge_close_feishu_reference_import_progress_window",
  );
}

export function knowledgeInspectLocalEmbeddingModelDirectory(
  path: string,
): Promise<EmbeddingLocalModelDirectoryInspection> {
  return ipcInvoke<EmbeddingLocalModelDirectoryInspection>(
    "knowledge_inspect_local_embedding_model_directory",
    { path },
  );
}

export function knowledgeRebuildLexicalIndex(): Promise<number> {
  return ipcInvoke<number>("knowledge_rebuild_lexical_index");
}

export function knowledgeGetLexicalRebuildStatus(): Promise<LexicalRebuildStatus> {
  return ipcInvoke<LexicalRebuildStatus>(
    "knowledge_get_lexical_rebuild_status",
  );
}

export function knowledgeGetOverview(): Promise<KnowledgeRetrievalOverview> {
  return ipcInvoke<KnowledgeRetrievalOverview>("knowledge_get_overview");
}

export function knowledgeGetUnityReferenceImportStatus(
  targetPath?: string | null,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_get_unity_reference_import_status",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export async function knowledgeFindUnityReferenceDirectory(): Promise<KnowledgeDirectoryConfigRecord | null> {
  const payload = await ipcInvoke<KnowledgeDirectoryConfigPayload | null>(
    "knowledge_find_unity_reference_directory",
  );
  return payload ? normalizeDirectoryConfig(payload) : null;
}

export function knowledgeGetFeishuReferenceImportStatus(
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_get_feishu_reference_import_status",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export function knowledgeCancelUnityReferenceImport(
  targetPath?: string | null,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_cancel_unity_reference_import",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export function knowledgeCancelFeishuReferenceImport(
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_cancel_feishu_reference_import",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export function knowledgeImportUnityReferenceDocs(
  targetPath?: string | null,
  locale?: UnityReferenceImportLocale,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_import_unity_reference_docs",
    {
      targetPath: targetPath ?? null,
      locale,
    },
  );
}

export function knowledgeSaveFeishuReferenceConfig(
  config: FeishuReferenceConfigInput,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_save_feishu_reference_config",
    {
      config,
    },
  );
}

export function knowledgeTestFeishuReferenceConnection(
  targetPath?: string | null,
): Promise<FeishuSourceTestResult> {
  return ipcInvoke<FeishuSourceTestResult>(
    "knowledge_test_feishu_reference_connection",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export function knowledgeStartFeishuReferenceOauth(): Promise<FeishuReferenceOauthStartResult> {
  return ipcInvoke<FeishuReferenceOauthStartResult>(
    "knowledge_start_feishu_reference_oauth",
  );
}

export function knowledgeCancelFeishuReferenceOauthWait(
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_cancel_feishu_reference_oauth_wait",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export function knowledgeListFeishuReferenceSpaceNodes(
  spaceId: string,
  parentNodeToken?: string | null,
): Promise<FeishuReferenceNodeSummary[]> {
  return ipcInvoke<FeishuReferenceNodeSummary[]>(
    "knowledge_list_feishu_reference_space_nodes",
    {
      spaceId,
      parentNodeToken: parentNodeToken ?? null,
    },
  );
}

export function knowledgeImportFeishuReferenceDocs(
  request: FeishuReferenceImportRequest,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_import_feishu_reference_docs",
    {
      request,
    },
  );
}

export function knowledgeDeleteUnityReferenceDocs(
  targetPath?: string | null,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_delete_unity_reference_docs",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export function knowledgeDeleteFeishuReferenceDocs(
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_delete_feishu_reference_docs",
    {
      targetPath: targetPath ?? null,
    },
  );
}

export function knowledgeRevealTarget(input: {
  kind: "document" | "directory";
  docType: KnowledgeDocument["type"];
  path: string;
}): Promise<void> {
  return ipcInvoke<void>("knowledge_reveal_target", {
    request: input,
  });
}

export async function knowledgeRead(
  input: KnowledgeReadInput,
): Promise<KnowledgeReadResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const payload = await ipcInvoke<KnowledgeReadResultPayload>(
    "knowledge_read",
    {
      request: {
        kind: input.kind,
        path,
        type: input.type,
        part: input.part ?? "full",
      },
    },
  );
  return normalizeReadResult(payload);
}

export async function knowledgeCreate(
  input: KnowledgeCreateInput,
): Promise<KnowledgeMutationResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const payload = await ipcInvoke<KnowledgeMutationPayload>(
    "knowledge_create",
    {
      request: {
        kind: input.kind,
        path,
        type: input.type,
        document: input.document,
      },
    },
  );
  return normalizeMutationResult(payload);
}

export async function knowledgeEdit(
  input: KnowledgeEditInput,
): Promise<KnowledgeMutationResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const payload = await ipcInvoke<KnowledgeMutationPayload>("knowledge_edit", {
    request: {
      kind: input.kind,
      path,
      type: input.type,
      document: input.document,
      config: input.config,
    },
  });
  return normalizeMutationResult(payload);
}

export async function knowledgeMove(
  input: KnowledgeMoveInput,
): Promise<KnowledgeMutationResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const newPath =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.newPath, input.type)
      : resolveKnowledgeDocumentPath(input.newPath, input.type);
  const payload = await ipcInvoke<KnowledgeMutationPayload>("knowledge_move", {
    request: {
      kind: input.kind,
      path,
      type: input.type,
      newPath,
    },
  });
  return normalizeMutationResult(payload);
}

export async function knowledgeDelete(
  input: KnowledgeDeleteInput,
): Promise<KnowledgeMutationResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const payload = await ipcInvoke<KnowledgeMutationPayload>(
    "knowledge_delete",
    {
      request: {
        kind: input.kind,
        path,
        type: input.type,
      },
    },
  );
  return normalizeMutationResult(payload);
}

export function knowledgeDeleteExternalReferenceDirectory(
  path: string,
): Promise<void> {
  return ipcInvoke<void>("knowledge_delete_external_reference_directory", {
    path: resolveKnowledgeDirectoryPath(path, "reference"),
  });
}

export function getSkillConfig(
  relPath: string,
  source?: "app" | "project",
): Promise<SkillConfig> {
  return ipcInvoke<SkillConfig>("get_skill_config", { relPath, source });
}

export function setSkillConfig(
  relPath: string,
  source: "app" | "project" | undefined,
  config: SkillConfig,
): Promise<void> {
  return ipcInvoke("set_skill_config", {
    relPath,
    source,
    enabled: config.enabled,
    surface: config.surface,
    description: config.description,
    commandTrigger: config.commandTrigger,
  });
}

export function getAllSkillConfigs(): Promise<Record<string, SkillConfig>> {
  return ipcInvoke<Record<string, SkillConfig>>("get_all_skill_configs");
}

export function listSkills(): Promise<SkillManifest[]> {
  return ipcInvoke<SkillManifest[]>("list_skills");
}

export function readSkillManifest(
  dirName: string,
  source?: string,
): Promise<string> {
  return ipcInvoke<string>("read_skill_manifest", { dirName, source });
}

export function createSkillScaffold(name: string): Promise<SkillManifest> {
  return ipcInvoke<SkillManifest>("create_skill_scaffold", { name });
}
