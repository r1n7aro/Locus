import { getVersion } from "@tauri-apps/api/app";
import type { AppUpdateChannel } from "../types";
import packageJson from "../../package.json";

export const APP_VERSION_FALLBACK = packageJson.version;
const PACKAGE_JSON_WITH_RELEASE_CHANNEL = packageJson as typeof packageJson & {
  releaseChannel?: string;
};

function normalizeReleaseChannel(value: unknown): AppUpdateChannel {
  return typeof value === "string" && value.trim().toLowerCase() === "experimental"
    ? "experimental"
    : "stable";
}

export const APP_RELEASE_CHANNEL_FALLBACK = normalizeReleaseChannel(
  PACKAGE_JSON_WITH_RELEASE_CHANNEL.releaseChannel,
);

let runtimeVersionPromise: Promise<string> | null = null;

export function getAppRuntimeVersion(): Promise<string> {
  if (!runtimeVersionPromise) {
    runtimeVersionPromise = getVersion()
      .then((version) => version.trim() || APP_VERSION_FALLBACK)
      .catch(() => APP_VERSION_FALLBACK);
  }

  return runtimeVersionPromise;
}

export function getAppRuntimeReleaseChannel(): Promise<AppUpdateChannel> {
  return Promise.resolve(APP_RELEASE_CHANNEL_FALLBACK);
}
