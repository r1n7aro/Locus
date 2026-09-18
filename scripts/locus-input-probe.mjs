// This function is serialized into the inspected page. Keep it self-contained.
export function installInputProbe(key, durationMs) {
  // The bridge maintains this marker when debug mode changes. The injected
  // startup flag alone can be stale after the user disables debugging.
  const debugEnabled = () => {
    try {
      return localStorage.getItem("locus:webview-bridge:debug-enabled:v1") === "1";
    } catch {
      return false;
    }
  };
  if (!debugEnabled()) return { enabled: false };
  if (window[key]) throw new Error("An input capture already owns this probe key.");
  const describe = (target) => target instanceof Element ? {
    tag: target.tagName,
    // Do not record DOM ids, text, input values, or pressed keys.
    classes: (target.getAttribute("class") ?? "").slice(0, 200),
    role: target.getAttribute("role"),
  } : null;
  const counts = {};
  const events = [];
  const captures = new Map();
  let lastPointer = null;
  let lastMoveAt = 0;
  let stopped = false;
  let timer;
  const types = [
    "pointermove", "pointerdown", "pointerup", "pointercancel",
    "mousedown", "mouseup", "click", "wheel", "keydown",
    "focus", "blur", "dragstart", "dragend", "drop",
    "gotpointercapture", "lostpointercapture",
  ];
  const stop = () => {
    if (stopped) return;
    stopped = true;
    for (const type of types) window.removeEventListener(type, onEvent, true);
    clearInterval(timer);
    captures.clear();
    if (window[key]?.stop === stop) delete window[key];
  };
  const onEvent = (event) => {
    if (!debugEnabled()) { stop(); return; }
    if (!event.isTrusted) return;
    const atMs = Date.now();
    counts[event.type] = (counts[event.type] ?? 0) + 1;
    if (event.type.startsWith("pointer")) {
      lastPointer = { atMs, x: event.clientX, y: event.clientY, buttons: event.buttons };
    }
    if (event.type === "gotpointercapture") captures.set(event.pointerId, event.target);
    if (event.type === "lostpointercapture") captures.delete(event.pointerId);
    if (event.type === "pointermove") {
      if (atMs - lastMoveAt < 500) return;
      lastMoveAt = atMs;
    }
    events.push({ atMs, type: event.type, target: describe(event.target) });
    if (events.length > 64) events.shift();
  };
  const sample = () => {
    if (!debugEnabled()) { stop(); return { enabled: false }; }
    const hit = lastPointer ? document.elementFromPoint(lastPointer.x, lastPointer.y) : null;
    const style = hit ? getComputedStyle(hit) : null;
    return {
      enabled: true, stopped, atMs: Date.now(), performanceNowMs: performance.now(),
      focused: document.hasFocus(), visibility: document.visibilityState,
      viewport: { width: innerWidth, height: innerHeight, scale: devicePixelRatio },
      active: describe(document.activeElement), counts: { ...counts },
      lastPointer, hit: describe(hit), hitCursor: style?.cursor,
      hitPointerEvents: style?.pointerEvents,
      bodyPointerEvents: getComputedStyle(document.body).pointerEvents,
      dialogs: [...document.querySelectorAll("dialog[open],[aria-modal=true],[inert]")].slice(0, 16).map(describe),
      captures: [...captures].slice(0, 16).map(([id, target]) => ({ id, target: describe(target), connected: target.isConnected })),
      events: events.slice(),
    };
  };
  window[key] = { sample, stop };
  for (const type of types) window.addEventListener(type, onEvent, { capture: true, passive: true });
  const deadline = Date.now() + durationMs;
  timer = setInterval(() => {
    if (!debugEnabled() || Date.now() >= deadline) {
      stop();
    }
  }, 250);
  return { enabled: true };
}
