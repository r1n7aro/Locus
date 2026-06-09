import { t } from "../i18n";
import type { AppErrorPayload, NotificationLevel } from "../types";

export function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.code === "string" &&
    typeof v.message === "string" &&
    typeof v.retryable === "boolean"
  );
}

const PLUGIN_MANAGED_VIEW_ERROR_PATTERN =
  /^View '(.+)' is managed by plugin '(.+)'\. Uninstall the plugin to (remove|rename|move) it\.$/;

function localizeErrorPayload(payload: AppErrorPayload): AppErrorPayload {
  const pluginManagedViewMatch = payload.message.match(PLUGIN_MANAGED_VIEW_ERROR_PATTERN);
  if (pluginManagedViewMatch) {
    const [, viewName, pluginId, action] = pluginManagedViewMatch;
    return {
      ...payload,
      message: t(`view.error.pluginManaged.${action}`, viewName, pluginId),
    };
  }

  return payload;
}

export function normalizeAppError(e: unknown): AppErrorPayload {
  if (isAppErrorPayload(e)) return localizeErrorPayload(e);

  if (typeof e === "string") {
    return localizeErrorPayload({
      code: "unknown",
      message: e,
      retryable: false,
      severity: "error",
    });
  }

  if (e instanceof Error) {
    return localizeErrorPayload({
      code: "unknown",
      message: e.message,
      detail: e.stack,
      retryable: false,
      severity: "error",
    });
  }

  if (typeof e === "object" && e !== null) {
    const obj = e as Record<string, unknown>;
    if (typeof obj.message === "string") {
      return localizeErrorPayload({
        code: typeof obj.code === "string" ? obj.code : "unknown",
        message: obj.message,
        detail: typeof obj.detail === "string" ? obj.detail : undefined,
        operation: typeof obj.operation === "string" ? obj.operation : undefined,
        retryable: typeof obj.retryable === "boolean" ? obj.retryable : false,
        severity: (typeof obj.severity === "string" && ["error", "warning", "success", "info"].includes(obj.severity))
          ? obj.severity as NotificationLevel
          : "error",
      });
    }
  }

  return localizeErrorPayload({
    code: "unknown",
    message: "An unexpected error occurred",
    detail: JSON.stringify(e),
    retryable: false,
    severity: "error",
  });
}

const UNITY_CONNECTION_ERROR_PATTERNS = [
  /failed to connect to unity editor/i,
  /unity editor not connected/i,
  /unity pipe disconnected/i,
  /unity pipe connection is closing/i,
  /unity pipe write timed out/i,
  /unity response timed out/i,
  /unity response failed/i,
  /pipe write failed/i,
  /newline write failed/i,
  /pipe flush failed/i,
  /unity bridge is only supported on windows/i,
];

export function isUnityConnectionError(e: unknown): boolean {
  const err = normalizeAppError(e);
  const code = err.code.trim().toLowerCase();
  if (
    code === "unity.connection_required" ||
    code === "unity.connection_required_named" ||
    code === "unity.not_connected" ||
    code === "unity.disconnected"
  ) {
    return true;
  }

  const text = [err.message, err.detail, err.operation]
    .filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    .join("\n");
  return UNITY_CONNECTION_ERROR_PATTERNS.some((pattern) => pattern.test(text));
}
