// @vitest-environment jsdom
import { createApp, h, nextTick, Teleport } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import ModelEffortSelector from "../components/ModelEffortSelector.vue";

const cleanups: Array<() => void> = [];
afterEach(() => {
  cleanups.splice(0).reverse().forEach((cleanup) => cleanup());
  vi.restoreAllMocks();
});

async function mountSelector(detached = true, align: "start" | "end" = "end") {
  const frame = document.createElement("iframe");
  if (detached) document.body.append(frame);
  const ownerDocument = detached ? frame.contentDocument! : document;
  const ownerWindow = ownerDocument.defaultView!;
  const viewport = { width: 640, height: 720 };
  vi.spyOn(ownerWindow, "innerWidth", "get").mockImplementation(() => viewport.width);
  vi.spyOn(ownerWindow, "innerHeight", "get").mockImplementation(() => viewport.height);
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(320);
  const host = document.createElement("div");
  document.body.append(host);
  const selectModel = vi.fn();
  const app = createApp({
    render: () => h(Teleport, { to: ownerDocument.body }, h(ModelEffortSelector, {
      models: [{ id: "test-model", name: "Test Model", provider: "custom" }],
      selectedId: "test-model",
      effort: "high",
      align,
      onSelectModel: selectModel,
    })),
  });
  app.mount(host);
  const selector = ownerDocument.querySelector<HTMLElement>(".model-effort-selector")!;
  const trigger = selector.querySelector<HTMLButtonElement>("button")!;
  let bounds = new DOMRect(400, 600, 160, 28);
  vi.spyOn(selector, "getBoundingClientRect").mockImplementation(() => bounds);
  let mounted = true;
  const unmount = () => {
    if (mounted) app.unmount();
    mounted = false;
  };
  cleanups.push(() => { unmount(); host.remove(); frame.remove(); });
  trigger.click();
  await nextTick();
  return {
    ownerDocument, ownerWindow, viewport, trigger, selectModel, unmount,
    menu: () => ownerDocument.querySelector<HTMLElement>(".model-effort-dropdown"),
    setBounds: (next: DOMRect) => { bounds = next; },
  };
}

describe("model selector window ownership", () => {
  it("teleports the dropdown into the detached trigger's document", async () => {
    const fixture = await mountSelector();
    expect(fixture.menu()).not.toBeNull();
    expect(fixture.menu()!.parentElement).toBe(fixture.ownerDocument.body);
    expect(document.querySelector(".model-effort-dropdown")).toBeNull();
    expect(fixture.menu()!.style.left).toBe("240px");
    expect(fixture.menu()!.style.bottom).toBe("126px");
  });

  it("preserves trailing alignment in the main window", async () => {
    const fixture = await mountSelector(false);
    expect(fixture.menu()!.parentElement).toBe(document.body);
    expect(fixture.menu()!.style.left).toBe("240px");
    expect(fixture.menu()!.style.bottom).toBe("126px");
  });

  it("uses the detached viewport for resize, scroll and edge clamping", async () => {
    const fixture = await mountSelector(true, "start");
    expect(fixture.menu()!.style.left).toBe("308px");
    fixture.viewport.width = 480;
    fixture.viewport.height = 680;
    fixture.ownerWindow.dispatchEvent(new Event("resize"));
    await nextTick();
    expect(fixture.menu()!.style.left).toBe("148px");
    expect(fixture.menu()!.style.bottom).toBe("86px");

    fixture.setBounds(new DOMRect(24, 540, 160, 28));
    fixture.ownerDocument.body.dispatchEvent(new Event("scroll"));
    await nextTick();
    expect(fixture.menu()!.style.left).toBe("24px");
    expect(fixture.menu()!.style.bottom).toBe("146px");
  });

  it("handles selection and outside clicks in the detached document", async () => {
    const fixture = await mountSelector();
    fixture.menu()!.querySelector<HTMLButtonElement>(".model-effort-multi-agent")!.click();
    await nextTick();
    expect(fixture.trigger.classList.contains("open")).toBe(true);
    document.body.click();
    await nextTick();
    expect(fixture.trigger.classList.contains("open")).toBe(true);

    fixture.ownerDocument.body.click();
    await nextTick();
    expect(fixture.trigger.classList.contains("open")).toBe(false);
    fixture.trigger.click();
    await nextTick();
    fixture.menu()!.querySelector<HTMLButtonElement>(".model-effort-model-panel button")!.click();
    await nextTick();
    expect(fixture.selectModel).toHaveBeenCalledWith("test-model");
    expect(fixture.trigger.classList.contains("open")).toBe(false);
  });

  it.each(["close", "unmount"])("cleans up detached listeners on %s", async (action) => {
    const fixture = await mountSelector();
    const removeDocumentListener = vi.spyOn(fixture.ownerDocument, "removeEventListener");
    const removeWindowListener = vi.spyOn(fixture.ownerWindow, "removeEventListener");
    if (action === "unmount") fixture.unmount();
    else fixture.menu()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await nextTick();
    expect(removeDocumentListener).toHaveBeenCalledWith("click", expect.any(Function));
    expect(removeWindowListener).toHaveBeenCalledWith("resize", expect.any(Function));
    expect(removeWindowListener).toHaveBeenCalledWith("scroll", expect.any(Function), true);
  });
});
