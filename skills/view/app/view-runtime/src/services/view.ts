import { t } from "../i18n";
import type {
  AppErrorPayload,
  AssetRefAttachment,
  ChatMessage,
  ImageAttachment,
  KnowledgeAccessMode,
  PendingSessionInput,
  SessionDetail,
  SessionEventRecord,
  SessionRunSummary,
  UserIntentMeta,
} from "../types";
import { normalizeAppError } from "./errors";
import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";
import { checkUnityConnectionStatus } from "./unity";

export interface ViewScriptManifest {
  name: string;
  path: string;
  entryType: string;
}

export interface ViewCapabilities {
  unity: boolean;
}

export interface ViewRequirements {
  unityConnection: boolean;
}

export interface ViewManifest {
  schema: string;
  apiVersion: string;
  id: string;
  name: string;
  version: string;
  template: string;
  displayPath?: string | null;
  icon?: string | null;
  entry: string;
  style: string;
  scripts: ViewScriptManifest[];
  capabilities: ViewCapabilities;
  requirements: ViewRequirements;
}

export interface ViewTemplateSummary {
  id: string;
  name: string;
  description: string;
}

export interface ViewPackageSummary {
  id: string;
  name: string;
  apiVersion: string;
  version: string;
  template: string;
  icon?: string | null;
  displayPath: string;
  packageRelPath?: string;
  packageRoot: string;
  manifestPath: string;
  updatedAt: number;
  capabilities: ViewCapabilities;
  requirements: ViewRequirements;
  temporary?: boolean;
  source?: string;
  pluginId?: string | null;
  pluginScope?: "app" | "project" | string | null;
}

export interface ViewFolderSummary {
  relPath: string;
  name: string;
  packageRoot: string;
  updatedAt: number;
}

export interface ViewTreeSnapshot {
  views: ViewPackageSummary[];
  folders: ViewFolderSummary[];
  order?: string[];
}

export interface ViewPackageFile {
  relPath: string;
  kind: string;
  content: string;
  size: number;
  truncated: boolean;
}

export interface ViewPackageDetail {
  summary: ViewPackageSummary;
  manifest: ViewManifest;
  files: ViewPackageFile[];
}

export interface ViewCreateRequest {
  id: string;
  packageName?: string | null;
  name?: string | null;
  template?: string | null;
  icon?: string | null;
  displayPath?: string | null;
  temporary?: boolean;
}

export interface ViewCreateFolderRequest {
  parentRelPath?: string | null;
  name: string;
}

export interface ViewDeleteEntryRequest {
  relPath: string;
}

export interface ViewRenameEntryRequest {
  relPath: string;
  name: string;
}

export interface ViewMoveEntryRequest {
  sourceRelPath: string;
  targetDirRelPath?: string | null;
  insertBeforeRelPath?: string | null;
  insertAfterRelPath?: string | null;
}

export interface ViewExportPackageRequest {
  viewId: string;
  filePath: string;
}

export interface ViewImportPackageRequest {
  filePath: string;
  targetDirRelPath?: string | null;
}

export interface ViewPackageImportResult {
  summary: ViewPackageSummary;
  snapshot: ViewTreeSnapshot;
}

export interface ViewRunResult {
  id: string;
  windowLabel: string;
  hostUrl: string;
  packageRoot: string;
}

export interface ViewSetTabHostRequest {
  hostLabel: string;
  viewIds: string[];
  keepExistingForHost?: boolean;
}

export interface ViewDetachTabRequest {
  viewId: string;
  sourceHostLabel?: string | null;
  x?: number | null;
  y?: number | null;
}

export interface ViewContentMountRequest {
  viewId: string;
  hostLabel: string;
  x: number;
  y: number;
  width: number;
  height: number;
  visible?: boolean;
}

export const VIEW_UNITY_CONNECTION_REQUIRED_ERROR_CODE = "view.unity_connection_required";

export interface ViewCompileScriptRequest {
  viewId: string;
  scriptName: string;
}

export interface ViewCompileScriptResult {
  name: string;
  hash: string;
  cacheHit: boolean;
  assemblyId: string;
  domainFingerprint: string;
  path: string;
}

export interface ViewCallScriptRequest {
  viewId: string;
  scriptName: string;
  method: string;
  args?: unknown;
}

export interface ViewCallScriptResult {
  compile: ViewCompileScriptResult;
  method: string;
  result: unknown;
}

export type ViewFrontendLogLevel = "debug" | "log" | "info" | "warn" | "error";

