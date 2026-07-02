const traceStartMs =
  typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();

const ENABLED_SESSION_STORAGE_KEY = "locus.toolCollapseTraceEnabled";
const MODE_SESSION_STORAGE_KEY = "locus.toolCollapseTrace";
const MODE_QUERY_KEYS = ["toolCollapseTrace", "locusToolCollapseTrace"];
const ENABLED_QUERY_KEYS = ["toolCollapseTraceEnabled", "locusToolCollapseTraceEnabled"];
const TOOL_COLLAPSE_TRACE_MODES = ["all", "handoff", "waiting"] as const;

type ToolCollapseTraceMode = typeof TOOL_COLLAPSE_TRACE_MODES[number];

const TOOL_COLLAPSE_HANDOFF_EVENTS = new Set([
  "activeToolCallsCleared",
  "activeToolCallsResumedWithHandoff",
  "animateCollapseOnMount",
  "beginToolCallHandoff",
  "clearToolCallHandoff",
  "collapseArmed",
  "expandedChanged",
  "historyToolSegmentPinnedStateChanged",
  "historyToolSegmentExpansionDecision",
  "onTransientToolCallsCollapseFinished",
  "panelAfterLeave",
  "pendingContinuationToolItemIdsChanged",
  "promotableHistoryToolCallsChanged",
  "promotedHistoryToolCallsRenderGap",
  "promotedHistoryToolCallsVisibilityChanged",
  "clearRetainedCollapsedToolCalls",
  "retainCollapsedToolCallHandoff",
  "retainCollapsedToolCallHandoffSkipped",
  "transientPromotedToolCallsCoverage",
  "transientToolCallsCollapseEnabledChanged",
  "waitingLayoutStateChanged",
  "applyStreamMutation",
  "deferUserMessageDuringToolRound",
  "embeddedApplyStreamMutation",
  "embeddedDeferUserMessageDuringToolRound",
  "embeddedFlushDeferredUserMessages",
  "embeddedStreamEventReceived",
  "flushDeferredUserMessages",
  "historyToolBlockOrderChanged",
  "messagesOrderChanged",
  "streamEventMutationBatch",
  "streamEventReceived",
  "transcriptBlockOrderChanged",
  "transientRenderSegmentsChanged",
]);

function normalizeTraceMode(value: string | null | undefined): ToolCollapseTraceMode | null {
  return TOOL_COLLAPSE_TRACE_MODES.includes(value as ToolCollapseTraceMode)
    ? value as ToolCollapseTraceMode
    : null;
}

function queryTraceMode(): ToolCollapseTraceMode | null {
  if (typeof window === "undefined") return null;
  try {
    const params = new URLSearchParams(window.location.search);
    for (const key of MODE_QUERY_KEYS) {
      const mode = normalizeTraceMode(params.get(key));
      if (mode) return mode;
    }
    if (ENABLED_QUERY_KEYS.some((key) => params.get(key) === "1")) {
      return "handoff";
    }
  } catch {
    // ignore URL parsing failures
  }
  return null;
}

function sessionTraceMode(): ToolCollapseTraceMode | null {
  if (typeof sessionStorage === "undefined") return null;
  try {
    if (sessionStorage.getItem(ENABLED_SESSION_STORAGE_KEY) !== "true") return null;
    return normalizeTraceMode(sessionStorage.getItem(MODE_SESSION_STORAGE_KEY));
  } catch {
    return null;
  }
}

// Trace checks sit on hot render paths (per stream delta, per watch tick), so
// the query/sessionStorage lookup is cached for a short window instead of
// re-parsing the URL on every call. Toggling the sessionStorage flag at
// runtime takes effect within TRACE_MODE_CACHE_TTL_MS.
const TRACE_MODE_CACHE_TTL_MS = 1000;
let cachedTraceMode: ToolCollapseTraceMode | null = null;
let cachedTraceModeAt = Number.NEGATIVE_INFINITY;

function resolveTraceMode(): ToolCollapseTraceMode | null {
  const now = Date.now();
  if (now - cachedTraceModeAt >= TRACE_MODE_CACHE_TTL_MS) {
    cachedTraceMode = queryTraceMode() ?? sessionTraceMode();
    cachedTraceModeAt = now;
  }
  return cachedTraceMode;
}

export function resetToolCollapseTraceCacheForTest() {
  cachedTraceModeAt = Number.NEGATIVE_INFINITY;
}

function shouldTraceEvent(event: string) {
  const mode = resolveTraceMode();
  if (mode === "all") return true;
  if (mode === "handoff") return TOOL_COLLAPSE_HANDOFF_EVENTS.has(event);
  if (mode === "waiting") return event === "waitingLayoutStateChanged";
  return false;
}

export function isToolCollapseTraceEnabled(event: string) {
  return shouldTraceEvent(event);
}

function nowMs() {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

function elapsedMs() {
  return Math.round((nowMs() - traceStartMs) * 10) / 10;
}

export function previewTraceText(text: string, maxLength = 80) {
  const compact = text.replace(/\s+/g, " ").trim();
  if (compact.length <= maxLength) return compact;
  return `${compact.slice(0, maxLength - 1)}…`;
}

// Pass a thunk for details that are expensive to build (full-text previews,
// per-message snapshots) so disabled traces cost nothing beyond the mode check.
export function logToolCollapseTrace(
  scope: string,
  event: string,
  detail?: Record<string, unknown> | (() => Record<string, unknown>),
) {
  if (!shouldTraceEvent(event)) return;

  const resolvedDetail = typeof detail === "function" ? detail() : detail;
  const prefix = `[tool-collapse][+${elapsedMs()}ms][${scope}] ${event}`;
  if (!resolvedDetail || Object.keys(resolvedDetail).length === 0) {
    console.info(prefix);
    return;
  }
  console.info(prefix, resolvedDetail);
}
