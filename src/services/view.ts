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
import { checkUnityConnectionStatus } from "./unity";

export interface ViewScriptManifest {
  name: string;
  path: string;
  entryType: string;
}

export interface ViewCapabilities {
  unity: boolean;
  bindings: boolean;
  writeBack: boolean;
}

export interface ViewRequirements {
  unityConnection: boolean;
}

export interface ViewManifest {
  schema: string;
  id: string;
  name: string;
  version: string;
  template: string;
  displayPath?: string | null;
  icon?: string | null;
  entry: string;
  style: string;
  bindings: string;
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

export interface ViewAutomationRequest {
  requestId: string;
  viewId: string;
  kind: string;
  payload: Record<string, unknown>;
}

export interface ViewBindingTarget {
  kind: string;
  path?: string | null;
  scenePath?: string | null;
  objectPath?: string | null;
  objectFileId?: number | null;
  targetFileId?: number | null;
  componentType?: string | null;
  componentIndex?: number | null;
  propertyPath?: string | null;
}

export interface ViewManagedReferenceTypeOption {
  label: string;
  value: string;
  fullName: string;
  assembly: string;
}

export interface ViewEnumOption {
  label: string;
  value: string;
  name: string;
  index: number;
  numericValue: number;
}

export interface ViewSerializedPropertyAttributeInfo {
  type: string;
  displayName: string;
  value: string;
}

export interface ViewSerializedPropertySnapshot {
  propertyPath: string;
  displayName: string;
  name: string;
  type: string;
  valueType: string;
  fieldTypeFullName: string;
  fieldTypeAssembly: string;
  value: unknown;
  displayValue: string;
  editable: boolean;
  hasChildren: boolean;
  isArray: boolean;
  arraySize: number;
  isFlagsEnum: boolean;
  enumValueIndex: number;
  enumValueFlag: number;
  enumOptions: ViewEnumOption[];
  children: ViewSerializedPropertySnapshot[];
  isManagedReference: boolean;
  managedReferenceFullTypename: string;
  managedReferenceFieldTypename: string;
  managedReferenceDisplayName: string;
  managedReferenceTypes: ViewManagedReferenceTypeOption[];
  tooltip: string;
  header: string;
  hasRange: boolean;
  rangeMin: number;
  rangeMax: number;
  numberStep: number;
  multiline: boolean;
  minLines: number;
  maxLines: number;
  referenceTypeFullName: string;
  referenceTypeAssembly: string;
  attributes: ViewSerializedPropertyAttributeInfo[];
}

export interface ViewBindingReadRequest {
  viewId: string;
  bindingId?: string | null;
  target?: ViewBindingTarget | null;
  maxDepth?: number | null;
  maxArrayItems?: number | null;
}

export interface ViewBindingDiscoverRequest {
  viewId: string;
  bindingId?: string | null;
  target?: ViewBindingTarget | null;
  query?: string | null;
  fieldName?: string | null;
  fieldType?: string | null;
  maxDepth?: number | null;
  maxResults?: number | null;
}

export interface ViewBindingDiscoverMatch {
  propertyPath: string;
  displayName: string;
  name: string;
  type: string;
  valueType: string;
  fieldTypeFullName: string;
  fieldTypeAssembly: string;
  displayValue: string;
  editable: boolean;
  hasChildren: boolean;
  isArray: boolean;
  isManagedReference: boolean;
  depth: number;
}

export interface ViewBindingDiscoverResult {
  ok: boolean;
  bindingId?: string | null;
  message: string;
  target: ViewBindingTarget;
  matches: ViewBindingDiscoverMatch[];
}

export interface ViewBindingReadResult extends ViewSerializedPropertySnapshot {
  ok: boolean;
  bindingId?: string | null;
  message: string;
  target: ViewBindingTarget;
}

export interface ViewBindingWriteRequest {
  viewId: string;
  bindingId?: string | null;
  target?: ViewBindingTarget | null;
  value: unknown;
}

export interface ViewBindingWriteResult extends ViewBindingReadResult {
  saved: boolean;
}

export interface ViewBindingApplyWrite {
  bindingId?: string | null;
  target?: ViewBindingTarget | null;
  value: unknown;
}

export interface ViewBindingApplyRequest {
  viewId: string;
  writes: ViewBindingApplyWrite[];
}

export interface ViewBindingApplyResult {
  ok: boolean;
  message: string;
  results: ViewBindingWriteResult[];
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

export function isViewHostWindowLocation(): boolean {
  return window.location.pathname === VIEW_HOST_PATH;
}

export function isViewContentWindowLocation(): boolean {
  return window.location.pathname === VIEW_CONTENT_PATH;
}

export function viewHostIdFromLocation(): string {
  return new URLSearchParams(window.location.search).get("id") || "";
}

export function isViewHostPoolWindowLocation(): boolean {
  return window.location.pathname === VIEW_HOST_PATH
    && new URLSearchParams(window.location.search).get("pool") === "1";
}

export function viewTemplates(): Promise<ViewTemplateSummary[]> {
  return ipcInvoke<ViewTemplateSummary[]>("view_templates");
}

export function viewList(): Promise<ViewPackageSummary[]> {
  return ipcInvoke<ViewPackageSummary[]>("view_list");
}

export function viewTree(): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_tree");
}

export function viewCreate(request: ViewCreateRequest): Promise<ViewPackageDetail> {
  return ipcInvoke<ViewPackageDetail>("view_create", { request });
}

export function viewCreateFolder(request: ViewCreateFolderRequest): Promise<ViewFolderSummary> {
  return ipcInvoke<ViewFolderSummary>("view_create_folder", { request });
}

export function viewDeleteEntry(request: ViewDeleteEntryRequest): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_delete_entry", { request });
}

