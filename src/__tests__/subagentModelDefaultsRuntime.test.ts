// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, h, nextTick, ref, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";
import ModelDefaultsPanel from "../components/settings/ModelDefaults.vue";
import { useAgentStore } from "../stores/agent";
import type { AgentInfo, ModelDefaults, ModelOption } from "../types";

vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../services/agent", () => ({
  listAgents: async () => [agent("unity")],
  listSubagentDefs: async () => [agent("explorer")],
  listWorkspaceAgents: async () => [agent("project-agent")],
  listWorkspaceSubagentDefs: async () => [],
}));

function agent(id: string): AgentInfo {
  return { id, name: id, description: "", projectTypes: [], isDefault: true, source: "app" };
}

let app: App | undefined;
const scrollIntoViewDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollIntoView");
afterEach(() => {
  app?.unmount();
  app = undefined;
  document.body.innerHTML = "";
  if (scrollIntoViewDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", scrollIntoViewDescriptor);
  } else {
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
  }
  vi.restoreAllMocks();
});

async function selectModel(value: string) {
  const trigger = document.querySelector<HTMLButtonElement>(
    '[aria-label="explorer settings.models.subagentModel"]',
  );
  expect(trigger).not.toBeNull();
  trigger!.click();
  await nextTick();
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')]
    .find((element) => element.id.endsWith(`-option-${value}`));
  expect(option).toBeDefined();
  option!.click();
  await nextTick();
}

describe("sub-agent model defaults panel", () => {
  it("renders the app catalog during a workspace session and saves model overrides or inheritance", async () => {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true, value: vi.fn(),
    });
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useAgentStore();
    const defaults = ref<ModelDefaults>({
      mainModel: "main-model", planModel: "plan-model",
      subagentModels: { unity: "unity-model" },
      subagentEfforts: { explorer: "high" },
      subagentFastModes: { explorer: false },
    });
    const initial = JSON.parse(JSON.stringify(defaults.value));
    const model: ModelOption = {
      id: "openai/gpt-5.6-sol", name: "GPT-5.6-Sol", provider: "openai_codex",
      supportedEfforts: ["low", "medium", "high"],
    };
    const save = vi.fn(() => JSON.parse(JSON.stringify(defaults.value)));
    const host = document.createElement("div");
    document.body.append(host);
    app = createApp(() => h(ModelDefaultsPanel, {
      modelDefaults: defaults.value, allModels: [model],
      agents: store.appAgents, subagents: store.appSubagents, modelSaveMsg: "",
      "onUpdate:modelDefaults": (value: ModelDefaults) => { defaults.value = value; },
      onSave: save,
    }));
    app.use(pinia);
    app.mount(host);
    expect(host.querySelectorAll(".subagent-default-row")).toHaveLength(0);

    await Promise.all([store.loadAppAgents(), store.loadWorkspaceAgents({ checkoutId: "workspace" })]);
    await nextTick();
    expect([...host.querySelectorAll(".subagent-default-row .model-default-label")]
      .map((element) => element.textContent)).toEqual(["unity", "explorer"]);

    await selectModel(model.id);
    expect(save).toHaveReturnedWith({
      ...initial, subagentModels: { unity: "unity-model", explorer: model.id },
    });
    await store.loadAppAgents();
    await nextTick();
    expect(host.querySelector('[aria-label="explorer settings.models.subagentModel"]')?.textContent)
      .toContain("GPT-5.6-Sol");

    await selectModel("");
    expect(save).toHaveBeenCalledTimes(2);
    expect(save).toHaveLastReturnedWith(initial);
    expect(store.selectedAgentId).toBe("project-agent");
    expect(store.workspaceCheckoutId).toBe("workspace");
  });
});
