import { emitTo } from "@tauri-apps/api/event";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  PhysicalPosition,
  cursorPosition,
  monitorFromPoint,
  type Window as TauriWindowHandle,
} from "@tauri-apps/api/window";
import type {
  WorkbenchEditorTransferRecord,
  WorkbenchWindowDropIntent,
} from "../types/workbench";
import { currentThemeBackgroundColor } from "../composables/useTheme";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const WORKBENCH_WINDOW_FLAG = "workbenchWindow";
export const WORKBENCH_WINDOW_POOL_FLAG = "workbenchWindowPool";
export const WORKBENCH_WINDOW_TRANSFER_PARAM = "transferToken";
export const WORKBENCH_WINDOW_RESTORE_FLAG = "restore";

export const WORKBENCH_WINDOW_DRAG_OVER_EVENT = "workbench-window:drag-over";
export const WORKBENCH_WINDOW_DRAG_LEAVE_EVENT = "workbench-window:drag-leave";
export const WORKBENCH_WINDOW_DRAG_TARGET_EVENT = "workbench-window:drag-target";
export const WORKBENCH_WINDOW_TRANSFER_PREPARE_EVENT = "workbench-window:transfer-prepare";
export const WORKBENCH_WINDOW_TRANSFER_ACK_EVENT = "workbench-window:transfer-ack";
export const WORKBENCH_WINDOW_TRANSFER_CANCEL_EVENT = "workbench-window:transfer-cancel";
export const WORKBENCH_WINDOW_POOL_CLAIM_EVENT = "workbench-window:pool-claim";

export const WORKBENCH_WINDOW_LABEL_PREFIX = "workbench-";
export const WORKBENCH_WINDOW_POOL_LABEL_PREFIX = "workbench-pool-";
export const WORKBENCH_TRANSFER_TIMEOUT_MS = 8_000;

const TRANSFER_STORAGE_PREFIX = "locus:workbench-transfer:";
const TRANSFER_DB_NAME = "locus-workbench-transfer-v1";
const TRANSFER_DB_STORE = "transfers";
const AUX_WINDOW_REGISTRY_KEY = "locus:workbench-aux-windows:v1";
const POOL_STATE_KEY = "locus:workbench-window-pool:v1";
const TRANSFER_MAX_AGE_MS = 60_000;
const DEFAULT_WIDTH = 1120;
const DEFAULT_HEIGHT = 760;

export interface WorkbenchWindowScreenPoint {
  x: number;
  y: number;
}

export interface WorkbenchWindowDragOverPayload {
  sequence: number;
  sourceLabel: string;
  screenPoint: WorkbenchWindowScreenPoint;
  editorId: string;
  title: string;
}

export interface WorkbenchWindowDragLeavePayload {
  sourceLabel: string;
}

export interface WorkbenchWindowDragTargetPayload {
  sequence: number;
  targetLabel: string;
  intent: WorkbenchWindowDropIntent | null;
}

export interface WorkbenchWindowTransferPreparePayload {
  token: string;
  target: WorkbenchWindowDropIntent;
}

export interface WorkbenchWindowTransferAckPayload {
  token: string;
  targetWindowId: string;
  paneId?: string;
  editorId?: string;
  inserted?: boolean;
  readyAt: number;
  error?: string;
}

export interface WorkbenchWindowTransferCancelPayload {
  token: string;
}

export interface WorkbenchWindowPoolClaimPayload {
  token: string;
  startedAt: number;
  geometry: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
}

export interface WorkbenchAuxWindowRecord {
  label: string;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  maximized?: boolean;
  updatedAt: number;
}

interface WorkbenchWindowPoolState {
  label: string;
  ready: boolean;
  createdAt: number;
}

export interface WorkbenchWindowMetric {
  name: string;
  token?: string;
  at: number;
  durationMs?: number;
  detail?: Record<string, unknown>;
}

function nowMs(): number {
  return Date.now();
}

function randomId(prefix: string): string {
  const suffix = globalThis.crypto?.randomUUID?.()
    ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  return `${prefix}${suffix}`;
}

