import {
  computed,
  inject,
  onBeforeUnmount,
  onMounted,
  provide,
  ref,
  shallowRef,
  type ComputedRef,
  type InjectionKey,
  type Ref,
  type ShallowRef,
} from "vue";
import type { IconNode } from "lucide";
import { acquireSelectionLock } from "./useSelectionLock";

export type InternalDragOperation = "move" | "copy";
export type InternalDragPhase = "idle" | "pending" | "dragging";
export type InternalDragPreviewMode = "floating" | "inline" | "floating-with-gap";
export type InternalDragFinishReason = "drop" | "cancel" | "pointercancel" | "escape" | "blur" | "replaced";

export interface InternalDragPayload<T = unknown> {
  type: string;
  data: T;
}

export interface InternalDragPreview {
  label: string;
  count?: number;
  kind?: "file" | "folder" | "package" | "item";
  icon?: IconNode;
  iconClass?: string;
}

export interface InternalDragSource<T = unknown> {
  id: string;
  payload: InternalDragPayload<T>;
  preview: InternalDragPreview;
  allowedOperations?: readonly InternalDragOperation[];
  onActivated?: () => void;
  onFinished?: (result: {
    dropped: boolean;
    reason: InternalDragFinishReason;
  }) => void;
}

export interface InternalDropDecision<I = unknown> {
  key: string;
  operation: InternalDragOperation;
  intent: I;
}

export interface InternalDropResolveContext<T = unknown> {
  source: InternalDragSource<T>;
  point: InternalDragPoint;
  hit: Element;
}

export interface InternalDropCommitContext<T = unknown, I = unknown>
  extends InternalDropResolveContext<T> {
  decision: InternalDropDecision<I>;
}

export interface InternalDropTargetRegistration<T = unknown, I = unknown> {
  id: string;
  root: () => HTMLElement | null;
  accepts: (source: InternalDragSource) => boolean;
  resolve: (context: InternalDropResolveContext<T>) => InternalDropDecision<I> | null;
  drop: (context: InternalDropCommitContext<T, I>) => void | Promise<void>;
  onTargetChange?: (decision: InternalDropDecision<I> | null) => void;
  previewMode?: InternalDragPreviewMode | ((context: InternalDropResolveContext<T>) => InternalDragPreviewMode);
  priority?: number;
}

export interface InternalDragPoint {
  x: number;
  y: number;
}

export function internalDragFloatingTransform(
  point: InternalDragPoint,
  anchor: InternalDragPoint,
): string {
  return `translate3d(${point.x - anchor.x}px, ${point.y - anchor.y}px, 0)`;
}

interface ActiveTarget {
  registration: InternalDropTargetRegistration;
  decision: InternalDropDecision;
}

interface PendingPointer {
  pointerId: number;
  start: InternalDragPoint;
  captureElement: HTMLElement;
}

export interface InternalDragController {
  phase: Ref<InternalDragPhase>;
  source: ShallowRef<InternalDragSource | null>;
  point: Ref<InternalDragPoint>;
  previewAnchor: Ref<InternalDragPoint>;
  activeTarget: ShallowRef<{
    registrationId: string;
    decision: InternalDropDecision;
  } | null>;
  previewMode: Ref<InternalDragPreviewMode>;
  dragging: ComputedRef<boolean>;
  start: <T>(event: PointerEvent, source: InternalDragSource<T>) => boolean;
  cancel: (reason?: InternalDragFinishReason) => void;
  registerTarget: <T, I>(registration: InternalDropTargetRegistration<T, I>) => () => void;
  subscribeVisualPoint: (listener: (point: InternalDragPoint) => void) => () => void;
  isDraggingType: (type: string) => boolean;
  dispose: () => void;
}

const INTERNAL_DRAG_THRESHOLD_PX = 5;
const INTERNAL_DRAG_BODY_CLASS = "is-internal-dragging";
const internalDragKey: InjectionKey<InternalDragController> = Symbol("locus-internal-drag");

