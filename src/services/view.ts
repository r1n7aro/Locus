import { ipcInvoke } from "./ipc";

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

export interface ViewManifest {
  schema: string;
  id: string;
  name: string;
  version: string;
  template: string;
  icon?: string | null;
  entry: string;
  style: string;
  bindings: string;
  scripts: ViewScriptManifest[];
  capabilities: ViewCapabilities;
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
  packageRelPath?: string;
  packageRoot: string;
  manifestPath: string;
  updatedAt: number;
  capabilities: ViewCapabilities;
  temporary?: boolean;
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
  name?: string | null;
  template?: string | null;
  icon?: string | null;
  temporary?: boolean;
}

export interface ViewCreateFolderRequest {
  parentRelPath?: string | null;
  name: string;
}

export interface ViewDeleteEntryRequest {
  relPath: string;
}

export interface ViewMoveEntryRequest {
  sourceRelPath: string;
  targetDirRelPath?: string | null;
}

export interface ViewRunResult {
  id: string;
  windowLabel: string;
  hostUrl: string;
  packageRoot: string;
}

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

export interface ViewBindingTarget {
  kind: string;
  path?: string | null;
  scenePath?: string | null;
  objectPath?: string | null;
  componentType?: string | null;
  propertyPath?: string | null;
}

export interface ViewBindingReadRequest {
  viewId: string;
  bindingId?: string | null;
  target?: ViewBindingTarget | null;
}

export interface ViewBindingReadResult {
  ok: boolean;
  bindingId?: string | null;
  message: string;
  target: ViewBindingTarget;
  propertyPath: string;
  displayName: string;
  valueType: string;
  value: unknown;
  editable: boolean;
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

export const VIEW_HOST_PATH = "/view-host";

export function isViewHostWindowLocation(): boolean {
  return window.location.pathname === VIEW_HOST_PATH;
}

export function viewHostIdFromLocation(): string {
  return new URLSearchParams(window.location.search).get("id") || "";
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

export function viewMoveEntry(request: ViewMoveEntryRequest): Promise<ViewTreeSnapshot> {
  return ipcInvoke<ViewTreeSnapshot>("view_move_entry", { request });
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

export function viewBindingRead(request: ViewBindingReadRequest): Promise<ViewBindingReadResult> {
  return ipcInvoke<ViewBindingReadResult>("view_binding_read", { request });
}

export function viewBindingWrite(request: ViewBindingWriteRequest): Promise<ViewBindingWriteResult> {
  return ipcInvoke<ViewBindingWriteResult>("view_binding_write", { request });
}

export function viewBindingApply(request: ViewBindingApplyRequest): Promise<ViewBindingApplyResult> {
  return ipcInvoke<ViewBindingApplyResult>("view_binding_apply", { request });
}
