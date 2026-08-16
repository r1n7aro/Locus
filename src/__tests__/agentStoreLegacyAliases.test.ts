import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAgentStore } from "../stores/agent";

const agentServiceMocks = vi.hoisted(() => ({
  listAgents: vi.fn(),
  listSubagentDefs: vi.fn(),
}));

vi.mock("../services/agent", () => agentServiceMocks);

describe("Agent store legacy aliases", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    agentServiceMocks.listAgents.mockResolvedValue([
      {
        id: "dev",
        name: "Unity",
        description: "Unity development",
        isDefault: true,
        source: "app",
      },
    ]);
    agentServiceMocks.listSubagentDefs.mockResolvedValue([]);
  });

  it("routes retired built-in ids to Unity for historical sessions", async () => {
    const store = useAgentStore();
    await store.loadAgents();

    for (const id of ["git", "knowledge", "runtime_debugger", "doc", "wiki"]) {
      store.selectAgent(id);
      expect(store.selectedAgentId).toBe("dev");
    }
  });
});
