const traceStartMs =
  typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();

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
  const prefix = `[tool-collapse][+${elapsedMs()}ms][${scope}] ${event}`;
  if (!detail || Object.keys(detail).length === 0) {
    console.info(prefix);
    return;
  }
  console.info(prefix, detail);
}
