export const CODEX_DEFAULT_CONTEXT_WINDOW = 272_000;
export const CODEX_LEGACY_EXTENDED_CONTEXT_WINDOW = 372_000;
export const CODEX_MIN_CONTEXT_WINDOW = 16_000;
export const CODEX_MAX_CONTEXT_WINDOW = 1_000_000;
export const CODEX_EFFECTIVE_CONTEXT_WINDOW_PERCENT = 95;

export function normalizeCodexContextWindow(
  value: unknown,
  legacyExtendedContext = false,
): number {
  const fallback = legacyExtendedContext
    ? CODEX_LEGACY_EXTENDED_CONTEXT_WINDOW
    : CODEX_DEFAULT_CONTEXT_WINDOW;
  if (typeof value !== "number" && typeof value !== "string") return fallback;
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) return fallback;
  return Math.min(
    CODEX_MAX_CONTEXT_WINDOW,
    Math.max(CODEX_MIN_CONTEXT_WINDOW, Math.round(numeric)),
  );
}

export function codexEffectiveContextWindow(contextWindow: number): number {
  return Math.floor(
    normalizeCodexContextWindow(contextWindow) * CODEX_EFFECTIVE_CONTEXT_WINDOW_PERCENT / 100,
  );
}
