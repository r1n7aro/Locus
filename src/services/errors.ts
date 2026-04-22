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

export function normalizeAppError(e: unknown): AppErrorPayload {
  if (isAppErrorPayload(e)) return e;

  if (typeof e === "string") {
    return {
      code: "unknown",
      message: e,
      retryable: false,
      severity: "error",
    };
  }

  if (e instanceof Error) {
    return {
      code: "unknown",
      message: e.message,
      detail: e.stack,
      retryable: false,
      severity: "error",
    };
  }

  if (typeof e === "object" && e !== null) {
    const obj = e as Record<string, unknown>;
    if (typeof obj.message === "string") {
      return {
        code: typeof obj.code === "string" ? obj.code : "unknown",
        message: obj.message,
        detail: typeof obj.detail === "string" ? obj.detail : undefined,
        operation: typeof obj.operation === "string" ? obj.operation : undefined,
        retryable: typeof obj.retryable === "boolean" ? obj.retryable : false,
        severity: (typeof obj.severity === "string" && ["error", "warning", "success", "info"].includes(obj.severity))
          ? obj.severity as NotificationLevel
          : "error",
      };
    }
  }

  return {
    code: "unknown",
    message: "An unexpected error occurred",
    detail: JSON.stringify(e),
    retryable: false,
    severity: "error",
  };
}
