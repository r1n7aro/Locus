// One settings snapshot per window, backed by `settings.json`.
//
// - Reads are synchronous: the snapshot starts from the boot cache in
//   localStorage (or, before the first migration, from the legacy keys) so the
//   first frame matches the last session; the backend snapshot replaces it.
// - Writes are optimistic and coalesced per key; a failed write rolls back.
// - Every window applies `settings-changed`, whatever wrote the change (this
//   or another window, the agent, a reset).
// - Without the desktop runtime (Remote app, Unity native Views, tests) the
//   snapshot lives in the boot cache only, like the old localStorage keys.
//
// Design: `design/设置系统/设置页面与存储整理.md`.
import { computed, reactive, readonly, shallowRef, type ComputedRef } from "vue";
import { getLocusRuntime } from "../services/locusRuntime";
import {
  SETTINGS_CHANGED_EVENT,
  getSettings,
  getSettingsSchema,
  importFrontendLegacySettings,
  setSettings,
  settingsErrorMessage,
  type SettingSchemaEntry,
  type SettingWrite,
  type SettingsChangedEvent,
} from "../services/settings";
import type { WorkspaceRef } from "../services/project";
import { readLegacyFrontendSettings } from "./settingsLegacy";

export const SETTINGS_BOOT_CACHE_KEY = "locus-settings-boot-cache";
export const SETTINGS_LEGACY_MIGRATED_KEY = "locus-settings-migrated-v1";
const BOOT_CACHE_VERSION = 1;

type Values = Record<string, unknown>;

interface StoreStatus {
  /** The backend snapshot has been applied (always true without a backend). */
  ready: boolean;
  /** Keys with a write in flight. */
  busy: Record<string, boolean>;
  /** Last write error per key, cleared by the next successful write. */
  errors: Record<string, string>;
  loadError: string | null;
}

interface PendingWrite {
  value: unknown;
  /** Workspace the change is made from; effects use it (e.g. LSP warm-up). */
  workspaceRef: WorkspaceRef | null;
  waiters: { resolve: () => void; reject: (error: unknown) => void }[];
}

const values = reactive<Values>({});
/** Last value the backend acknowledged; the rollback target of failed writes. */
const confirmed: Values = {};
const status = reactive<StoreStatus>({ ready: false, busy: {}, errors: {}, loadError: null });
let schema: Map<string, SettingSchemaEntry> | null = null;
/** Reactive copy of `schema` for components (set once it loads). */
const schemaRef = shallowRef<Map<string, SettingSchemaEntry> | null>(null);
let schemaRequest: Promise<Map<string, SettingSchemaEntry>> | null = null;
let startRequest: Promise<void> | null = null;
const inFlight = new Set<string>();
const queued = new Map<string, PendingWrite>();

function storage(): globalThis.Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

function clone<T>(value: T): T {
  return value === null || typeof value !== "object" ? value : JSON.parse(JSON.stringify(value));
}

function same(left: unknown, right: unknown): boolean {
  return left === right || JSON.stringify(left) === JSON.stringify(right);
}

function readBootCache(): Values | null {
  const raw = storage()?.getItem(SETTINGS_BOOT_CACHE_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as { version?: unknown; values?: unknown };
    if (parsed.version !== BOOT_CACHE_VERSION || !parsed.values || typeof parsed.values !== "object") {
      return null;
    }
    return parsed.values as Values;
  } catch {
    return null;
  }
}

function writeBootCache(): void {
  const cached: Values = {};
  for (const [key, value] of Object.entries(values)) {
    if (!schema || schema.get(key)?.boot) cached[key] = value;
  }
  try {
    storage()?.setItem(
      SETTINGS_BOOT_CACHE_KEY,
      JSON.stringify({ version: BOOT_CACHE_VERSION, values: cached }),
    );
  } catch {
    /* The cache only speeds up the first frame. */
  }
}

/** Decided once per window, when the store starts: a window's runtime does
 *  not change kind, and a runtime installed later (the Remote app installs
 *  its own) must not redirect writes to a backend that never loaded. */
let backendMode: boolean | null = null;

function hasBackend(): boolean {
  if (backendMode === null) {
    try {
      backendMode = getLocusRuntime().kind === "tauri";
    } catch {
      backendMode = false;
    }
  }
  return backendMode;
}

function seed(): void {
  const initial = readBootCache() ?? readLegacyFrontendSettings();
  for (const [key, value] of Object.entries(initial)) {
    values[key] = value;
    confirmed[key] = clone(value);
  }
}

function applyRemote(key: string, value: unknown): void {
  confirmed[key] = clone(value);
  // A newer local value is still on its way; its own result settles the key.
  if (queued.has(key)) return;
  if (!same(values[key], value)) values[key] = clone(value);
}

