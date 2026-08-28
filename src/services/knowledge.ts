import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";
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
  KnowledgeInjectMode,
  KnowledgeInjectModeSetting,
  KnowledgeAiMaintainedSetting,
  KnowledgeAiEditMode,
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
  LocalReferenceImportRequest,
  LocalReferenceImportStatus,
  LocalReferenceScanPreview,
  SkillCreateInput,
  UnityReferenceImportLocale,
  UnityReferenceImportStatus,
  SkillConfig,
  SkillManifest,
  SkillPackageArchiveResult,
  SkillUnityInstallStatus,
} from "../types";

interface KnowledgeReadPayload {
  id: string;
  type: KnowledgeDocument["type"];
  path: string;
  title: string;
  injectMode: KnowledgeInjectMode;
  inheritInjectMode?: boolean;
  injectModeSource?: KnowledgeConfigSource | null;
  summaryEnabled: boolean;
  commandEnabled: boolean;
  readOnly: boolean;
  aiMaintained: boolean;
  aiEditMode?: KnowledgeAiEditMode;
  storageSource?: KnowledgeDocument["storageSource"];
  inheritAiConfig?: boolean;
  aiConfigSource?: KnowledgeConfigSource | null;
  explicitMaintenanceRules: boolean;
  externalSource?: KnowledgeDocument["externalSource"];
  skillEnabled?: KnowledgeDocument["skillEnabled"];
  skillSurface?: KnowledgeDocument["skillSurface"];
  commandTrigger?: KnowledgeDocument["commandTrigger"];
  argumentHint?: KnowledgeDocument["argumentHint"];
  tools?: KnowledgeDocument["tools"];
  summary?: string | null;
  body?: string;
  maintenanceRules?: string | null;
  createdAt: number;
  updatedAt: number;
  hasBodyContent?: boolean;
  part?: "full" | "summary" | "body" | "maintenanceRules";
  fileMetadata?: KnowledgeDocument["fileMetadata"];
}

interface KnowledgeQueryPayload {
  id: string;
  type: KnowledgeSearchResult["type"];
  path: string;
  title: string;
  storageSource?: KnowledgeSearchResult["storageSource"];
  injectMode: KnowledgeInjectMode;
  aiMaintained: boolean;
  score: number;
  snippet: string;
  matchedSection?: KnowledgeSearchResult["matchedSection"] | null;
  matchedTerms?: string[];
  hasSummary: boolean;
  updatedAt: number;
  matchKind?: KnowledgeSearchResult["matchKind"];
  semanticScore?: number | null;
  semanticConfidence?: number | null;
  estimatedTokens?: number | null;
  physicalPath?: string;
  displayPath?: string;
  startLine?: number;
  endLine?: number;
  summaryStartLine?: number | null;
  bodyStartLine?: number;
}