export function viewRenameEntry(request: ViewRenameEntryRequest): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_rename_entry", { request });
}

export function viewMoveEntry(request: ViewMoveEntryRequest): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_move_entry", { request });
}

export function viewExportPackage(request: ViewExportPackageRequest): Promise<string> {
  return ipcInvoke<string>("view_export_package", { request });
}

export function viewImportPackage(
  request: ViewImportPackageRequest,
): Promise<ViewPackageImportResult> {
  return ipcInvoke<ViewPackageImportResult>("view_import_package", { request });
}

export function viewRead(viewId: string): Promise<ViewPackageDetail> {
  return ipcInvoke<ViewPackageDetail>("view_read", { viewId });
}

export function viewReload(viewId: string): Promise<ViewPackageSummary> {
  return ipcInvoke<ViewPackageSummary>("view_reload", { viewId });
}

export function viewRun(viewId: string): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_run", { viewId });
}

export function viewRunInUnity(viewId: string): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_run_in_unity", { viewId });
}

export function viewSetTabHost(request: ViewSetTabHostRequest): Promise<void> {
  return ipcInvoke<void>("view_set_tab_host", { request });
}

export function viewDetachTab(request: ViewDetachTabRequest): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_detach_tab", { request });
}

export function viewHostPoolPrepare(): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_host_pool_prepare");
}

export function viewHostPoolReady(hostLabel: string): Promise<void> {
  return ipcInvoke<void>("view_host_pool_ready", { hostLabel });
}

export function viewHostRevealed(hostLabel: string): Promise<void> {
  return ipcInvoke<void>("view_host_revealed", { hostLabel });
}

export function viewContentMount(request: ViewContentMountRequest): Promise<ViewRunResult> {
  return ipcInvoke<ViewRunResult>("view_content_mount", { request });
}

export function viewContentHide(viewId: string): Promise<void> {
  return ipcInvoke<void>("view_content_hide", { viewId });
}

export function viewContentDestroy(viewId: string): Promise<void> {
  return ipcInvoke<void>("view_content_destroy", { viewId });
}

export function viewRequiresUnityConnection(
  view: { requirements?: ViewRequirements | null; capabilities?: ViewCapabilities | null },
): boolean {
  return view.requirements?.unityConnection
    ?? !!(view.capabilities?.unity || view.capabilities?.bindings || view.capabilities?.writeBack);
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
  view: {
    name?: string | null;
    requirements?: ViewRequirements | null;
    capabilities?: ViewCapabilities | null;
  },
): Promise<AppErrorPayload | null> {
  if (!viewRequiresUnityConnection(view)) return null;

  const status = await checkUnityConnectionStatus();
  return status.connected ? null : viewUnityConnectionRequiredError(view.name);
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
  request: ViewCompileScriptRequest,
): Promise<ViewCompileScriptResult> {
  return ipcInvoke<ViewCompileScriptResult>("view_compile_script", { request });
}

export function viewCallScript(request: ViewCallScriptRequest): Promise<ViewCallScriptResult> {
  return ipcInvoke<ViewCallScriptResult>("view_call_script", { request });
}

export function viewAppendFrontendLog(request: ViewFrontendLogRequest): Promise<void> {
  return ipcInvoke<void>("view_append_frontend_log", { request });
}

export function viewReadFrontendLog(request: ViewFrontendLogReadRequest): Promise<ViewFrontendLogEntry[]> {
  return ipcInvoke<ViewFrontendLogEntry[]>("view_read_frontend_log", { request });
}

export function viewOpenFrontendLog(viewId: string): Promise<void> {
  return ipcInvoke<void>("view_open_frontend_log", { viewId });
}

export function viewStorageGet(request: ViewStorageGetRequest): Promise<unknown | null> {
  return ipcInvoke<unknown | null>("view_storage_get", { request });
}

export function viewStorageSet(request: ViewStorageSetRequest): Promise<void> {
  return ipcInvoke<void>("view_storage_set", { request });
}

export function viewStorageRemove(request: ViewStorageRemoveRequest): Promise<void> {
  return ipcInvoke<void>("view_storage_remove", { request });
}

export function viewAutomationRespond(
  requestId: string,
  ok: boolean,
  result?: unknown,
  error?: string | null,
): Promise<void> {
  return ipcInvoke<void>("view_automation_respond", {
    requestId,
    ok,
    result: result ?? null,
    error: error ?? null,
  });
}

export function viewBindingRead(request: ViewBindingReadRequest): Promise<ViewBindingReadResult> {
  return ipcInvoke<ViewBindingReadResult>("view_binding_read", { request });
}

export function viewBindingDiscover(request: ViewBindingDiscoverRequest): Promise<ViewBindingDiscoverResult> {
  return ipcInvoke<ViewBindingDiscoverResult>("view_binding_discover", { request });
}

export function viewBindingWrite(request: ViewBindingWriteRequest): Promise<ViewBindingWriteResult> {
  return ipcInvoke<ViewBindingWriteResult>("view_binding_write", { request });
}

export function viewBindingApply(request: ViewBindingApplyRequest): Promise<ViewBindingApplyResult> {
  return ipcInvoke<ViewBindingApplyResult>("view_binding_apply", { request });
}
