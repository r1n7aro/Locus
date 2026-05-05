import { t } from "../i18n";
import { normalizeAppError } from "./errors";

export const CUSTOM_ENDPOINT_HTML_RESPONSE_CODE = "endpoint_test.html_response";

const OPEN_HTML_MARKER_PATTERN = /\[OPEN_HTML:(.+)\]/;
const LEGACY_HTML_RESPONSE_TEXT = "Server returned an HTML page instead of JSON";
const HTTP_PREFIX_PATTERN = /^(HTTP\s+\d+\s+[—-]\s*)/i;

type ErrorLike = {
  code?: string | null;
  message?: string | null;
};

export type CustomEndpointReplyStatus = "success" | "error";

function messageOf(value: string | ErrorLike): string {
  return typeof value === "string" ? value : value.message ?? "";
}

export function customEndpointTestHtmlPath(result: string): string {
  return result.match(OPEN_HTML_MARKER_PATTERN)?.[1]?.trim() ?? "";
}

export function stripCustomEndpointTestHtmlMarker(result: string): string {
  return result.replace(/\s*\[OPEN_HTML:.*\]/, "").trim();
}

export function isCustomEndpointHtmlResponse(value: string | ErrorLike): boolean {
  const message = messageOf(value);
  return (
    (typeof value !== "string" && value.code === CUSTOM_ENDPOINT_HTML_RESPONSE_CODE)
    || OPEN_HTML_MARKER_PATTERN.test(message)
    || message.includes(LEGACY_HTML_RESPONSE_TEXT)
  );
}

export function customEndpointTestDetail(result: string): string {
  if (!isCustomEndpointHtmlResponse(result)) {
    return stripCustomEndpointTestHtmlMarker(result);
  }

  const clean = stripCustomEndpointTestHtmlMarker(result);
  const prefix = clean.match(HTTP_PREFIX_PATTERN)?.[1] ?? "";
  return `${prefix}${t("settings.custom.testHtmlResponse")}`;
}

export function normalizeCustomEndpointTestErrorMessage(error: unknown): string {
  const appError = normalizeAppError(error);
  return appError.message;
}

export function customEndpointTestStatusForReply(reply: string): CustomEndpointReplyStatus {
  return isCustomEndpointHtmlResponse(reply) ? "error" : "success";
}
