const activeMotions = new WeakMap<Element, () => void>();
const easing = "cubic-bezier(0.2, 0.8, 0.2, 1)";

function reduceMotion(element: Element): boolean {
  return element.ownerDocument.defaultView
    ?.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
}

export function cancelWorkbenchSidebarMotion(element: Element | null): void {
  if (element) activeMotions.get(element)?.();
}

function playMotion(
  element: Element,
  frames: Keyframe[],
  timing: KeyframeAnimationOptions,
  done: () => void,
  surface = element,
): void {
  cancelWorkbenchSidebarMotion(element);
  if (reduceMotion(element) || typeof surface.animate !== "function") {
    done();
    return;
  }

  const animations: Animation[] = [];
  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    activeMotions.delete(element);
    for (const animation of animations) {
      animation.onfinish = null;
      animation.oncancel = null;
      animation.cancel();
    }
    done();
  };
  activeMotions.set(element, finish);
  const options: KeyframeAnimationOptions = { easing, fill: "both", ...timing };
  const animation = surface.animate(frames, options);
  animations.push(animation);
  animation.onfinish = finish;
  animation.oncancel = finish;
}

interface LayoutFrame {
  width: number;
  border: number;
  opacity: number;
}

// Vue can replace a leaving v-if element before its animation finishes. Keep
// its current geometry on the parent so the replacement continues from it.
const activeLayouts = new WeakMap<Element, { element: Element; read: () => LayoutFrame }>();
const interruptedLayouts = new WeakMap<Element, LayoutFrame>();

function temporaryStyles(element: HTMLElement, styles: Record<string, string>): () => void {
  const previous = Object.keys(styles).map((name) => ({
    name, value: element.style.getPropertyValue(name), priority: element.style.getPropertyPriority(name),
  }));
  for (const [name, value] of Object.entries(styles)) element.style.setProperty(name, value);
  return () => {
    for (const { name, value, priority } of previous) {
      if (value) element.style.setProperty(name, value, priority);
      else element.style.removeProperty(name);
    }
  };
}

function animateSidebarLayout(element: Element, done: () => void, opening: boolean): void {
  const sidebar = element as HTMLElement;
  const ownerWindow = element.ownerDocument.defaultView;
  const parent = sidebar.parentElement;
  if (!parent || reduceMotion(element) || typeof element.animate !== "function") {
    cancelWorkbenchSidebarMotion(element);
    done();
    return;
  }

  const previousLayout = activeLayouts.get(parent);
  const interrupted = previousLayout?.read() ?? interruptedLayouts.get(parent);
  if (previousLayout) cancelWorkbenchSidebarMotion(previousLayout.element);
  cancelWorkbenchSidebarMotion(element);
  interruptedLayouts.delete(parent);

  const surface = sidebar.querySelector<HTMLElement>(".secondary-sidebar-surface") ?? sidebar;
  const bounds = sidebar.getBoundingClientRect();
  const border = parseFloat(ownerWindow?.getComputedStyle(sidebar).borderRightWidth || "0") || 0;
  const expanded = { width: bounds.width, border, opacity: 1 };
  const collapsed = { width: 0, border: 0, opacity: 0 };
  const from = interrupted ?? (opening ? collapsed : expanded);
  const to = opening ? expanded : collapsed;
  const restoreShell = temporaryStyles(sidebar, {
    "min-width": "0",
    "box-sizing": "border-box",
    overflow: "clip",
    ...(!opening ? { "pointer-events": "none" } : {}),
  });
  // Only the outer flex allocation shrinks. Freeze the list at its full width
  // so text, toolbars and tree rows do not repeatedly reflow during the motion.
  const restoreSurface = surface === sidebar ? () => {} : temporaryStyles(surface, {
    width: `${bounds.width - border}px`,
    "align-self": "flex-start",
  });
  const wasInert = sidebar.inert;
  if (!opening) {
    sidebar.inert = true;
    sidebar.dataset.sidebarLeaving = "";
  }

  const timing: KeyframeAnimationOptions = {
    duration: opening ? 160 : 120,
    easing: opening ? easing : "cubic-bezier(0.4, 0, 1, 1)",
    fill: "both",
  };
  const layout = sidebar.animate([
    { width: `${from.width}px`, borderRightWidth: `${from.border}px` },
    { width: `${to.width}px`, borderRightWidth: `${to.border}px` },
  ], timing);
  const fade = surface.animate([{ opacity: from.opacity }, { opacity: to.opacity }], timing);
  const read = (): LayoutFrame => {
    const progress = layout.effect?.getComputedTiming().progress ?? 0;
    return {
      width: from.width + (to.width - from.width) * progress,
      border: from.border + (to.border - from.border) * progress,
      opacity: from.opacity + (to.opacity - from.opacity) * progress,
    };
  };
  let finished = false;
  const finish = (completed: boolean) => {
    if (finished) return;
    finished = true;
    if (!completed) {
      const frame = read();
      interruptedLayouts.set(parent, frame);
      // Only a reversal in this Vue update may inherit cancelled geometry.
      // A later resize/open must start from its own saved width instead.
      queueMicrotask(() => {
        if (interruptedLayouts.get(parent) === frame) interruptedLayouts.delete(parent);
      });
    }
    activeLayouts.delete(parent);
    activeMotions.delete(element);
    layout.onfinish = null;
    layout.oncancel = null;
    fade.oncancel = null;
    layout.cancel();
    fade.cancel();
    restoreSurface();
    restoreShell();
    sidebar.inert = wasInert;
    delete sidebar.dataset.sidebarLeaving;
    done();
  };
  activeLayouts.set(parent, { element, read });
  activeMotions.set(element, () => finish(false));
  layout.onfinish = () => finish(true);
  layout.oncancel = fade.oncancel = () => finish(false);
}

export function enterWorkbenchSidebar(element: Element, done: () => void): void {
  animateSidebarLayout(element, done, true);
}

export function leaveWorkbenchSidebar(element: Element, done: () => void): void {
  animateSidebarLayout(element, done, false);
}

export function switchWorkbenchSidebarContent(element: Element | null): void {
  if (!element) return;
  playMotion(element, [
    { opacity: 0, transform: "translateX(-6px)" },
    { opacity: 1, transform: "translateX(0)" },
  ], { duration: 100 }, () => {});
}
