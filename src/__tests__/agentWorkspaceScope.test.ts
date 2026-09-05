import { readFileSync } from "node:fs";
import { resolve } from "node:path";
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
      { explorer: "openai/gpt-5.6-luna" },
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
      subagentModels: { explorer: "openai/gpt-5.6-luna" },
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

  it("binds the main Agent view and its detached window to the same workspace", () => {
    const root = process.cwd();
    const app = readFileSync(resolve(root, "src/App.vue"), "utf8");
    const view = readFileSync(resolve(root, "src/components/AgentView.vue"), "utf8");
    const detached = readFileSync(resolve(root, "src/components/WorkspacePageWindow.vue"), "utf8");

    expect(app).toContain(':workspace-ref="agentWorkspaceRef"');
    expect(app).toContain(':working-dir="agentWorkingDir"');
    expect(app).toContain('tab.id === "agent" ? agentWorkspaceRuntime.value');
    expect(app).toContain("const runtime = topTabWorkspaceRuntime(tab);");
    expect(app).toContain(':agent-list="[...agentStore.agents, ...agentStore.subagents]"');
    expect(view).toContain("listWorkspaceAgents(workspaceRef)");
    expect(view).toContain("listWorkspaceAgentInjectedItems(");
    expect(view).toContain("getWorkspaceAgentSystemPromptStats(");
    expect(detached).toContain("workspaceRef: checkoutWorkspaceRef.value");
  });

  it("keeps configured tools visible and renders runtime unavailability reasons only in details", () => {
    const root = process.cwd();
    const view = readFileSync(resolve(root, "src/components/AgentView.vue"), "utf8");

    expect(view).toContain('toolMetaBoolean(item.meta, "runtimeAvailable") !== false');
    expect(view).toContain('toolMetaString(item.meta, "unavailableReason")');
    expect(view).toContain("'tool-unavailable': !toolItemRuntimeAvailable(item)");
    expect(view).not.toContain('class="tool-unavailable-reason"');
    expect(view).not.toContain(':title="toolItemUnavailableReason(item) || undefined"');
    expect(view).toContain('class="tool-section tool-availability-section"');
    expect(view).toContain("{{ selectedToolUnavailableReason }}");
  });
});
