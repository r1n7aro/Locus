import type { Locale } from "../i18n";
import type {
  AppUpdateChangeGroup,
  AppUpdateDownloadChannel,
  AppUpdateChannel,
  AppUpdateInfo,
  AppUpdateInstallerDownload,
  AppUpdateLocaleEntry,
  AppUpdateManifest,
  AppUpdateManifestFetchResult,
  AppUpdateSourceKind,
} from "../types";
import { ipcInvoke } from "./ipc";

const DOCS_BASE_URL = "https://unity.farlocus.com";
const GITHUB_RELEASES_URL = "https://github.com/r1n7aro/Locus/releases";
const STABLE_UPDATE_CHANNEL: AppUpdateChannel = "stable";
const EXPERIMENTAL_UPDATE_CHANNEL: AppUpdateChannel = "experimental";
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

function isAppUpdateInstallerDownload(value: unknown): value is AppUpdateInstallerDownload {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    isNonEmptyString(record.id)
    && isNonEmptyString(record.label)
    && isNonEmptyString(record.url)
    && isNonEmptyString(record.platform)
    && isNonEmptyString(record.arch)
    && typeof record.includesManagedPython === "boolean"
    && typeof record.includesManagedGit === "boolean"
    && typeof record.requiresSystemPython === "boolean"
    && typeof record.requiresSystemGit === "boolean"
  );
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
    || (
      record.installers !== undefined
      && (
        !Array.isArray(record.installers)
        || !record.installers.every(isAppUpdateInstallerDownload)
      )
    )
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

