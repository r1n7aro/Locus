// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, h, nextTick, type App } from "vue";
import UnityNumberField from "../components/unity/UnityNumberField.vue";
import UnityCurveField from "../components/unity/UnityCurveField.vue";
import UnityGradientField from "../components/unity/UnityGradientField.vue";
import { UNITY_PROPERTY_EDITING, UNITY_PROPERTY_WORKSPACE } from "../components/unity/unityPropertyEditingContext";
import { parseUnitySerializedEditValue } from "../components/unity/unitySerializedValue";

const capture = vi.hoisted(() => ({ open: vi.fn(async () => true), listen: vi.fn(async () => () => {}) }));
vi.mock("../services/unityValueEditorWindow", () => ({ openUnityValueEditorWindow: capture.open, listenUnityValueEditorCommitted: capture.listen }));
vi.mock("../stores/project", () => ({ useProjectStore: () => ({ requireWorkspaceRef: () => ({ checkoutId: "focused-other-workspace", expectedGeneration: 2 }) }) }));
const apps: App[] = [];
afterEach(() => { apps.splice(0).forEach(app => app.unmount()); document.body.innerHTML = ""; capture.open.mockClear(); capture.listen.mockReset().mockImplementation(async () => () => {}); });

describe("exact Unity values and editor ownership", () => {
  it.each(["9223372036854775807", "-9223372036854775808"])("preserves signed 64-bit %s in a real input", async value => {
    const committed = vi.fn();
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => h(UnityNumberField, { modelValue: "0", propertyType: "Long", onCommit: committed }) });
    apps.push(app); app.mount(root);
    const input = root.querySelector("input")!;
    input.value = value; input.dispatchEvent(new Event("input", { bubbles: true })); input.dispatchEvent(new Event("change", { bubbles: true })); await nextTick();
    expect(committed).toHaveBeenLastCalledWith(value);
  });
  it("validates exact integer ranges before transport", () => {
    expect(parseUnitySerializedEditValue("UnsignedLong", "18446744073709551615")).toBe("18446744073709551615");
    expect(() => parseUnitySerializedEditValue("Long", "9223372036854775808")).toThrow();
    expect(() => parseUnitySerializedEditValue("UnsignedLong", "-1")).toThrow();
    expect(() => parseUnitySerializedEditValue("Long", 9007199254740992)).toThrow();
    expect(parseUnitySerializedEditValue("Double", "1.23456789012345")).toBe(1.23456789012345);
  });
  it("commits a double without applying the float display precision", async () => {
    const committed = vi.fn(); const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => h(UnityNumberField, { modelValue: 0, propertyType: "Double", onCommit: committed }) });
    apps.push(app); app.mount(root); const input = root.querySelector("input")!;
    input.value = "1.23456789012345"; input.dispatchEvent(new Event("input")); input.dispatchEvent(new Event("change")); await nextTick();
    expect(committed).toHaveBeenLastCalledWith(1.23456789012345);
  });
  it.each([UnityCurveField, UnityGradientField])("opens value editors in the owning workspace and history", async component => {
    const workspaceRef = { checkoutId: "owner", expectedGeneration: 7, expectedMaterializationEpoch: 3 };
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => h(component, { modelValue: null, editable: true, bindingTarget: { kind: "asset", path: "Assets/A.asset", propertyPath: "value" } }) });
    app.provide(UNITY_PROPERTY_WORKSPACE, () => workspaceRef);
    app.provide(UNITY_PROPERTY_EDITING, { workspaceRef, historyOwner: "view-editor", adapter: { read: vi.fn(), write: vi.fn(), apply: vi.fn() } });
    apps.push(app); app.mount(root);
    root.querySelector<HTMLElement>('[role="button"]')!.click(); await nextTick();
    expect(capture.open).toHaveBeenCalledWith(expect.objectContaining({ workspaceRef, historyOwner: "view-editor" }));
  });
  it.each([UnityCurveField, UnityGradientField])("releases subscriptions that finish after the editor unmounts", async component => {
    let finish!: (release: () => void) => void;
    capture.listen.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => h(component, { modelValue: null }) }); app.mount(root); app.unmount();
    const dispose = vi.fn(); finish(dispose); await nextTick();
    expect(dispose).toHaveBeenCalledOnce();
  });
});