function readJson<T>(key: string, fallback: T): T {
  if (typeof window === "undefined") return fallback;
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? JSON.parse(raw) as T : fallback;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(key, JSON.stringify(value));
}

export function readWorkbenchAuxWindowRegistry(): WorkbenchAuxWindowRecord[] {
  const records = readJson<WorkbenchAuxWindowRecord[]>(AUX_WINDOW_REGISTRY_KEY, []);
  return Array.isArray(records)
    ? records.filter((record) => !!record?.label && record.label !== "main")
    : [];
}

function writeAuxWindowRegistry(records: WorkbenchAuxWindowRecord[]): void {
  writeJson(AUX_WINDOW_REGISTRY_KEY, records);
}

export function isWorkbenchWindowLabel(label: string): boolean {
  return label === "main" || label.startsWith(WORKBENCH_WINDOW_LABEL_PREFIX);
}

export function isWorkbenchAuxWindowLabel(label: string): boolean {
  return label !== "main" && label.startsWith(WORKBENCH_WINDOW_LABEL_PREFIX);
}

export function isWorkbenchWindowLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  return new URLSearchParams(locationLike.search).get(WORKBENCH_WINDOW_FLAG) === "1";
}

export function isWorkbenchWindowPoolLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  const params = new URLSearchParams(locationLike.search);
  return params.get(WORKBENCH_WINDOW_FLAG) === "1"
    && params.get(WORKBENCH_WINDOW_POOL_FLAG) === "1";
}

export function isWorkbenchWindowRestoreLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  const params = new URLSearchParams(locationLike.search);
  return params.get(WORKBENCH_WINDOW_FLAG) === "1"
    && params.get(WORKBENCH_WINDOW_RESTORE_FLAG) === "1";
}

export function workbenchTransferTokenFromLocation(search = window.location.search): string {
  return new URLSearchParams(search).get(WORKBENCH_WINDOW_TRANSFER_PARAM)?.trim() ?? "";
}

export function buildWorkbenchWindowUrl(options: {
  token?: string;
  pool?: boolean;
  restore?: boolean;
} = {}): string {
  const params = new URLSearchParams({ [WORKBENCH_WINDOW_FLAG]: "1" });
  if (options.token) params.set(WORKBENCH_WINDOW_TRANSFER_PARAM, options.token);
  if (options.pool) params.set(WORKBENCH_WINDOW_POOL_FLAG, "1");
  if (options.restore) params.set(WORKBENCH_WINDOW_RESTORE_FLAG, "1");
  return `/window.html?${params.toString()}`;
}

function openTransferDatabase(): Promise<IDBDatabase | null> {
  if (typeof indexedDB === "undefined") return Promise.resolve(null);
  return new Promise((resolve) => {
    const request = indexedDB.open(TRANSFER_DB_NAME, 1);
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(TRANSFER_DB_STORE)) {
        request.result.createObjectStore(TRANSFER_DB_STORE, { keyPath: "token" });
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => resolve(null);
  });
}

async function writeTransferRecord(record: WorkbenchEditorTransferRecord): Promise<void> {
  const database = await openTransferDatabase();
  if (database) {
    const completed = await new Promise<boolean>((resolve) => {
      const transaction = database.transaction(TRANSFER_DB_STORE, "readwrite");
      transaction.objectStore(TRANSFER_DB_STORE).put(record);
      transaction.oncomplete = () => resolve(true);
      transaction.onerror = () => resolve(false);
      transaction.onabort = () => resolve(false);
    });
    database.close();
    if (completed) return;
  }
  writeJson(`${TRANSFER_STORAGE_PREFIX}${record.token}`, record);
}

export function createInMemoryWorkbenchTransferRecord(
  input: Omit<WorkbenchEditorTransferRecord, "version" | "token" | "createdAt">,
): WorkbenchEditorTransferRecord {
  return {
    ...input,
    version: 1,
    token: randomId("transfer-"),
    createdAt: nowMs(),
  };
}

export async function persistWorkbenchTransferRecord(
  record: WorkbenchEditorTransferRecord,
): Promise<void> {
  await writeTransferRecord(record);
}

