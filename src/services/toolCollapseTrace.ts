const traceStartMs =
  typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();

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

function shouldTraceEvent(event: string) {
  if (typeof localStorage === "undefined") return false;

  const enabled = localStorage.getItem("locus.toolCollapseTraceEnabled") === "true";
  if (!enabled) return false;

  const mode = localStorage.getItem("locus.toolCollapseTrace");
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