interface KnowledgeDocumentSummaryPayload
  extends Omit<
    KnowledgeReadPayload,
    "body" | "maintenanceRules" | "part" | "fileMetadata"
  > {
  hasBodyContent?: boolean;
  byteSize?: number | null;
  lexicalSearchEnabled?: boolean | null;
  semanticSearchEnabled?: boolean | null;
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
  injectMode?: KnowledgeInjectMode;
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
  if (/^(design|plan|memory|skill|reference)\//.test(normalized)) {
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
  if (/^(design|plan|memory|skill|reference)\//.test(normalized)) {
    return normalized;
  }
  if (type && normalized === type) {
    return normalized;
  }
  if (/^(design|plan|memory|skill|reference)$/.test(normalized) && !type) {
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
  const summary = payload.summaryEnabled && payload.summary?.trim()
    ? payload.summary.trim()
    : null;
  const injectMode: KnowledgeInjectModeSetting = payload.inheritInjectMode
    ? "inherit"
    : payload.injectMode;
  const aiMaintained: KnowledgeAiMaintainedSetting = payload.inheritAiConfig
    ? "inherit"
    : payload.aiMaintained;
  const aiEditMode: KnowledgeAiEditMode = payload.aiEditMode
    ?? (payload.inheritAiConfig ? "inherit" : payload.aiMaintained ? "auto" : "confirm");
  const maintenanceRules = !payload.inheritAiConfig
    && payload.explicitMaintenanceRules
    && payload.maintenanceRules?.trim()
    ? payload.maintenanceRules
    : null;
  return {
    id: payload.id,
    type: payload.type,
    path: payload.path,
    title: payload.title,
    injectMode,
    effectiveInjectMode: payload.injectMode,
    injectModeSource: payload.injectModeSource ?? { kind: "self", path: null },
    readOnly: payload.readOnly,
    aiEditMode,
    aiMaintained,
    effectiveAiMaintained: payload.aiMaintained,
    storageSource: payload.storageSource ?? "project",
    aiConfigSource: payload.aiConfigSource ?? { kind: "self", path: null },
    externalSource: payload.externalSource ?? null,
    skillEnabled: payload.skillEnabled ?? null,
    skillSurface: payload.skillSurface ?? null,
    commandTrigger: payload.commandTrigger ?? null,
    argumentHint: payload.argumentHint ?? null,
    tools: payload.tools ?? [],
    summary,
    body: payload.body ?? "",
    maintenanceRules,
    effectiveMaintenanceRules: payload.maintenanceRules ?? null,
    modifiedAt: payload.updatedAt,
    hasBodyContent: payload.hasBodyContent ?? !!payload.body?.trim(),
    fileMetadata: payload.fileMetadata ?? null,
  };
}

function normalizeDocumentSummary(
  payload: KnowledgeDocumentSummaryPayload,
): KnowledgeDocumentSummary {
  const document = normalizeDocument({
    ...payload,
    body: "",
    maintenanceRules: null,
  });
  const {
    body: _body,
    maintenanceRules: _maintenanceRules,
    effectiveMaintenanceRules: _effectiveMaintenanceRules,
    fileMetadata: _fileMetadata,
    ...summary
  } = document;
  return {
    ...summary,
    hasBodyContent: payload.hasBodyContent ?? false,
    byteSize: payload.byteSize ?? undefined,
    lexicalSearchEnabled: payload.lexicalSearchEnabled ?? undefined,
    semanticSearchEnabled: payload.semanticSearchEnabled ?? undefined,
  };
}

function normalizeDirectoryConfig(
  payload: KnowledgeDirectoryConfigPayload,
): KnowledgeDirectoryConfigRecord {
  const injectMode: KnowledgeInjectModeSetting = payload.inheritInjectMode
    ? "inherit"
    : (payload.injectMode ?? "excerpt");
  const aiMaintained: KnowledgeAiMaintainedSetting = payload.inheritAiConfig
    ? "inherit"
    : !!payload.aiMaintained;
  const maintenanceRules = !payload.inheritAiConfig
    && payload.explicitMaintenanceRules
    && payload.maintenanceRules?.trim()
    ? payload.maintenanceRules
    : null;
  return {
    version: payload.version,
    summary: payload.summary ?? "",
    injectMode,
    effectiveInjectMode: payload.injectMode ?? "excerpt",
    aiMaintained,
    effectiveAiMaintained: !!payload.aiMaintained,
    lexicalSearch: payload.lexicalSearch ?? "inherit",
    vectorSearch: payload.vectorSearch ?? "inherit",
    inheritToChildren: payload.inheritToChildren !== false,
    allowCreateDocuments: payload.allowCreateDocuments !== false,
    allowCreateDirectories: payload.allowCreateDirectories !== false,
    allowMoveDocuments: payload.allowMoveDocuments !== false,
    allowMoveDirectories: payload.allowMoveDirectories !== false,
    maintenanceRules,
    effectiveMaintenanceRules: payload.maintenanceRules ?? null,
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

export async function knowledgeList(
  input: KnowledgeDocumentListInput = {},
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeDocumentSummary[]> {
  const payload = await ipcInvoke<KnowledgeDocumentSummaryPayload[]>(
    "knowledge_list",
    {
      workspaceRef,
      docType: input.type,
      pathPrefix: input.pathPrefix,
      includeHidden: input.includeHidden ?? true,
    },
  );
  return payload.map(normalizeDocumentSummary);
}

function documentPatchForIpc(
  patch: NonNullable<KnowledgeCreateInput["document"] | KnowledgeEditInput["document"]>,
) {
  const payload: Record<string, unknown> = { ...patch };
  delete payload.effectiveInjectMode;
  delete payload.effectiveAiMaintained;
  delete payload.effectiveMaintenanceRules;
  if (patch.injectMode !== undefined) {
    if (patch.injectMode === "inherit") {
      payload.inheritInjectMode = true;
      delete payload.injectMode;
    } else {
      payload.inheritInjectMode = false;
      payload.injectMode = patch.injectMode;
    }
  }
  if (Object.prototype.hasOwnProperty.call(patch, "summary")) {
    payload.summaryEnabled = !!patch.summary?.trim();
  }
  if (patch.aiEditMode !== undefined) {
    payload.aiEditMode = patch.aiEditMode;
    delete payload.aiMaintained;
    delete payload.inheritAiConfig;
  } else if (patch.aiMaintained !== undefined) {
    if (patch.aiMaintained === "inherit") {
      payload.inheritAiConfig = true;
      delete payload.aiMaintained;
    } else {
      payload.inheritAiConfig = false;
      payload.aiMaintained = patch.aiMaintained;
    }
  }
  if (Object.prototype.hasOwnProperty.call(patch, "maintenanceRules")) {
    payload.explicitMaintenanceRules = !!patch.maintenanceRules?.trim();
  }
  return payload;
}

function directoryPatchForIpc(
  patch: NonNullable<KnowledgeEditInput["config"]>,
) {
  const payload: Record<string, unknown> = { ...patch };
  delete payload.effectiveInjectMode;
  delete payload.effectiveAiMaintained;
  delete payload.effectiveMaintenanceRules;
  if (patch.injectMode !== undefined) {
    if (patch.injectMode === "inherit") {
      payload.inheritInjectMode = true;
      delete payload.injectMode;
    } else {
      payload.inheritInjectMode = false;
      payload.injectMode = patch.injectMode;
    }
  }
  if (patch.aiMaintained !== undefined) {
    if (patch.aiMaintained === "inherit") {
      payload.inheritAiConfig = true;
      delete payload.aiMaintained;
      delete payload.explicitMaintenanceRules;
      delete payload.maintenanceRules;
    } else {
      payload.inheritAiConfig = false;
      payload.aiMaintained = patch.aiMaintained;
    }
  }
  if (
    patch.aiMaintained !== "inherit" &&
    Object.prototype.hasOwnProperty.call(patch, "maintenanceRules")
  ) {
    payload.explicitMaintenanceRules = !!patch.maintenanceRules?.trim();
    payload.maintenanceRules = patch.maintenanceRules ?? "";
  }
  return payload;
}

export async function knowledgeListPage(
  input: KnowledgeDocumentListInput = {},
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeDocumentListPage> {
  const payload = await ipcInvoke<{
    items: KnowledgeDocumentSummaryPayload[];
    nextCursor?: string | null;
  }>("knowledge_list_page", {
    workspaceRef,
    docType: input.type,
    pathPrefix: input.pathPrefix,
    cursor: input.cursor,
    limit: input.limit,
  });
  return {
    items: payload.items.map(normalizeDocumentSummary),
    nextCursor: payload.nextCursor ?? null,
  };
}

export function knowledgeListDirectories(
  type: KnowledgeDocument["type"],
  workspaceRef: WorkspaceRef,
): Promise<string[]> {
  return ipcInvoke<string[]>("knowledge_list_directories", { workspaceRef, docType: type });
}

export async function knowledgeListDirectoryDocuments(
  type: KnowledgeDocument["type"],
  path: string,
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeDocumentSummary[]> {
  const payload = await ipcInvoke<KnowledgeDocumentSummaryPayload[]>(
    "knowledge_list_directory_documents",
    { workspaceRef, docType: type, path },
  );
  return payload.map(normalizeDocumentSummary);
}

export async function knowledgeListDirectoryDocumentsPage(
  type: KnowledgeDocument["type"],
  path: string,
  workspaceRef: WorkspaceRef,
  options: { cursor?: string | null; limit?: number } = {},
): Promise<KnowledgeDocumentListPage> {
  const payload = await ipcInvoke<{
    items: KnowledgeDocumentSummaryPayload[];
    nextCursor?: string | null;
  }>(
    "knowledge_list_directory_documents_page",
    {
      docType: type,
      path,
      workspaceRef,
      cursor: options.cursor,
      limit: options.limit,
    },
  );
  return {
    items: payload.items.map(normalizeDocumentSummary),
    nextCursor: payload.nextCursor ?? null,
  };
}

export async function knowledgeListExternalReferenceDirectories(
  workspaceRef: WorkspaceRef,
): Promise<
  KnowledgeExternalDirectoryBinding[]
> {
  const payload = await ipcInvoke<KnowledgeExternalDirectoryBindingPayload[]>(
    "knowledge_list_external_reference_directories",
    { workspaceRef },
  );
  return payload.map(normalizeExternalDirectoryBinding);
}

export function knowledgeListUnityManagedDirectoryStats(
  workspaceRef: WorkspaceRef,
): Promise<
  KnowledgeManagedDirectoryStat[]
> {
  return ipcInvoke<KnowledgeManagedDirectoryStat[]>(
    "knowledge_list_unity_managed_directory_stats",
    { workspaceRef },
  );
}

export async function knowledgeQuery(
  input: KnowledgeDocumentQueryInput,
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeSearchResult[]> {
  const results = await ipcInvoke<KnowledgeQueryPayload[]>("knowledge_query", {
    workspaceRef,
    query: input.query,
    limit: input.limit,
    types: input.types,
    pathPrefix: input.pathPrefix,
    includeHidden: input.includeHidden ?? false,
  });

  return results.map((result) => ({
    id: result.id,
    type: result.type,
    path: result.path,
    title: result.title,
    storageSource: result.storageSource ?? "project",
    effectiveInjectMode: result.injectMode,
    effectiveAiMaintained: result.aiMaintained,
    score: result.score,
    snippet: result.snippet,
    matchKind: result.matchKind ?? "lexical",
    matchedSection: result.matchedSection ?? null,
    matchedTerms: result.matchedTerms ?? [],
    semanticScore: result.semanticScore ?? null,
    semanticConfidence: result.semanticConfidence ?? null,
    estimatedTokens: result.estimatedTokens ?? undefined,
    modifiedAt: result.updatedAt,
    physicalPath: result.physicalPath || undefined,
    displayPath: result.displayPath || undefined,
    startLine: result.startLine || undefined,
    endLine: result.endLine || undefined,
    summaryStartLine: result.summaryStartLine ?? undefined,
    bodyStartLine: result.bodyStartLine || undefined,
  }));
}

export function knowledgeGetGeneralConfig(workspaceRef: WorkspaceRef): Promise<KnowledgeGeneralConfig> {
  return ipcInvoke<KnowledgeGeneralConfig>("knowledge_get_general_config", { workspaceRef });
}

export function knowledgeSaveGeneralConfig(
  config: KnowledgeGeneralConfig,
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeGeneralConfig> {
  return ipcInvoke<KnowledgeGeneralConfig>("knowledge_save_general_config", {
    config,
    workspaceRef,
  });
}

export function knowledgeGetEmbeddingConfig(workspaceRef: WorkspaceRef): Promise<EmbeddingConfig> {
  return ipcInvoke<EmbeddingConfig>("knowledge_get_embedding_config", { workspaceRef });
}

export function knowledgeSaveEmbeddingConfig(
  config: EmbeddingConfig,
  workspaceRef: WorkspaceRef,
): Promise<EmbeddingConfig> {
  return ipcInvoke<EmbeddingConfig>("knowledge_save_embedding_config", {
    config,
    workspaceRef,
  });
}

export function knowledgeActivateEmbedding(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("knowledge_activate_embedding", { workspaceRef });
}

export function knowledgeDeactivateEmbedding(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("knowledge_deactivate_embedding", { workspaceRef });
}

export function knowledgeGetEmbeddingStatus(workspaceRef: WorkspaceRef): Promise<EmbeddingStatus> {
  return ipcInvoke<EmbeddingStatus>("knowledge_get_embedding_status", { workspaceRef });
}

export function knowledgeTestEmbeddingRuntime(workspaceRef: WorkspaceRef): Promise<EmbeddingRuntimeTestResult> {
  return ipcInvoke<EmbeddingRuntimeTestResult>(
    "knowledge_test_embedding_runtime",
    { workspaceRef },
  );
}

export function knowledgeGetLocalEmbeddingModelCatalog(): Promise<EmbeddingLocalModelCatalog> {
  return ipcInvoke<EmbeddingLocalModelCatalog>(
    "knowledge_get_local_embedding_model_catalog",
  );
}

export function knowledgeDownloadLocalEmbeddingModel(
  modelId: string,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke<void>("knowledge_download_local_embedding_model", {
    modelId,
    workspaceRef,
  });
}

export function knowledgeCancelLocalEmbeddingModelDownload(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("knowledge_cancel_local_embedding_model_download", { workspaceRef });
}

export function knowledgeInspectLocalEmbeddingModelDirectory(
  path: string,
): Promise<EmbeddingLocalModelDirectoryInspection> {
  return ipcInvoke<EmbeddingLocalModelDirectoryInspection>(
    "knowledge_inspect_local_embedding_model_directory",
    { path },
  );
}

export function knowledgeRebuildLexicalIndex(workspaceRef: WorkspaceRef): Promise<number> {
  return ipcInvoke<number>("knowledge_rebuild_lexical_index", { workspaceRef });
}

export function knowledgeGetLexicalRebuildStatus(workspaceRef: WorkspaceRef): Promise<LexicalRebuildStatus> {
  return ipcInvoke<LexicalRebuildStatus>(
    "knowledge_get_lexical_rebuild_status",
    { workspaceRef },
  );
}

export function knowledgeGetOverview(workspaceRef: WorkspaceRef): Promise<KnowledgeRetrievalOverview> {
  return ipcInvoke<KnowledgeRetrievalOverview>("knowledge_get_overview", { workspaceRef });
}

export function knowledgeGetUnityReferenceImportStatus(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_get_unity_reference_import_status",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export async function knowledgeFindUnityReferenceDirectory(workspaceRef: WorkspaceRef): Promise<KnowledgeDirectoryConfigRecord | null> {
  const payload = await ipcInvoke<KnowledgeDirectoryConfigPayload | null>(
    "knowledge_find_unity_reference_directory",
    { workspaceRef },
  );
  return payload ? normalizeDirectoryConfig(payload) : null;
}

export function knowledgeGetFeishuReferenceImportStatus(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_get_feishu_reference_import_status",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeCancelUnityReferenceImport(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_cancel_unity_reference_import",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeCancelFeishuReferenceImport(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_cancel_feishu_reference_import",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeImportUnityReferenceDocs(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
  locale?: UnityReferenceImportLocale,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_import_unity_reference_docs",
    {
      targetPath: targetPath ?? null,
      locale,
      workspaceRef,
    },
  );
}

export function knowledgeSaveFeishuReferenceConfig(
  config: FeishuReferenceConfigInput,
  workspaceRef: WorkspaceRef,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_save_feishu_reference_config",
    {
      config,
      workspaceRef,
    },
  );
}

export function knowledgeTestFeishuReferenceConnection(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<FeishuSourceTestResult> {
  return ipcInvoke<FeishuSourceTestResult>(
    "knowledge_test_feishu_reference_connection",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeStartFeishuReferenceOauth(workspaceRef: WorkspaceRef): Promise<FeishuReferenceOauthStartResult> {
  return ipcInvoke<FeishuReferenceOauthStartResult>(
    "knowledge_start_feishu_reference_oauth",
    { workspaceRef },
  );
}

export function knowledgeCancelFeishuReferenceOauthWait(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_cancel_feishu_reference_oauth_wait",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeListFeishuReferenceSpaceNodes(
  workspaceRef: WorkspaceRef,
  spaceId: string,
  parentNodeToken?: string | null,
): Promise<FeishuReferenceNodeSummary[]> {
  return ipcInvoke<FeishuReferenceNodeSummary[]>(
    "knowledge_list_feishu_reference_space_nodes",
    {
      spaceId,
      parentNodeToken: parentNodeToken ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeImportFeishuReferenceDocs(
  request: FeishuReferenceImportRequest,
  workspaceRef: WorkspaceRef,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_import_feishu_reference_docs",
    {
      request,
      workspaceRef,
    },
  );
}

export function knowledgeDeleteUnityReferenceDocs(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<UnityReferenceImportStatus> {
  return ipcInvoke<UnityReferenceImportStatus>(
    "knowledge_delete_unity_reference_docs",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeDeleteFeishuReferenceDocs(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<FeishuReferenceImportStatus> {
  return ipcInvoke<FeishuReferenceImportStatus>(
    "knowledge_delete_feishu_reference_docs",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgePreviewLocalReferenceImport(
  sourcePath: string,
  workspaceRef: WorkspaceRef,
): Promise<LocalReferenceScanPreview> {
  return ipcInvoke<LocalReferenceScanPreview>(
    "knowledge_preview_local_reference_import",
    {
      sourcePath,
      workspaceRef,
    },
  );
}

export function knowledgeImportLocalReferenceDocs(
  request: LocalReferenceImportRequest,
  workspaceRef: WorkspaceRef,
): Promise<LocalReferenceImportStatus> {
  return ipcInvoke<LocalReferenceImportStatus>(
    "knowledge_import_local_reference_docs",
    {
      request,
      workspaceRef,
    },
  );
}

export function knowledgeGetLocalReferenceImportStatus(
  workspaceRef: WorkspaceRef,
  targetPath?: string | null,
): Promise<LocalReferenceImportStatus> {
  return ipcInvoke<LocalReferenceImportStatus>(
    "knowledge_get_local_reference_import_status",
    {
      targetPath: targetPath ?? null,
      workspaceRef,
    },
  );
}

export function knowledgeCancelLocalReferenceImport(workspaceRef: WorkspaceRef): Promise<LocalReferenceImportStatus> {
  return ipcInvoke<LocalReferenceImportStatus>(
    "knowledge_cancel_local_reference_import",
    { workspaceRef },
  );
}

export function knowledgeSyncLocalReferenceDocs(
  targetPath: string,
  workspaceRef: WorkspaceRef,
): Promise<LocalReferenceImportStatus> {
  return ipcInvoke<LocalReferenceImportStatus>(
    "knowledge_sync_local_reference_docs",
    {
      targetPath,
      workspaceRef,
    },
  );
}

export function knowledgeDeleteLocalReferenceDocs(
  targetPath: string,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke<void>("knowledge_delete_local_reference_docs", {
    targetPath,
    workspaceRef,
  });
}

export function knowledgeRevealTarget(input: {
  kind: "document" | "directory";
  docType: KnowledgeDocument["type"];
  path: string;
}, workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("knowledge_reveal_target", {
    request: input,
    workspaceRef,
  });
}

export async function knowledgeRead(
  input: KnowledgeReadInput,
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeReadResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const payload = await ipcInvoke<KnowledgeReadResultPayload>(
    "knowledge_read",
    {
      workspaceRef,
      request: {
        kind: input.kind,
        path,
        type: input.type,
        part: input.part ?? "full",
        includeHistory: input.includeHistory ?? false,
      },
    },
  );
  return normalizeReadResult(payload);
}

export async function knowledgeCreate(
  input: KnowledgeCreateInput,
  workspaceRef: WorkspaceRef,
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
        document: input.document ? documentPatchForIpc(input.document) : undefined,
      },
      workspaceRef,
    },
  );
  return normalizeMutationResult(payload);
}

export async function knowledgeEdit(
  input: KnowledgeEditInput,
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeMutationResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const payload = await ipcInvoke<KnowledgeMutationPayload>("knowledge_edit", {
    workspaceRef,
    request: {
      kind: input.kind,
      path,
      type: input.type,
      document: input.document ? documentPatchForIpc(input.document) : undefined,
      config: input.config ? directoryPatchForIpc(input.config) : undefined,
    },
  });
  return normalizeMutationResult(payload);
}

export async function knowledgeMove(
  input: KnowledgeMoveInput,
  workspaceRef: WorkspaceRef,
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
    workspaceRef,
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
  workspaceRef: WorkspaceRef,
): Promise<KnowledgeMutationResult> {
  const path =
    input.kind === "directory"
      ? resolveKnowledgeDirectoryPath(input.path, input.type)
      : resolveKnowledgeDocumentPath(input.path, input.type);
  const payload = await ipcInvoke<KnowledgeMutationPayload>(
    "knowledge_delete",
    {
      workspaceRef,
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
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke<void>("knowledge_delete_external_reference_directory", {
    path: resolveKnowledgeDirectoryPath(path, "reference"),
    workspaceRef,
  });
}

export function getSkillConfig(
  relPath: string,
  source: string | undefined,
  workspaceRef: WorkspaceRef,
): Promise<SkillConfig> {
  return ipcInvoke<SkillConfig>("get_skill_config", {
    relPath,
    source,
    workspaceRef,
  });
}

export function setSkillConfig(
  relPath: string,
  source: string | undefined,
  config: SkillConfig,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke("set_skill_config", {
    relPath,
    source,
    enabled: config.enabled,
    surface: config.surface,
    description: config.description,
    commandTrigger: config.commandTrigger,
    injectMode: config.injectMode,
    readOnly: config.readOnly,
    aiEditMode: config.aiEditMode,
    maintenanceRules: config.maintenanceRules,
    workspaceRef,
  });
}

export function getAllSkillConfigs(
  workspaceRef: WorkspaceRef,
): Promise<Record<string, SkillConfig>> {
  return ipcInvoke<Record<string, SkillConfig>>("get_all_skill_configs", {
    workspaceRef,
  });
}

export function listSkills(
  workspaceRef: WorkspaceRef,
): Promise<SkillManifest[]> {
  return ipcInvoke<SkillManifest[]>("list_skills", {
    workspaceRef,
  });
}

/** Rescan the external agent skill directories (~/.claude/skills, ...). */
export function refreshExternalSkills(
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke<void>("refresh_external_skills", {
    workspaceRef,
  });
}

export function readSkillManifest(
  dirName: string,
  source: string | undefined,
  workspaceRef: WorkspaceRef,
): Promise<string> {
  return ipcInvoke<string>("read_skill_manifest", {
    dirName,
    source,
    workspaceRef,
  });
}

export function getDefaultSkillPackageNamespace(): Promise<string> {
  return ipcInvoke<string>("get_default_skill_package_namespace");
}

export function setDefaultSkillPackageNamespace(value: string): Promise<string> {
  return ipcInvoke<string>("set_default_skill_package_namespace", { value });
}

export function createSkillScaffold(
  input: SkillCreateInput,
  workspaceRef: WorkspaceRef,
): Promise<SkillManifest> {
  return ipcInvoke<SkillManifest>("create_skill_scaffold", {
    kind: input.kind ?? "md",
    source: input.source,
    name: input.name,
    path: input.path,
    packageId: input.packageId,
    version: input.version,
    summary: input.summary,
    body: input.body,
    argumentHint: input.argumentHint,
    commandTrigger: input.commandTrigger,
    commandEnabled: input.commandEnabled,
    modelInvocationEnabled: input.modelInvocationEnabled,
    tools: input.tools,
    workspaceRef,
  });
}

export function deleteSkillPackage(
  packageId: string,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke<void>("delete_skill_package", {
    packageId,
    workspaceRef,
  });
}

export function importSkillPackage(
  sourcePath: string,
  workspaceRef: WorkspaceRef,
): Promise<SkillManifest> {
  return ipcInvoke<SkillManifest>("import_skill_package", {
    sourcePath,
    workspaceRef,
  });
}

export function exportSkillPackage(
  packageId: string,
  filePath: string,
  workspaceRef: WorkspaceRef,
): Promise<SkillPackageArchiveResult> {
  return ipcInvoke<SkillPackageArchiveResult>("export_skill_package", {
    packageId,
    filePath,
    workspaceRef,
  });
}

export function getSkillUnityInstallStatus(
  packageId: string,
  workspaceRef: WorkspaceRef,
): Promise<SkillUnityInstallStatus> {
  return ipcInvoke<SkillUnityInstallStatus>("get_skill_unity_install_status", {
    packageId,
    workspaceRef,
  });
}

export function installSkillUnityFiles(
  packageId: string,
  workspaceRef: WorkspaceRef,
): Promise<SkillUnityInstallStatus> {
  return ipcInvoke<SkillUnityInstallStatus>("install_skill_unity_files", {
    packageId,
    workspaceRef,
  });
}

export function removeSkillUnityFiles(
  packageId: string,
  workspaceRef: WorkspaceRef,
): Promise<SkillUnityInstallStatus> {
  return ipcInvoke<SkillUnityInstallStatus>("remove_skill_unity_files", {
    packageId,
    workspaceRef,
  });
}
