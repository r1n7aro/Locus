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

function shouldTraceEvent(event: string) {
  const mode = queryTraceMode() ?? sessionTraceMode();
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

export function logToolCollapseTrace(
  scope: string,
  event: string,
  detail?: Record<string, unknown>,
) {
  if (!shouldTraceEvent(event)) return;

  const prefix = `[tool-collapse][+${elapsedMs()}ms][${scope}] ${event}`;
  if (!detail || Object.keys(detail).length === 0) {
    console.info(prefix);
    return;
  }
  console.info(prefix, detail);
}
