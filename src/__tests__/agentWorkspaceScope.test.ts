
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import {
  getWorkspaceAgentEnvTemplate,
  getWorkspaceAgentRenderedEnvPrompt,
  getWorkspaceAgentSystemPrompt,
  getWorkspaceAgentSystemPromptStats,
  listAppRules,
  listRules,
  listWorkspaceAgentInjectedItems,
  listWorkspaceAgents,
  listWorkspaceSubagentDefs,
  readAppRule,
  readRule,
} from "../services/agent";

const mockedInvoke = vi.mocked(ipcInvoke);
const workspaceRef = {
  checkoutId: "checkout-agent",
  expectedGeneration: 17,
};

describe("Agent workspace scope", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(undefined);
  });

  it("forwards the active checkout through effective Agent preview IPC", async () => {
    await listWorkspaceAgents(workspaceRef);
    await listWorkspaceSubagentDefs(workspaceRef);
    await getWorkspaceAgentSystemPrompt(workspaceRef, "unity");
    await getWorkspaceAgentEnvTemplate(workspaceRef, "unity");
    await getWorkspaceAgentRenderedEnvPrompt(workspaceRef, "unity", "openai/gpt-5.6");
    await getWorkspaceAgentSystemPromptStats(workspaceRef, "unity", "openai/gpt-5.6");
    await listWorkspaceAgentInjectedItems(
      workspaceRef,
      "unity",
      null,
      "openai/gpt-5.6-sol",
      { explorer: ["openai/gpt-5.6-luna"] },
    );
    await listRules(workspaceRef, "unity");
    await readRule(workspaceRef, "unity", "workflow.md");

    for (const call of mockedInvoke.mock.calls) {
      expect(call[1]).toMatchObject({ workspaceRef });
    }
    expect(mockedInvoke).toHaveBeenCalledWith("get_workspace_agent_rendered_env_prompt", {
      workspaceRef,
      agentId: "unity",
      selectedModel: "openai/gpt-5.6",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("get_workspace_agent_system_prompt_stats", {
      workspaceRef,
      agentId: "unity",
      selectedModel: "openai/gpt-5.6",
    });
    expect(mockedInvoke).toHaveBeenCalledWith("list_workspace_agent_injected_items", {
      workspaceRef,
      agentId: "unity",
      knowledgeMode: null,
      selectedModel: "openai/gpt-5.6-sol",
      subagentModels: { explorer: ["openai/gpt-5.6-luna"] },
    });
  });

  it("keeps app Agent rules available without an active checkout", async () => {
    await listAppRules("simple");
    await readAppRule("simple", "baseline.md");

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "list_app_rules", { agentId: "simple" });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "read_app_rule", {
      agentId: "simple",
      fileName: "baseline.md",
    });
  });
});
