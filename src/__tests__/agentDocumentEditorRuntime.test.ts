// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ read: vi.fn(), save: vi.fn(), readRule: vi.fn(), saveRule: vi.fn() }));
vi.mock("../services/agent", () => ({
  readWorkspaceAgentDocument: api.read, saveWorkspaceAgentDocument: api.save,
  readRule: api.readRule, saveRule: api.saveRule,
}));
vi.mock("../components/ui/BaseMarkdownEditor.vue", () => ({ default: defineComponent({
  props: ["modelValue", "contentKey"], emits: ["update:modelValue", "shortcutSave"],
  setup: (props, { emit }) => () => h("textarea", {
    value: props.modelValue, "data-key": props.contentKey,
    onInput: (event: Event) => emit("update:modelValue", (event.target as HTMLTextAreaElement).value),
  }),
}) }));
import AgentDocumentEditor, { type AgentDocumentTarget } from "../components/agent/AgentDocumentEditor.vue";

let app: App | undefined;
let host: HTMLDivElement;
const workspaceRef = { checkoutId: "a", expectedGeneration: 2, expectedMaterializationEpoch: 3 };
async function flush() { for (let i = 0; i < 8; i++) await nextTick(); }
function mount() {
  const props = reactive({ target: { kind: "soul", name: "", title: "soul.md" } as AgentDocumentTarget | null,
    workspaceRef: { ...workspaceRef }, workingDir: "F:/project-a", agentId: "unity" });
  const dirty = vi.fn();
  let editor: InstanceType<typeof AgentDocumentEditor> | null = null;
  host = document.createElement("div"); document.body.appendChild(host);
  app = createApp(() => h(AgentDocumentEditor, { ...props, ref: value => { editor = value as typeof editor; }, onDirtyChange: dirty }));
  app.mount(host);
  return { props, dirty, save: () => editor!.saveFile() };
}
async function edit(content: string) {
  const input = host.querySelector("textarea")!;
  input.value = content; input.dispatchEvent(new Event("input", { bubbles: true }));
  await flush();
}
beforeEach(() => {
  vi.clearAllMocks();
  api.read.mockImplementation(async (scope, agentId, kind, name) => ({ content: `${scope.checkoutId}:${agentId}:${kind}:${name}`, path: "F:/project/Locus/agent/unity/soul.md", revision: null }));
  api.save.mockImplementation(async (_scope, _agentId, _kind, _name, content) => ({ content, path: "soul.md", revision: content }));
  api.readRule.mockResolvedValue("# Built-in rule");
  api.saveRule.mockResolvedValue({});
});
afterEach(() => { app?.unmount(); document.body.innerHTML = ""; });

describe("Agent document editing", () => {
  it("retains edits across soul, rule and tool navigation and saves to their captured workspace", async () => {
    const vm = mount(); await flush();
    await edit("# Project soul");
    vm.props.target = { kind: "rule", name: "workflow.md", title: "Workflow" }; await flush();
    await edit("# Project rule");
    vm.props.target = { kind: "tool", name: "python", title: "Python" }; await flush();
    await edit("Project tool help");
    vm.props.target = { kind: "soul", name: "", title: "soul.md" }; await flush();
    expect(host.querySelector("textarea")!.value).toBe("# Project soul");
    expect(vm.dirty).toHaveBeenLastCalledWith(true);
    expect(await vm.save()).toBe(true); await flush();
    expect(api.save).toHaveBeenCalledWith(workspaceRef, "unity", "soul", "", "# Project soul", null);
    expect(api.saveRule).toHaveBeenCalledWith(workspaceRef, "unity", "workflow.md", "# Project rule", "# Built-in rule");
    expect(api.save).toHaveBeenCalledWith(workspaceRef, "unity", "tool", "python", "Project tool help", null);
    expect(vm.dirty).toHaveBeenLastCalledWith(false);
  });

  it("keeps drafts scoped when the workspace changes", async () => {
    const vm = mount(); await flush(); await edit("Draft A");
    vm.props.workspaceRef = { ...workspaceRef, checkoutId: "b" }; await flush();
    expect(host.querySelector("textarea")!.value).toBe("b:unity:soul:");
    await edit("Draft B");
    expect(await vm.save()).toBe(true);
    expect(api.save.mock.calls.map(call => [call[0].checkoutId, call[4]])).toEqual([["a", "Draft A"], ["b", "Draft B"]]);
  });

  it("keeps unsaved text after a failed save and allows a retry", async () => {
    const vm = mount(); await flush(); await edit("Do not lose this");
    api.save.mockRejectedValueOnce(new Error("Changed on disk"));
    expect(await vm.save()).toBe(false); await flush();
    expect(host.querySelector("textarea")!.value).toBe("Do not lose this");
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Changed on disk");
    expect(vm.dirty).toHaveBeenLastCalledWith(true);
    expect(await vm.save()).toBe(true);
  });

  it("does not let an old read replace a newly selected document", async () => {
    let finish: (value: unknown) => void = () => {};
    api.read.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const vm = mount();
    vm.props.target = { kind: "tool", name: "read", title: "Read" }; await flush();
    finish({ content: "late soul", path: "soul.md", revision: null }); await flush();
    expect(host.querySelector("textarea")!.value).toBe("a:unity:tool:read");
  });

  it("keeps text typed during a save dirty and uses the returned revision on the next save", async () => {
    let finish: (value: unknown) => void = () => {};
    const vm = mount(); await flush(); await edit("first");
    api.save.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const saving = vm.save(); await edit("second");
    finish({ content: "first", path: "soul.md", revision: "revision-1" });
    expect(await saving).toBe(false); await flush();
    expect(vm.dirty).toHaveBeenLastCalledWith(true);
    await vm.save();
    expect(api.save).toHaveBeenLastCalledWith(workspaceRef, "unity", "soul", "", "second", "revision-1");
  });
});
