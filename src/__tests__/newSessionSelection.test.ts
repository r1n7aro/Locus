import { describe, expect, it } from "vitest";
import { clampEffortToLevels, resolveNewSessionAgentId, resolveNewSessionModel } from "../stores/newSessionSelection";
import type { WorkspaceSelectionSnapshot } from "../services/selectionMemory";

const agents = [
  { id: "unity", isDefault: true },
  { id: "simple", isDefault: false },
  { id: "custom", isDefault: false },
];
const models = [{ id: "m-default", isDefault: true }, { id: "m-a" }, { id: "m-b" }, { id: "m-c" }];

function snapshot(overrides: Partial<WorkspaceSelectionSnapshot["memory"]> = {}, extra: Partial<WorkspaceSelectionSnapshot> = {}): WorkspaceSelectionSnapshot {
  return {
    projectId: "p",
    memory: { agentId: "", modelId: "", effort: "", agents: {}, updatedAtMs: 0, ...overrides },
    lastAgentId: "",
    workspaceDefaultAgentId: null,
    ...extra,
  };
}

describe("resolveNewSessionAgentId", () => {
  it("prefers the workspace's remembered Agent, then its explicit default, then the last Agent anywhere", () => {
    expect(resolveNewSessionAgentId({ agents, mode: "remember", snapshot: snapshot({ agentId: "simple" }, { workspaceDefaultAgentId: "custom" }) })).toBe("simple");
    expect(resolveNewSessionAgentId({ agents, mode: "remember", snapshot: snapshot({}, { workspaceDefaultAgentId: "custom", lastAgentId: "simple" }) })).toBe("custom");
    expect(resolveNewSessionAgentId({ agents, mode: "remember", snapshot: snapshot({}, { lastAgentId: "simple" }) })).toBe("simple");
    expect(resolveNewSessionAgentId({ agents, mode: "remember", snapshot: null, legacyAgentId: "custom" })).toBe("custom");
    expect(resolveNewSessionAgentId({ agents, mode: "remember", snapshot: null })).toBe("unity");
  });

  it("skips remembered Agents that this workspace does not offer", () => {
    expect(resolveNewSessionAgentId({ agents, mode: "remember", snapshot: snapshot({ agentId: "gone" }, { lastAgentId: "simple" }) })).toBe("simple");
  });

  it("ignores memory when following the workspace default or a fixed Agent", () => {
    const remembered = snapshot({ agentId: "simple" });
    expect(resolveNewSessionAgentId({ agents, mode: "workspace", snapshot: remembered })).toBe("unity");
    expect(resolveNewSessionAgentId({ agents, mode: "fixed", fixedAgentId: "custom", snapshot: remembered })).toBe("custom");
    expect(resolveNewSessionAgentId({ agents, mode: "fixed", fixedAgentId: "gone", snapshot: remembered })).toBe("unity");
  });
});

describe("resolveNewSessionModel", () => {
  it("uses a pinned model first, keeping its remembered effort", () => {
    const resolved = resolveNewSessionModel({
      models,
      agentId: "unity",
      pinnedModelId: "m-b",
      snapshot: snapshot({ modelId: "m-a", effort: "high", agents: { unity: { modelId: "m-b", effort: "low" } } }),
    });
    expect(resolved).toEqual({ modelId: "m-b", effort: "low" });
  });

  it("prefers the Agent's model in this workspace, then the workspace's last model, then global fallbacks", () => {
    const memory = snapshot({ agentId: "unity", modelId: "m-a", effort: "high", agents: { simple: { modelId: "m-b", effort: "medium" } } });
    expect(resolveNewSessionModel({ models, agentId: "simple", snapshot: memory })).toEqual({ modelId: "m-b", effort: "medium" });
    expect(resolveNewSessionModel({ models, agentId: "unity", snapshot: memory })).toEqual({ modelId: "m-a", effort: "high" });
    expect(resolveNewSessionModel({
      models,
      agentId: "custom",
      snapshot: null,
      agentPreferences: { custom: { modelId: "m-c", effort: "xhigh" } },
      lastModelId: "m-a",
    })).toEqual({ modelId: "m-c", effort: "xhigh" });
    expect(resolveNewSessionModel({ models, agentId: "custom", snapshot: null, lastModelId: "m-a" })).toEqual({ modelId: "m-a", effort: null });
    expect(resolveNewSessionModel({ models, agentId: "custom", snapshot: null })).toEqual({ modelId: "m-default", effort: null });
  });

  it("skips unavailable models and returns nothing without a catalog", () => {
    expect(resolveNewSessionModel({ models, agentId: "unity", pinnedModelId: "gone", snapshot: snapshot({ modelId: "gone2" }), lastModelId: "m-c" }).modelId).toBe("m-c");
    expect(resolveNewSessionModel({ models: [], agentId: "unity", lastModelId: "m-c" })).toEqual({ modelId: "", effort: null });
  });
});

describe("clampEffortToLevels", () => {
  it("keeps supported levels and passes through when a model has no effort control", () => {
    expect(clampEffortToLevels(["none", "low", "high", "xhigh"], "xhigh")).toBe("xhigh");
    expect(clampEffortToLevels([], "xhigh")).toBe("xhigh");
  });

  it("falls back to the nearest lower level, never to the first listed level", () => {
    expect(clampEffortToLevels(["none", "low", "medium", "high", "max"], "xhigh")).toBe("high");
    expect(clampEffortToLevels(["none", "low", "medium", "high"], "max")).toBe("high");
  });

  it("uses the lowest supported level when nothing lower exists", () => {
    expect(clampEffortToLevels(["low", "medium", "high"], "none")).toBe("low");
    expect(clampEffortToLevels(["medium", "high"], "low")).toBe("medium");
  });
});
