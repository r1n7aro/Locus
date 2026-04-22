export const MARKDOWN_EDITOR_PANEL_HEIGHT = "100%";
export const MARKDOWN_EDITOR_PANEL_MAX_WIDTH = Number.MAX_SAFE_INTEGER;

interface LayoutStyleTarget {
  style: Record<string, string> | CSSStyleDeclaration;
}

interface LayoutQueryRoot {
  querySelector(selector: string): LayoutStyleTarget | null;
}

interface ResizeSyncHandle {
  disconnect(): void;
}

type ResizeObserverCtor = new (callback: ResizeObserverCallback) => {
  observe(target: Element): void;
  disconnect(): void;
};

type MutationObserverCtor = new (callback: MutationCallback) => {
  observe(target: Node, options?: MutationObserverInit): void;
  disconnect(): void;
};

function toCssProperty(property: string) {
  return property.replace(/[A-Z]/g, (match) => `-${match.toLowerCase()}`);
}

function isCssStyleDeclaration(style: LayoutStyleTarget["style"]): style is CSSStyleDeclaration {
  return typeof (style as CSSStyleDeclaration).setProperty === "function"
    && typeof (style as CSSStyleDeclaration).getPropertyValue === "function"
    && typeof (style as CSSStyleDeclaration).getPropertyPriority === "function";
}

function setStyleValue(
  target: LayoutStyleTarget,
  property: string,
  value: string,
  important = false,
) {
  const style = target.style;
  if (isCssStyleDeclaration(style)) {
    const cssProperty = toCssProperty(property);
    const currentValue = style.getPropertyValue(cssProperty);
    const currentPriority = style.getPropertyPriority(cssProperty);
    const nextPriority = important ? "important" : "";
    if (currentValue === value && currentPriority === nextPriority) return;
    style.setProperty(cssProperty, value, nextPriority);
    return;
  }

  if ((style as Record<string, string>)[property] === value) return;
  (style as Record<string, string>)[property] = value;
}

function assignStyle(
  root: LayoutQueryRoot | null | undefined,
  selector: string,
  style: Record<string, string>,
  importantProperties: string[] = [],
) {
  const target = root?.querySelector(selector);
  if (!target) return;
  for (const [property, value] of Object.entries(style)) {
    setStyleValue(target, property, value, importantProperties.includes(property));
  }
}

export function applyMarkdownEditorPanelLayout(root: LayoutQueryRoot | null | undefined) {
  assignStyle(root, ".vditor", {
    width: "100%",
    height: MARKDOWN_EDITOR_PANEL_HEIGHT,
    minHeight: "0px",
  });
  assignStyle(root, ".vditor-content", {
    minHeight: "0px",
  });
  assignStyle(root, ".vditor-ir", {
    padding: "0px",
    minHeight: "0px",
  }, ["padding"]);
  assignStyle(root, ".vditor-ir .vditor-reset", {
    padding: "14px 14px 16px 16px",
    minHeight: "100%",
  }, ["padding"]);
}

export function createMarkdownEditorResizeSync(
  target: Element | null | undefined,
  syncLayout: () => void,
  ResizeObserverImpl: ResizeObserverCtor | null = typeof ResizeObserver === "undefined" ? null : ResizeObserver,
  MutationObserverImpl: MutationObserverCtor | null = typeof MutationObserver === "undefined" ? null : MutationObserver,
): ResizeSyncHandle | null {
  if (!target || (!ResizeObserverImpl && !MutationObserverImpl)) return null;

  let scheduled = false;
  const scheduleSync = () => {
    if (scheduled) return;
    scheduled = true;
    queueMicrotask(() => {
      scheduled = false;
      syncLayout();
    });
  };

  const resizeObserver = ResizeObserverImpl ? new ResizeObserverImpl(() => scheduleSync()) : null;
  resizeObserver?.observe(target);

  const mutationObserver = MutationObserverImpl ? new MutationObserverImpl(() => scheduleSync()) : null;
  mutationObserver?.observe(target, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["style", "class"],
  });

  return {
    disconnect() {
      resizeObserver?.disconnect();
      mutationObserver?.disconnect();
    },
  };
}
