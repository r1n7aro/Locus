// Normalizes subscription rate-limit responses (Codex, Anthropic) into the
// shapes the provider cards render.
import type {
  AnthropicRateLimitWindow as RemoteAnthropicRateLimitWindow,
  AnthropicRateLimitsResponse,
  CodexRateLimitSnapshot,
  CodexRateLimitWindow as RemoteCodexRateLimitWindow,
  CodexRateLimitsResponse,
} from "../services/auth";

export interface CodexQuotaWindowState {
  id: string;
  limitId: string;
  limitName: string | null;
  windowType: "primary" | "secondary";
  usedPercent: number;
  remainingPercent: number;
  windowMinutes: number | null;
  resetsAt: number | null;
}

export interface CodexQuotaCreditsState {
  unlimited: boolean;
  balance: string | null;
}

export interface CodexQuotaResetCreditState {
  id: string | null;
  title: string | null;
  description: string | null;
  expiresAt: number | null;
}

export interface CodexQuotaState {
  loading: boolean;
  loaded: boolean;
  error: string | null;
  fetchedAtMs: number | null;
  windows: CodexQuotaWindowState[];
  credits: CodexQuotaCreditsState | null;
  resetCreditsAvailable: number | null;
  resetCredits: CodexQuotaResetCreditState[];
  planType: string | null;
}

export interface AnthropicQuotaWindowState {
  id: string;
  limitId: string;
  limitName: string | null;
  usedPercent: number;
  remainingPercent: number;
  windowMinutes: number | null;
  resetsAt: number | null;
}

export interface AnthropicQuotaState {
  loading: boolean;
  loaded: boolean;
  error: string | null;
  fetchedAtMs: number | null;
  windows: AnthropicQuotaWindowState[];
}

export function emptyCodexQuota(): CodexQuotaState {
  return {
    loading: false,
    loaded: false,
    error: null,
    fetchedAtMs: null,
    windows: [],
    credits: null,
    resetCreditsAvailable: null,
    resetCredits: [],
    planType: null,
  };
}

export function emptyAnthropicQuota(): AnthropicQuotaState {
  return {
    loading: false,
    loaded: false,
    error: null,
    fetchedAtMs: null,
    windows: [],
  };
}

function clampPercent(value: unknown, fallback: number): number {
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.max(0, Math.min(100, numeric));
}

function normalizeRateLimitWindow(
  window: RemoteCodexRateLimitWindow | RemoteAnthropicRateLimitWindow | null | undefined,
  fallbackRemaining: number,
): Pick<CodexQuotaWindowState, "usedPercent" | "remainingPercent" | "windowMinutes" | "resetsAt"> | null {
  if (!window) return null;
  const usedPercent = clampPercent(window.usedPercent, 100 - fallbackRemaining);
  const remainingPercent = clampPercent(window.remainingPercent, 100 - usedPercent);
  const windowMinutes = typeof window.windowMinutes === "number" && Number.isFinite(window.windowMinutes)
    ? window.windowMinutes
    : null;
  const resetsAt = typeof window.resetsAt === "number" && Number.isFinite(window.resetsAt)
    ? window.resetsAt
    : null;

  return { usedPercent, remainingPercent, windowMinutes, resetsAt };
}

function appendQuotaWindows(
  result: CodexQuotaWindowState[],
  limitId: string,
  snapshot: CodexRateLimitSnapshot,
) {
  const limitName = snapshot.limitName ?? null;
  const primary = normalizeRateLimitWindow(snapshot.primary, 100);
  if (primary) {
    result.push({
      id: `${limitId}:primary`,
      limitId,
      limitName,
      windowType: "primary",
      ...primary,
    });
  }

  const secondary = normalizeRateLimitWindow(snapshot.secondary, 100);
  if (secondary) {
    result.push({
      id: `${limitId}:secondary`,
      limitId,
      limitName,
      windowType: "secondary",
      ...secondary,
    });
  }
}

export function normalizeCodexQuota(response?: CodexRateLimitsResponse | null): CodexQuotaState {
  if (!response?.rateLimits) return emptyCodexQuota();

  const windows: CodexQuotaWindowState[] = [];
  const primaryLimitId = response.rateLimits.limitId ?? "codex";
  appendQuotaWindows(windows, primaryLimitId, response.rateLimits);

  const credits = response.rateLimits.credits?.hasCredits
    ? {
        unlimited: !!response.rateLimits.credits.unlimited,
        balance: response.rateLimits.credits.balance ?? null,
      }
    : null;
  const resetCreditsAvailable = typeof response.rateLimitResetCredits?.availableCount === "number"
    && Number.isFinite(response.rateLimitResetCredits.availableCount)
    ? Math.max(0, Math.trunc(response.rateLimitResetCredits.availableCount))
    : null;
  const resetCreditLimit = resetCreditsAvailable ?? 0;
  const resetCredits = (response.rateLimitResetCredits?.credits ?? [])
    .filter((credit) => credit.status.trim().toLowerCase() === "available")
    .map((credit): CodexQuotaResetCreditState | null => {
      const id = credit.id.trim();
      if (!id) return null;
      const expiresAt = typeof credit.expiresAt === "number"
        && Number.isFinite(credit.expiresAt)
        && credit.expiresAt > 0
        ? credit.expiresAt
        : null;
      const title = credit.title?.trim() || null;
      const description = credit.description?.trim() || null;
      return { id, title, description, expiresAt };
    })
    .filter((credit): credit is CodexQuotaResetCreditState => credit !== null)
    .sort((left, right) => (left.expiresAt ?? Number.MAX_SAFE_INTEGER) - (right.expiresAt ?? Number.MAX_SAFE_INTEGER))
    .slice(0, resetCreditLimit);
  if (resetCreditLimit > 0 && resetCredits.length === 0) {
    resetCredits.push({
      id: null,
      title: null,
      description: null,
      expiresAt: null,
    });
  }

  return {
    loading: false,
    loaded: true,
    error: null,
    fetchedAtMs: response.fetchedAtMs,
    windows,
    credits,
    resetCreditsAvailable,
    resetCredits,
    planType: response.rateLimits.planType ?? null,
  };
}

export function normalizeAnthropicQuota(response?: AnthropicRateLimitsResponse | null): AnthropicQuotaState {
  if (!response) return emptyAnthropicQuota();

  const windows = (response.windows ?? [])
    .map((window): AnthropicQuotaWindowState | null => {
      const normalized = normalizeRateLimitWindow(window, 100);
      if (!normalized) return null;
      return {
        id: window.limitId,
        limitId: window.limitId,
        limitName: window.limitName ?? null,
        ...normalized,
      };
    })
    .filter((window): window is AnthropicQuotaWindowState => !!window);

  return {
    loading: false,
    loaded: true,
    error: null,
    fetchedAtMs: response.fetchedAtMs,
    windows,
  };
}