export interface ViewFrontendLogRequest {
  viewId: string;
  level: ViewFrontendLogLevel;
  message: string;
}

export interface ViewFrontendLogReadRequest {
  viewId: string;
  limit?: number;
}

export interface ViewFrontendLogEntry {
  time: number;
  level: ViewFrontendLogLevel;
  message: string;
}

export interface ViewStorageGetRequest {
  viewId: string;
  key: string;
}

export interface ViewStorageSetRequest {
  viewId: string;
  key: string;
  value: unknown;
}

export interface ViewStorageRemoveRequest {
  viewId: string;
  key: string;
}

export type ViewFsFileData = string | number[];

export interface ViewFsPathRequest {
  path: string;
}

export interface ViewFsReadFileRequest {
  path: string;
  encoding?: string | null;
}

export interface ViewFsReadFileResult {
  path: string;
  encoding?: string | null;
  data: ViewFsFileData;
}

export interface ViewFsWriteFileRequest {
  path: string;
  data: ViewFsFileData;
  encoding?: string | null;
}

export interface ViewFsMkdirRequest {
  path: string;
  recursive?: boolean | null;
}

export interface ViewFsReaddirRequest {
  path: string;
  withFileTypes?: boolean | null;
}

export interface ViewFsDirEntry {
  name: string;
  path: string;
  isFile: boolean;
  isDirectory: boolean;
  isSymbolicLink: boolean;
}

export interface ViewFsReaddirResult {
  entries: ViewFsDirEntry[];
}

export interface ViewFsStatResult {
  path: string;
  size: number;
  isFile: boolean;
  isDirectory: boolean;
  isSymbolicLink: boolean;
  modifiedMs?: number | null;
  accessedMs?: number | null;
  createdMs?: number | null;
}

export interface ViewFsRmRequest {
  path: string;
  recursive?: boolean | null;
  force?: boolean | null;
}

export interface ViewFsRenameRequest {
  oldPath: string;
  newPath: string;
}

export interface ViewFsCopyFileRequest {
  src: string;
  dest: string;
}

export interface ViewAutomationRequest {
  requestId: string;
  viewId: string;
  kind: string;
  payload: Record<string, unknown>;
}

export interface ViewRuntimeSelectionSnapshot {
  kind: string;
  name: string;
  type: string;
  path: string;
  instanceId: number;
}

export interface ViewRuntimeUpdateEvent {
  sequence: number;
  timeSinceStartup: number;
  isPlaying: boolean;
  isPaused: boolean;
  activeScenePath: string;
  selection: ViewRuntimeSelectionSnapshot;
}

export interface ViewSessionCreateRequest {
  title?: string | null;
  parentSessionId?: string | null;
  sessionType?: string | null;
  agentId?: string | null;
}

export type ViewSessionWaitStatus =
  | "running"
  | "waiting_input"
  | "done"
  | "cancelled"
  | "error"
  | "timeout"
  | "unknown";

export interface ViewSessionWaitRequest {
  sessionId: string;
  runId?: string | null;
  afterSeq?: number | null;
  timeoutMs?: number | null;
  pollIntervalMs?: number | null;
  includeEvents?: boolean | null;
  returnOnWaitingInput?: boolean | null;
}

export interface ViewSessionWaitResult {
  sessionId: string;
  runId?: string | null;
  status: ViewSessionWaitStatus;
  detail: SessionDetail;
  activeRun?: SessionRunSummary | null;
  events: SessionEventRecord[];
  message?: ChatMessage | null;
  finalText: string;
  error?: AppErrorPayload | null;
}

export interface ViewSessionChatRequest {
  sessionId?: string | null;
  text: string;
  title?: string | null;
  sessionTitle?: string | null;
  sessionType?: string | null;
  agentId?: string | null;
  model?: string | null;
  effort?: string | null;
  images?: ImageAttachment[] | null;
  assetRefs?: AssetRefAttachment[] | null;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  subagentModels?: Record<string, string> | null;
  subagentEfforts?: Record<string, string> | null;
  subagentFastModes?: Record<string, boolean> | null;
  knowledgeMode?: KnowledgeAccessMode | null;
  show?: boolean | null;
  wait?: boolean | ViewSessionWaitRequest | null;
}

export interface ViewSessionChatResult {
  sessionId: string;
  runId: string;
  result?: ViewSessionWaitResult | null;
}

