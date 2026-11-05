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
  // The multi-select menu stays open between selections.
  if (trigger!.getAttribute("aria-expanded") !== "true") {
    trigger!.click();
    await nextTick();
  }
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
      subagentModels: { unity: ["unity-model"] },
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
      ...initial, subagentModels: { unity: ["unity-model"], explorer: [model.id] },
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

  it("selects several allowed models, reorders the default, and limits shared options", async () => {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true, value: vi.fn(),
    });
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useAgentStore();
    const defaults = ref<ModelDefaults>({
      mainModel: "", planModel: "", subagentModels: {}, subagentEfforts: {}, subagentFastModes: {},
    });
    const sol: ModelOption = {
      id: "openai/gpt-5.6-sol", name: "GPT-5.6-Sol", provider: "openai_codex",
      supportedEfforts: ["low", "medium", "high"],
    };
    const luna: ModelOption = {
      id: "custom/fixture/luna", name: "Luna", provider: "custom",
      supportedEfforts: ["medium", "high", "max"],
    };
    const save = vi.fn(() => JSON.parse(JSON.stringify(defaults.value.subagentModels)));
    const host = document.createElement("div");
    document.body.append(host);
    app = createApp(() => h(ModelDefaultsPanel, {
      modelDefaults: defaults.value, allModels: [sol, luna],
      agents: store.appAgents, subagents: store.appSubagents, modelSaveMsg: "",
      "onUpdate:modelDefaults": (value: ModelDefaults) => { defaults.value = value; },
      onSave: save,
    }));
    app.use(pinia);
    app.mount(host);
    await store.loadAppAgents();
    await nextTick();

    await selectModel(sol.id);
    await selectModel(luna.id);
    expect(save).toHaveLastReturnedWith({ explorer: [sol.id, luna.id] });
    const selected = [...document.querySelectorAll<HTMLElement>('[role="option"][aria-selected="true"]')]
      .map((element) => element.textContent?.trim());
    expect(selected).toHaveLength(2);
    expect(selected[0]).toContain("settings.models.subagentModelDefaultHint");

    const makeDefault = [...document.querySelectorAll<HTMLButtonElement>(".subagent-model-order-row button")];
    expect(makeDefault).toHaveLength(1);
    makeDefault[0].click();
    await nextTick();
    expect(save).toHaveLastReturnedWith({ explorer: [luna.id, sol.id] });

    const effortTrigger = host.querySelector<HTMLButtonElement>(
      '[aria-label="explorer settings.models.subagentEffort"]',
    );
    effortTrigger!.click();
    await nextTick();
    const effortList = document.getElementById(effortTrigger!.getAttribute("aria-controls") ?? "");
    const efforts = [...effortList!.querySelectorAll<HTMLElement>('[role="option"]')]
      .map((element) => element.id.replace(/^.*-option-/, ""));
    expect(efforts).toEqual(["", "medium", "high"]);
    effortTrigger!.click();
    await nextTick();

    await selectModel(luna.id);
    expect(save).toHaveLastReturnedWith({ explorer: [sol.id] });
    await selectModel("");
    expect(save).toHaveLastReturnedWith({});
  });
});
