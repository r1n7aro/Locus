import type { Locale } from "../i18n";
import type {
  AppUpdateChangeGroup,
  AppUpdateDownloadChannel,
  AppUpdateInfo,
  AppUpdateLocaleEntry,
  AppUpdateManifest,
  AppUpdateManifestFetchResult,
  AppUpdateSourceKind,
} from "../types";
import { ipcInvoke } from "./ipc";

const DOCS_BASE_URL = "https://unity.farlocus.com";
const SEMVER_PATTERN =
  /^v?(?<core>\d+(?:\.\d+)*)(?:-(?<prerelease>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+(?<build>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/i;

type ParsedVersion = {
  core: number[];
  prerelease: string[];
};

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function isAppUpdateChangeGroup(value: unknown): value is AppUpdateChangeGroup {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return isNonEmptyString(record.title) && isStringArray(record.items);
}

function isAppUpdateDownloadChannel(value: unknown): value is AppUpdateDownloadChannel {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return isNonEmptyString(record.label) && isNonEmptyString(record.url);
}

function isAppUpdateLocaleEntry(value: unknown): value is AppUpdateLocaleEntry {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    isNonEmptyString(record.title)
    && typeof record.summary === "string"
    && isNonEmptyString(record.changelogUrl)
    && Array.isArray(record.changes)
    && record.changes.every(isAppUpdateChangeGroup)
    && (
      record.downloadChannels === undefined
      || (
        Array.isArray(record.downloadChannels)
        && record.downloadChannels.every(isAppUpdateDownloadChannel)
      )
    )
  );
}

export function isAppUpdateManifest(value: unknown): value is AppUpdateManifest {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  if (
    !isNonEmptyString(record.version)
    || !isNonEmptyString(record.releasedAt)
    || !isNonEmptyString(record.channel)
    || typeof record.locales !== "object"
    || record.locales === null
  ) {
    return false;
  }

  const localeEntries = Object.values(record.locales as Record<string, unknown>);
  return localeEntries.length > 0 && localeEntries.every(isAppUpdateLocaleEntry);
}

function isAppUpdateSourceKind(value: unknown): value is AppUpdateSourceKind {
  return value === "local" || value === "remote";
}

export function isAppUpdateManifestFetchResult(value: unknown): value is AppUpdateManifestFetchResult {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    isAppUpdateManifest(record.manifest)
    && isAppUpdateSourceKind(record.sourceKind)
    && isNonEmptyString(record.sourceBaseUrl)
  );
}

export function publicDocsBaseUrl(): string {
  return DOCS_BASE_URL;
}

function parseVersion(value: string): ParsedVersion | null {
  const trimmed = value.trim();
  if (!trimmed) return null;

  const match = trimmed.match(SEMVER_PATTERN);
  if (!match?.groups?.core) {
    return null;
  }

  const core = match.groups.core
    .split(".")
    .map((segment) => Number.parseInt(segment, 10));
  if (core.length === 0 || core.some((segment) => Number.isNaN(segment))) {
    return null;
  }

  while (core.length < 3) {
    core.push(0);
  }

  return {
    core,
    prerelease: match.groups.prerelease
      ? match.groups.prerelease.split(".").filter((segment) => segment.length > 0)
      : [],
  };
}

function comparePrerelease(left: string[], right: string[]): number {
  if (left.length === 0 && right.length === 0) return 0;
  if (left.length === 0) return 1;
  if (right.length === 0) return -1;

  const total = Math.max(left.length, right.length);
  for (let index = 0; index < total; index += 1) {
    const leftPart = left[index];
    const rightPart = right[index];
    if (leftPart === undefined) return -1;
    if (rightPart === undefined) return 1;

    const leftNumeric = /^\d+$/.test(leftPart);
    const rightNumeric = /^\d+$/.test(rightPart);

    if (leftNumeric && rightNumeric) {
      const delta = Number.parseInt(leftPart, 10) - Number.parseInt(rightPart, 10);
      if (delta !== 0) return delta > 0 ? 1 : -1;
      continue;
    }

    if (leftNumeric !== rightNumeric) {
      return leftNumeric ? -1 : 1;
    }

    if (leftPart !== rightPart) {
      return leftPart < rightPart ? -1 : 1;
    }
  }

  return 0;
}

export function compareReleaseVersions(left: string, right: string): number {
  const leftVersion = parseVersion(left);
  const rightVersion = parseVersion(right);
  if (!leftVersion || !rightVersion) {
    const normalizedLeft = left.trim().replace(/^v/i, "");
    const normalizedRight = right.trim().replace(/^v/i, "");
    if (normalizedLeft === normalizedRight) {
      return 0;
    }
    return normalizedLeft < normalizedRight ? -1 : 1;
  }

  const total = Math.max(leftVersion.core.length, rightVersion.core.length);
  for (let index = 0; index < total; index += 1) {
    const leftPart = leftVersion.core[index] ?? 0;
    const rightPart = rightVersion.core[index] ?? 0;
    if (leftPart !== rightPart) {
      return leftPart > rightPart ? 1 : -1;
    }
  }

  return comparePrerelease(leftVersion.prerelease, rightVersion.prerelease);
}

function pickLocaleEntry(
  manifest: AppUpdateManifest,
  targetLocale: Locale,
): AppUpdateLocaleEntry | null {
  return (
    manifest.locales[targetLocale]
    ?? manifest.locales.zh
    ?? manifest.locales.en
    ?? Object.values(manifest.locales)[0]
    ?? null
  );
}

function sanitizeChangeGroups(groups: AppUpdateChangeGroup[]): AppUpdateChangeGroup[] {
  return groups
    .map((group) => ({
      title: group.title.trim(),
      items: group.items.map((item) => item.trim()).filter((item) => item.length > 0),
    }))
    .filter((group) => group.title.length > 0 && group.items.length > 0);
}

export function resolveUpdateUrl(url: string, baseUrl = DOCS_BASE_URL): string {
  try {
    return new URL(url, `${baseUrl.replace(/\/$/, "")}/`).toString();
  } catch {
    return `${baseUrl.replace(/\/$/, "")}/overview/latest-version`;
  }
}

export function resolveAppUpdateInfo(
  manifest: AppUpdateManifest,
  currentVersion: string,
  targetLocale: Locale,
  sourceBaseUrl = DOCS_BASE_URL,
  sourceKind: AppUpdateSourceKind = "remote",
): AppUpdateInfo | null {
  if (compareReleaseVersions(currentVersion, manifest.version) >= 0) {
    return null;
  }

  const localeEntry = pickLocaleEntry(manifest, targetLocale);
  if (!localeEntry) {
    return null;
  }

  return {
    currentVersion: currentVersion.trim().replace(/^v/i, ""),
    latestVersion: manifest.version.trim().replace(/^v/i, ""),
    releasedAt: manifest.releasedAt.trim(),
    channel: manifest.channel.trim(),
    title: localeEntry.title.trim(),
    summary: localeEntry.summary.trim(),
    changelogUrl: resolveUpdateUrl(localeEntry.changelogUrl, sourceBaseUrl),
    changes: sanitizeChangeGroups(localeEntry.changes),
    sourceKind,
    sourceBaseUrl,
  };
}

export async function fetchAppUpdateManifest(options?: {
  throwOnError?: boolean;
}): Promise<AppUpdateManifestFetchResult | null> {
  const result = await ipcInvoke<unknown>(
    "fetch_app_update_manifest",
    undefined,
    {
      throwOnError: options?.throwOnError ?? false,
    },
  );

  if (result == null) {
    return null;
  }

  if (isAppUpdateManifestFetchResult(result)) {
    return result;
  }

  if (options?.throwOnError) {
    throw new Error("Invalid app update manifest");
  }

  return null;
}
