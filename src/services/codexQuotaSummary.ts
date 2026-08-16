import {
  codexRateLimits,
  type CodexRateLimitSnapshot,
  type CodexRateLimitWindow,
  type CodexRateLimitsResponse,
} from "./auth";

export interface CodexQuotaSummaryWindow {
  key: "primary" | "secondary";
  limitId: string;
  limitName: string | null;
  remainingPercent: number;
  windowMinutes: number | null;
}

const CACHE_TTL_MS = 60_000;
let cachedAt = 0;
let cachedSummary: CodexQuotaSummaryWindow[] = [];
let pending: Promise<CodexQuotaSummaryWindow[]> | null = null;

function clampPercent(value: unknown, fallback: number): number {
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.max(0, Math.min(100, numeric));
}

function normalizeWindow(
  key: "primary" | "secondary",
  limitId: string,
  limitName: string | null,
  window: CodexRateLimitWindow | null | undefined,
): CodexQuotaSummaryWindow | null {
  if (!window) return null;
  const usedPercent = clampPercent(window.usedPercent, 0);
  const remainingPercent = clampPercent(window.remainingPercent, 100 - usedPercent);
  const windowMinutes = typeof window.windowMinutes === "number"
    && Number.isFinite(window.windowMinutes)
    && window.windowMinutes > 0
    ? window.windowMinutes
    : null;
  return { key, limitId, limitName, remainingPercent, windowMinutes };
}

function appendSnapshot(
  output: CodexQuotaSummaryWindow[],
  snapshot: CodexRateLimitSnapshot,
) {
  const limitId = snapshot.limitId?.trim() || "codex";
  const limitName = snapshot.limitName?.trim() || null;
  const primary = normalizeWindow("primary", limitId, limitName, snapshot.primary);
  const secondary = normalizeWindow("secondary", limitId, limitName, snapshot.secondary);
  if (primary) output.push(primary);
  if (secondary) output.push(secondary);
}

export function buildCodexQuotaSummary(
  response: CodexRateLimitsResponse | null | undefined,
): CodexQuotaSummaryWindow[] {
  if (!response?.rateLimits) return [];
  const output: CodexQuotaSummaryWindow[] = [];
  appendSnapshot(output, response.rateLimits);
  return output;
}

export async function loadCodexQuotaSummary(
  force = false,
): Promise<CodexQuotaSummaryWindow[]> {
  const now = Date.now();
  if (!force && cachedAt > 0 && now - cachedAt < CACHE_TTL_MS) {
    return cachedSummary;
  }
  if (pending) return pending;
  pending = codexRateLimits()
    .then((response) => {
      cachedSummary = buildCodexQuotaSummary(response);
      cachedAt = Date.now();
      return cachedSummary;
    })
    .finally(() => {
      pending = null;
    });
  return pending;
}
