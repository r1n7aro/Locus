import { getVersion } from "@tauri-apps/api/app";
import packageJson from "../../package.json";

export const APP_VERSION_FALLBACK = packageJson.version;

let runtimeVersionPromise: Promise<string> | null = null;

export function getAppRuntimeVersion(): Promise<string> {
  if (!runtimeVersionPromise) {
    runtimeVersionPromise = getVersion()
      .then((version) => version.trim() || APP_VERSION_FALLBACK)
      .catch(() => APP_VERSION_FALLBACK);
  }

  return runtimeVersionPromise;
}
