// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelWorkbenchSidebarMotion,
  enterWorkbenchSidebar,
  leaveWorkbenchSidebar,
  switchWorkbenchSidebarContent,
} from "../components/workbench/workbenchSidebarMotion";

interface MotionStub {
  onfinish: (() => void) | null;
  oncancel: (() => void) | null;
  cancel: ReturnType<typeof vi.fn>;
  effect: { getComputedTiming: ReturnType<typeof vi.fn> };
}
const motions: MotionStub[] = [];
const animate = vi.fn(function () {
  const motion: MotionStub = {
    onfinish: null, oncancel: null, cancel: vi.fn(),
    effect: { getComputedTiming: vi.fn(() => ({ progress: 0 })) },
  };
  motion.cancel.mockImplementation(() => motion.oncancel?.());
  motions.push(motion);
  return motion as unknown as Animation;
});

function makeColumn() {
  const host = document.createElement("div");
  host.style.position = "relative";
  const sidebar = document.createElement("aside");
  sidebar.style.width = "320px";
  sidebar.style.borderRightWidth = "1px";
  const surface = document.createElement("div");
  surface.className = "secondary-sidebar-surface";
  sidebar.append(surface);
  const editor = document.createElement("main");
  editor.className = "development-editor";
  host.append(sidebar, editor);
  document.body.append(host);
  vi.spyOn(host, "getBoundingClientRect").mockReturnValue({ left: 20, top: 30 } as DOMRect);
  const bounds = vi.spyOn(sidebar, "getBoundingClientRect").mockReturnValue({ width: 320, height: 600, left: 260, top: 30 } as DOMRect);
  return { host, sidebar, surface, editor, bounds };
}

beforeEach(() => {
  motions.length = 0;
  animate.mockClear();
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false })));
  vi.stubGlobal("requestAnimationFrame", vi.fn());
  Object.defineProperty(Element.prototype, "animate", { configurable: true, value: animate });
});
afterEach(() => {
  motions.forEach((motion) => motion.oncancel?.());
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(Element.prototype, "animate");
  document.body.innerHTML = "";
});

