import type { Ref, Component } from "vue";
import type { ViewPackageDetail, ViewManifest, ViewFrontendLogEntry } from "../../services/view";
interface AutomationPoint { x: number; y: number; }
let automationElementSeq = 0;
export function createViewAutomation(options: {
  root(): HTMLElement;
  globals(): Record<string, unknown>;
  signal: AbortSignal;
  activeViewId: Readonly<Ref<string>>;
  detail: Readonly<Ref<ViewPackageDetail | null>>;
  manifest: Readonly<Ref<ViewManifest | null>>;
  loading: Readonly<Ref<boolean>>;
  error: Readonly<Ref<string>>;
  latestFrontendLog: Readonly<Ref<ViewFrontendLogEntry | null>>;
  runtimeComponent: Readonly<Ref<Component | null>>;
}) {
const { activeViewId, detail, manifest, loading, error, latestFrontendLog, runtimeComponent } = options;
const document = options.root().ownerDocument;
const window = document.defaultView! as Window & typeof globalThis;
const { MouseEvent, PointerEvent, Event, InputEvent, KeyboardEvent } = window;
const CSS = window.CSS ?? globalThis.CSS;
function isElement(value: unknown): value is Element {
  return !!value && typeof value === "object" && (value as Node).nodeType === 1 && typeof (value as Element).tagName === "string";
}
function hasTag<T extends keyof HTMLElementTagNameMap>(element: Element, tag: T): element is HTMLElementTagNameMap[T] {
  return element.tagName.toLowerCase() === tag;
}
function automationRoot(): HTMLElement {
  return options.root();
}

function automationVisible(element: Element): boolean {
  const style = window.getComputedStyle(element);
  if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
    return false;
  }
  const rect = element.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}

function automationText(value: string, limit = 240): string {
  const normalized = value.replace(/\s+/g, " ").trim();
  return normalized.length > limit ? `${normalized.slice(0, limit)}...` : normalized;
}

function automationElementName(element: Element): string {
  const input = element as HTMLInputElement;
  return automationText(
    element.getAttribute("aria-label")
      || element.getAttribute("title")
      || element.getAttribute("alt")
      || input.placeholder
      || input.value
      || (element as HTMLElement).innerText
      || element.textContent
      || "",
  );
}

function ensureAutomationElementId(element: Element): string {
  const html = element as HTMLElement;
  if (!html.dataset.locusAutomationId) {
    automationElementSeq += 1;
    html.dataset.locusAutomationId = `view-el-${automationElementSeq}`;
  }
  return html.dataset.locusAutomationId;
}

