import { ref } from "vue";
import { defineStore } from "pinia";
import * as agentService from "../services/agent";
import type { AgentInfo } from "../types";
import type { WorkspaceRef } from "../services/project";

export const useAgentStore = defineStore("agent", () => {
  const agents = ref<AgentInfo[]>([]);
  const subagents = ref<AgentInfo[]>([]);
  const appAgents = ref<AgentInfo[]>([]);
  const appSubagents = ref<AgentInfo[]>([]);
  const workspaceCheckoutId = ref<string | null>(null);
  const selectedAgentId = ref("");
  let agentLoadEpoch = 0;

  function resolveAgentId(id: string) {
    const trimmed = id.trim();
    if (!trimmed || trimmed === "dev") return "";
    if (["doc", "wiki", "git", "knowledge", "runtime_debugger"].includes(trimmed)) {
      return agents.value.some((agent) => agent.id === "unity") ? "unity" : trimmed;
    }
    return trimmed;
  }

  async function loadAgents() {
    const epoch = ++agentLoadEpoch;
    try {
      const [list, subList] = await Promise.all([
        agentService.listAgents(),
        agentService.listSubagentDefs(),
      ]);
      if (epoch !== agentLoadEpoch) return;
      agents.value = list;
      subagents.value = subList;
      appAgents.value = list;
      appSubagents.value = subList;
      workspaceCheckoutId.value = null;
      const resolvedCurrent = resolveAgentId(selectedAgentId.value);
      if (resolvedCurrent && list.some((agent) => agent.id === resolvedCurrent)) {
        selectedAgentId.value = resolvedCurrent;
        return;
      }
      const def = list.find((a) => a.isDefault);
      if (def) selectedAgentId.value = def.id;
      else if (list.length > 0) selectedAgentId.value = list[0].id;
    } catch (e) {
      console.error("list_agents failed:", e);
    }
  }

  async function loadWorkspaceAgents(workspaceRef: WorkspaceRef) {
    const epoch = ++agentLoadEpoch;
    const targetCheckoutId = workspaceRef.checkoutId;
    const workspaceChanged = workspaceCheckoutId.value !== targetCheckoutId;
    try {
      const [list, subList] = await Promise.all([
        agentService.listWorkspaceAgents(workspaceRef),
        agentService.listWorkspaceSubagentDefs(workspaceRef),
      ]);
      if (epoch !== agentLoadEpoch) return;
      agents.value = list;
      subagents.value = subList;
      workspaceCheckoutId.value = targetCheckoutId;
      const resolvedCurrent = resolveAgentId(selectedAgentId.value);
      if (
        !workspaceChanged
        && resolvedCurrent
        && list.some((agent) => agent.id === resolvedCurrent)
      ) {
        selectedAgentId.value = resolvedCurrent;
        return;
      }
      const fallback = list.find((agent) => agent.isDefault) ?? list[0];
      selectedAgentId.value = fallback?.id ?? "";
    } catch (e) {
      console.error("list_workspace_agents failed:", e);
    }
  }

  function useAppAgents() {
    agentLoadEpoch += 1;
    agents.value = appAgents.value;
    subagents.value = appSubagents.value;
    workspaceCheckoutId.value = null;
  }

  function selectAgent(id: string) {
    selectedAgentId.value = resolveAgentId(id);
  }

  function resetToDefault() {
    const def = agents.value.find((a) => a.isDefault);
    if (def) selectedAgentId.value = def.id;
    else if (agents.value.length > 0) selectedAgentId.value = agents.value[0].id;
  }

  return {
    agents,
    subagents,
    appAgents,
    appSubagents,
    workspaceCheckoutId,
    selectedAgentId,
    loadAgents,
    loadWorkspaceAgents,
    useAppAgents,
    selectAgent,
    resetToDefault,
  };
});
