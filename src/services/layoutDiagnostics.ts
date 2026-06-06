const SESSION_STORAGE_KEY = "locus:layoutDiagnostics";
const QUERY_KEYS = ["layoutDiagnostics", "locusLayoutDiagnostics"];
const FLUSH_DELAY_MS = 500;
const TOOL_LAYOUT_SELECTOR = [
  "[data-tool-layout-kind='group']",
  "[data-tool-layout-kind='collection']",
  "[data-tool-layout-kind='block']",
].join(",");
const TRANSCRIPT_MESSAGE_SELECTOR = ".chat-transcript-message";
const TRANSCRIPT_SCROLL_ANCHOR_SELECTOR = "[data-scroll-anchor-id]";
const TRANSCRIPT_RENDER_PART_SELECTOR = "[data-render-part-kind]";
const TRANSCRIPT_ACTIVE_THINKING_SELECTOR = "[data-render-part-kind='thinking'][data-render-part-scope='transient']";
const TRANSCRIPT_STANDALONE_WAITING_SELECTOR = "[data-render-part-kind='waiting'][data-render-part-scope='transient']";
const TRANSCRIPT_TOOL_WAITING_STATUS_SELECTOR = ".tool-call-collection-waiting-status";
const TRANSCRIPT_PAINT_CANDIDATE_SELECTOR = [
  TRANSCRIPT_RENDER_PART_SELECTOR,
  TRANSCRIPT_MESSAGE_SELECTOR,
  ".chat-transcript-item-stack",
  ".chat-transcript-message-content",
  ".tool-call-collection",
  ".tool-call-collection-waiting-status",
  ".tool-call-collection-panel",
  ".tool-call-collection-list",
  ".tool-call-batch-summary",
  ".tool-call-block",
  ".tool-call-header",
  ".chat-waiting-indicator",
].join(",");
const TOOL_LAYOUT_SHIFT_THRESHOLD_PX = 1.5;
const TRANSCRIPT_LAYOUT_SHIFT_THRESHOLD_PX = 0.5;
const TEXT_RENDER_PART_KIND = "text";
const TEXT_PREVIEW_LIMIT = 120;
const TOOL_LAYOUT_CHILD_SELECTORS = {
  summary: ".tool-call-batch-summary",
  panel: ".tool-call-collection-panel",
  list: ".tool-call-collection-list",
  waitingRow: ".chat-transcript-tool-waiting-row",
} as const;

interface PendingDiagnostic {
  count: number;
  firstAt: number;
  lastAt: number;
  lastDetail: Record<string, unknown>;
}

export interface ToolLayoutRect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface ToolLayoutStyle {
  display: string;
  visibility: string;
  opacity: string;
  position: string;
  overflow: string;
  height: string;
  maxHeight: string;
  marginTop: string;
  marginBottom: string;
  paddingTop: string;
  paddingBottom: string;
  gap: string;
  rowGap: string;
  transform: string;
}

interface PaintStyleSample {
  display: string;
  visibility: string;
  opacity: string;
  position: string;
  zIndex: string;
  pointerEvents: string;
  overflow: string;
  contain: string;
  isolation: string;
  contentVisibility: string;
  transform: string;
  filter: string;
  clipPath: string;
}

interface PaintElementSample {
  tag: string;
  id: string;
  className: string;
  renderPartKind: string;
  renderPartScope: string;
  renderPartKey: string;
  toolLayoutKind: string;
  toolLayoutScope: string;
  toolLayoutKey: string;
  toolLayoutWaitingStatus: string;
  textPreview: string;
  rect: ToolLayoutRect;
  style: PaintStyleSample;
}

interface PaintHitSample {
  name: string;
  x: number;
  y: number;
  insideTarget: boolean;
  insideTargetWhenTargetHitTestable: boolean;
  top: PaintElementSample | null;
  hitTestableTop: PaintElementSample | null;
  stack: PaintElementSample[];
  hitTestableStack: PaintElementSample[];
}

interface PaintTargetCandidate {
  targetType: "active-thinking" | "standalone-waiting" | "tool-waiting-status";
  element: HTMLElement;
}

interface PaintTargetReport {
  targetType: PaintTargetCandidate["targetType"];
  target: PaintElementSample;
  targetAncestors: PaintElementSample[];
  hitTests: PaintHitSample[];
  occludedHitTests: PaintHitSample[];
  intersectingCandidates: Array<PaintElementSample & { overlapArea: number; containsTargetCenter: boolean }>;
}

export interface ToolLayoutChildSample {
  exists: boolean;
  rect: ToolLayoutRect | null;
  className: string;
  style: ToolLayoutStyle | null;
}

export interface ToolLayoutChildDelta {
  existsChanged: boolean;
  classChanged: boolean;
  styleChanged: string[];
  delta: {
    top: number;
    height: number;
    width: number;
  };
  before: ToolLayoutChildSample;
  after: ToolLayoutChildSample;
}

export interface ToolBlockLayoutSample {
  key: string;
  kind: string;
  scope: string;
  rawKey: string;
  toolCallIds: string[];
  statuses: string;
  state: Record<string, string>;
  rect: ToolLayoutRect;
  offsetTop: number;
  offsetLeft: number;
  contentTop: number;
  className: string;
  ancestorClassChain: string[];
  style: ToolLayoutStyle;
  children: Record<keyof typeof TOOL_LAYOUT_CHILD_SELECTORS, ToolLayoutChildSample>;
}

export interface ToolBlockLayoutSnapshot {
  scope: string;
  reason: string;
  at: number;
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  contentHeight: number;
  samples: ToolBlockLayoutSample[];
}

export interface TranscriptElementSample {
  key: string;
  kind: string;
  scrollAnchorId: string;
  messageAnchorId: string;
  renderPartKind: string;
  renderPartKey: string;
  renderPartScope: string;
  textLength: number;
  textPreview: string;
  textSignature: string;
  firstChildTag: string;
  lastChildTag: string;
  className: string;
  rect: ToolLayoutRect;
  offsetTop: number;
  contentTop: number;
  style: ToolLayoutStyle;
}

export interface TranscriptLayoutSnapshot {
  scope: string;
  reason: string;
  at: number;
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  contentHeight: number;
  content: TranscriptElementSample | null;
  currentAnchor: TranscriptElementSample | null;
  messages: TranscriptElementSample[];
  anchors: TranscriptElementSample[];
  renderParts: TranscriptElementSample[];
}

export interface TranscriptElementShift {
  key: string;
  kind: string;
  scrollAnchorId: string;
  classChanged: boolean;
  styleChanged: string[];
  delta: {
    viewportTop: number;
    offsetTop: number;
    contentTop: number;
    height: number;
    width: number;
  };
  before: TranscriptElementSample;
  after: TranscriptElementSample;
}

export interface TranscriptElementPresenceChange {
  key: string;
  kind: string;
  scrollAnchorId: string;
  renderPartKind: string;
  renderPartKey: string;
  renderPartScope: string;
  textLength: number;
  textPreview: string;
  textSignature: string;
  sample: TranscriptElementSample;
}