export async function createWorkbenchTransferRecord(
  input: Omit<WorkbenchEditorTransferRecord, "version" | "token" | "createdAt">,
): Promise<WorkbenchEditorTransferRecord> {
  const record = createInMemoryWorkbenchTransferRecord(input);
  await persistWorkbenchTransferRecord(record);
  return record;
}

export async function readWorkbenchTransferRecord(
  token: string,
): Promise<WorkbenchEditorTransferRecord | null> {
  const database = await openTransferDatabase();
  let record: WorkbenchEditorTransferRecord | null = null;
  if (database) {
    record = await new Promise<WorkbenchEditorTransferRecord | null>((resolve) => {
      const request = database.transaction(TRANSFER_DB_STORE, "readonly")
        .objectStore(TRANSFER_DB_STORE)
        .get(token);
      request.onsuccess = () => resolve(request.result as WorkbenchEditorTransferRecord | null);
      request.onerror = () => resolve(null);
    });
    database.close();
  }
  record ??= readJson<WorkbenchEditorTransferRecord | null>(
    `${TRANSFER_STORAGE_PREFIX}${token}`,
    null,
  );
  if (
    !record
    || record.version !== 1
    || record.token !== token
    || nowMs() - record.createdAt > TRANSFER_MAX_AGE_MS
  ) return null;
  return record;
}

export async function removeWorkbenchTransferRecord(token: string): Promise<void> {
  if (!token || typeof window === "undefined") return;
  const database = await openTransferDatabase();
  if (database) {
    await new Promise<void>((resolve) => {
      const transaction = database.transaction(TRANSFER_DB_STORE, "readwrite");
      transaction.objectStore(TRANSFER_DB_STORE).delete(token);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => resolve();
      transaction.onabort = () => resolve();
    });
    database.close();
  }
  window.localStorage.removeItem(`${TRANSFER_STORAGE_PREFIX}${token}`);
}

export async function cleanupStaleWorkbenchTransfers(): Promise<void> {
  if (typeof window === "undefined") return;
  const database = await openTransferDatabase();
  if (database) {
    const records = await new Promise<WorkbenchEditorTransferRecord[]>((resolve) => {
      const request = database.transaction(TRANSFER_DB_STORE, "readonly")
        .objectStore(TRANSFER_DB_STORE)
        .getAll();
      request.onsuccess = () => resolve(
        Array.isArray(request.result) ? request.result as WorkbenchEditorTransferRecord[] : [],
      );
      request.onerror = () => resolve([]);
    });
    database.close();
    await Promise.all(records
      .filter((record) => nowMs() - record.createdAt > TRANSFER_MAX_AGE_MS)
      .map((record) => removeWorkbenchTransferRecord(record.token)));
  }
  const staleKeys: string[] = [];
  for (let index = 0; index < window.localStorage.length; index += 1) {
    const key = window.localStorage.key(index);
    if (!key?.startsWith(TRANSFER_STORAGE_PREFIX)) continue;
    const record = readJson<WorkbenchEditorTransferRecord | null>(key, null);
    if (!record || nowMs() - record.createdAt > TRANSFER_MAX_AGE_MS) staleKeys.push(key);
  }
  for (const key of staleKeys) window.localStorage.removeItem(key);
}

export function recordWorkbenchWindowMetric(
  name: string,
  options: {
    token?: string;
    startedAt?: number;
    detail?: Record<string, unknown>;
  } = {},
): WorkbenchWindowMetric {
  const at = nowMs();
  const metric: WorkbenchWindowMetric = {
    name,
    token: options.token,
    at,
    durationMs: options.startedAt == null ? undefined : Math.max(0, at - options.startedAt),
    detail: options.detail,
  };
  const target = window as unknown as {
    __LOCUS_WORKBENCH_WINDOW_METRICS__?: WorkbenchWindowMetric[];
  };
  const metrics = target.__LOCUS_WORKBENCH_WINDOW_METRICS__ ?? [];
  metrics.push(metric);
  if (metrics.length > 100) metrics.splice(0, metrics.length - 100);
  target.__LOCUS_WORKBENCH_WINDOW_METRICS__ = metrics;
  console.info("[workbench-window-perf]", JSON.stringify(metric));
  return metric;
}