describe("secondary sidebar motion", () => {
  it("expands the flex column with a fixed-width list and no editor transform", () => {
    const { sidebar, surface, editor, bounds } = makeColumn();
    const done = vi.fn();
    enterWorkbenchSidebar(sidebar, done);
    expect(bounds).toHaveBeenCalledOnce();
    expect(animate.mock.contexts).toEqual([sidebar, surface]);
    const calls = animate.mock.calls as unknown as [Keyframe[], KeyframeAnimationOptions][];
    expect(calls[0]![0]).toEqual([{ width: "0px", borderRightWidth: "0px" }, { width: "320px", borderRightWidth: "1px" }]);
    expect(surface.style.width).toBe("319px");
    expect(sidebar.style.minWidth).toBe("0px");
    expect(sidebar.style.overflow).toBe("clip");
    expect(sidebar.style.width).toBe("320px");
    expect(sidebar.style.transform).toBe("");
    expect(editor.style.transform).toBe("");
    expect(requestAnimationFrame).not.toHaveBeenCalled();
    expect(done).not.toHaveBeenCalled();
    motions[0]!.onfinish?.();
    expect(done).toHaveBeenCalledOnce();
    expect(sidebar.style.minWidth).toBe("");
    expect(surface.style.width).toBe("");
    expect(motions.every((motion) => motion.cancel.mock.calls.length === 1)).toBe(true);
  });

  it("shrinks the closing column in normal flow while clipping the fixed-width list", () => {
    const { sidebar, surface, editor, bounds } = makeColumn();
    const done = vi.fn(() => sidebar.remove());
    leaveWorkbenchSidebar(sidebar, done);
    expect(sidebar.isConnected).toBe(true);
    expect(sidebar.style.position).toBe("");
    expect(sidebar.style.left).toBe("");
    expect(editor.style.transform).toBe("");
    expect(sidebar.style.width).toBe("320px");
    expect(surface.style.width).toBe("319px");
    expect(sidebar.inert).toBe(true);
    expect(sidebar.style.pointerEvents).toBe("none");
    expect(animate.mock.contexts).toEqual([sidebar, surface]);
    const calls = animate.mock.calls as unknown as [Keyframe[], KeyframeAnimationOptions][];
    expect(calls[0]![0]).toEqual([{ width: "320px", borderRightWidth: "1px" }, { width: "0px", borderRightWidth: "0px" }]);
    expect(calls[0]![1].duration).toBe(120);
    expect(calls[1]![0]).toEqual([{ opacity: 1 }, { opacity: 0 }]);
    expect(bounds).toHaveBeenCalledOnce();
    expect(requestAnimationFrame).not.toHaveBeenCalled();
    expect(done).not.toHaveBeenCalled();
    motions[0]!.onfinish?.();
    expect(sidebar.isConnected).toBe(false);
    expect(sidebar.style.position).toBe("");
    expect(sidebar.style.height).toBe("");
    expect(sidebar.style.width).toBe("320px");
    expect(motions.every((motion) => motion.cancel.mock.calls.length === 1)).toBe(true);
  });

  it("continues a cancelled entrance from the visible width and ignores its stale callback", () => {
    const { sidebar } = makeColumn();
    const entered = vi.fn();
    const left = vi.fn();
    enterWorkbenchSidebar(sidebar, entered);
    const staleFinish = motions[0]!.onfinish;
    motions[0]!.effect.getComputedTiming.mockReturnValue({ progress: 0.4 });
    // Vue calls enter-cancelled before invoking leave.
    cancelWorkbenchSidebarMotion(sidebar);
    leaveWorkbenchSidebar(sidebar, left);
    expect(entered).toHaveBeenCalledOnce();
    expect(motions[0]!.cancel).toHaveBeenCalledOnce();
    const calls = animate.mock.calls as unknown as [Keyframe[], KeyframeAnimationOptions][];
    expect(calls[2]![0][0]).toEqual({ width: "128px", borderRightWidth: "0.4px" });
    expect(calls[3]![0][0]!.opacity).toBe(0.4);
    staleFinish?.();
    expect(left).not.toHaveBeenCalled();
    cancelWorkbenchSidebarMotion(sidebar);
    expect(left).toHaveBeenCalledOnce();
    expect(sidebar.style.position).toBe("");
    expect(sidebar.style.pointerEvents).toBe("");
    expect(sidebar.hasAttribute("data-sidebar-leaving")).toBe(false);
    expect(motions.every((motion) => motion.cancel.mock.calls.length === 1)).toBe(true);
  });

  it("reopens a replacement v-if column from the outgoing width without restoring a full column", () => {
    const { host, sidebar } = makeColumn();
    const left = vi.fn();
    leaveWorkbenchSidebar(sidebar, left);
    motions[0]!.effect.getComputedTiming.mockReturnValue({ progress: 0.25 });
    sidebar.remove();
    const replacement = makeColumn();
    host.prepend(replacement.sidebar);
    enterWorkbenchSidebar(replacement.sidebar, vi.fn());
    const calls = animate.mock.calls as unknown as [Keyframe[], KeyframeAnimationOptions][];
    expect(calls[2]![0][0]).toEqual({ width: "240px", borderRightWidth: "0.75px" });
    expect(calls[3]![0][0]!.opacity).toBe(0.75);
    expect(left).toHaveBeenCalledOnce();
    expect(motions[0]!.cancel).toHaveBeenCalledOnce();
    motions[2]!.onfinish?.();
    expect(replacement.sidebar.style.width).toBe("320px");
    expect(replacement.surface.style.width).toBe("");
  });

  it("does not inherit an old cancelled width after a resize", async () => {
    const { sidebar } = makeColumn();
    enterWorkbenchSidebar(sidebar, vi.fn());
    cancelWorkbenchSidebarMotion(sidebar);
    await Promise.resolve();
    leaveWorkbenchSidebar(sidebar, vi.fn());
    const calls = animate.mock.calls as unknown as [Keyframe[], KeyframeAnimationOptions][];
    expect(calls[2]![0][0]!.width).toBe("320px");
  });

  it("switches content in place and releases the previous effect", () => {
    const content = document.createElement("div");
    content.textContent = "current list";
    switchWorkbenchSidebarContent(content);
    switchWorkbenchSidebarContent(content);
    expect(motions[0]!.cancel).toHaveBeenCalledOnce();
    expect(content.textContent).toBe("current list");
    cancelWorkbenchSidebarMotion(content);
    expect(motions[1]!.cancel).toHaveBeenCalledOnce();
  });

  it("skips motion and geometry reads when reduced motion is requested", () => {
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: true })));
    const { sidebar, bounds } = makeColumn();
    const done = vi.fn();
    enterWorkbenchSidebar(sidebar, done);
    leaveWorkbenchSidebar(sidebar, done);
    switchWorkbenchSidebarContent(sidebar);
    expect(done).toHaveBeenCalledTimes(2);
    expect(animate).not.toHaveBeenCalled();
    expect(bounds).not.toHaveBeenCalled();
  });
});