function loadSchema(): Promise<Map<string, SettingSchemaEntry>> {
  if (schema) return Promise.resolve(schema);
  if (!hasBackend()) return Promise.reject(new Error("Settings metadata needs the desktop runtime."));
  schemaRequest ??= getSettingsSchema()
    .then((entries) => {
      schema = new Map(entries.map((entry) => [entry.key, entry]));
      schemaRef.value = schema;
      return schema;
    })
    .finally(() => {
      schemaRequest = null;
    });
  return schemaRequest;
}

async function importLegacyOnce(): Promise<void> {
  const store = storage();
  if (!store || store.getItem(SETTINGS_LEGACY_MIGRATED_KEY)) return;
  const legacy = readLegacyFrontendSettings(store);
  if (Object.keys(legacy).length > 0) {
    // Only fills settings that are still unset, so concurrent windows and
    // repeated attempts are harmless.
    await importFrontendLegacySettings(legacy);
  }
  store.setItem(SETTINGS_LEGACY_MIGRATED_KEY, new Date().toISOString());
}

async function start(): Promise<void> {
  const changedDuringLoad = new Set<string>();
  let loading = true;
  await getLocusRuntime().subscribe<SettingsChangedEvent>(SETTINGS_CHANGED_EVENT, (event) => {
    for (const change of event?.changes ?? []) {
      if (loading) changedDuringLoad.add(change.key);
      if (!schema || schema.get(change.key)?.scope !== "project") applyRemote(change.key, change.new);
    }
    if (!loading) writeBootCache();
  });
  try {
    await importLegacyOnce().catch((error) => {
      console.warn("[settings] legacy preference import failed", error);
    });
    const [entries, snapshot] = await Promise.all([loadSchema(), getSettings()]);
    for (const [key, value] of Object.entries(snapshot)) {
      if (entries.get(key)?.scope !== "app" || changedDuringLoad.has(key)) continue;
      applyRemote(key, value);
    }
    status.loadError = null;
    writeBootCache();
  } catch (error) {
    status.loadError = settingsErrorMessage(error);
    console.warn("[settings] failed to load settings", error);
  } finally {
    loading = false;
    status.ready = true;
  }
}

/** Loads the backend snapshot once per window; resolves when it is applied. */
export function ensureSettingsLoaded(): Promise<void> {
  if (!hasBackend()) {
    status.ready = true;
    return Promise.resolve();
  }
  startRequest ??= start();
  return startRequest;
}

/** Setting metadata (types, defaults, flags); requires the desktop runtime. */
export function loadSettingsSchema(): Promise<Map<string, SettingSchemaEntry>> {
  return loadSchema();
}

/** Metadata of one setting; `null` until the schema loads (or without a backend). */
export function useSettingSchema(key: string): ComputedRef<SettingSchemaEntry | null> {
  void loadSchema().catch(() => {});
  return computed(() => schemaRef.value?.get(key) ?? null);
}

/** Every setting's metadata, for search and generic rendering. */
export function useSettingsSchema(): ComputedRef<Map<string, SettingSchemaEntry> | null> {
  void loadSchema().catch(() => {});
  return computed(() => schemaRef.value);
}

/** Reactive read; `undefined` until the key is cached or loaded. */
export function settingValue(key: string): unknown {
  return values[key];
}

async function reportWriteError(key: string, error: unknown): Promise<void> {
  const message = settingsErrorMessage(error);
  console.warn(`[settings] failed to save '${key}':`, error);
  try {
    const [{ useNotificationStore }] = await Promise.all([import("../stores/notification")]);
    useNotificationStore().addNotice("error", message, {
      code: "settings.write_failed",
      operation: `settings:${key}`,
      replaceOperation: true,
    });
  } catch {
    /* No notification surface in this window. */
  }
}

async function flush(key: string): Promise<void> {
  const pending = queued.get(key);
  if (!pending) return;
  queued.delete(key);
  inFlight.add(key);
  status.busy[key] = true;
  try {
    const [change] = await setSettings([{ key, value: pending.value }], pending.workspaceRef);
    const stored = change ? change.new : pending.value;
    confirmed[key] = clone(stored);
    delete status.errors[key];
    // Adopt the backend's canonical form unless something newer (a queued
    // write or another writer's event) already replaced the sent value.
    if (!queued.has(key) && same(values[key], pending.value)) values[key] = clone(stored);
    for (const waiter of pending.waiters) waiter.resolve();
  } catch (error) {
    status.errors[key] = settingsErrorMessage(error);
    if (!queued.has(key) && same(values[key], pending.value)) values[key] = clone(confirmed[key]);
    for (const waiter of pending.waiters) waiter.reject(error);
  } finally {
    inFlight.delete(key);
    if (queued.has(key)) {
      void flush(key);
    } else {
      delete status.busy[key];
      writeBootCache();
    }
  }
}

