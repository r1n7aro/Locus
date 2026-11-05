import { t } from "../i18n";
import { normalizeAppError } from "./errors";

/** Backend `blender.*` error codes → localized summaries. */
const SUMMARY_KEYS: Record<string, string> = {
  "blender.disabled": "blender.error.disabled",
  "blender.launch_failed": "blender.error.launchFailed",
  "blender.install_failed": "blender.error.installFailed",
  "blender.uninstall_failed": "blender.error.uninstallFailed",
  "blender.unknown_installation": "blender.error.unknownInstallation",
  "blender.reload_failed": "blender.error.reloadFailed",
  "blender.service_failed": "blender.error.serviceFailed",
  "blender.not_running": "blender.error.notRunning",
  "blender.invalid_target": "blender.error.invalidTarget",
  "blender.version_failed": "blender.error.versionFailed",
  "blender.status_failed": "blender.error.statusFailed",
  "blender.remove_installation_failed": "blender.error.removeFailed",
  "blender.unhide_installation_failed": "blender.error.unhideFailed",
  "blender.default_installation_failed": "blender.error.defaultFailed",
  "blender.refresh_failed": "blender.error.refreshFailed",
};

/** Manual add refusals carry a complete explanation of their own. */
const ADD_ERROR_KINDS = new Set(["not_found", "not_blender", "duplicate", "too_old", "probe_failed"]);

/**
 * Text for a failed Blender command: a localized summary plus the backend's
 * detail (the concrete cause), e.g. "Blender 启动失败：Blender exited before
 * it connected".
 */
export function blenderErrorText(cause: unknown): string {
  const payload = normalizeAppError(cause);
  if (payload.code.startsWith("blender.add.")) {
    const kind = payload.code.slice("blender.add.".length);
    if (ADD_ERROR_KINDS.has(kind)) return t(`settings.blender.addError.${kind}`, payload.detail ?? "");
  }
  const key = SUMMARY_KEYS[payload.code];
  const summary = key ? t(key) : payload.message;
  const detail = payload.detail?.trim();
  return detail && detail !== summary ? t("blender.error.withDetail", summary, detail) : summary;
}