export interface ViewSessionQueueInputRequest {
  sessionId: string;
  runId: string;
  mergeGroupId: string;
  text: string;
  displayText?: string | null;
  images?: ImageAttachment[] | null;
  assetRefs?: AssetRefAttachment[] | null;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  clientMessageId?: string | null;
  delivery?: "after_run" | "immediate" | string | null;
}

export type ViewSessionQueueInputResult = PendingSessionInput;

export interface ViewLlmCallRequest {
  prompt: string;
  sessionId?: string | null;
  title?: string | null;
  sessionTitle?: string | null;
  sessionType?: string | null;
  agentId?: string | null;
  model?: string | null;
  effort?: string | null;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  subagentModels?: Record<string, string> | null;
  subagentEfforts?: Record<string, string> | null;
  subagentFastModes?: Record<string, boolean> | null;
  knowledgeMode?: KnowledgeAccessMode | null;
  show?: boolean | null;
  wait?: boolean | ViewSessionWaitRequest | null;
  timeoutMs?: number | null;
}

export interface ViewLlmCallResult {
  sessionId: string;
  runId: string;
  status: ViewSessionWaitStatus;
  text: string;
  message?: ChatMessage | null;
  detail?: SessionDetail | null;
  events: SessionEventRecord[];
  error?: AppErrorPayload | null;
}

export const VIEW_HOST_PATH = "/view-host";
export const VIEW_CONTENT_PATH = "/view-content";
export type ViewWorkspaceRef = WorkspaceRef & { expectedGeneration: number };

export function isViewContentWindowLocation(): boolean {
  return window.location.pathname === VIEW_CONTENT_PATH
    || new URLSearchParams(window.location.search).get("viewContent") === "1";
}

export function viewHostIdFromLocation(): string {
  return new URLSearchParams(window.location.search).get("id") || "";
}

export function viewWorkspaceRefFromLocation(
  search = window.location.search,
): ViewWorkspaceRef | null {
  const params = new URLSearchParams(search);
  const checkoutId = params.get("checkoutId")?.trim() ?? "";
  const generationText = params.get("workspaceGeneration") ?? "";
  if (checkoutId && /^\d+$/.test(generationText)) {
    const expectedGeneration = Number(generationText);
    if (Number.isSafeInteger(expectedGeneration) && expectedGeneration >= 0) {
      return { checkoutId, expectedGeneration };
    }
  }
  return null;
}

export function isExactViewWorkspaceBinding(
  expected: ViewWorkspaceRef,
  actual: { checkoutId: string; workspaceGeneration: number } | null | undefined,
): boolean {
  return actual?.checkoutId === expected.checkoutId
    && actual.workspaceGeneration === expected.expectedGeneration;
}

export function isViewHostPoolWindowLocation(): boolean {
  const params = new URLSearchParams(window.location.search);
  return params.get("viewHost") === "1" && params.get("pool") === "1";
}

export function viewTemplates(): Promise<ViewTemplateSummary[]> {
  return ipcInvoke<ViewTemplateSummary[]>("view_templates");
}

export function viewList(workspaceRef: WorkspaceRef): Promise<ViewPackageSummary[]> {
  return ipcInvoke<ViewPackageSummary[]>("view_list", { workspaceRef });
}

export function viewTree(workspaceRef: WorkspaceRef): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_tree", { workspaceRef });
}

export function viewCreate(workspaceRef: WorkspaceRef, request: ViewCreateRequest): Promise<ViewPackageDetail> {
  return ipcInvoke<ViewPackageDetail>("view_create", { workspaceRef, request });
}

export function viewCreateFolder(workspaceRef: WorkspaceRef, request: ViewCreateFolderRequest): Promise<ViewFolderSummary> {
  return ipcInvoke<ViewFolderSummary>("view_create_folder", { workspaceRef, request });
}

export function viewDeleteEntry(workspaceRef: WorkspaceRef, request: ViewDeleteEntryRequest): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_delete_entry", { workspaceRef, request });
}

export function viewRenameEntry(workspaceRef: WorkspaceRef, request: ViewRenameEntryRequest): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_rename_entry", { workspaceRef, request });
}

export function viewMoveEntry(workspaceRef: WorkspaceRef, request: ViewMoveEntryRequest): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_move_entry", { workspaceRef, request });
}

export function viewExportPackage(workspaceRef: WorkspaceRef, request: ViewExportPackageRequest): Promise<string> {
  return ipcInvoke<string>("view_export_package", { workspaceRef, request });
}

export function viewImportPackage(
  workspaceRef: WorkspaceRef,
  request: ViewImportPackageRequest,
): Promise<ViewPackageImportResult> {
  return ipcInvoke<ViewPackageImportResult>("view_import_package", { workspaceRef, request });
}