/**
 * Sets one setting: the snapshot updates at once (and synchronously for
 * watchers), the backend write follows. Rapid writes to one key are
 * coalesced into the latest value. Rejects after rolling back on failure.
 * `workspaceRef` names the workspace the change is made from, for settings
 * whose effect targets it.
 */
export function setSettingValue(
  key: string,
  value: unknown,
  workspaceRef: WorkspaceRef | null = null,
): Promise<void> {
  if (same(values[key], value) && !queued.has(key) && !inFlight.has(key)) return Promise.resolve();
  values[key] = clone(value);
  if (!hasBackend()) {
    confirmed[key] = clone(value);
    writeBootCache();
    return Promise.resolve();
  }
  return new Promise<void>((resolve, reject) => {
    const pending = queued.get(key) ?? { value, workspaceRef, waiters: [] };
    pending.value = clone(value);
    pending.workspaceRef = workspaceRef;
    pending.waiters.push({ resolve, reject });
    queued.set(key, pending);
    status.busy[key] = true;
    if (!inFlight.has(key)) void flush(key);
  });
}

/** Fire-and-forget write for synchronous setters; failures are reported. */
export function saveSetting(key: string, value: unknown): void {
  void setSettingValue(key, value).catch((error) => reportWriteError(key, error));
}

/** Writes several settings atomically (all or none). */
export async function setSettingValues(changes: readonly SettingWrite[]): Promise<void> {
  const previous = changes.map(({ key }) => [key, values[key]] as const);
  for (const { key, value } of changes) values[key] = clone(value);
  if (!hasBackend()) {
    for (const { key, value } of changes) confirmed[key] = clone(value);
    writeBootCache();
    return;
  }
  for (const { key } of changes) status.busy[key] = true;
  try {
    const applied = await setSettings(changes);
    for (const change of applied) {
      confirmed[change.key] = clone(change.new);
      delete status.errors[change.key];
      if (!queued.has(change.key)) values[change.key] = clone(change.new);
    }
    writeBootCache();
  } catch (error) {
    for (const [key, value] of previous) {
      status.errors[key] = settingsErrorMessage(error);
      if (!queued.has(key)) values[key] = value;
    }
    throw error;
  } finally {
    for (const { key } of changes) if (!inFlight.has(key)) delete status.busy[key];
  }
}

/** Fire-and-forget batch write; failures are reported. */
export function saveSettings(changes: readonly SettingWrite[]): void {
  void setSettingValues(changes).catch((error) =>
    reportWriteError(changes.map(({ key }) => key).join(", "), error));
}

export interface UseSetting<T> {
  value: ComputedRef<T>;
  ready: ComputedRef<boolean>;
  busy: ComputedRef<boolean>;
  error: ComputedRef<string | null>;
  set: (value: T) => Promise<void>;
}

/** One setting with its load and write state, normalized by `read`. */
export function useSetting<T>(key: string, read: (raw: unknown) => T): UseSetting<T> {
  void ensureSettingsLoaded();
  return {
    value: computed(() => read(values[key])),
    ready: computed(() => status.ready),
    busy: computed(() => Boolean(status.busy[key])),
    error: computed(() => status.errors[key] ?? null),
    set: (value: T) => setSettingValue(key, value),
  };
}

export function useSettingsStore() {
  void ensureSettingsLoaded();
  return {
    values: readonly(values),
    status: readonly(status),
    get: settingValue,
    set: setSettingValue,
    setMany: setSettingValues,
  };
}

// Common readers for `useSetting` and the preference modules.
export const readBool = (fallback: boolean) => (raw: unknown): boolean =>
  typeof raw === "boolean" ? raw : fallback;
export const readString = (fallback: string) => (raw: unknown): string =>
  typeof raw === "string" ? raw : fallback;
export const readEnum = <T extends string>(allowed: readonly T[], fallback: T) => (raw: unknown): T =>
  allowed.includes(raw as T) ? raw as T : fallback;
export const readNumber = (fallback: number, min = -Infinity, max = Infinity) => (raw: unknown): number =>
  typeof raw === "number" && Number.isFinite(raw) && raw >= min && raw <= max ? raw : fallback;

/** Test hook: forget every value and restart from storage. */
export function resetSettingsStoreForTests(): void {
  for (const key of Object.keys(values)) delete values[key];
  for (const key of Object.keys(confirmed)) delete confirmed[key];
  status.ready = false;
  status.busy = {};
  status.errors = {};
  status.loadError = null;
  schema = null;
  schemaRef.value = null;
  schemaRequest = null;
  startRequest = null;
  backendMode = null;
  inFlight.clear();
  queued.clear();
  seed();
}

seed();
// Windows start syncing as soon as any preference module loads.
void ensureSettingsLoaded();