export interface TranscriptRenderPartMove {
  textSignature: string;
  textLength: number;
  textPreview: string;
  delta: {
    viewportTop: number;
    offsetTop: number;
    contentTop: number;
    height: number;
    width: number;
  };
  from: TranscriptElementSample;
  to: TranscriptElementSample;
}

export interface TranscriptLayoutComparison {
  delta: {
    scrollTop: number;
    scrollHeight: number;
    contentHeight: number;
  };
  contentShift: TranscriptElementShift | null;
  currentAnchorShift: TranscriptElementShift | null;
  messageShifts: TranscriptElementShift[];
  anchorShifts: TranscriptElementShift[];
  renderPartShifts: TranscriptElementShift[];
  addedRenderParts: TranscriptElementPresenceChange[];
  removedRenderParts: TranscriptElementPresenceChange[];
  textPartMoves: TranscriptRenderPartMove[];
}

export interface ToolBlockLayoutShift {
  key: string;
  kind: string;
  toolCallIds: string[];
  primaryCause: string;
  causes: string[];
  delta: {
    viewportTop: number;
    offsetTop: number;
    contentTop: number;
    height: number;
    width: number;
    scrollTop: number;
    scrollHeight: number;
    contentHeight: number;
  };
  visualShiftOnly: boolean;
  childDeltas: Partial<Record<keyof typeof TOOL_LAYOUT_CHILD_SELECTORS, ToolLayoutChildDelta>>;
  before: ToolBlockLayoutSample;
  after: ToolBlockLayoutSample;
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
    return sessionStorage.getItem(SESSION_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

export function isLayoutDiagnosticsEnabled(): boolean {
  return diagnosticsEnabled();
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

function requestFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

function roundPx(value: number): number {
  return Math.round(value * 10) / 10;
}

function splitCsv(value: string): string[] {
  return value
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

function readRect(rect: DOMRect): ToolLayoutRect {
  return {
    top: roundPx(rect.top),
    left: roundPx(rect.left),
    width: roundPx(rect.width),
    height: roundPx(rect.height),
  };
}

function emptyStyle(): ToolLayoutStyle {
  return {
    display: "",
    visibility: "",
    opacity: "",
    position: "",
    overflow: "",
    height: "",
    maxHeight: "",
    marginTop: "",
    marginBottom: "",
    paddingTop: "",
    paddingBottom: "",
    gap: "",
    rowGap: "",
    transform: "",
  };
}

function readStyle(element: HTMLElement): ToolLayoutStyle {
  if (typeof window === "undefined" || typeof window.getComputedStyle !== "function") {
    return emptyStyle();
  }

  const style = window.getComputedStyle(element);
  return {
    display: style.display,
    visibility: style.visibility,
    opacity: style.opacity,
    position: style.position,
    overflow: style.overflow,
    height: style.height,
    maxHeight: style.maxHeight,
    marginTop: style.marginTop,
    marginBottom: style.marginBottom,
    paddingTop: style.paddingTop,
    paddingBottom: style.paddingBottom,
    gap: style.gap,
    rowGap: style.rowGap,
    transform: style.transform,
  };
}

function emptyPaintStyle(): PaintStyleSample {
  return {
    display: "",
    visibility: "",
    opacity: "",
    position: "",
    zIndex: "",
    pointerEvents: "",
    overflow: "",
    contain: "",
    isolation: "",
    contentVisibility: "",
    transform: "",
    filter: "",
    clipPath: "",
  };
}

function readPaintStyle(element: Element): PaintStyleSample {
  if (typeof window === "undefined" || typeof window.getComputedStyle !== "function") {
    return emptyPaintStyle();
  }

  const style = window.getComputedStyle(element);
  return {
    display: style.display,
    visibility: style.visibility,
    opacity: style.opacity,
    position: style.position,
    zIndex: style.zIndex,
    pointerEvents: style.pointerEvents,
    overflow: style.overflow,
    contain: style.contain,
    isolation: style.isolation,
    contentVisibility: style.contentVisibility,
    transform: style.transform,
    filter: style.filter,
    clipPath: style.clipPath,
  };
}

function elementClassName(element: Element): string {
  const className = (element as HTMLElement).className;
  return typeof className === "string" ? className : "";
}

function readPaintElementSample(element: Element): PaintElementSample {
  const htmlElement = element as HTMLElement;
  const dataset = htmlElement.dataset ?? {};
  return {
    tag: element.tagName.toLowerCase(),
    id: htmlElement.id ?? "",
    className: elementClassName(element),
    renderPartKind: dataset.renderPartKind ?? "",
    renderPartScope: dataset.renderPartScope ?? "",
    renderPartKey: dataset.renderPartKey ?? "",
    toolLayoutKind: dataset.toolLayoutKind ?? "",
    toolLayoutScope: dataset.toolLayoutScope ?? "",
    toolLayoutKey: dataset.toolLayoutKey ?? "",
    toolLayoutWaitingStatus: dataset.toolLayoutWaitingStatus ?? "",
    textPreview: normalizedElementText(htmlElement).slice(0, 80),
    rect: readRect(element.getBoundingClientRect()),
    style: readPaintStyle(element),
  };
}

function readPaintAncestorChain(element: HTMLElement, stopAt: HTMLElement): PaintElementSample[] {
  const chain: PaintElementSample[] = [];
  let current: HTMLElement | null = element;
  while (current && chain.length < 10) {
    chain.push(readPaintElementSample(current));
    if (current === stopAt) break;
    current = current.parentElement;
  }
  return chain;
}

function rectsIntersect(first: DOMRect, second: DOMRect): boolean {
  return first.left < second.right
    && first.right > second.left
    && first.top < second.bottom
    && first.bottom > second.top;
}

function expandedRect(rect: DOMRect, amount: number): DOMRect {
  return DOMRect.fromRect({
    x: rect.left - amount,
    y: rect.top - amount,
    width: rect.width + amount * 2,
    height: rect.height + amount * 2,
  });
}

function overlapArea(first: DOMRect, second: DOMRect): number {
  const width = Math.max(0, Math.min(first.right, second.right) - Math.max(first.left, second.left));
  const height = Math.max(0, Math.min(first.bottom, second.bottom) - Math.max(first.top, second.top));
  return roundPx(width * height);
}

function clampViewportPoint(value: number, max: number): number {
  return roundPx(Math.max(0, Math.min(Math.max(0, max - 1), value)));
}

function readElementsFromPoint(x: number, y: number): Element[] {
  if (typeof document !== "undefined" && typeof document.elementsFromPoint === "function") {
    return document.elementsFromPoint(x, y);
  }

  const element = typeof document !== "undefined" ? document.elementFromPoint(x, y) : null;
  return element ? [element] : [];
}

function sampleTargetHitTestableStack(x: number, y: number, target: HTMLElement): Element[] {
  const previousPointerEvents = target.style.pointerEvents;
  target.style.pointerEvents = "auto";
  try {
    return readElementsFromPoint(x, y);
  } finally {
    target.style.pointerEvents = previousPointerEvents;
  }
}

function samplePaintHit(
  name: string,
  x: number,
  y: number,
  target: HTMLElement,
): PaintHitSample {
  const viewportWidth = typeof window !== "undefined" ? window.innerWidth : Number.MAX_SAFE_INTEGER;
  const viewportHeight = typeof window !== "undefined" ? window.innerHeight : Number.MAX_SAFE_INTEGER;
  const sampleX = clampViewportPoint(x, viewportWidth);
  const sampleY = clampViewportPoint(y, viewportHeight);
  const stack = readElementsFromPoint(sampleX, sampleY);
  const hitTestableStack = sampleTargetHitTestableStack(sampleX, sampleY, target);
  const top = stack[0] ?? null;
  const hitTestableTop = hitTestableStack[0] ?? null;
  return {
    name,
    x: sampleX,
    y: sampleY,
    insideTarget: !!top && target.contains(top),
    insideTargetWhenTargetHitTestable: !!hitTestableTop && target.contains(hitTestableTop),
    top: top ? readPaintElementSample(top) : null,
    hitTestableTop: hitTestableTop ? readPaintElementSample(hitTestableTop) : null,
    stack: stack.slice(0, 8).map(readPaintElementSample),
    hitTestableStack: hitTestableStack.slice(0, 8).map(readPaintElementSample),
  };
}

function readIntersectingPaintCandidates(
  scrollElement: HTMLElement,
  target: HTMLElement,
): Array<PaintElementSample & { overlapArea: number; containsTargetCenter: boolean }> {
  const targetRect = target.getBoundingClientRect();
  const searchRect = expandedRect(targetRect, 4);
  const centerX = targetRect.left + targetRect.width / 2;
  const centerY = targetRect.top + targetRect.height / 2;
  return Array.from(scrollElement.querySelectorAll<HTMLElement>(TRANSCRIPT_PAINT_CANDIDATE_SELECTOR))
    .filter((element) => element !== target)
    .map((element) => {
      const rect = element.getBoundingClientRect();
      return {
        element,
        rect,
      };
    })
    .filter(({ rect }) => rect.width > 0 && rect.height > 0 && rectsIntersect(searchRect, rect))
    .map(({ element, rect }) => ({
      ...readPaintElementSample(element),
      overlapArea: overlapArea(targetRect, rect),
      containsTargetCenter: centerX >= rect.left && centerX <= rect.right && centerY >= rect.top && centerY <= rect.bottom,
    }))
    .sort((a, b) => b.overlapArea - a.overlapArea)
    .slice(0, 24);
}

function collectPaintTargetCandidates(scrollElement: HTMLElement): PaintTargetCandidate[] {
  const activeThinkingCandidates = Array.from(
    scrollElement.querySelectorAll<HTMLElement>(TRANSCRIPT_ACTIVE_THINKING_SELECTOR),
  ).filter((element) => !!element.querySelector(".chat-waiting-indicator"));
  const standaloneWaitingCandidates = Array.from(
    scrollElement.querySelectorAll<HTMLElement>(TRANSCRIPT_STANDALONE_WAITING_SELECTOR),
  );
  const toolWaitingStatusCandidates = Array.from(
    scrollElement.querySelectorAll<HTMLElement>(TRANSCRIPT_TOOL_WAITING_STATUS_SELECTOR),
  );

  return [
    ...activeThinkingCandidates.map((element): PaintTargetCandidate => ({
      targetType: "active-thinking",
      element,
    })),
    ...standaloneWaitingCandidates.map((element): PaintTargetCandidate => ({
      targetType: "standalone-waiting",
      element,
    })),
    ...toolWaitingStatusCandidates.map((element): PaintTargetCandidate => ({
      targetType: "tool-waiting-status",
      element,
    })),
  ];
}

function paintTargetCounts(scrollElement: HTMLElement) {
  return {
    activeThinking: scrollElement.querySelectorAll(TRANSCRIPT_ACTIVE_THINKING_SELECTOR).length,
    activeThinkingWithIndicator: Array.from(
      scrollElement.querySelectorAll<HTMLElement>(TRANSCRIPT_ACTIVE_THINKING_SELECTOR),
    ).filter((element) => !!element.querySelector(".chat-waiting-indicator")).length,
    standaloneWaiting: scrollElement.querySelectorAll(TRANSCRIPT_STANDALONE_WAITING_SELECTOR).length,
    toolWaitingStatus: scrollElement.querySelectorAll(TRANSCRIPT_TOOL_WAITING_STATUS_SELECTOR).length,
  };
}

function readPaintTargetReport(
  targetCandidate: PaintTargetCandidate,
  scrollElement: HTMLElement,
): PaintTargetReport {
  const target = targetCandidate.element;
  const targetRect = target.getBoundingClientRect();
  const hitPoints = [
    ["center", targetRect.left + targetRect.width / 2, targetRect.top + targetRect.height / 2],
    ["left-center", targetRect.left + Math.min(12, Math.max(1, targetRect.width / 4)), targetRect.top + targetRect.height / 2],
    ["right-center", targetRect.right - Math.min(12, Math.max(1, targetRect.width / 4)), targetRect.top + targetRect.height / 2],
    ["top-center", targetRect.left + targetRect.width / 2, targetRect.top + Math.min(6, Math.max(1, targetRect.height / 4))],
    ["bottom-center", targetRect.left + targetRect.width / 2, targetRect.bottom - Math.min(6, Math.max(1, targetRect.height / 4))],
  ] as const;
  const hitTests = hitPoints.map(([name, x, y]) => samplePaintHit(name, x, y, target));
  const occludedHitTests = hitTests.filter((sample) => !sample.insideTargetWhenTargetHitTestable);

  return {
    targetType: targetCandidate.targetType,
    target: readPaintElementSample(target),
    targetAncestors: readPaintAncestorChain(target, scrollElement),
    hitTests,
    occludedHitTests,
    intersectingCandidates: readIntersectingPaintCandidates(scrollElement, target),
  };
}

function missingChildSample(): ToolLayoutChildSample {
  return {
    exists: false,
    rect: null,
    className: "",
    style: null,
  };
}

function readChildSample(root: HTMLElement, selector: string): ToolLayoutChildSample {
  const element = root.querySelector<HTMLElement>(selector);
  if (!element) return missingChildSample();
  const className = element.className;
  return {
    exists: true,
    rect: readRect(element.getBoundingClientRect()),
    className: typeof className === "string" ? className : "",
    style: readStyle(element),
  };
}

function readChildren(root: HTMLElement): Record<keyof typeof TOOL_LAYOUT_CHILD_SELECTORS, ToolLayoutChildSample> {
  return {
    summary: readChildSample(root, TOOL_LAYOUT_CHILD_SELECTORS.summary),
    panel: readChildSample(root, TOOL_LAYOUT_CHILD_SELECTORS.panel),
    list: readChildSample(root, TOOL_LAYOUT_CHILD_SELECTORS.list),
    waitingRow: readChildSample(root, TOOL_LAYOUT_CHILD_SELECTORS.waitingRow),
  };
}

function readState(dataset: DOMStringMap): Record<string, string> {
  return {
    allowCollapse: dataset.toolLayoutAllowCollapse ?? "",
    animateCollapseOnMount: dataset.toolLayoutAnimateCollapseOnMount ?? "",
    canCollapse: dataset.toolLayoutCanCollapse ?? "",
    collapseEnabled: dataset.toolLayoutCollapseEnabled ?? "",
    expanded: dataset.toolLayoutExpanded ?? "",
    keepExpanded: dataset.toolLayoutKeepExpanded ?? "",
    panelLeaving: dataset.toolLayoutPanelLeaving ?? "",
    panelVisible: dataset.toolLayoutPanelVisible ?? "",
    showWaiting: dataset.toolLayoutShowWaiting ?? "",
    waitingStatus: dataset.toolLayoutWaitingStatus ?? "",
  };
}

function readAncestorClassChain(element: HTMLElement, stopAt: HTMLElement): string[] {
  const classes: string[] = [];
  let current: HTMLElement | null = element.parentElement;
  while (current && current !== stopAt && classes.length < 6) {
    const className = current.className;
    classes.push(typeof className === "string" ? className : "");
    current = current.parentElement;
  }
  return classes;
}

function sampleKey(
  kind: string,
  toolCallIds: string[],
  rawKey: string,
  index: number,
) {
  const identity = toolCallIds.length > 0 ? toolCallIds.join("|") : rawKey || String(index);
  return `${kind}:${identity}`;
}

function collectToolLayoutElements(scrollElement: HTMLElement): HTMLElement[] {
  return Array.from(scrollElement.querySelectorAll<HTMLElement>(TOOL_LAYOUT_SELECTOR));
}

function ownScrollAnchorId(element: HTMLElement): string {
  return element.dataset.scrollAnchorId?.trim() ?? "";
}

function nearestScrollAnchorId(element: HTMLElement): string {
  const ownAnchorId = ownScrollAnchorId(element);
  if (ownAnchorId) return ownAnchorId;
  return element.closest<HTMLElement>(TRANSCRIPT_SCROLL_ANCHOR_SELECTOR)?.dataset.scrollAnchorId?.trim() ?? "";
}

function renderPartKind(element: HTMLElement): string {
  return element.dataset.renderPartKind?.trim() ?? "";
}

function renderPartKey(element: HTMLElement): string {
  return element.dataset.renderPartKey?.trim() ?? "";
}

function renderPartScope(element: HTMLElement): string {
  return element.dataset.renderPartScope?.trim() ?? "";
}

function normalizedElementText(element: HTMLElement): string {
  const source = element.innerText || element.textContent || "";
  return source
    .replace(/\u258d/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function textSignature(text: string): string {
  if (!text) return "";
  const head = text.slice(0, 80);
  const tail = text.slice(Math.max(0, text.length - 80));
  return `${text.length}:${head}:${tail}`;
}

function transcriptElementKind(element: HTMLElement): string {
  const partKind = renderPartKind(element);
  if (partKind) return `render-part:${partKind}`;
  const className = typeof element.className === "string" ? element.className : "";
  if (className.includes("transient")) return "transient-message";
  if (className.includes("assistant")) return "assistant-message";
  if (className.includes("user")) return "user-message";
  if (element.matches(TRANSCRIPT_SCROLL_ANCHOR_SELECTOR)) return "scroll-anchor";
  if (className.includes("chat-transcript-content")) return "content";
  return "element";
}

function transcriptSampleKey(element: HTMLElement, kind: string, index: number): string {
  const partKind = renderPartKind(element);
  if (partKind) {
    const partKey = renderPartKey(element);
    const partScope = renderPartScope(element);
    const anchorId = nearestScrollAnchorId(element);
    const identity = partKey || anchorId || String(index);
    return `render-part:${partScope}:${partKind}:${identity}`;
  }
  const anchorId = ownScrollAnchorId(element);
  if (anchorId) return `${kind}:${anchorId}`;
  return `${kind}:${index}`;
}

function readTranscriptElementSample(
  element: HTMLElement,
  kind: string,
  index: number,
  scrollRect: DOMRect,
  contentRect: DOMRect,
): TranscriptElementSample {
  const rect = element.getBoundingClientRect();
  const className = element.className;
  const normalizedText = normalizedElementText(element);
  const partKind = renderPartKind(element);
  const ownAnchorId = ownScrollAnchorId(element);
  const messageAnchorId = nearestScrollAnchorId(element);
  return {
    key: transcriptSampleKey(element, kind, index),
    kind,
    scrollAnchorId: ownAnchorId || messageAnchorId,
    messageAnchorId,
    renderPartKind: partKind,
    renderPartKey: renderPartKey(element),
    renderPartScope: renderPartScope(element),
    textLength: normalizedText.length,
    textPreview: normalizedText.slice(0, TEXT_PREVIEW_LIMIT),
    textSignature: textSignature(normalizedText),
    firstChildTag: element.firstElementChild?.tagName.toLowerCase() ?? "",
    lastChildTag: element.lastElementChild?.tagName.toLowerCase() ?? "",
    className: typeof className === "string" ? className : "",
    rect: readRect(rect),
    offsetTop: roundPx(rect.top - scrollRect.top),
    contentTop: roundPx(rect.top - contentRect.top),
    style: readStyle(element),
  };
}

function collectTranscriptElements(
  scrollElement: HTMLElement,
  selector: string,
  kindForElement: (element: HTMLElement) => string,
  scrollRect: DOMRect,
  contentRect: DOMRect,
): TranscriptElementSample[] {
  return Array.from(scrollElement.querySelectorAll<HTMLElement>(selector))
    .map((element, index) =>
      readTranscriptElementSample(element, kindForElement(element), index, scrollRect, contentRect),
    );
}

function findCurrentScrollAnchor(
  scrollElement: HTMLElement,
  scrollRect: DOMRect,
  contentRect: DOMRect,
): TranscriptElementSample | null {
  const anchors = Array.from(scrollElement.querySelectorAll<HTMLElement>(TRANSCRIPT_SCROLL_ANCHOR_SELECTOR));
  for (let index = 0; index < anchors.length; index += 1) {
    const anchor = anchors[index];
    if (!anchor) continue;
    const anchorId = anchor.dataset.scrollAnchorId?.trim();
    if (!anchorId) continue;
    const rect = anchor.getBoundingClientRect();
    if (rect.height <= 0) continue;
    if (rect.bottom <= scrollRect.top) continue;
    return readTranscriptElementSample(anchor, "current-scroll-anchor", index, scrollRect, contentRect);
  }
  return null;
}

export function captureToolBlockLayoutSnapshot(
  scope: string,
  reason: string,
  scrollElement: HTMLElement,
  contentElement: HTMLElement | null = null,
): ToolBlockLayoutSnapshot {
  const scrollRect = scrollElement.getBoundingClientRect();
  const contentRect = contentElement?.getBoundingClientRect() ?? scrollRect;
  const samples = collectToolLayoutElements(scrollElement).map((element, index): ToolBlockLayoutSample => {
    const dataset = element.dataset;
    const rect = element.getBoundingClientRect();
    const kind = dataset.toolLayoutKind ?? "unknown";
    const rawKey = dataset.toolLayoutKey ?? "";
    const toolCallIds = splitCsv(dataset.toolLayoutToolCallIds ?? "");
    const scopeElement = element.closest<HTMLElement>("[data-tool-layout-scope]");
    return {
      key: sampleKey(kind, toolCallIds, rawKey, index),
      kind,
      scope: dataset.toolLayoutScope ?? scopeElement?.dataset.toolLayoutScope ?? "",
      rawKey,
      toolCallIds,
      statuses: dataset.toolLayoutStatuses ?? "",
      state: readState(dataset),
      rect: readRect(rect),
      offsetTop: roundPx(rect.top - scrollRect.top),
      offsetLeft: roundPx(rect.left - scrollRect.left),
      contentTop: roundPx(rect.top - contentRect.top),
      className: typeof element.className === "string" ? element.className : "",
      ancestorClassChain: readAncestorClassChain(element, scrollElement),
      style: readStyle(element),
      children: readChildren(element),
    };
  });

  return {
    scope,
    reason,
    at: nowMs(),
    scrollTop: roundPx(scrollElement.scrollTop),
    scrollHeight: roundPx(scrollElement.scrollHeight),
    clientHeight: roundPx(scrollElement.clientHeight),
    contentHeight: roundPx(contentElement?.scrollHeight ?? scrollElement.scrollHeight),
    samples,
  };
}

export function captureTranscriptLayoutSnapshot(
  scope: string,
  reason: string,
  scrollElement: HTMLElement,
  contentElement: HTMLElement | null = null,
): TranscriptLayoutSnapshot {
  const scrollRect = scrollElement.getBoundingClientRect();
  const content = contentElement ?? scrollElement.querySelector<HTMLElement>(".chat-transcript-content");
  const contentRect = content?.getBoundingClientRect() ?? scrollRect;
  const contentSample = content
    ? readTranscriptElementSample(content, "content", 0, scrollRect, contentRect)
    : null;
  const messages = collectTranscriptElements(
    scrollElement,
    TRANSCRIPT_MESSAGE_SELECTOR,
    transcriptElementKind,
    scrollRect,
    contentRect,
  );
  const anchors = collectTranscriptElements(
    scrollElement,
    TRANSCRIPT_SCROLL_ANCHOR_SELECTOR,
    (element) => element.matches(TRANSCRIPT_MESSAGE_SELECTOR) ? transcriptElementKind(element) : "scroll-anchor",
    scrollRect,
    contentRect,
  );
  const renderParts = collectTranscriptElements(
    scrollElement,
    TRANSCRIPT_RENDER_PART_SELECTOR,
    transcriptElementKind,
    scrollRect,
    contentRect,
  );

  return {
    scope,
    reason,
    at: nowMs(),
    scrollTop: roundPx(scrollElement.scrollTop),
    scrollHeight: roundPx(scrollElement.scrollHeight),
    clientHeight: roundPx(scrollElement.clientHeight),
    contentHeight: roundPx(contentElement?.scrollHeight ?? scrollElement.scrollHeight),
    content: contentSample,
    currentAnchor: findCurrentScrollAnchor(scrollElement, scrollRect, contentRect),
    messages,
    anchors,
    renderParts,
  };
}

function hasMeaningfulDelta(value: number) {
  return Math.abs(value) >= TOOL_LAYOUT_SHIFT_THRESHOLD_PX;
}

function hasMeaningfulTranscriptDelta(value: number) {
  return Math.abs(value) >= TRANSCRIPT_LAYOUT_SHIFT_THRESHOLD_PX;
}

function stateChanged(
  before: ToolBlockLayoutSample,
  after: ToolBlockLayoutSample,
  key: keyof ToolBlockLayoutSample["state"],
) {
  return before.state[key] !== after.state[key];
}

function styleDiff(before: ToolLayoutStyle | null, after: ToolLayoutStyle | null): string[] {
  if (!before || !after) return before === after ? [] : ["style"];
  return (Object.keys(before) as Array<keyof ToolLayoutStyle>)
    .filter((key) => before[key] !== after[key]);
}

function childDelta(
  before: ToolLayoutChildSample,
  after: ToolLayoutChildSample,
): ToolLayoutChildDelta | null {
  const delta = {
    top: roundPx((after.rect?.top ?? 0) - (before.rect?.top ?? 0)),
    height: roundPx((after.rect?.height ?? 0) - (before.rect?.height ?? 0)),
    width: roundPx((after.rect?.width ?? 0) - (before.rect?.width ?? 0)),
  };
  const existsChanged = before.exists !== after.exists;
  const classChanged = before.className !== after.className;
  const styleChanged = styleDiff(before.style, after.style);
  const geometryChanged = [delta.top, delta.height, delta.width].some(hasMeaningfulDelta);
  if (!existsChanged && !classChanged && styleChanged.length === 0 && !geometryChanged) return null;

  return {
    existsChanged,
    classChanged,
    styleChanged,
    delta,
    before,
    after,
  };
}

function compareChildren(
  before: ToolBlockLayoutSample,
  after: ToolBlockLayoutSample,
): Partial<Record<keyof typeof TOOL_LAYOUT_CHILD_SELECTORS, ToolLayoutChildDelta>> {
  const deltas: Partial<Record<keyof typeof TOOL_LAYOUT_CHILD_SELECTORS, ToolLayoutChildDelta>> = {};
  for (const key of Object.keys(TOOL_LAYOUT_CHILD_SELECTORS) as Array<keyof typeof TOOL_LAYOUT_CHILD_SELECTORS>) {
    const delta = childDelta(before.children[key], after.children[key]);
    if (delta) {
      deltas[key] = delta;
    }
  }
  return deltas;
}

function transcriptElementShift(
  before: TranscriptElementSample,
  after: TranscriptElementSample,
): TranscriptElementShift | null {
  const delta = {
    viewportTop: roundPx(after.rect.top - before.rect.top),
    offsetTop: roundPx(after.offsetTop - before.offsetTop),
    contentTop: roundPx(after.contentTop - before.contentTop),
    height: roundPx(after.rect.height - before.rect.height),
    width: roundPx(after.rect.width - before.rect.width),
  };
  const classChanged = before.className !== after.className;
  const styleChanged = styleDiff(before.style, after.style);
  const geometryChanged = [
    delta.viewportTop,
    delta.offsetTop,
    delta.contentTop,
    delta.height,
    delta.width,
  ].some(hasMeaningfulTranscriptDelta);
  if (!classChanged && styleChanged.length === 0 && !geometryChanged) return null;

  return {
    key: before.key,
    kind: after.kind,
    scrollAnchorId: after.scrollAnchorId || before.scrollAnchorId,
    classChanged,
    styleChanged,
    delta,
    before,
    after,
  };
}

function compareTranscriptSamples(
  beforeSamples: TranscriptElementSample[],
  afterSamples: TranscriptElementSample[],
): TranscriptElementShift[] {
  const beforeByKey = new Map(beforeSamples.map((sample) => [sample.key, sample]));
  return afterSamples
    .map((after) => {
      const before = beforeByKey.get(after.key);
      if (!before) return null;
      return transcriptElementShift(before, after);
    })
    .filter((shift): shift is TranscriptElementShift => !!shift);
}

function transcriptPresenceChange(sample: TranscriptElementSample): TranscriptElementPresenceChange {
  return {
    key: sample.key,
    kind: sample.kind,
    scrollAnchorId: sample.scrollAnchorId,
    renderPartKind: sample.renderPartKind,
    renderPartKey: sample.renderPartKey,
    renderPartScope: sample.renderPartScope,
    textLength: sample.textLength,
    textPreview: sample.textPreview,
    textSignature: sample.textSignature,
    sample,
  };
}

function addedTranscriptSamples(
  beforeSamples: TranscriptElementSample[],
  afterSamples: TranscriptElementSample[],
): TranscriptElementPresenceChange[] {
  const beforeKeys = new Set(beforeSamples.map((sample) => sample.key));
  return afterSamples
    .filter((sample) => !beforeKeys.has(sample.key))
    .map(transcriptPresenceChange);
}

function removedTranscriptSamples(
  beforeSamples: TranscriptElementSample[],
  afterSamples: TranscriptElementSample[],
): TranscriptElementPresenceChange[] {
  const afterKeys = new Set(afterSamples.map((sample) => sample.key));
  return beforeSamples
    .filter((sample) => !afterKeys.has(sample.key))
    .map(transcriptPresenceChange);
}

function isTextRenderPart(sample: TranscriptElementSample): boolean {
  return sample.renderPartKind === TEXT_RENDER_PART_KIND || sample.kind === `render-part:${TEXT_RENDER_PART_KIND}`;
}

function compareTextRenderPartMoves(
  beforeSamples: TranscriptElementSample[],
  afterSamples: TranscriptElementSample[],
): TranscriptRenderPartMove[] {
  const beforeKeys = new Set(beforeSamples.map((sample) => sample.key));
  const afterKeys = new Set(afterSamples.map((sample) => sample.key));
  const removedTextParts = beforeSamples
    .filter((sample) => !afterKeys.has(sample.key) && isTextRenderPart(sample) && sample.textSignature);
  const addedTextParts = afterSamples
    .filter((sample) => !beforeKeys.has(sample.key) && isTextRenderPart(sample) && sample.textSignature);
  const usedRemovedKeys = new Set<string>();
  const moves: TranscriptRenderPartMove[] = [];

  for (const added of addedTextParts) {
    const candidates = removedTextParts
      .filter((removed) =>
        !usedRemovedKeys.has(removed.key)
        && removed.textSignature === added.textSignature,
      )
      .sort((left, right) =>
        Math.abs(left.contentTop - added.contentTop) - Math.abs(right.contentTop - added.contentTop),
      );
    const removed = candidates[0];
    if (!removed) continue;
    usedRemovedKeys.add(removed.key);
    moves.push({
      textSignature: added.textSignature,
      textLength: added.textLength,
      textPreview: added.textPreview,
      delta: {
        viewportTop: roundPx(added.rect.top - removed.rect.top),
        offsetTop: roundPx(added.offsetTop - removed.offsetTop),
        contentTop: roundPx(added.contentTop - removed.contentTop),
        height: roundPx(added.rect.height - removed.rect.height),
        width: roundPx(added.rect.width - removed.rect.width),
      },
      from: removed,
      to: added,
    });
  }

  return moves;
}

export function compareTranscriptLayoutSnapshots(
  before: TranscriptLayoutSnapshot,
  after: TranscriptLayoutSnapshot,
): TranscriptLayoutComparison {
  const contentShift = before.content && after.content
    ? transcriptElementShift(before.content, after.content)
    : null;
  const currentAnchorShift = before.currentAnchor && after.currentAnchor
    ? transcriptElementShift(before.currentAnchor, after.currentAnchor)
    : null;
  return {
    delta: {
      scrollTop: roundPx(after.scrollTop - before.scrollTop),
      scrollHeight: roundPx(after.scrollHeight - before.scrollHeight),
      contentHeight: roundPx(after.contentHeight - before.contentHeight),
    },
    contentShift,
    currentAnchorShift,
    messageShifts: compareTranscriptSamples(before.messages, after.messages),
    anchorShifts: compareTranscriptSamples(before.anchors, after.anchors),
    renderPartShifts: compareTranscriptSamples(before.renderParts, after.renderParts),
    addedRenderParts: addedTranscriptSamples(before.renderParts, after.renderParts),
    removedRenderParts: removedTranscriptSamples(before.renderParts, after.renderParts),
    textPartMoves: compareTextRenderPartMoves(before.renderParts, after.renderParts),
  };
}

function transcriptComparisonHasSignal(comparison: TranscriptLayoutComparison) {
  return [
    comparison.delta.scrollTop,
    comparison.delta.scrollHeight,
    comparison.delta.contentHeight,
  ].some(hasMeaningfulTranscriptDelta)
    || !!comparison.contentShift
    || !!comparison.currentAnchorShift
    || comparison.messageShifts.length > 0
    || comparison.anchorShifts.length > 0
    || comparison.renderPartShifts.length > 0
    || comparison.addedRenderParts.length > 0
    || comparison.removedRenderParts.length > 0
    || comparison.textPartMoves.length > 0;
}

function hasChildDelta(
  childDeltas: Partial<Record<keyof typeof TOOL_LAYOUT_CHILD_SELECTORS, ToolLayoutChildDelta>>,
) {
  return Object.keys(childDeltas).length > 0;
}

function childDeltaHasMeaningfulGeometry(delta: ToolLayoutChildDelta | undefined) {
  if (!delta) return false;
  return [delta.delta.top, delta.delta.height, delta.delta.width].some(hasMeaningfulDelta);
}

function analyzeToolBlockLayoutShift(
  before: ToolBlockLayoutSample,
  after: ToolBlockLayoutSample,
  beforeSnapshot: ToolBlockLayoutSnapshot,
  afterSnapshot: ToolBlockLayoutSnapshot,
): ToolBlockLayoutShift | null {
  const delta = {
    viewportTop: roundPx(after.rect.top - before.rect.top),
    offsetTop: roundPx(after.offsetTop - before.offsetTop),
    contentTop: roundPx(after.contentTop - before.contentTop),
    height: roundPx(after.rect.height - before.rect.height),
    width: roundPx(after.rect.width - before.rect.width),
    scrollTop: roundPx(afterSnapshot.scrollTop - beforeSnapshot.scrollTop),
    scrollHeight: roundPx(afterSnapshot.scrollHeight - beforeSnapshot.scrollHeight),
    contentHeight: roundPx(afterSnapshot.contentHeight - beforeSnapshot.contentHeight),
  };
  const childDeltas = compareChildren(before, after);

  const moved = [
    delta.viewportTop,
    delta.offsetTop,
    delta.contentTop,
    delta.height,
    delta.width,
  ].some(hasMeaningfulDelta);
  const stateMutated =
    before.scope !== after.scope
    || before.statuses !== after.statuses
    || before.className !== after.className
    || before.ancestorClassChain.join(" > ") !== after.ancestorClassChain.join(" > ")
    || styleDiff(before.style, after.style).length > 0
    || hasChildDelta(childDeltas)
    || Object.keys(before.state).some((key) =>
      before.state[key] !== after.state[key],
    );

  if (!moved && !stateMutated) return null;

  const causes: string[] = [];
  if (before.scope !== after.scope) causes.push("render-scope-changed");
  if (before.statuses !== after.statuses) causes.push("tool-status-changed");
  if (stateChanged(before, after, "expanded") || stateChanged(before, after, "panelVisible")) {
    causes.push("collection-expanded-state-changed");
  }
  if (stateChanged(before, after, "panelLeaving") || stateChanged(before, after, "animateCollapseOnMount")) {
    causes.push("collapse-animation-state-changed");
  }
  if (stateChanged(before, after, "allowCollapse") || stateChanged(before, after, "collapseEnabled") || stateChanged(before, after, "canCollapse")) {
    causes.push("collapse-policy-changed");
  }
  if (stateChanged(before, after, "keepExpanded")) causes.push("history-pin-state-changed");
  if (stateChanged(before, after, "showWaiting")) causes.push("waiting-row-state-changed");
  if (childDeltas.summary) causes.push("summary-layout-changed");
  if (childDeltas.panel) causes.push("panel-layout-changed");
  if (childDeltas.list) causes.push("list-layout-changed");
  if (childDeltas.waitingRow) causes.push("waiting-row-layout-changed");
  if (before.className !== after.className) causes.push("element-class-changed");
  if (before.ancestorClassChain.join(" > ") !== after.ancestorClassChain.join(" > ")) {
    causes.push("ancestor-layout-class-changed");
  }
  if (hasMeaningfulDelta(delta.height)) causes.push("element-height-changed");
  if (hasMeaningfulDelta(delta.width)) causes.push("element-width-changed");
  if (hasMeaningfulDelta(delta.contentTop)) causes.push("document-flow-position-changed");
  if (hasMeaningfulDelta(delta.scrollTop)) causes.push("scroll-top-adjusted");
  if (hasMeaningfulDelta(delta.scrollHeight) || hasMeaningfulDelta(delta.contentHeight)) {
    causes.push("transcript-height-changed");
  }
  if (causes.length === 0 && hasMeaningfulDelta(delta.viewportTop)) {
    causes.push("viewport-position-changed");
  }

  let primaryCause = causes[0] ?? "unknown";
  const visualShiftOnly = !moved;
  if (visualShiftOnly) {
    primaryCause = "visual-state-only";
  } else if (childDeltaHasMeaningfulGeometry(childDeltas.waitingRow)) {
    primaryCause = "waiting-row-layout";
  } else if (
    childDeltaHasMeaningfulGeometry(childDeltas.summary)
    || childDeltaHasMeaningfulGeometry(childDeltas.panel)
    || childDeltaHasMeaningfulGeometry(childDeltas.list)
  ) {
    primaryCause = "collection-inner-layout";
  } else if (
    causes.includes("render-scope-changed")
    && causes.includes("document-flow-position-changed")
  ) {
    primaryCause = "tool-handoff-reposition";
  } else if (
    causes.includes("collection-expanded-state-changed")
    || causes.includes("collapse-animation-state-changed")
  ) {
    primaryCause = "tool-collapse-transition";
  } else if (
    causes.includes("scroll-top-adjusted")
    && !causes.includes("document-flow-position-changed")
  ) {
    primaryCause = "scroll-anchor-adjustment";
  } else if (causes.includes("tool-status-changed")) {
    primaryCause = "tool-status-render-change";
  }

  return {
    key: before.key,
    kind: after.kind,
    toolCallIds: after.toolCallIds.length > 0 ? after.toolCallIds : before.toolCallIds,
    primaryCause,
    causes,
    delta,
    visualShiftOnly,
    childDeltas,
    before,
    after,
  };
}

export function compareToolBlockLayoutSnapshots(
  beforeSnapshot: ToolBlockLayoutSnapshot,
  afterSnapshot: ToolBlockLayoutSnapshot,
): ToolBlockLayoutShift[] {
  const beforeByKey = new Map(beforeSnapshot.samples.map((sample) => [sample.key, sample]));
  return afterSnapshot.samples
    .map((after) => {
      const before = beforeByKey.get(after.key);
      if (!before) return null;
      return analyzeToolBlockLayoutShift(before, after, beforeSnapshot, afterSnapshot);
    })
    .filter((shift): shift is ToolBlockLayoutShift => !!shift);
}

function summarizeSnapshot(snapshot: ToolBlockLayoutSnapshot) {
  return {
    reason: snapshot.reason,
    scrollTop: snapshot.scrollTop,
    scrollHeight: snapshot.scrollHeight,
    clientHeight: snapshot.clientHeight,
    contentHeight: snapshot.contentHeight,
    sampleCount: snapshot.samples.length,
  };
}

function summarizeTranscriptElement(sample: TranscriptElementSample) {
  return {
    key: sample.key,
    kind: sample.kind,
    scrollAnchorId: sample.scrollAnchorId,
    messageAnchorId: sample.messageAnchorId,
    renderPartKind: sample.renderPartKind,
    renderPartKey: sample.renderPartKey,
    renderPartScope: sample.renderPartScope,
    textLength: sample.textLength,
    textPreview: sample.textPreview,
    textSignature: sample.textSignature,
    firstChildTag: sample.firstChildTag,
    lastChildTag: sample.lastChildTag,
    rect: sample.rect,
    offsetTop: sample.offsetTop,
    contentTop: sample.contentTop,
    className: sample.className,
  };
}

function summarizeTranscriptSnapshot(snapshot: TranscriptLayoutSnapshot) {
  const textRenderParts = snapshot.renderParts.filter(isTextRenderPart);
  return {
    reason: snapshot.reason,
    scrollTop: snapshot.scrollTop,
    scrollHeight: snapshot.scrollHeight,
    clientHeight: snapshot.clientHeight,
    contentHeight: snapshot.contentHeight,
    messageCount: snapshot.messages.length,
    anchorCount: snapshot.anchors.length,
    renderPartCount: snapshot.renderParts.length,
    textRenderPartCount: textRenderParts.length,
    currentAnchor: snapshot.currentAnchor ? summarizeTranscriptElement(snapshot.currentAnchor) : null,
  };
}

function summarizeTranscriptComparison(comparison: TranscriptLayoutComparison) {
  return {
    delta: comparison.delta,
    contentShift: comparison.contentShift,
    currentAnchorShift: comparison.currentAnchorShift,
    messageShifts: comparison.messageShifts,
    anchorShifts: comparison.anchorShifts,
    renderPartShifts: comparison.renderPartShifts,
    addedRenderParts: comparison.addedRenderParts,
    removedRenderParts: comparison.removedRenderParts,
    textPartMoves: comparison.textPartMoves,
  };
}

function textRenderPartShifts(comparison: TranscriptLayoutComparison): TranscriptElementShift[] {
  return comparison.renderPartShifts.filter((shift) =>
    isTextRenderPart(shift.before) || isTextRenderPart(shift.after),
  );
}

function textRenderPartPresenceChanges(
  changes: TranscriptElementPresenceChange[],
): TranscriptElementPresenceChange[] {
  return changes.filter((change) =>
    change.renderPartKind === TEXT_RENDER_PART_KIND || change.kind === `render-part:${TEXT_RENDER_PART_KIND}`,
  );
}

function transcriptBodyComparisonHasSignal(comparison: TranscriptLayoutComparison) {
  return textRenderPartShifts(comparison).length > 0
    || textRenderPartPresenceChanges(comparison.addedRenderParts).length > 0
    || textRenderPartPresenceChanges(comparison.removedRenderParts).length > 0
    || comparison.textPartMoves.length > 0;
}

function summarizeTranscriptBodySnapshot(snapshot: TranscriptLayoutSnapshot) {
  return {
    ...summarizeTranscriptSnapshot(snapshot),
    textRenderParts: snapshot.renderParts
      .filter(isTextRenderPart)
      .map(summarizeTranscriptElement),
  };
}

function summarizeTranscriptBodyComparison(comparison: TranscriptLayoutComparison) {
  return {
    delta: comparison.delta,
    textPartShifts: textRenderPartShifts(comparison),
    addedTextParts: textRenderPartPresenceChanges(comparison.addedRenderParts),
    removedTextParts: textRenderPartPresenceChanges(comparison.removedRenderParts),
    textPartMoves: comparison.textPartMoves,
    renderPartShifts: comparison.renderPartShifts,
    currentAnchorShift: comparison.currentAnchorShift,
  };
}

export function traceViewportAnchorSample(params: {
  scope: string;
  phase: string;
  scrollElement: HTMLElement | null | undefined;
  contentElement?: HTMLElement | null;
  anchor?: HTMLElement | null;
  anchorState?: { offsetTop?: number; fallbackScrollTop?: number } | null;
  restored?: boolean;
  before?: TranscriptLayoutSnapshot | null;
  detail?: Record<string, unknown>;
}) {
  if (!diagnosticsEnabled()) return;
  const {
    scope,
    phase,
    scrollElement,
    contentElement = null,
    anchor = null,
    anchorState = null,
    restored,
    before = null,
    detail = {},
  } = params;
  if (!scrollElement) return;

  const after = captureTranscriptLayoutSnapshot(scope, phase, scrollElement, contentElement);
  const comparison = before ? compareTranscriptLayoutSnapshots(before, after) : null;
  const anchorSample = anchor && scrollElement.contains(anchor)
    ? readTranscriptElementSample(
        anchor,
        "tool-viewport-anchor",
        0,
        scrollElement.getBoundingClientRect(),
        (contentElement ?? scrollElement).getBoundingClientRect(),
      )
    : null;

  console.debug(`[Locus layout][viewport-anchor][${scope}] ${phase}`, {
    detail,
    restored,
    anchorState,
    anchor: anchorSample,
    before: before ? summarizeTranscriptSnapshot(before) : null,
    after: summarizeTranscriptSnapshot(after),
    comparison: comparison ? summarizeTranscriptComparison(comparison) : null,
  });
}

export function traceTranscriptPaintOcclusion(params: {
  scope: string;
  reason: string;
  scrollElement: HTMLElement | null | undefined;
  contentElement?: HTMLElement | null;
  detail?: Record<string, unknown>;
}) {
  if (!diagnosticsEnabled()) return;
  const { scope, reason, scrollElement, contentElement = null, detail = {} } = params;
  if (!scrollElement) return;

  requestFrame(() => {
    requestFrame(() => {
      if (!scrollElement.isConnected) return;
      const content = contentElement ?? scrollElement;
      const targetCandidates = collectPaintTargetCandidates(scrollElement);
      const targetCounts = paintTargetCounts(scrollElement);
      if (targetCandidates.length === 0) {
        recordLayoutDiagnostic(`${scope}.paintOcclusion.noTransientStatusTarget`, {
          reason,
          targetCounts,
          ...detail,
        });
        return;
      }

      const renderPartOrder = collectTranscriptElements(
        scrollElement,
        TRANSCRIPT_RENDER_PART_SELECTOR,
        transcriptElementKind,
        scrollElement.getBoundingClientRect(),
        content.getBoundingClientRect(),
      ).map((sample, index) => ({
        index,
        kind: sample.renderPartKind,
        scope: sample.renderPartScope,
        key: sample.renderPartKey,
        className: sample.className,
        rect: sample.rect,
        style: sample.style,
        textPreview: sample.textPreview,
      }));
      const targetReports = targetCandidates
        .slice(0, 8)
        .map((targetCandidate) => readPaintTargetReport(targetCandidate, scrollElement));

      console.debug(`[Locus layout][paint-occlusion][${scope}] ${reason}`, {
        detail,
        scroll: {
          scrollTop: roundPx(scrollElement.scrollTop),
          scrollHeight: roundPx(scrollElement.scrollHeight),
          clientHeight: roundPx(scrollElement.clientHeight),
        },
        targetCounts,
        targets: targetReports,
        occludedTargets: targetReports.filter((report) => report.occludedHitTests.length > 0),
        renderPartOrder,
      });
    });
  });
}

export function traceToolBlockLayoutChange(params: {
  scope: string;
  reason: string;
  scrollElement: HTMLElement | null | undefined;
  contentElement?: HTMLElement | null;
  detail?: Record<string, unknown>;
}) {
  if (!diagnosticsEnabled()) return;
  const { scope, reason, scrollElement, contentElement = null, detail = {} } = params;
  if (!scrollElement) return;

  const before = captureToolBlockLayoutSnapshot(scope, reason, scrollElement, contentElement);
  const transcriptBefore = captureTranscriptLayoutSnapshot(scope, reason, scrollElement, contentElement);
  requestFrame(() => {
    requestFrame(() => {
      if (!scrollElement.isConnected) return;
      const after = captureToolBlockLayoutSnapshot(scope, reason, scrollElement, contentElement);
      const transcriptAfter = captureTranscriptLayoutSnapshot(scope, reason, scrollElement, contentElement);
      const shifts = compareToolBlockLayoutSnapshots(before, after);
      const transcriptComparison = compareTranscriptLayoutSnapshots(transcriptBefore, transcriptAfter);
      if (transcriptComparisonHasSignal(transcriptComparison)) {
        console.debug(`[Locus layout][transcript][${scope}] ${reason}`, {
          detail,
          before: summarizeTranscriptSnapshot(transcriptBefore),
          after: summarizeTranscriptSnapshot(transcriptAfter),
          comparison: summarizeTranscriptComparison(transcriptComparison),
        });
      }
      if (transcriptBodyComparisonHasSignal(transcriptComparison)) {
        console.debug(`[Locus layout][transcript-body][${scope}] ${reason}`, {
          detail,
          before: summarizeTranscriptBodySnapshot(transcriptBefore),
          after: summarizeTranscriptBodySnapshot(transcriptAfter),
          comparison: summarizeTranscriptBodyComparison(transcriptComparison),
        });
      }
      if (shifts.length === 0) {
        recordLayoutDiagnostic(`${scope}.toolBlockLayout.stable`, {
          reason,
          sampleCount: after.samples.length,
          ...detail,
        });
        return;
      }

      console.debug(`[Locus layout][tool-block][${scope}] ${reason}`, {
        detail,
        before: summarizeSnapshot(before),
        after: summarizeSnapshot(after),
        transcript: summarizeTranscriptComparison(transcriptComparison),
        shifts,
      });
    });
  });
}
