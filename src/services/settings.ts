// Generic settings IPC (`settings_schema` / `settings_get` / `settings_set`).
// Kept free of `ipc.ts` and `errors.ts`: those import i18n, and i18n reads its
// language from the settings store built on this module.
import { getLocusRuntime } from "./locusRuntime";
import type { WorkspaceRef } from "./project";

export const SETTINGS_CHANGED_EVENT = "settings-changed";

export type SettingScope = "app" | "project";
export type SettingSensitivity = "normal" | "sensitive";

export interface SettingKindDescription {
  type: "bool" | "int" | "enum" | "text" | "path" | "stringList" | "map";
  min?: number;
  max?: number;
  nullable?: boolean;
  values?: string[];
  maxLength?: number;
  valueType?: SettingKindDescription;
}

export interface SettingSchemaEntry {
  key: string;
  scope: SettingScope;
  sensitivity: SettingSensitivity;
  experimental: boolean;
  restart: boolean;
  /** Read before the first frame; mirrored into the webview boot cache. */
  boot: boolean;
  kind: SettingKindDescription;
  default: unknown;
}

export interface SettingChange {
  key: string;
  old: unknown;
  new: unknown;
}

export interface SettingsChangedEvent {
  changes: SettingChange[];
}

export interface SettingWrite {
  key: string;
  value: unknown;
}

export function getSettingsSchema(): Promise<SettingSchemaEntry[]> {
  return getLocusRuntime().invoke<SettingSchemaEntry[]>("settings_schema");
}

/** Effective values by key. Project settings need `workspaceRef`. */
export function getSettings(
  keys?: readonly string[] | null,
  workspaceRef?: WorkspaceRef | null,
): Promise<Record<string, unknown>> {
  return getLocusRuntime().invoke<Record<string, unknown>>("settings_get", {
    keys: keys ? [...keys] : null,
    workspaceRef: workspaceRef ?? null,
  });
}

/** Validates and writes the batch atomically; returns one change per key. */
export function setSettings(
  changes: readonly SettingWrite[],
  workspaceRef?: WorkspaceRef | null,
): Promise<SettingChange[]> {
  return getLocusRuntime().invoke<SettingChange[]>("settings_set", {
    changes: [...changes],
    workspaceRef: workspaceRef ?? null,
  });
}

/** One-time import of localStorage preferences; fills only unset settings. */
export function importFrontendLegacySettings(values: Record<string, unknown>): Promise<string[]> {
  return getLocusRuntime().invoke<string[]>("settings_import_frontend_legacy", { values });
}

/** Error text from an IPC rejection (AppError object, Error or string). */
export function settingsErrorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message) return message;
  }
  return String(error);
}