export function registerWorkbenchAuxWindow(
  label: string,
  patch: Partial<Omit<WorkbenchAuxWindowRecord, "label">> = {},
): void {
  if (!isWorkbenchAuxWindowLabel(label)) return;
  const records = readWorkbenchAuxWindowRegistry();
  const existing = records.find((record) => record.label === label);
  const next: WorkbenchAuxWindowRecord = {
    ...(existing ?? { label, updatedAt: nowMs() }),
    ...patch,
    label,
    updatedAt: nowMs(),
  };
  writeAuxWindowRegistry([
    ...records.filter((record) => record.label !== label),
    next,
  ]);
}

export function getWorkbenchAuxWindowRecord(label: string): WorkbenchAuxWindowRecord | null {
  return readWorkbenchAuxWindowRegistry().find((record) => record.label === label) ?? null;
}

export function unregisterWorkbenchAuxWindow(label: string): void {
  writeAuxWindowRegistry(readWorkbenchAuxWindowRegistry().filter((record) => record.label !== label));
}

export async function persistWorkbenchWindowBounds(
  appWindow: TauriWindowHandle,
): Promise<void> {
  if (!isWorkbenchAuxWindowLabel(appWindow.label)) return;
  const [position, size, factor, maximized] = await Promise.all([
    appWindow.outerPosition(),
    appWindow.outerSize(),
    appWindow.scaleFactor(),
    appWindow.isMaximized(),
  ]);
  const logicalPosition = position.toLogical(factor);
  const logicalSize = size.toLogical(factor);
  registerWorkbenchAuxWindow(appWindow.label, {
    x: logicalPosition.x,
    y: logicalPosition.y,
    width: logicalSize.width,
    height: logicalSize.height,
    maximized,
  });
}

export async function detachedWindowGeometry(
  point: WorkbenchWindowScreenPoint,
  tabAnchor?: { x: number; y: number },
): Promise<{
  x: number;
  y: number;
  width: number;
  height: number;
}> {
  const monitor = await monitorFromPoint(point.x, point.y).catch(() => null);
  const factor = monitor?.scaleFactor ?? 1;
  const logicalPoint = new PhysicalPosition(point.x, point.y).toLogical(factor);
  const workAreaPosition = monitor?.workArea.position.toLogical(factor);
  const workAreaSize = monitor?.workArea.size.toLogical(factor);
  const width = Math.min(DEFAULT_WIDTH, workAreaSize?.width ?? DEFAULT_WIDTH);
  const height = Math.min(DEFAULT_HEIGHT, workAreaSize?.height ?? DEFAULT_HEIGHT);
  const tabOrigin = { x: 1, y: 33 };
  const desiredX = tabAnchor
    ? logicalPoint.x - tabOrigin.x - tabAnchor.x
    : logicalPoint.x - 96;
  const desiredY = tabAnchor
    ? logicalPoint.y - tabOrigin.y - tabAnchor.y
    : logicalPoint.y - 18;
  const minX = workAreaPosition?.x ?? desiredX;
  const minY = workAreaPosition?.y ?? desiredY;
  const maxX = minX + Math.max(0, (workAreaSize?.width ?? width) - width);
  const maxY = minY + Math.max(0, (workAreaSize?.height ?? height) - height);
  return {
    x: Math.min(maxX, Math.max(minX, desiredX)),
    y: Math.min(maxY, Math.max(minY, desiredY)),
    width,
    height,
  };
}

function createWorkbenchWebviewWindow(
  label: string,
  url: string,
  geometry: { x?: number; y?: number; width?: number; height?: number } = {},
): WebviewWindow {
  return new WebviewWindow(label, {
    url,
    title: "Locus",
    x: geometry.x,
    y: geometry.y,
    width: geometry.width ?? DEFAULT_WIDTH,
    height: geometry.height ?? DEFAULT_HEIGHT,
    minWidth: 620,
    minHeight: 420,
    visible: false,
    focus: false,
    decorations: false,
    shadow: true,
    resizable: true,
    maximizable: true,
    minimizable: true,
    closable: true,
    backgroundColor: currentThemeBackgroundColor(),
    dragDropEnabled: true,
  });
}