function automationSelector(element: Element): string {
  if (element.id) return `#${CSS.escape(element.id)}`;
  const parts: string[] = [];
  let current: Element | null = element;
  const root = automationRoot();
  while (current && current !== root && current !== document.body) {
    const parent: Element | null = current.parentElement;
    const tag = current.tagName.toLowerCase();
    if (!parent) {
      parts.unshift(tag);
      break;
    }
    const currentTagName = current.tagName;
    const siblings = Array.from(parent.children).filter((item) => item.tagName === currentTagName);
    const index = siblings.indexOf(current) + 1;
    parts.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${index})` : tag);
    current = parent;
  }
  return parts.join(" > ");
}

function elementRole(element: Element): string {
  const explicit = element.getAttribute("role");
  if (explicit) return explicit;
  const tag = element.tagName.toLowerCase();
  if (tag === "button" || tag === "summary") return "button";
  if (tag === "a" && element.hasAttribute("href")) return "link";
  if (tag === "textarea") return "textbox";
  if (tag === "select") return (element as HTMLSelectElement).multiple ? "listbox" : "combobox";
  if (tag === "input") {
    const type = (element as HTMLInputElement).type;
    if (["checkbox", "radio"].includes(type)) return type;
    if (["button", "submit", "reset"].includes(type)) return "button";
    return type === "number" ? "spinbutton" : type === "range" ? "slider" : "textbox";
  }
  return "";
}

function automationElementSnapshot(element: Element) {
  const rect = element.getBoundingClientRect();
  const input = element as HTMLInputElement;
  return {
    id: ensureAutomationElementId(element),
    tag: element.tagName.toLowerCase(),
    role: elementRole(element),
    name: automationElementName(element),
    text: automationText((element as HTMLElement).innerText || element.textContent || ""),
    selector: automationSelector(element),
    rect: {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    },
    visible: automationVisible(element),
    disabled: !!(input as { disabled?: boolean }).disabled || element.getAttribute("aria-disabled") === "true",
    checked: typeof input.checked === "boolean" ? input.checked : undefined,
    value: "value" in input ? input.value : undefined,
  };
}

function collectAutomationElements(payload: Record<string, unknown>) {
  const root = typeof payload.selector === "string" && payload.selector.trim()
    ? automationRoot().querySelector(payload.selector)
    : automationRoot();
  if (!root) throw new Error(`Selector not found: ${String(payload.selector)}`);

  const maxElements = Math.max(1, Math.min(Number(payload.maxElements ?? 120), 500));
  const includeHidden = payload.includeHidden === true;
  const selector = [
    "button",
    "a[href]",
    "input",
    "textarea",
    "select",
    "summary",
    "label",
    "h1",
    "h2",
    "h3",
    "h4",
    "[role]",
    "[tabindex]",
    "[contenteditable='true']",
    "[data-locus-action]",
    "[data-node-id]",
    "[data-canvas-item-id]",
    "[data-locus-status]",
    ".status",
    ".error",
    ".warning",
    ".empty",
  ].join(", ");
  const elements = Array.from(root.querySelectorAll(selector))
    .filter((element) => includeHidden || automationVisible(element))
    .slice(0, maxElements)
    .map(automationElementSnapshot);
  return { root, elements };
}

function automationSnapshot(payload: Record<string, unknown> = {}) {
  const root = automationRoot();
  const { elements } = collectAutomationElements(payload);
  const active = isElement(document.activeElement) && automationRoot().contains(document.activeElement)
    ? automationElementSnapshot(document.activeElement)
    : null;
  const rect = root.getBoundingClientRect();
  return {
    ok: true,
    viewId: activeViewId.value,
    status: {
      loading: loading.value,
      error: error.value,
      manifest: manifest.value
        ? {
            id: manifest.value.id,
            name: manifest.value.name,
            version: manifest.value.version,
          }
        : null,
      latestFrontendLog: latestFrontendLog.value,
    },
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      scrollX: Math.round(window.scrollX),
      scrollY: Math.round(window.scrollY),
    },
    frame: {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    },
    focus: active,
    elements,
  };
}

function targetFromPayload(payload: Record<string, unknown>): Element {
  const target = (payload.target && typeof payload.target === "object"
    ? payload.target
    : payload) as Record<string, unknown>;
  const root = automationRoot();

  const elementId = typeof target.elementId === "string" ? target.elementId.trim() : typeof target.id === "string" ? target.id.trim() : "";
  if (elementId) {
    const found = root.querySelector(`[data-locus-automation-id="${CSS.escape(elementId)}"]`);
    if (found) return found;
    throw new Error(`Element id not found: ${elementId}`);
  }

  const selector = typeof target.selector === "string" ? target.selector.trim() : "";
  if (selector) {
    const found = root.querySelector(selector);
    if (found) return found;
    throw new Error(`Selector not found: ${selector}`);
  }

  const x = Number(target.x);
  const y = Number(target.y);
  if (Number.isFinite(x) && Number.isFinite(y)) {
    const found = document.elementFromPoint(x, y);
    if (found && root.contains(found)) return found;
    throw new Error(`No element inside this UI root at point ${x},${y}`);
  }

  const text = typeof target.text === "string" ? target.text.trim().toLowerCase() : "";
  const name = typeof target.name === "string" ? target.name.trim().toLowerCase() : "";
  const role = typeof target.role === "string" ? target.role.trim().toLowerCase() : "";
  if (text || name || role) {
    const candidates = Array.from(root.querySelectorAll("*"))
      .filter((element) => automationVisible(element));
    const found = candidates.find((element) => {
      const snapshot = automationElementSnapshot(element);
      const candidateName = snapshot.name.toLowerCase();
      const candidateText = snapshot.text.toLowerCase();
      const candidateRole = snapshot.role.toLowerCase();
      return (!text || candidateText.includes(text))
        && (!name || candidateName.includes(name))
        && (!role || candidateRole === role);
    });
    if (found) return found;
  }

  throw new Error("View action target is required.");
}

function automationPointForElement(element: Element): AutomationPoint {
  const rect = element.getBoundingClientRect();
  return {
    x: rect.left + rect.width / 2,
    y: rect.top + rect.height / 2,
  };
}

function automationPointFromLocator(
  locator: Record<string, unknown> | null | undefined,
  fallback: Element,
): AutomationPoint {
  const x = Number(locator?.x);
  const y = Number(locator?.y);
  if (Number.isFinite(x) && Number.isFinite(y)) {
    return { x, y };
  }
  return automationPointForElement(fallback);
}

function dispatchMouseEventAt(
  target: Element | Window,
  type: string,
  point: AutomationPoint,
  button = 0,
  buttons = 0,
) {
  target.dispatchEvent(new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: point.x,
    clientY: point.y,
    button,
    buttons,
  }));
}

function dispatchPointerEventAt(
  target: Element | Window,
  type: string,
  point: AutomationPoint,
  button = 0,
  buttons = 0,
) {
  if (typeof PointerEvent !== "function") return;
  target.dispatchEvent(new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: point.x,
    clientY: point.y,
    button,
    buttons,
    pointerId: 1,
    pointerType: "mouse",
    isPrimary: true,
  }));
}

function dispatchMouseSequence(element: Element, type: "click" | "doubleClick" | "hover") {
  const point = automationPointForElement(element);
  if (type === "hover") {
    dispatchPointerEventAt(element, "pointerover", point);
    dispatchPointerEventAt(element, "pointerenter", point);
    dispatchPointerEventAt(element, "pointermove", point);
    dispatchMouseEventAt(element, "mouseover", point);
    dispatchMouseEventAt(element, "mouseenter", point);
    dispatchMouseEventAt(element, "mousemove", point);
    return;
  }
  dispatchPointerEventAt(element, "pointerdown", point, 0, 1);
  dispatchMouseEventAt(element, "mousedown", point, 0, 1);
  dispatchPointerEventAt(element, "pointerup", point, 0, 0);
  dispatchMouseEventAt(element, "mouseup", point, 0, 0);
  (element as HTMLElement).click?.();
  if (type === "doubleClick") {
    dispatchMouseEventAt(element, "dblclick", point);
  }
}

function interpolateAutomationPoint(from: AutomationPoint, to: AutomationPoint, ratio: number): AutomationPoint {
  return {
    x: from.x + (to.x - from.x) * ratio,
    y: from.y + (to.y - from.y) * ratio,
  };
}

function dispatchDragSequence(
  source: Element,
  destination: Element,
  from: AutomationPoint,
  to: AutomationPoint,
) {
  const distance = Math.hypot(to.x - from.x, to.y - from.y);
  const steps = Math.max(2, Math.min(12, Math.ceil(distance / 80)));

  dispatchPointerEventAt(source, "pointerover", from, 0, 1);
  dispatchMouseEventAt(source, "mouseover", from, 0, 1);
  dispatchPointerEventAt(source, "pointerdown", from, 0, 1);
  dispatchMouseEventAt(source, "mousedown", from, 0, 1);

  for (let step = 1; step <= steps; step += 1) {
    const point = interpolateAutomationPoint(from, to, step / steps);
    const candidate = document.elementFromPoint(point.x, point.y);
    const mouseTarget = candidate && automationRoot().contains(candidate) ? candidate : destination;
    dispatchPointerEventAt(source, "pointermove", point, 0, 1);
    dispatchMouseEventAt(mouseTarget, "mousemove", point, 0, 1);
  }

  dispatchPointerEventAt(source, "pointerup", to, 0, 0);
  dispatchMouseEventAt(destination, "mouseup", to, 0, 0);
}

function setElementValue(element: Element, value: unknown, append = false) {
  const next = String(value ?? "");
  if (hasTag(element, "input") || hasTag(element, "textarea")) {
    element.focus();
    element.value = append ? `${element.value}${next}` : next;
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
    return;
  }
  if (hasTag(element, "select")) {
    element.focus();
    element.value = next;
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
    return;
  }
  if ((element as HTMLElement).isContentEditable) {
    (element as HTMLElement).focus();
    element.textContent = append ? `${element.textContent ?? ""}${next}` : next;
    element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: next }));
    return;
  }
  throw new Error("Target does not accept text input.");
}

function performAutomationAction(payload: Record<string, unknown>) {
  const action = String(payload.action || "").trim();
  if (!action) throw new Error("Action is required.");
  const element = action === "scroll" && !payload.target ? automationRoot() : targetFromPayload(payload);
  const before = automationElementSnapshot(element);

  if (action === "click" || action === "doubleClick" || action === "hover") {
    dispatchMouseSequence(element, action);
  } else if (action === "focus") {
    (element as HTMLElement).focus();
  } else if (action === "type") {
    setElementValue(element, payload.text, true);
  } else if (action === "setValue" || action === "selectOption") {
    setElementValue(element, payload.value);
  } else if (action === "check" || action === "uncheck") {
    if (!hasTag(element, "input") || !["checkbox", "radio"].includes(element.type)) {
      throw new Error("Target is not a checkbox or radio input.");
    }
    element.checked = action === "check";
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
  } else if (action === "press") {
    const key = String(payload.key || payload.text || "").trim();
    if (!key) throw new Error("Key is required for press.");
    (element as HTMLElement).focus();
    element.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    if (key === "Enter" && hasTag(element, "button")) element.click();
    element.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true }));
  } else if (action === "scroll") {
    const deltaX = Number(payload.deltaX ?? 0);
    const deltaY = Number(payload.deltaY ?? 0);
    if (element === automationRoot()) {
      (element as HTMLElement).scrollBy({ left: deltaX, top: deltaY, behavior: "auto" });
    } else {
      (element as HTMLElement).scrollBy({ left: deltaX, top: deltaY, behavior: "auto" });
    }
  } else if (action === "drag") {
    const toPayload = payload.to && typeof payload.to === "object" ? payload.to as Record<string, unknown> : {};
    const fromPayload = payload.target && typeof payload.target === "object"
      ? payload.target as Record<string, unknown>
      : payload;
    const target = targetFromPayload({ target: toPayload });
    dispatchDragSequence(
      element,
      target,
      automationPointFromLocator(fromPayload, element),
      automationPointFromLocator(toPayload, target),
    );
  } else {
    throw new Error(`Unsupported View action: ${action}`);
  }

  return {
    ok: true,
    action,
    before,
    after: automationElementSnapshot(element),
  };
}

function serializeAutomationValue(value: unknown, depth = 0, seen = new WeakSet<object>()): unknown {
  if (value == null || typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return value;
  }
  if (typeof value === "bigint") return value.toString();
  if (typeof value === "function") return `[Function ${(value as Function).name || "anonymous"}]`;
  if (isElement(value)) return automationElementSnapshot(value);
  if (value instanceof Error) return { name: value.name, message: value.message, stack: value.stack };
  if (typeof value !== "object") return String(value);
  if (seen.has(value)) return "[Circular]";
  if (depth > 4) return "[MaxDepth]";
  seen.add(value);
  if (Array.isArray(value)) {
    return value.slice(0, 100).map((item) => serializeAutomationValue(item, depth + 1, seen));
  }
  const out: Record<string, unknown> = {};
  for (const [key, nested] of Object.entries(value as Record<string, unknown>).slice(0, 100)) {
    out[key] = serializeAutomationValue(nested, depth + 1, seen);
  }
  return out;
}

async function evaluateAutomationExpression(payload: Record<string, unknown>) {
  const source = String(payload.expression || "").trim();
  if (!source) throw new Error("Expression is required.");
  const root = automationRoot();
  const args = payload.args;
  let fn: Function;
  try {
    fn = new Function("root", "detail", "args", "window", "globalThis", `"use strict"; return (${source});`);
  } catch {
    fn = new Function("root", "detail", "args", "window", "globalThis", `"use strict"; ${source}`);
  }
  const value = await fn(root, detail.value, args, options.globals().window, options.globals().globalThis);
  return {
    ok: true,
    value: serializeAutomationValue(value),
  };
}

async function waitForAutomationCondition(payload: Record<string, unknown>) {
  const condition = String(payload.condition || "runtimeReady").trim();
  const timeoutMs = Math.max(0, Math.min(Number(payload.timeoutMs ?? 5000), 60000));
  const pollIntervalMs = Math.max(50, Math.min(Number(payload.pollIntervalMs ?? 100), 2000));
  const startedAt = Date.now();
  let lastError = "";

  const check = async () => {
    try {
      if (condition === "runtimeReady") return !!runtimeComponent.value && !loading.value && !error.value;
      if (condition === "selectorVisible") {
        const selector = String(payload.selector || "");
        const element = selector ? automationRoot().querySelector(selector) : null;
        return !!element && automationVisible(element);
      }
      if (condition === "selectorHidden") {
        const selector = String(payload.selector || "");
        const element = selector ? automationRoot().querySelector(selector) : null;
        return !element || !automationVisible(element);
      }
      if (condition === "textPresent") {
        return automationRoot().innerText.includes(String(payload.text || ""));
      }
      if (condition === "textAbsent") {
        return !automationRoot().innerText.includes(String(payload.text || ""));
      }
      if (condition === "noConsoleError") {
        return latestFrontendLog.value?.level !== "error";
      }
      if (condition === "expression") {
        const result = await evaluateAutomationExpression(payload);
        return !!result.value;
      }
      throw new Error(`Unsupported wait condition: ${condition}`);
    } catch (conditionError) {
      lastError = conditionError instanceof Error ? conditionError.message : String(conditionError);
      return false;
    }
  };

  while (!options.signal.aborted && Date.now() - startedAt <= timeoutMs) {
    if (await check()) {
      return {
        ok: true,
        condition,
        elapsedMs: Date.now() - startedAt,
      };
    }
    await new Promise((resolve) => window.setTimeout(resolve, pollIntervalMs));
  }
  throw new Error(lastError || `Wait condition timed out: ${condition}`);
}


return { snapshot: automationSnapshot, async handle(kind: string, payload: Record<string, unknown>) {
  if (kind === "snapshot" || kind === "captureBounds") return automationSnapshot(payload);
  if (kind === "action") return performAutomationAction(payload);
  if (kind === "wait") return waitForAutomationCondition(payload);
  if (kind === "debugEval") return evaluateAutomationExpression(payload);
  throw new Error("Unsupported View automation kind: " + kind);
}};
}
