import { describe, expect, it, vi } from "vitest";
import {
  applyMarkdownEditorPanelLayout,
  createMarkdownEditorResizeSync,
  MARKDOWN_EDITOR_PANEL_HEIGHT,
  MARKDOWN_EDITOR_PANEL_MAX_WIDTH,
} from "../components/ui/markdownEditorLayout";

function createLayoutRoot() {
  const elements = new Map<string, { style: Record<string, string> }>([
    [".vditor", { style: { width: "", height: "auto", minHeight: "60px" } }],
    [".vditor-content", { style: { minHeight: "60px" } }],
    [".vditor-ir", { style: { padding: "10px 400px", minHeight: "60px" } }],
    [".vditor-ir .vditor-reset", { style: { padding: "10px 400px", minHeight: "" } }],
  ]);

  return {
    elements,
    root: {
      querySelector(selector: string) {
        return elements.get(selector) ?? null;
      },
    },
  };
}

describe("markdownEditorLayout", () => {
  it("pins the editor to panel height and clears stale centered padding", () => {
    const { root, elements } = createLayoutRoot();

    applyMarkdownEditorPanelLayout(root);

    expect(MARKDOWN_EDITOR_PANEL_HEIGHT).toBe("100%");
    expect(MARKDOWN_EDITOR_PANEL_MAX_WIDTH).toBe(Number.MAX_SAFE_INTEGER);
    expect(elements.get(".vditor")?.style.height).toBe("100%");
    expect(elements.get(".vditor")?.style.width).toBe("100%");
    expect(elements.get(".vditor-ir")?.style.padding).toBe("0px");
    expect(elements.get(".vditor-ir .vditor-reset")?.style.padding).toBe("14px 14px 16px 16px");
    expect(elements.get(".vditor-ir .vditor-reset")?.style.minHeight).toBe("100%");
  });

  it("creates one resize observer per editor instance", async () => {
    const observedTargets: Element[] = [];
    const disconnected: number[] = [];
    const observers: Array<{ callback: ResizeObserverCallback; index: number }> = [];

    class FakeResizeObserver {
      private readonly index: number;
      constructor(public readonly callback: ResizeObserverCallback) {
        this.index = observers.length;
        observers.push({ callback, index: this.index });
      }

      observe(target: Element) {
        observedTargets.push(target);
      }

      disconnect() {
        disconnected.push(this.index);
      }
    }

    const syncA = vi.fn();
    const syncB = vi.fn();
    const targetA = {} as Element;
    const targetB = {} as Element;

    const handleA = createMarkdownEditorResizeSync(targetA, syncA, FakeResizeObserver as unknown as typeof ResizeObserver);
    const handleB = createMarkdownEditorResizeSync(targetB, syncB, FakeResizeObserver as unknown as typeof ResizeObserver);

    expect(observedTargets).toEqual([targetA, targetB]);

    observers[0]?.callback([], {} as ResizeObserver);
    await Promise.resolve();
    expect(syncA).toHaveBeenCalledTimes(1);
    expect(syncB).not.toHaveBeenCalled();

    observers[1]?.callback([], {} as ResizeObserver);
    await Promise.resolve();
    expect(syncB).toHaveBeenCalledTimes(1);

    handleA?.disconnect();
    handleB?.disconnect();
    expect(disconnected).toEqual([0, 1]);
  });

  it("re-syncs layout when Vditor mutates inline styles", async () => {
    const resizeObservedTargets: Element[] = [];
    const mutationObservedTargets: Array<{ target: Node; options: MutationObserverInit | undefined }> = [];
    const mutationObservers: Array<{ callback: MutationCallback; index: number }> = [];

    class FakeResizeObserver {
      constructor(public readonly callback: ResizeObserverCallback) {}

      observe(target: Element) {
        resizeObservedTargets.push(target);
      }

      disconnect() {}
    }

    class FakeMutationObserver {
      private readonly index: number;
      constructor(public readonly callback: MutationCallback) {
        this.index = mutationObservers.length;
        mutationObservers.push({ callback, index: this.index });
      }

      observe(target: Node, options?: MutationObserverInit) {
        mutationObservedTargets.push({ target, options });
      }

      disconnect() {}
    }

    const sync = vi.fn();
    const target = {} as Element;
    const handle = createMarkdownEditorResizeSync(
      target,
      sync,
      FakeResizeObserver as unknown as typeof ResizeObserver,
      FakeMutationObserver as unknown as typeof MutationObserver,
    );

    expect(resizeObservedTargets).toEqual([target]);
    expect(mutationObservedTargets).toEqual([{
      target,
      options: {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ["style", "class"],
      },
    }]);

    mutationObservers[0]?.callback([], {} as MutationObserver);
    await Promise.resolve();
    expect(sync).toHaveBeenCalledTimes(1);

    handle?.disconnect();
  });
});
