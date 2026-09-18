// @vitest-environment jsdom
import { createApp, h, nextTick, reactive } from "vue";
import { afterEach, describe, expect, it } from "vitest";
import WorkbenchEditorStack from "../components/workbench/WorkbenchEditorStack.vue";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../types/workbench";

const cleanups: Array<() => void> = [];
afterEach(() => cleanups.splice(0).forEach((cleanup) => cleanup()));

function editor(id: string, kind: "session" | "knowledge" | "newSession" = "session"): WorkbenchEditorInput {
  return {
    editorId: id,
    resource: kind === "knowledge"
      ? { kind, projectId: "project", documentId: id }
      : kind === "session"
        ? { kind, projectId: "project", sessionId: id }
        : { kind, projectId: "project" },
    title: id, preview: true, pinned: false, dirty: false,
    capabilities: { split: true, detach: true, duplicate: true },
    availability: "available",
  };
}

function mountStack(initial: WorkbenchEditorInput[] = [editor("a")]) {
  const group = reactive<WorkbenchEditorGroup>({
    paneId: "pane", tabs: initial, activeEditorId: initial[0]?.editorId ?? null,
  });
  const ready = new Map<string, () => void>();
  const content = reactive<Record<string, string>>({});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp({
    render: () => h(WorkbenchEditorStack, { group }, {
      default: (slot: { editor: WorkbenchEditorInput; ready: () => void; contentActive: boolean }) => {
        ready.set(slot.editor.editorId, slot.ready);
        // Markdown editors release their DOM on deactivation, independently of
        // whether their parent surface is still mounted.
        return slot.contentActive ? h("button", content[slot.editor.editorId] ?? "unloaded") : null;
      },
      empty: () => h("span", { class: "empty" }, "empty"),
    }),
  });
  app.mount(host);
  cleanups.push(() => { app.unmount(); host.remove(); });
  const surface = (id: string) => host.querySelector<HTMLElement>(`[data-editor-id="${id}"]`);
  const visibleIds = () => Array.from(host.querySelectorAll<HTMLElement>(".workbench-editor-instance"))
    .filter((element) => element.style.display !== "none" && !element.classList.contains("is-preparing"))
    .map((element) => element.dataset.editorId);
  const finish = async (id: string, text = `content-${id}`) => {
    content[id] = text;
    await nextTick();
    ready.get(id)!();
    await nextTick();
  };
  const replace = async (next: WorkbenchEditorInput) => {
    group.tabs = [next];
    group.activeEditorId = next.editorId;
    await nextTick();
  };
  return { host, group, ready, surface, visibleIds, finish, replace };
}

describe("workbench editor presentation", () => {
  it("does not expose an unloaded session as the new-session welcome screen", async () => {
    const stack = mountStack();
    expect(stack.visibleIds()).toEqual([]);
    expect(stack.host.querySelector('[role="status"]')).not.toBeNull();
    expect(stack.surface("a")?.classList.contains("is-preparing")).toBe(true);
    // The incoming editor stays laid out while hidden, for scroll restoration.
    expect(stack.surface("a")?.style.display).not.toBe("none");
    await stack.finish("a");
    expect(stack.visibleIds()).toEqual(["a"]);
    expect(stack.host.querySelector('[role="status"]')).toBeNull();
  });

  it.each(["session", "knowledge"] as const)("retains the actual previous DOM until %s content is ready", async (kind) => {
    const stack = mountStack();
    await stack.finish("a");
    const previous = stack.surface("a");
    await stack.replace(editor("b", kind));
    expect(stack.visibleIds()).toEqual(["a"]);
    expect(stack.surface("a")).toBe(previous);
    expect(previous?.textContent).toBe("content-a");
    expect(previous?.hasAttribute("inert")).toBe(true);
    expect(stack.surface("b")?.hasAttribute("inert")).toBe(true);
    await stack.finish("b");
    expect(stack.visibleIds()).toEqual(["b"]);
    expect(stack.surface("b")?.textContent).toBe("content-b");
    expect(stack.surface("b")?.hasAttribute("inert")).toBe(false);
    expect(stack.surface("a")).toBeNull();
  });

  it("ignores superseded loads and releases replaced editors", async () => {
    const stack = mountStack();
    await stack.finish("a");
    await stack.replace(editor("b"));
    const staleReady = stack.ready.get("b")!;
    await stack.replace(editor("c", "knowledge"));
    staleReady();
    await nextTick();
    expect(stack.visibleIds()).toEqual(["a"]);
    expect(stack.surface("b")).toBeNull();
    await stack.finish("c");
    expect(stack.visibleIds()).toEqual(["c"]);
    expect(stack.host.querySelectorAll(".workbench-editor-instance")).toHaveLength(1);
  });

  it("keeps the outgoing document active until the replacement can render", async () => {
    const stack = mountStack([editor("a", "knowledge")]);
    await stack.finish("a");
    await stack.replace(editor("b", "knowledge"));
    expect(stack.surface("a")?.textContent).toBe("content-a");
    expect(stack.surface("a")?.hasAttribute("inert")).toBe(true);
    await stack.finish("b");
    expect(stack.visibleIds()).toEqual(["b"]);
    expect(stack.surface("a")).toBeNull();
  });

  it("switches already loaded tabs immediately without remounting", async () => {
    const stack = mountStack([editor("a"), editor("b", "knowledge")]);
    await stack.finish("a");
    await stack.finish("b");
    const previous = stack.surface("a");
    const next = stack.surface("b");
    stack.group.activeEditorId = "b";
    await nextTick();
    expect(stack.visibleIds()).toEqual(["b"]);
    expect(stack.surface("b")).toBe(next);
    stack.group.activeEditorId = "a";
    await nextTick();
    expect(stack.visibleIds()).toEqual(["a"]);
    expect(stack.surface("a")).toBe(previous);
  });

  it("can present an error result and navigate away while another load is pending", async () => {
    const stack = mountStack();
    await stack.finish("a");
    await stack.replace(editor("b", "knowledge"));
    await stack.finish("b", "load failed");
    expect(stack.visibleIds()).toEqual(["b"]);
    expect(stack.surface("b")?.textContent).toBe("load failed");
    await stack.replace(editor("c"));
    await stack.replace(editor("new", "newSession"));
    expect(stack.visibleIds()).toEqual(["new"]);
    expect(stack.surface("b")).toBeNull();
    stack.group.tabs = [];
    stack.group.activeEditorId = null;
    await nextTick();
    expect(stack.visibleIds()).toEqual([]);
    expect(stack.host.querySelector(".empty")).not.toBeNull();
  });
});