function readPoolState(): WorkbenchWindowPoolState | null {
  const state = readJson<WorkbenchWindowPoolState | null>(POOL_STATE_KEY, null);
  return state?.label ? state : null;
}

function writePoolState(state: WorkbenchWindowPoolState | null): void {
  if (state) writeJson(POOL_STATE_KEY, state);
  else window.localStorage.removeItem(POOL_STATE_KEY);
}

export function markWorkbenchWindowPoolReady(label: string): void {
  writePoolState({ label, ready: true, createdAt: nowMs() });
  recordWorkbenchWindowMetric("pool-ready", { detail: { label } });
}

export async function prepareWorkbenchWindowPool(): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  const current = readPoolState();
  if (current) {
    const existing = await WebviewWindow.getByLabel(current.label).catch(() => null);
    if (existing) return;
    writePoolState(null);
  }
  const label = randomId(WORKBENCH_WINDOW_POOL_LABEL_PREFIX);
  writePoolState({ label, ready: false, createdAt: nowMs() });
  const pool = createWorkbenchWebviewWindow(label, buildWorkbenchWindowUrl({ pool: true }), {
    x: -32000,
    y: -32000,
    width: DEFAULT_WIDTH,
    height: DEFAULT_HEIGHT,
  });
  await pool.once("tauri://error", () => {
    const state = readPoolState();
    if (state?.label === label) writePoolState(null);
  }).catch(() => undefined);
}

async function claimWorkbenchWindowPool(
  token: string,
  startedAt: number,
  geometry: { x: number; y: number; width: number; height: number },
): Promise<string | null> {
  const state = readPoolState();
  if (!state?.ready) return null;
  const pool = await WebviewWindow.getByLabel(state.label).catch(() => null);
  if (!pool) {
    writePoolState(null);
    return null;
  }
  writePoolState(null);
  registerWorkbenchAuxWindow(state.label);
  await emitTo<WorkbenchWindowPoolClaimPayload>(
    state.label,
    WORKBENCH_WINDOW_POOL_CLAIM_EVENT,
    { token, startedAt, geometry },
  );
  recordWorkbenchWindowMetric("pool-claimed", { token, detail: { label: state.label } });
  window.setTimeout(() => void prepareWorkbenchWindowPool(), 0);
  return state.label;
}

export async function createDetachedWorkbenchWindow(
  token: string,
  point: WorkbenchWindowScreenPoint,
  startedAt = nowMs(),
): Promise<{ label: string; pooled: boolean }> {
  const geometry = await detachedWindowGeometry(point);
  const pooledLabel = await claimWorkbenchWindowPool(token, startedAt, geometry).catch(() => null);
  if (pooledLabel) return { label: pooledLabel, pooled: true };

  const label = randomId(WORKBENCH_WINDOW_LABEL_PREFIX);
  registerWorkbenchAuxWindow(label, geometry);
  createWorkbenchWebviewWindow(label, buildWorkbenchWindowUrl({ token }), geometry);
  recordWorkbenchWindowMetric("direct-window-created", { token, detail: { label, ...geometry } });
  window.setTimeout(() => void prepareWorkbenchWindowPool(), 0);
  return { label, pooled: false };
}

export async function restoreWorkbenchAuxWindows(): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  await cleanupStaleWorkbenchTransfers();
  for (const record of readWorkbenchAuxWindowRegistry()) {
    const existing = await WebviewWindow.getByLabel(record.label).catch(() => null);
    if (existing) continue;
    createWorkbenchWebviewWindow(record.label, buildWorkbenchWindowUrl({ restore: true }), {
      x: record.x,
      y: record.y,
      width: record.width,
      height: record.height,
    });
  }
}

export async function currentWorkbenchCursorPosition(): Promise<WorkbenchWindowScreenPoint> {
  const point = await cursorPosition();
  return { x: point.x, y: point.y };
}
