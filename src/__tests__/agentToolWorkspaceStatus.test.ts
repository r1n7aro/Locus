// @vitest-environment jsdom
import { createPinia, setActivePinia } from "pinia";
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AgentView from "../components/AgentView.vue";
import { useAgentWorkspaceScope } from "../composables/useAgentWorkspaceScope";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { ipcInvoke } from "../services/ipc";
import { useWorkbenchStore } from "../stores/workbench";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type { AgentInfo, InjectedPromptItem } from "../types";

vi.mock("../services/ipc", () => ({ ipcInvoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../stores/model", () => ({ useModelStore: () => ({ selectedModelId: "model-a" }) }));
vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../components/MarkdownRenderer.vue", () => ({
  default: defineComponent({ props: ["content"], setup: (props) => () => h("div", props.content) }),
}));

const unityAgent: AgentInfo = {
  id: "unity", name: "Unity", description: "Unity agent", projectTypes: ["unity"],
  isDefault: true, source: "builtIn",
};
const active = ref(true);
let enabled = true;
let unavailableReason: string | null = null;
let app: ReturnType<typeof createApp> | null = null;
let host: HTMLDivElement;

function toolItem(checkoutId: string): InjectedPromptItem {
  const reason = checkoutId === "unity" ? unavailableReason : "requires_unity_workspace";
  return {
    id: "available_tool::unity_set_play_mode", title: "unity_set_play_mode",
    kind: "tools", source: "runtime", content: "Set play mode",
    meta: {
      function: { name: "unity_set_play_mode", description: "Set play mode", parameters: { type: "object" } },
      loadMode: "direct", enabled, canToggleEnabled: true,
      runtimeAvailable: reason === null, unavailableReason: reason,
    },
  };
}

async function flush() {
  for (let index = 0; index < 12; index += 1) await nextTick();
}

async function mountAgent() {
  app = createApp(defineComponent({
    setup() {
      const scope = useAgentWorkspaceScope();
      return () => h(AgentView, {
        active: active.value, workingDir: scope.workingDir.value,
        workspaceRef: scope.workspaceRef.value, agentList: [unityAgent],
      });
    },
  }));
  app.use(pinia);
  app.mount(host);
  await flush();
}

let pinia: ReturnType<typeof createPinia>;

describe("Agent tool workspace status", () => {
  beforeEach(() => {
    localStorage.clear();
    pinia = createPinia();
    setActivePinia(pinia);
    useDisplaySettings().state.workspaceDisplayMode = "single";
    active.value = true;
    enabled = true;
    unavailableReason = null;
    const contexts = useWorkspaceContextStore();
    for (const checkoutId of ["unity", "general"]) {
      const root = `F:/projects/${checkoutId}`;
      contexts.checkoutsById[checkoutId] = {
        checkoutId, projectId: checkoutId, root, normalizedRoot: root, lastOpenedAt: 1,
        runtime: {
          checkoutId, projectId: checkoutId, root, workspaceGeneration: 7, leaseCount: 1,
          detectedServices: checkoutId === "unity" ? ["unity"] : [],
        },
      };
    }
    contexts.paneContexts["main\u0000main"] = {
      windowId: "main", paneId: "main", focusedCheckoutId: "general",
      workspaceGeneration: 7, intentEpoch: 1, revision: 1,
    };
    useWorkbenchStore().switchWorkspaceScope("main", "unity");
    vi.mocked(ipcInvoke).mockReset();
    vi.mocked(ipcInvoke).mockImplementation(async (command, args) => {
      if (command === "list_workspace_agents") return [unityAgent];
      if (command === "list_workspace_subagent_defs" || command === "list_rules") return [];
      if (command === "get_workspace_agent_system_prompt_stats") return null;
      if (command === "list_workspace_agent_injected_items") {
        return [toolItem((args?.workspaceRef as { checkoutId: string }).checkoutId)];
      }
      if (command === "set_agent_tool_enabled") { enabled = args?.enabled === true; return; }
      return "";
    });
    host = document.createElement("div");
    document.body.appendChild(host);
  });

  afterEach(() => { app?.unmount(); app = null; host.remove(); });

  it("uses the tree workspace for tool status and enable changes", async () => {
    await mountAgent();
    const row = host.querySelector<HTMLButtonElement>(".tool-item")!;
    expect(row).not.toBeNull();
    expect(row.classList.contains("tool-unavailable")).toBe(false);
    row.click();
    await flush();
    expect(host.querySelector(".tool-availability-section")).toBeNull();

    row.querySelector<HTMLButtonElement>("[role=checkbox]")!.click();
    await flush();
    expect(ipcInvoke).toHaveBeenCalledWith("set_agent_tool_enabled", {
      workspaceRef: { checkoutId: "unity", expectedGeneration: 7 },
      agentId: "unity", toolName: "unity_set_play_mode", enabled: false,
    });
    expect(host.querySelector(".tool-item")!.classList.contains("tool-disabled")).toBe(true);
  });

  it("refreshes availability on return and when the main workspace changes", async () => {
    await mountAgent();
    host.querySelector<HTMLButtonElement>(".tool-item")!.click();
    await flush();
    active.value = false;
    await flush();
    unavailableReason = "unity_service_unavailable";
    active.value = true;
    await flush();
    expect(host.querySelector(".tool-availability-reason")?.textContent)
      .toBe("agent.tool.unavailableReason.unityServiceUnavailable");

    useWorkbenchStore().switchWorkspaceScope("main", "general");
    await flush();
    host.querySelector<HTMLButtonElement>(".tool-item")!.click();
    await flush();
    expect(host.querySelector(".tool-availability-reason")?.textContent)
      .toBe("agent.tool.unavailableReason.requiresUnityWorkspace");
  });
});
