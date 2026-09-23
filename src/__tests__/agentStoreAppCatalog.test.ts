import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAgentStore } from "../stores/agent";
import type { AgentInfo } from "../types";

const services = vi.hoisted(() => ({
  listAgents: vi.fn(),
  listSubagentDefs: vi.fn(),
  listWorkspaceAgents: vi.fn(),
  listWorkspaceSubagentDefs: vi.fn(),
}));
vi.mock("../services/agent", () => services);

function agent(id: string, source = "app"): AgentInfo {
  return { id, name: id, description: "", projectTypes: [], isDefault: true, source };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

const appAgents = [agent("unity")];
const appSubagents = [agent("explorer")];
const workspaceAgents = [agent("project-agent", "workspace")];
const workspaceSubagents = [agent("project-explorer", "workspace")];
const workspace = { checkoutId: "checkout-a" };

describe("app Agent catalog for model defaults", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    services.listAgents.mockResolvedValue(appAgents);
    services.listSubagentDefs.mockResolvedValue(appSubagents);
    services.listWorkspaceAgents.mockResolvedValue(workspaceAgents);
    services.listWorkspaceSubagentDefs.mockResolvedValue(workspaceSubagents);
  });

  it.each(["app", "workspace"])("loads both catalogs when %s finishes first", async (first) => {
    const appResult = deferred<AgentInfo[]>();
    const workspaceResult = deferred<AgentInfo[]>();
    services.listAgents.mockReturnValue(appResult.promise);
    services.listWorkspaceAgents.mockReturnValue(workspaceResult.promise);
    const store = useAgentStore();
    const appLoad = store.loadAppAgents();
    const workspaceLoad = store.loadWorkspaceAgents(workspace);

    if (first === "app") {
      appResult.resolve(appAgents);
      await appLoad;
      workspaceResult.resolve(workspaceAgents);
    } else {
      workspaceResult.resolve(workspaceAgents);
      await workspaceLoad;
      appResult.resolve(appAgents);
    }
    await Promise.all([appLoad, workspaceLoad]);

    expect(store.appAgents).toEqual(appAgents);
    expect(store.appSubagents).toEqual(appSubagents);
    expect(store.agents).toEqual(workspaceAgents);
    expect(store.subagents).toEqual(workspaceSubagents);
    expect(store.workspaceCheckoutId).toBe(workspace.checkoutId);
    expect(store.selectedAgentId).toBe("project-agent");
  });

  it("refreshes settings definitions without resetting a manual workspace selection", async () => {
    services.listWorkspaceAgents.mockResolvedValue([...workspaceAgents, agent("manual", "workspace")]);
    const store = useAgentStore();
    await store.loadWorkspaceAgents(workspace);
    store.selectAgent("manual");

    await store.loadAppAgents();

    expect(store.appAgents).toEqual(appAgents);
    expect(store.appSubagents).toEqual(appSubagents);
    expect(store.agents.map((item) => item.id)).toEqual(["project-agent", "manual"]);
    expect(store.subagents).toEqual(workspaceSubagents);
    expect(store.workspaceCheckoutId).toBe(workspace.checkoutId);
    expect(store.selectedAgentId).toBe("manual");
  });

  it("still fills settings when workspace activation supersedes a global selection load", async () => {
    const appResult = deferred<AgentInfo[]>();
    services.listAgents.mockReturnValue(appResult.promise);
    const store = useAgentStore();
    const pending = store.loadAgents();
    await store.loadWorkspaceAgents(workspace);
    appResult.resolve(appAgents);
    await pending;

    expect(store.appAgents).toEqual(appAgents);
    expect(store.appSubagents).toEqual(appSubagents);
    expect(store.agents).toEqual(workspaceAgents);
    expect(store.workspaceCheckoutId).toBe(workspace.checkoutId);
    expect(store.selectedAgentId).toBe("project-agent");
  });

  it("keeps the latest app definitions when refreshes finish out of order", async () => {
    const stale = deferred<AgentInfo[]>();
    services.listAgents.mockReturnValueOnce(stale.promise);
    const store = useAgentStore();
    const pending = store.loadAppAgents();
    await store.loadAppAgents();
    stale.resolve([agent("removed-agent")]);
    await pending;

    expect(store.appAgents).toEqual(appAgents);
    expect(store.appSubagents).toEqual(appSubagents);
  });
});