export function viewRead(workspaceRef: WorkspaceRef, viewId: string): Promise<ViewPackageDetail> {
  return ipcInvoke<ViewPackageDetail>("view_read", { workspaceRef, viewId });
}

export function viewReload(workspaceRef: WorkspaceRef, viewId: string): Promise<ViewPackageSummary> {
  return ipcInvoke<ViewPackageSummary>("view_reload", { workspaceRef, viewId });
}

export function viewRun(workspaceRef: WorkspaceRef, viewId: string): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_run", { workspaceRef, viewId });
}

export function viewRunInUnity(workspaceRef: WorkspaceRef, viewId: string): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_run_in_unity", { workspaceRef, viewId });
}

export function viewSetTabHost(workspaceRef: WorkspaceRef, request: ViewSetTabHostRequest): Promise<void> {
  return ipcInvoke<void>("view_set_tab_host", { workspaceRef, request });
}

export function viewDetachTab(workspaceRef: WorkspaceRef, request: ViewDetachTabRequest): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_detach_tab", { workspaceRef, request });
}

export function viewHostPoolPrepare(workspaceRef: WorkspaceRef): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_host_pool_prepare", { workspaceRef });
}

export function viewHostPoolReady(workspaceRef: WorkspaceRef, hostLabel: string): Promise<void> {
  return ipcInvoke<void>("view_host_pool_ready", { workspaceRef, hostLabel });
}

export function viewHostRevealed(workspaceRef: WorkspaceRef, hostLabel: string): Promise<void> {
  return ipcInvoke<void>("view_host_revealed", { workspaceRef, hostLabel });
}

export function viewContentMount(workspaceRef: WorkspaceRef, request: ViewContentMountRequest): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_content_mount", { workspaceRef, request });
}

export function viewContentHide(workspaceRef: WorkspaceRef, viewId: string): Promise<void> {
  return ipcInvoke<void>("view_content_hide", { workspaceRef, viewId });
}

export function viewContentDestroy(workspaceRef: WorkspaceRef, viewId: string): Promise<void> {
  return ipcInvoke<void>("view_content_destroy", { workspaceRef, viewId });
}

export function viewRequiresUnityConnection(
  view: { requirements?: ViewRequirements | null; capabilities?: ViewCapabilities | null },
): boolean {
  return view.requirements?.unityConnection
    ?? !!view.capabilities?.unity;
}

export function viewUnityConnectionRequiredMessage(viewName?: string | null): string {
  const name = viewName?.trim();
  return name
    ? t("view.error.unityConnectionRequiredNamed", name)
    : t("view.host.unityConnectionRequired");
}

export function viewUnityConnectionRequiredError(viewName?: string | null): AppErrorPayload {
  return {
    code: VIEW_UNITY_CONNECTION_REQUIRED_ERROR_CODE,
    message: viewUnityConnectionRequiredMessage(viewName),
    retryable: false,
    severity: "error",
  };
}

export async function checkViewOpenRequirements(
  workspaceRef: WorkspaceRef,
  view: {
    name?: string | null;
    requirements?: ViewRequirements | null;
    capabilities?: ViewCapabilities | null;
  },
): Promise<AppErrorPayload | null> {
  if (!viewRequiresUnityConnection(view)) return null;

  const status = await checkUnityConnectionStatus(workspaceRef);
  return status.ready ? null : viewUnityConnectionRequiredError(view.name);
}

function parseLegacyUnityConnectionRequiredMessage(message: string): string | null {
  const prefix = "View '";
  const suffix = "' requires a Unity Editor connection.";
  if (!message.startsWith(prefix) || !message.endsWith(suffix)) return null;
  return message.slice(prefix.length, message.length - suffix.length).trim() || null;
}

export function normalizeViewError(
  error: unknown,
  options: { viewName?: string | null } = {},
): AppErrorPayload {
  const normalized = normalizeAppError(error);
  const legacyViewName = parseLegacyUnityConnectionRequiredMessage(normalized.message);
  if (
    normalized.code === VIEW_UNITY_CONNECTION_REQUIRED_ERROR_CODE
    || legacyViewName
  ) {
    const viewName = options.viewName?.trim() ? options.viewName : legacyViewName;
    return {
      ...normalized,
      code: VIEW_UNITY_CONNECTION_REQUIRED_ERROR_CODE,
      message: viewUnityConnectionRequiredMessage(viewName),
    };
  }
  return normalized;
}

