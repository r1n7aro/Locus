// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, defineComponent, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../components/ui/BaseMarkdownEditor.vue", () => ({ default: defineComponent({
  props: ["modelValue"], emits: ["update:modelValue"],
  setup: (props, { emit }) => () => h("textarea", { value: props.modelValue,
    onInput: (event: Event) => emit("update:modelValue", (event.target as HTMLTextAreaElement).value) }),
}) }));
import AgentView from "../components/AgentView.vue";

const simple = { id: "simple", name: "Simple", description: "General development", projectTypes: [], isDefault: true, source: "app" };
const explorer = { ...simple, id: "explorer", name: "Explorer", isDefault: false };
const workspaceRef = { checkoutId: "a", expectedGeneration: 2, expectedMaterializationEpoch: 3 };
let app: App | undefined;
let host: HTMLDivElement;
async function flush() { for (let i = 0; i < 16; i++) await nextTick(); }
beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "list_workspace_agents": return [simple];
      case "list_workspace_subagent_defs": return [explorer];
      case "get_workspace_agent_system_prompt_stats": return { baseChars: 20, envChars: 0, rulesChars: 0, knowledgeChars: 0, totalChars: 20 };
      case "list_rules": case "list_workspace_agent_injected_items": return [];
      case "read_workspace_agent_document": return { content: `# ${args?.agentId} soul`, path: "F:/project/Locus/agent/explorer/soul.md", revision: null };
      case "save_workspace_agent_document": return { content: args?.content, path: "soul.md", revision: args?.content };
      default: return "";
    }
  });
  host = document.createElement("div"); document.body.appendChild(host);
});
afterEach(() => { app?.unmount(); document.body.innerHTML = ""; });

describe("Agent workspace navigation", () => {
  it("renders the secondary list alone and delegates selection to the workbench", async () => {
    const onOpenAgent = vi.fn();
    app = createApp(AgentView, { listOnly: true, workingDir: "F:/project", workspaceRef, onOpenAgent });
    app.use(createPinia()); app.mount(host); await flush();
    expect(host.querySelectorAll(".agent-tab")).toHaveLength(2);
    expect(host.querySelector(".dir-panel")).toBeNull();
    expect(host.querySelector(".guide-panel")).toBeNull();
    host.querySelectorAll<HTMLButtonElement>(".agent-tab")[1]!.click(); await flush();
    expect(onOpenAgent).toHaveBeenCalledWith(explorer);
    expect(mocks.invoke.mock.calls.map(call => call[0])).toEqual(["list_workspace_agents", "list_workspace_subagent_defs"]);
  });

  it("opens the requested Agent with its tertiary directory and forwards dirty/save behavior", async () => {
    const onDirtyChange = vi.fn();
    let editor: InstanceType<typeof AgentView> | null = null;
    const props = reactive({ embedded: true, agentId: "explorer", workingDir: "F:/project", workspaceRef });
    app = createApp(() => h(AgentView, { ...props, onDirtyChange, ref: value => { editor = value as typeof editor; } }));
    app.use(createPinia()); app.mount(host); await flush();
    expect(host.querySelector(".agent-sidebar")).toBeNull();
    expect(host.querySelector(".dir-title")?.textContent).toBe("Explorer");
    const input = host.querySelector("textarea")!;
    expect(input.value).toBe("# explorer soul");
    input.value = "# Project explorer"; input.dispatchEvent(new Event("input", { bubbles: true })); await flush();
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
    expect(await editor!.saveFile()).toBe(true); await flush();
    expect(mocks.invoke).toHaveBeenCalledWith("save_workspace_agent_document", {
      workspaceRef, agentId: "explorer", kind: "soul", name: "", content: "# Project explorer", expectedRevision: null,
    });
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  });
});