export function normalizeAppUpdateChannel(value: unknown): AppUpdateChannel {
  const normalized = typeof value === "string" ? value.trim().toLowerCase() : value;
  return normalized === EXPERIMENTAL_UPDATE_CHANNEL
    ? EXPERIMENTAL_UPDATE_CHANNEL
    : STABLE_UPDATE_CHANNEL;
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

export function resolveAppReleaseChannel(_version: string, explicitChannel?: string | null): AppUpdateChannel {
  const normalizedChannel = explicitChannel?.trim().toLowerCase();
  if (normalizedChannel === STABLE_UPDATE_CHANNEL || normalizedChannel === EXPERIMENTAL_UPDATE_CHANNEL) {
    return normalizedChannel;
  }
  return STABLE_UPDATE_CHANNEL;
}

export function isExperimentalAppUpdateChannel(channel: string | null | undefined): boolean {
  return normalizeAppUpdateChannel(channel) === EXPERIMENTAL_UPDATE_CHANNEL;
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

function sanitizeInstallers(
  installers: AppUpdateInstallerDownload[] | undefined,
  sourceBaseUrl: string,
): AppUpdateInstallerDownload[] {
  return (installers ?? [])
    .map((installer) => ({
      ...installer,
      id: installer.id.trim(),
      label: installer.label.trim(),
      url: resolveUpdateUrl(installer.url.trim(), sourceBaseUrl),
      platform: installer.platform.trim().toLowerCase(),
      arch: installer.arch.trim().toLowerCase(),
    }))
    .filter((installer) =>
      installer.id.length > 0
      && installer.label.length > 0
      && installer.url.length > 0
      && installer.platform.length > 0
      && installer.arch.length > 0,
    );
}

export function resolveUpdateUrl(url: string, baseUrl = DOCS_BASE_URL): string {
  try {
    return new URL(url, `${baseUrl.replace(/\/$/, "")}/`).toString();
  } catch {
    return `${baseUrl.replace(/\/$/, "")}/overview/latest-version`;
  }
}

function selectInstaller(
  installers: AppUpdateInstallerDownload[],
): AppUpdateInstallerDownload | null {
  if (installers.length === 0) {
    return null;
  }

  const platformInstallers = installers.filter((installer) => installer.platform === "windows");
  const candidates = platformInstallers.length > 0 ? platformInstallers : installers;
  return candidates.find((installer) =>
    !installer.requiresSystemPython && !installer.requiresSystemGit,
  ) ?? candidates[0] ?? null;
}

function githubReleaseUrlFromUrl(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (parsed.hostname.toLowerCase() !== "github.com") {
      return null;
    }

    const parts = parsed.pathname.split("/").filter(Boolean);
    if (parts.length < 3 || parts[2] !== "releases") {
      return null;
    }

    if (parts[3] === "download" && parts[4]) {
      return `${parsed.origin}/${parts[0]}/${parts[1]}/releases/tag/${parts[4]}`;
    }

    if (parts[3] === "tag" && parts[4]) {
      return `${parsed.origin}/${parts[0]}/${parts[1]}/releases/tag/${parts[4]}`;
    }

    return `${parsed.origin}/${parts[0]}/${parts[1]}/releases`;
  } catch {
    return null;
  }
}

function resolveGitHubReleaseUrl(
  localeEntry: AppUpdateLocaleEntry,
  installers: AppUpdateInstallerDownload[],
  sourceBaseUrl: string,
): string {
  for (const installer of installers) {
    const releaseUrl = githubReleaseUrlFromUrl(installer.url);
    if (releaseUrl) {
      return releaseUrl;
    }
  }

  for (const channel of localeEntry.downloadChannels ?? []) {
    const releaseUrl = githubReleaseUrlFromUrl(resolveUpdateUrl(channel.url.trim(), sourceBaseUrl));
    if (releaseUrl) {
      return releaseUrl;
    }
  }

  return GITHUB_RELEASES_URL;
}

export function resolveAppUpdateInfo(
  manifest: AppUpdateManifest,
  currentVersion: string,
  targetLocale: Locale,
  sourceBaseUrl = DOCS_BASE_URL,
  sourceKind: AppUpdateSourceKind = "remote",
  currentReleaseChannel: AppUpdateChannel = STABLE_UPDATE_CHANNEL,
): AppUpdateInfo | null {
  if (compareReleaseVersions(currentVersion, manifest.version) >= 0) {
    return null;
  }

  const localeEntry = pickLocaleEntry(manifest, targetLocale);
  if (!localeEntry) {
    return null;
  }

  const installers = sanitizeInstallers(manifest.installers, sourceBaseUrl);
  const installer = selectInstaller(installers);
  const changelogUrl = resolveUpdateUrl(localeEntry.changelogUrl, sourceBaseUrl);
  const releaseUrl = resolveGitHubReleaseUrl(localeEntry, installers, sourceBaseUrl);
  const currentChannel = normalizeAppUpdateChannel(currentReleaseChannel);
  const latestChannel = resolveAppReleaseChannel(manifest.version, manifest.channel);

  return {
    currentVersion: currentVersion.trim().replace(/^v/i, ""),
    latestVersion: manifest.version.trim().replace(/^v/i, ""),
    releasedAt: manifest.releasedAt.trim(),
    channel: latestChannel,
    currentChannel,
    latestChannel,
    currentIsExperimental: currentChannel === EXPERIMENTAL_UPDATE_CHANNEL,
    latestIsExperimental: latestChannel === EXPERIMENTAL_UPDATE_CHANNEL,
    title: localeEntry.title.trim(),
    summary: localeEntry.summary.trim(),
    changelogUrl,
    releaseUrl,
    downloadUrl: installer?.url ?? changelogUrl,
    downloadLabel: installer?.label ?? localeEntry.title.trim(),
    changes: sanitizeChangeGroups(localeEntry.changes),
    installer,
    sourceKind,
    sourceBaseUrl,
  };
}

export async function fetchAppUpdateManifest(options?: {
  throwOnError?: boolean;
  channel?: AppUpdateChannel;
}): Promise<AppUpdateManifestFetchResult | null> {
  const result = await ipcInvoke<unknown>(
    "fetch_app_update_manifest",
    { channel: normalizeAppUpdateChannel(options?.channel) },
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