export function viewCompileScript(
  workspaceRef: WorkspaceRef,
  request: ViewCompileScriptRequest,
): Promise<ViewCompileScriptResult> {
  return ipcInvoke<ViewCompileScriptResult>("view_compile_script", { workspaceRef, request });
}

export function viewCallScript(workspaceRef: WorkspaceRef, request: ViewCallScriptRequest): Promise<ViewCallScriptResult> {
  return ipcInvoke<ViewCallScriptResult>("view_call_script", { workspaceRef, request });
}

export function viewAppendFrontendLog(workspaceRef: WorkspaceRef, request: ViewFrontendLogRequest): Promise<void> {
  return ipcInvoke<void>("view_append_frontend_log", { workspaceRef, request });
}

export function viewReadFrontendLog(workspaceRef: WorkspaceRef, request: ViewFrontendLogReadRequest): Promise<ViewFrontendLogEntry[]> {
  return ipcInvoke<ViewFrontendLogEntry[]>("view_read_frontend_log", { workspaceRef, request });
}

export function viewOpenFrontendLog(workspaceRef: WorkspaceRef, viewId: string): Promise<void> {
  return ipcInvoke<void>("view_open_frontend_log", { workspaceRef, viewId });
}

export function viewStorageGet(workspaceRef: WorkspaceRef, request: ViewStorageGetRequest): Promise<unknown | null> {
  return ipcInvoke<unknown | null>("view_storage_get", { workspaceRef, request });
}

export function viewStorageSet(workspaceRef: WorkspaceRef, request: ViewStorageSetRequest): Promise<void> {
  return ipcInvoke<void>("view_storage_set", { workspaceRef, request });
}

export function viewStorageRemove(workspaceRef: WorkspaceRef, request: ViewStorageRemoveRequest): Promise<void> {
  return ipcInvoke<void>("view_storage_remove", { workspaceRef, request });
}

export function viewFsReadFile(workspaceRef: WorkspaceRef, request: ViewFsReadFileRequest): Promise<ViewFsReadFileResult> {
  return ipcInvoke<ViewFsReadFileResult>("view_fs_read_file", { workspaceRef, request });
}

export function viewFsWriteFile(workspaceRef: WorkspaceRef, request: ViewFsWriteFileRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_write_file", { workspaceRef, request });
}

export function viewFsAppendFile(workspaceRef: WorkspaceRef, request: ViewFsWriteFileRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_append_file", { workspaceRef, request });
}

export function viewFsMkdir(workspaceRef: WorkspaceRef, request: ViewFsMkdirRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_mkdir", { workspaceRef, request });
}

export function viewFsReaddir(workspaceRef: WorkspaceRef, request: ViewFsReaddirRequest): Promise<ViewFsReaddirResult> {
  return ipcInvoke<ViewFsReaddirResult>("view_fs_readdir", { workspaceRef, request });
}

export function viewFsStat(workspaceRef: WorkspaceRef, request: ViewFsPathRequest): Promise<ViewFsStatResult> {
  return ipcInvoke<ViewFsStatResult>("view_fs_stat", { workspaceRef, request });
}

export function viewFsLstat(workspaceRef: WorkspaceRef, request: ViewFsPathRequest): Promise<ViewFsStatResult> {
  return ipcInvoke<ViewFsStatResult>("view_fs_lstat", { workspaceRef, request });
}

export function viewFsAccess(workspaceRef: WorkspaceRef, request: ViewFsPathRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_access", { workspaceRef, request });
}

export function viewFsUnlink(workspaceRef: WorkspaceRef, request: ViewFsPathRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_unlink", { workspaceRef, request });
}

export function viewFsRm(workspaceRef: WorkspaceRef, request: ViewFsRmRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_rm", { workspaceRef, request });
}

export function viewFsRename(workspaceRef: WorkspaceRef, request: ViewFsRenameRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_rename", { workspaceRef, request });
}

export function viewFsCopyFile(workspaceRef: WorkspaceRef, request: ViewFsCopyFileRequest): Promise<void> {
  return ipcInvoke<void>("view_fs_copy_file", { workspaceRef, request });
}

export function viewAutomationRespond(
  workspaceRef: WorkspaceRef,
  requestId: string,
  ok: boolean,
  result?: unknown,
  error?: string | null,
): Promise<void> {
  return ipcInvoke<void>("view_automation_respond", {
    workspaceRef,
    requestId,
    ok,
    result: result ?? null,
    error: error ?? null,
  });
}