function targetDepth(element: HTMLElement): number {
  let depth = 0;
  let current: HTMLElement | null = element;
  while (current) {
    depth += 1;
    current = current.parentElement;
  }
  return depth;
}

function sameDecision(left: ActiveTarget | null, right: ActiveTarget | null): boolean {
  return left?.registration.id === right?.registration.id
    && left?.decision.key === right?.decision.key
    && left?.decision.operation === right?.decision.operation;
}

export function createInternalDragController(): InternalDragController {
  const phase = ref<InternalDragPhase>("idle");
  const source = shallowRef<InternalDragSource | null>(null);
  const point = ref<InternalDragPoint>({ x: 0, y: 0 });
  const previewAnchor = ref<InternalDragPoint>({ x: 10, y: 17 });
  const activeTarget = shallowRef<{
    registrationId: string;
    decision: InternalDropDecision;
  } | null>(null);
  const previewMode = ref<InternalDragPreviewMode>("floating");
  const registrations = new Map<string, InternalDropTargetRegistration>();
  const visualPointListeners = new Set<(point: InternalDragPoint) => void>();
  let pendingPointer: PendingPointer | null = null;
  let resolvedTarget: ActiveTarget | null = null;
  let releaseSelectionLock: (() => void) | null = null;
  let suppressClickTimer = 0;
  let releaseClickSuppression: (() => void) | null = null;
  let finishing = false;
  let autoScrollFrame = 0;
  let autoScrollElement: HTMLElement | null = null;
  let autoScrollVelocity = 0;

  const dragging = computed(() => phase.value === "dragging");

  function clearTarget(): void {
    if (resolvedTarget) resolvedTarget.registration.onTargetChange?.(null);
    resolvedTarget = null;
    activeTarget.value = null;
  }

  function setTarget(next: ActiveTarget | null): void {
    if (sameDecision(resolvedTarget, next)) {
      resolvedTarget = next;
      return;
    }
    if (resolvedTarget) resolvedTarget.registration.onTargetChange?.(null);
    resolvedTarget = next;
    activeTarget.value = next
      ? { registrationId: next.registration.id, decision: next.decision }
      : null;
    if (next) next.registration.onTargetChange?.(next.decision);
  }

  function candidateTargets(hit: Element, activeSource: InternalDragSource): Array<{
    registration: InternalDropTargetRegistration;
    root: HTMLElement;
  }> {
    return [...registrations.values()]
      .flatMap((registration) => {
        const root = registration.root();
        if (!root || !root.contains(hit) || !registration.accepts(activeSource)) return [];
        return [{ registration, root }];
      })
      .sort((left, right) => {
        const priority = (right.registration.priority ?? 0) - (left.registration.priority ?? 0);
        if (priority) return priority;
        return targetDepth(right.root) - targetDepth(left.root);
      });
  }

  function resolveAtPoint(hit: Element | null): { target: ActiveTarget | null; mode: InternalDragPreviewMode } {
    const activeSource = source.value;
    if (!activeSource || phase.value !== "dragging") {
      return { target: null, mode: "floating" };
    }
    if (!(hit instanceof Element)) return { target: null, mode: "floating" };
    const candidates = candidateTargets(hit, activeSource);
    let mode: InternalDragPreviewMode = "floating";
    for (const candidate of candidates) {
      const context = {
        source: activeSource,
        point: point.value,
        hit,
      };
      const candidateMode = typeof candidate.registration.previewMode === "function"
        ? candidate.registration.previewMode(context)
        : candidate.registration.previewMode;
      if (candidateMode) {
        mode = candidateMode;
        break;
      }
    }
    for (const candidate of candidates) {
      const decision = candidate.registration.resolve({
        source: activeSource,
        point: point.value,
        hit,
      });
      if (!decision) continue;
      const allowed = activeSource.allowedOperations ?? ["move", "copy"];
      if (!allowed.includes(decision.operation)) continue;
      return { target: { registration: candidate.registration, decision }, mode };
    }
    return { target: null, mode };
  }

  function resolveCurrentState(hit = hitAtPoint()): void {
    const resolved = resolveAtPoint(hit);
    previewMode.value = resolved.mode;
    setTarget(resolved.target);
  }

  function hitAtPoint(): Element | null {
    const hit = document.elementFromPoint(point.value.x, point.value.y);
    return hit instanceof Element ? hit : null;
  }

  function eventPoint(event: PointerEvent): InternalDragPoint {
    const coalesced = event.getCoalescedEvents?.();
    const latest = coalesced?.[coalesced.length - 1] ?? event;
    return { x: latest.clientX, y: latest.clientY };
  }

  function emitVisualPoint(nextPoint: InternalDragPoint): void {
    for (const listener of visualPointListeners) {
      try {
        listener(nextPoint);
      } catch (error) {
        console.error("[internal-drag] visual point listener failed", error);
      }
    }
  }

  function updatePoint(event: PointerEvent): void {
    const nextPoint = eventPoint(event);
    point.value = nextPoint;
    emitVisualPoint(nextPoint);
    const hit = hitAtPoint();
    updateAutoScroll(hit);
    resolveCurrentState(hit);
  }

  function scrollableAncestor(hit: Element | null): HTMLElement | null {
    let current = hit instanceof HTMLElement ? hit : hit?.parentElement ?? null;
    while (current && current !== document.body) {
      const style = window.getComputedStyle(current);
      if (
        current.scrollHeight > current.clientHeight
        && (style.overflowY === "auto" || style.overflowY === "scroll")
      ) return current;
      current = current.parentElement;
    }
    return null;
  }

  function stopAutoScroll(): void {
    window.cancelAnimationFrame(autoScrollFrame);
    autoScrollFrame = 0;
    autoScrollElement = null;
    autoScrollVelocity = 0;
  }

  function runAutoScroll(): void {
    autoScrollFrame = 0;
    if (!autoScrollElement || !autoScrollVelocity || phase.value !== "dragging") return;
    const previous = autoScrollElement.scrollTop;
    autoScrollElement.scrollTop += autoScrollVelocity;
    if (autoScrollElement.scrollTop === previous) {
      stopAutoScroll();
      return;
    }
    resolveCurrentState();
    autoScrollFrame = window.requestAnimationFrame(runAutoScroll);
  }

  function updateAutoScroll(hit: Element | null): void {
    const scrollable = scrollableAncestor(hit);
    if (!scrollable) {
      stopAutoScroll();
      return;
    }
    const bounds = scrollable.getBoundingClientRect();
    const edge = Math.min(36, Math.max(20, bounds.height * 0.12));
    let velocity = 0;
    if (point.value.y < bounds.top + edge) {
      velocity = -Math.ceil(14 * (1 - Math.max(0, point.value.y - bounds.top) / edge));
    } else if (point.value.y > bounds.bottom - edge) {
      velocity = Math.ceil(14 * (1 - Math.max(0, bounds.bottom - point.value.y) / edge));
    }
    if (!velocity) {
      stopAutoScroll();
      return;
    }
    autoScrollElement = scrollable;
    autoScrollVelocity = velocity;
    if (!autoScrollFrame) autoScrollFrame = window.requestAnimationFrame(runAutoScroll);
  }

  function activate(): void {
    if (!source.value || !pendingPointer || phase.value !== "pending") return;
    phase.value = "dragging";
    releaseSelectionLock = acquireSelectionLock();
    document.body.classList.add(INTERNAL_DRAG_BODY_CLASS);
    source.value.onActivated?.();
  }

  function onPointerMove(event: PointerEvent): void {
    if (!pendingPointer || event.pointerId !== pendingPointer.pointerId) return;
    if (phase.value === "pending") {
      const dx = event.clientX - pendingPointer.start.x;
      const dy = event.clientY - pendingPointer.start.y;
      if (Math.hypot(dx, dy) < INTERNAL_DRAG_THRESHOLD_PX) return;
      activate();
    }
    if (phase.value !== "dragging") return;
    event.preventDefault();
    updatePoint(event);
  }

  function onPointerRawUpdate(rawEvent: Event): void {
    const event = rawEvent as PointerEvent;
    if (
      !pendingPointer
      || event.pointerId !== pendingPointer.pointerId
      || phase.value !== "dragging"
    ) return;
    emitVisualPoint(eventPoint(event));
  }

  function suppressNextClick(): void {
    releaseClickSuppression?.();
    const suppress = (event: MouseEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      clear();
    };
    const clear = () => {
      window.removeEventListener("click", suppress, true);
      window.clearTimeout(suppressClickTimer);
      suppressClickTimer = 0;
      releaseClickSuppression = null;
    };
    releaseClickSuppression = clear;
    window.addEventListener("click", suppress, true);
    suppressClickTimer = window.setTimeout(clear, 160);
  }

  function removePointerListeners(): void {
    window.removeEventListener("pointermove", onPointerMove, true);
    window.removeEventListener("pointerrawupdate", onPointerRawUpdate, true);
    window.removeEventListener("pointerup", onPointerUp, true);
    window.removeEventListener("pointercancel", onPointerCancel, true);
    window.removeEventListener("keydown", onKeydown, true);
    window.removeEventListener("blur", onWindowBlur);
    pendingPointer?.captureElement.removeEventListener("lostpointercapture", onLostPointerCapture);
  }

  function resetState(): void {
    removePointerListeners();
    stopAutoScroll();
    clearTarget();
    releaseSelectionLock?.();
    releaseSelectionLock = null;
    document.body.classList.remove(INTERNAL_DRAG_BODY_CLASS);
    const capture = pendingPointer;
    pendingPointer = null;
    if (capture?.captureElement.hasPointerCapture?.(capture.pointerId)) {
      capture.captureElement.releasePointerCapture(capture.pointerId);
    }
    phase.value = "idle";
    previewMode.value = "floating";
    previewAnchor.value = { x: 10, y: 17 };
    source.value = null;
  }

  function finish(reason: InternalDragFinishReason, dropped: boolean): void {
    if (finishing) return;
    finishing = true;
    const completedSource = source.value;
    const completedTarget = dropped ? resolvedTarget : null;
    const completedPoint = point.value;
    const hit = document.elementFromPoint(completedPoint.x, completedPoint.y);
    const wasDragging = phase.value === "dragging";
    resetState();
    if (wasDragging) suppressNextClick();
    completedSource?.onFinished?.({ dropped: !!completedTarget, reason });
    if (completedTarget && completedSource && hit instanceof Element) {
      void Promise.resolve(completedTarget.registration.drop({
        source: completedSource,
        point: completedPoint,
        hit,
        decision: completedTarget.decision,
      })).catch((error) => {
        console.error("[internal-drag] drop failed", error);
      });
    }
    finishing = false;
  }

  function onPointerUp(event: PointerEvent): void {
    if (!pendingPointer || event.pointerId !== pendingPointer.pointerId) return;
    if (phase.value === "dragging") {
      event.preventDefault();
      updatePoint(event);
      finish("drop", true);
      return;
    }
    finish("cancel", false);
  }

  function onPointerCancel(event: PointerEvent): void {
    if (!pendingPointer || event.pointerId !== pendingPointer.pointerId) return;
    finish("pointercancel", false);
  }

  function onLostPointerCapture(event: PointerEvent): void {
    if (finishing || !pendingPointer || event.pointerId !== pendingPointer.pointerId) return;
    if (phase.value === "dragging" && !pendingPointer.captureElement.isConnected) return;
    finish("pointercancel", false);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== "Escape" || phase.value === "idle") return;
    event.preventDefault();
    event.stopPropagation();
    finish("escape", false);
  }

  function onWindowBlur(): void {
    if (phase.value !== "idle") finish("blur", false);
  }

  function start<T>(event: PointerEvent, nextSource: InternalDragSource<T>): boolean {
    if (event.button !== 0 || event.isPrimary === false) return false;
    if (phase.value !== "idle") finish("replaced", false);
    const captureElement = event.currentTarget instanceof HTMLElement
      ? event.currentTarget
      : event.target instanceof HTMLElement
        ? event.target
        : null;
    if (!captureElement) return false;

    source.value = nextSource as InternalDragSource;
    point.value = { x: event.clientX, y: event.clientY };
    const captureBounds = captureElement.getBoundingClientRect();
    previewAnchor.value = {
      x: Math.max(8, Math.min(220, event.clientX - captureBounds.left)),
      y: Math.max(6, Math.min(28, event.clientY - captureBounds.top)),
    };
    phase.value = "pending";
    pendingPointer = {
      pointerId: event.pointerId,
      start: point.value,
      captureElement,
    };
    try {
      captureElement.setPointerCapture?.(event.pointerId);
    } catch {
      // Window-level listeners still keep the in-document drag session alive.
    }
    captureElement.addEventListener("lostpointercapture", onLostPointerCapture);
    window.addEventListener("pointermove", onPointerMove, { capture: true, passive: false });
    window.addEventListener("pointerrawupdate", onPointerRawUpdate, { capture: true, passive: true });
    window.addEventListener("pointerup", onPointerUp, true);
    window.addEventListener("pointercancel", onPointerCancel, true);
    window.addEventListener("keydown", onKeydown, true);
    window.addEventListener("blur", onWindowBlur);
    return true;
  }

  function cancel(reason: InternalDragFinishReason = "cancel"): void {
    if (phase.value === "idle") return;
    finish(reason, false);
  }

  function registerTarget<T, I>(registration: InternalDropTargetRegistration<T, I>): () => void {
    registrations.set(registration.id, registration as InternalDropTargetRegistration);
    return () => {
      if (resolvedTarget?.registration.id === registration.id) clearTarget();
      registrations.delete(registration.id);
    };
  }

  function subscribeVisualPoint(listener: (point: InternalDragPoint) => void): () => void {
    visualPointListeners.add(listener);
    listener(point.value);
    return () => visualPointListeners.delete(listener);
  }

  function isDraggingType(type: string): boolean {
    return phase.value === "dragging" && source.value?.payload.type === type;
  }

  function dispose(): void {
    cancel("cancel");
    registrations.clear();
    visualPointListeners.clear();
    releaseClickSuppression?.();
  }

  return {
    phase,
    source,
    point,
    previewAnchor,
    activeTarget,
    previewMode,
    dragging,
    start,
    cancel,
    registerTarget,
    subscribeVisualPoint,
    isDraggingType,
    dispose,
  };
}

const fallbackInternalDragController = createInternalDragController();

export function provideInternalDragController(): InternalDragController {
  const controller = createInternalDragController();
  provide(internalDragKey, controller);
  return controller;
}

export function useInternalDragController(): InternalDragController {
  return inject(internalDragKey, fallbackInternalDragController);
}

export function useInternalDropTarget<T, I>(
  registration: InternalDropTargetRegistration<T, I>,
): InternalDragController {
  const controller = useInternalDragController();
  let unregister: (() => void) | null = null;
  onMounted(() => {
    unregister = controller.registerTarget(registration);
  });
  onBeforeUnmount(() => {
    unregister?.();
    unregister = null;
  });
  return controller;
}

export function internalDragBodyClass(): string {
  return INTERNAL_DRAG_BODY_CLASS;
}
