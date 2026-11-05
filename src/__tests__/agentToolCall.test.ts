import { describe, expect, it } from "vitest";
import {
  agentBackend,
  delegationDisplayName,
  delegationRoute,
  isDelegationToolName,
  LOCUS_AGENT_PROGRESS_TITLE,
  modelDisplayLabel,
} from "../components/agentToolCall";
import type { ClaudeAgentRoutes } from "../services/claudeAgentRoutes";

describe("Claude Code Agent cards", () => {
  it("treat Agent as a delegation card alongside the Locus subagent tools", () => {
    for (const name of ["Agent", "subagent", "task", "explore"]) expect(isDelegationToolName(name)).toBe(true);
    for (const name of ["SendMessage", "send_message", "read"]) expect(isDelegationToolName(name)).toBe(false);
  });

  it("name the agent type without the Locus namespace", () => {
    expect(delegationDisplayName("Agent", JSON.stringify({ subagent_type: "locus:explorer-sonnet" }))).toBe("explorer-sonnet");
    expect(delegationDisplayName("Agent", JSON.stringify({ subagent_type: "Explore" }))).toBe("Explore");
    expect(delegationDisplayName("Agent", JSON.stringify({ prompt: "Find it" }))).toBe("general-purpose");
    expect(delegationDisplayName("Agent", "{not json")).toBe("general-purpose");
    expect(delegationDisplayName("subagent", JSON.stringify({ subagent_type: "explorer" }))).toBe("explorer");
    expect(delegationDisplayName("subagent", "{}")).toBe("subagent");
  });

  it("label the backend from the running progress or the stored Locus task id", () => {
    const card = { name: "Agent", status: "running" as const, output: undefined, progress: null };
    expect(agentBackend(card)).toBe("claude");
    expect(agentBackend({ ...card, progress: { title: LOCUS_AGENT_PROGRESS_TITLE, info: "custom/luna", state: "running" } })).toBe("locus");
    expect(agentBackend({ ...card, status: "done", output: "Task id: t-1\nFound it." })).toBe("locus");
    expect(agentBackend({ ...card, status: "done", output: "Found it." })).toBe("claude");
    // A refused Agent call ran on neither backend.
    expect(agentBackend({ ...card, status: "error", output: "Unknown subagent_type" })).toBeNull();
    expect(agentBackend({ ...card, name: "subagent" })).toBeNull();
  });

  it("show the backend and model the conversation pinned for the subagent type", () => {
    const routes: ClaudeAgentRoutes = {
      types: [
        { subagentType: "locus:explorer-sonnet", agentType: "explorer", backend: "claudeNative", model: "claude_code/sonnet", modelLabel: "sonnet" },
        { subagentType: "locus:explorer-luna", agentType: "explorer", backend: "locus", model: "custom/child/luna", modelLabel: "custom/child/luna" },
      ],
      builtins: ["general-purpose", "Explore"],
    };
    const agent = (subagentType: string | null, extra: object = {}) => ({
      name: "Agent", status: "running" as const, output: undefined, progress: null,
      arguments: JSON.stringify(subagentType ? { subagent_type: subagentType, prompt: "Find it" } : { prompt: "Find it" }), ...extra,
    });
    expect(delegationRoute(agent("locus:explorer-sonnet"), routes))
      .toEqual({ backend: "claude", model: "sonnet", modelId: "claude_code/sonnet", builtin: false });
    // Claude may call a published type without its namespace.
    expect(delegationRoute(agent("explorer-luna"), routes))
      .toEqual({ backend: "locus", model: "custom/child/luna", modelId: "custom/child/luna", builtin: false });
    // Builtins, including the default type: Claude Code chooses the model.
    expect(delegationRoute(agent("Explore"), routes)).toEqual({ backend: "claude", model: null, modelId: null, builtin: true });
    expect(delegationRoute(agent(null), routes)).toMatchObject({ backend: "claude", builtin: true });
    // Before the routes load, a running Locus card reports its model.
    const running = agent("locus:explorer-luna", { progress: { title: LOCUS_AGENT_PROGRESS_TITLE, info: "openrouter/openai/gpt-5", state: "running" } });
    expect(delegationRoute(running, null)).toEqual({ backend: "locus", model: "openai/gpt-5", modelId: "openrouter/openai/gpt-5", builtin: false });
    // A refused call ran nowhere, whatever its route.
    expect(delegationRoute(agent("locus:explorer-sonnet", { status: "error", output: "Unknown subagent_type" }), routes)).toBeNull();
  });

  it("show the model a Locus subagent card asked for", () => {
    const card = { name: "subagent", status: "done" as const, output: "Done", progress: null };
    expect(delegationRoute({ ...card, arguments: JSON.stringify({ subagent_type: "explorer", model: "openrouter/openai/gpt-5" }) }, null))
      .toEqual({ backend: "locus", model: "openai/gpt-5", modelId: "openrouter/openai/gpt-5", builtin: false });
    expect(delegationRoute({ ...card, arguments: JSON.stringify({ subagent_type: "explorer" }) }, null))
      .toEqual({ backend: "locus", model: null, modelId: null, builtin: false });
    expect(modelDisplayLabel(" claude_code/opus ")).toBe("opus");
    expect(modelDisplayLabel("custom/child/luna")).toBe("custom/child/luna");
  });
});
