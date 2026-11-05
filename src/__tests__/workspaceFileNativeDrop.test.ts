// @vitest-environment jsdom
import { createApp, defineComponent, h, ref, type App } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { provideInternalDragController, type InternalDragController } from "../composables/useInternalDrag";
import { createNativeWorkspaceFileDrop, useWorkspaceFileDrop } from "../composables/useWorkspaceFileDrop";

const mocks = vi.hoisted(() => ({ importEntries: vi.fn(), move: vi.fn(), notice: vi.fn(), confirm: vi.fn() }));
vi.mock("../services/workspaceExplorer", () => ({ importWorkspaceEntries: mocks.importEntries, moveWorkspaceFile: mocks.move }));
vi.mock("../stores/notification", () => ({ useNotificationStore: () => ({ addNotice: mocks.notice }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: mocks.confirm }));

const workspaceRef = { checkoutId: "workspace", expectedGeneration: 2, expectedMaterializationEpoch: 0 };
const outside = ["F:/Downloads/hero.png", "F:/Downloads/Props"];
let app: App | undefined;
let controller: InternalDragController | undefined;
afterEach(() => {
  app?.unmount(); app = undefined;
  controller?.dispose();
  document.body.replaceChildren();
  vi.useRealTimers();
  vi.clearAllMocks();
});

/** A Files list per name; `inner` nests the second list inside the first. */
function mountLists(options: { nested?: boolean; expand?: (directory: string) => void } = {}) {
  const List = defineComponent({ props: { name: { type: String, required: true } }, setup(props, { slots }) {
    const root = ref<HTMLElement | null>(null);
    useWorkspaceFileDrop({
      root: () => root.value,
      resolve: hit => {
        const element = hit.closest<HTMLElement>("[data-folder]");
        return element ? { root: "F:/Game", projectId: "p", workspaceRef, directory: element.dataset.folder!, element } : null;
      },
      expand: target => options.expand?.(target.directory),
    });
    return () => h("div", { ref: root, "data-list": props.name }, [
      h("div", { "data-folder": `${props.name}/Target` }, "Target"),
      h("div", { "data-outside": props.name }, "header"),
      slots.default?.(),
    ]);
  } });
  const Parent = defineComponent({ setup() {
    controller = provideInternalDragController();
    return () => options.nested
      ? h(List, { name: "Outer" }, { default: () => h(List, { name: "Inner" }) })
      : h(List, { name: "Primary" });
  } });
  const host = document.createElement("div");
  document.body.appendChild(host);
  app = createApp(Parent);
  app.mount(host);
  return host;
}

describe("OS file drops into Files folders", () => {
  it("highlights the folder and copies the dropped paths into it", async () => {
    mocks.importEntries.mockResolvedValue({ imported: ["Primary/Target/hero.png"], conflicts: [] });
    const host = mountLists();
    const target = host.querySelector<HTMLElement>('[data-folder="Primary/Target"]')!;
    const drop = createNativeWorkspaceFileDrop();
    expect(drop.update(target)).toBe(true);
    expect(target.classList.contains("is-workspace-file-drop-target")).toBe(true);
    await expect(drop.drop(target, outside)).resolves.toBe(true);
    expect(mocks.importEntries).toHaveBeenCalledExactlyOnceWith(workspaceRef, outside, "Primary/Target");
    expect(target.classList.contains("is-workspace-file-drop-target")).toBe(false);
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  it("leaves positions without a Files folder to other drop handling", async () => {
    const host = mountLists();
    const header = host.querySelector<HTMLElement>('[data-outside="Primary"]')!;
    const drop = createNativeWorkspaceFileDrop();
    expect(drop.update(header)).toBe(false);
    expect(drop.update(null)).toBe(false);
    await expect(drop.drop(header, outside)).resolves.toBe(false);
    await expect(drop.drop(document.body, outside)).resolves.toBe(false);
    expect(mocks.importEntries).not.toHaveBeenCalled();
  });

  it("replaces existing items only after confirmation", async () => {
    mocks.importEntries.mockResolvedValueOnce({ imported: [], conflicts: ["Primary/Target/hero.png", "Primary/Target/hero.png.meta"] })
      .mockResolvedValueOnce({ imported: ["Primary/Target/hero.png"], conflicts: [] });
    mocks.confirm.mockResolvedValueOnce(true);
    const host = mountLists();
    const target = host.querySelector<HTMLElement>('[data-folder="Primary/Target"]')!;
    await createNativeWorkspaceFileDrop().drop(target, outside);
    expect(mocks.confirm).toHaveBeenCalledOnce();
    const [message, options] = mocks.confirm.mock.calls[0]!;
    // Names are listed relative to the folder that received the drop.
    expect(message).toContain("hero.png\nhero.png.meta");
    expect(message).not.toContain("Primary/Target/");
    expect(options).toMatchObject({ kind: "warning" });
    expect(mocks.importEntries).toHaveBeenNthCalledWith(2, workspaceRef, outside, "Primary/Target", true);
  });

  it("copies nothing when replacing is declined", async () => {
    mocks.importEntries.mockResolvedValue({ imported: [], conflicts: ["Primary/Target/hero.png"] });
    mocks.confirm.mockResolvedValueOnce(false);
    const host = mountLists();
    const target = host.querySelector<HTMLElement>('[data-folder="Primary/Target"]')!;
    await createNativeWorkspaceFileDrop().drop(target, outside);
    expect(mocks.importEntries).toHaveBeenCalledOnce();
    expect(mocks.importEntries).toHaveBeenCalledWith(workspaceRef, outside, "Primary/Target");
  });

  it("reports a failed copy", async () => {
    mocks.importEntries.mockRejectedValueOnce(new Error("The destination must be a folder within this workspace"));
    const host = mountLists();
    const target = host.querySelector<HTMLElement>('[data-folder="Primary/Target"]')!;
    await createNativeWorkspaceFileDrop().drop(target, outside);
    expect(mocks.notice).toHaveBeenCalledWith("error", "The destination must be a folder within this workspace");
  });

  it("prefers the innermost Files list", async () => {
    mocks.importEntries.mockResolvedValue({ imported: [], conflicts: [] });
    const host = mountLists({ nested: true });
    const inner = host.querySelector<HTMLElement>('[data-folder="Inner/Target"]')!;
    await createNativeWorkspaceFileDrop().drop(inner, outside);
    expect(mocks.importEntries).toHaveBeenCalledExactlyOnceWith(workspaceRef, outside, "Inner/Target");
  });

  it("opens a hovered folder once while native drag positions keep arriving", () => {
    vi.useFakeTimers();
    const expand = vi.fn();
    const host = mountLists({ expand });
    const target = host.querySelector<HTMLElement>('[data-folder="Primary/Target"]')!;
    const drop = createNativeWorkspaceFileDrop();
    for (let step = 0; step < 5; step += 1) {
      drop.update(target);
      vi.advanceTimersByTime(200);
    }
    expect(expand).toHaveBeenCalledExactlyOnceWith("Primary/Target");
    drop.clear();
    expect(target.classList.contains("is-workspace-file-drop-target")).toBe(false);
  });
});
