import { ipcInvoke } from "./ipc";
import type {
  AgentInfo,
  AgentSystemPromptStats,
  InjectedPromptItem,
  KnowledgeAccessMode,
  RuleItem,
} from "../types";

export function listAgents(): Promise<AgentInfo[]> {
  return ipcInvoke<AgentInfo[]>("list_agents");
}

export function listSubagentDefs(): Promise<AgentInfo[]> {
  return ipcInvoke<AgentInfo[]>("list_subagent_defs");
}

export function getAgentSystemPrompt(agentId: string): Promise<string> {
  return ipcInvoke<string>("get_agent_system_prompt", { agentId });
}

export function getAgentEnvTemplate(agentId: string): Promise<string> {
  return ipcInvoke<string>("get_agent_env_template", { agentId });
}

export function getAgentRenderedEnvPrompt(agentId: string): Promise<string> {
  return ipcInvoke<string>("get_agent_rendered_env_prompt", { agentId });
}

export function getAgentSystemPromptStats(agentId: string): Promise<AgentSystemPromptStats> {
  return ipcInvoke<AgentSystemPromptStats>("get_agent_system_prompt_stats", { agentId });
}

export function listAgentInjectedItems(
  agentId: string,
  knowledgeMode?: KnowledgeAccessMode | null,
): Promise<InjectedPromptItem[]> {
  return ipcInvoke<InjectedPromptItem[]>("list_agent_injected_items", {
    agentId,
    knowledgeMode: knowledgeMode ?? null,
  });
}

export function setAgentToolDirectLoad(agentId: string, toolName: string, directLoad: boolean): Promise<void> {
  return ipcInvoke("set_agent_tool_direct_load", { agentId, toolName, directLoad });
}

export function listRules(agentId: string): Promise<RuleItem[]> {
  return ipcInvoke<RuleItem[]>("list_rules", { agentId });
}

export function readRule(agentId: string, fileName: string): Promise<string> {
  return ipcInvoke<string>("read_rule", { agentId, fileName });
}

export function saveRule(agentId: string, fileName: string, content: string): Promise<RuleItem> {
  return ipcInvoke<RuleItem>("save_rule", { agentId, fileName, content });
}

export function deleteRule(agentId: string, fileName: string): Promise<void> {
  return ipcInvoke("delete_rule", { agentId, fileName });
}

export function setRuleEnabled(agentId: string, fileName: string, enabled: boolean): Promise<void> {
  return ipcInvoke("set_rule_enabled", { agentId, fileName, enabled });
}

export function setRuleOrder(agentId: string, fileNames: string[]): Promise<void> {
  return ipcInvoke("set_rule_order", { agentId, fileNames });
}
