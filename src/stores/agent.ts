import { ref } from "vue";
import { defineStore } from "pinia";
import * as agentService from "../services/agent";
import type { AgentInfo } from "../types";

export const useAgentStore = defineStore("agent", () => {
  const agents = ref<AgentInfo[]>([]);
  const subagents = ref<AgentInfo[]>([]);
  const selectedAgentId = ref("");

  function resolveAgentId(id: string) {
    const trimmed = id.trim();
    if (!trimmed) return "";
    if (trimmed === "doc" || trimmed === "wiki") {
      return agents.value.some((agent) => agent.id === "knowledge") ? "knowledge" : trimmed;
    }
    return trimmed;
  }

  async function loadAgents() {
    try {
      const [list, subList] = await Promise.all([
        agentService.listAgents(),
        agentService.listSubagentDefs(),
      ]);
      agents.value = list;
      subagents.value = subList;
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
    selectedAgentId,
    loadAgents,
    selectAgent,
    resetToDefault,
  };
});
