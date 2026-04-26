const STORAGE_KEY = "locus:layoutDiagnostics";
const QUERY_KEYS = ["layoutDiagnostics", "locusLayoutDiagnostics"];
const FLUSH_DELAY_MS = 500;

interface PendingDiagnostic {
  count: number;
  firstAt: number;
  lastAt: number;
  lastDetail: Record<string, unknown>;
}

const pendingDiagnostics = new Map<string, PendingDiagnostic>();
let flushTimer: ReturnType<typeof setTimeout> | null = null;

function nowMs(): number {
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return performance.now();
  }
  return Date.now();
}

function diagnosticsEnabled(): boolean {
  if (typeof window === "undefined") return false;

  try {
    const params = new URLSearchParams(window.location.search);
    if (QUERY_KEYS.some((key) => params.get(key) === "1")) return true;
  } catch {
    // ignore URL parsing failures
  }

  try {
    return localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function flushDiagnostics() {
  flushTimer = null;
  if (pendingDiagnostics.size === 0) return;

  for (const [eventName, entry] of pendingDiagnostics.entries()) {
    console.debug("[Locus layout]", eventName, {
      count: entry.count,
      durationMs: Math.round(entry.lastAt - entry.firstAt),
      ...entry.lastDetail,
    });
  }
  pendingDiagnostics.clear();
}

export function recordLayoutDiagnostic(
  eventName: string,
  detail: Record<string, unknown> = {},
) {
  if (!diagnosticsEnabled()) return;

  const timestamp = nowMs();
  const existing = pendingDiagnostics.get(eventName);
  pendingDiagnostics.set(eventName, {
    count: (existing?.count ?? 0) + 1,
    firstAt: existing?.firstAt ?? timestamp,
    lastAt: timestamp,
    lastDetail: detail,
  });

  if (flushTimer !== null) return;
  flushTimer = setTimeout(flushDiagnostics, FLUSH_DELAY_MS);
}
