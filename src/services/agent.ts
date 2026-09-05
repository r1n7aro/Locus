import { ipcInvoke } from "./ipc";
import type {
  AgentInfo,
  AgentSystemPromptStats,
  InjectedPromptItem,
  KnowledgeAccessMode,
  RuleItem,
} from "../types";
import type { WorkspaceRef } from "./project";

export function listAgents(): Promise<AgentInfo[]> {
  return ipcInvoke<AgentInfo[]>("list_agents");
}

export function listSubagentDefs(): Promise<AgentInfo[]> {
  return ipcInvoke<AgentInfo[]>("list_subagent_defs");
}

export function listWorkspaceAgents(workspaceRef: WorkspaceRef): Promise<AgentInfo[]> {
  return ipcInvoke<AgentInfo[]>("list_workspace_agents", { workspaceRef });
}

export function listWorkspaceSubagentDefs(workspaceRef: WorkspaceRef): Promise<AgentInfo[]> {
  return ipcInvoke<AgentInfo[]>("list_workspace_subagent_defs", { workspaceRef });
}

export function getAgentSystemPrompt(agentId: string): Promise<string> {
  return ipcInvoke<string>("get_agent_system_prompt", { agentId });
}

export function getWorkspaceAgentSystemPrompt(workspaceRef: WorkspaceRef, agentId: string): Promise<string> {
  return ipcInvoke<string>("get_workspace_agent_system_prompt", { workspaceRef, agentId });
}

export function getAgentEnvTemplate(agentId: string): Promise<string> {
  return ipcInvoke<string>("get_agent_env_template", { agentId });
}

export function getWorkspaceAgentEnvTemplate(workspaceRef: WorkspaceRef, agentId: string): Promise<string> {
  return ipcInvoke<string>("get_workspace_agent_env_template", { workspaceRef, agentId });
}

export function getAgentRenderedEnvPrompt(agentId: string): Promise<string> {
  return ipcInvoke<string>("get_agent_rendered_env_prompt", { agentId });
}

export function getWorkspaceAgentRenderedEnvPrompt(
  workspaceRef: WorkspaceRef,
  agentId: string,
  selectedModel?: string | null,
): Promise<string> {
  return ipcInvoke<string>("get_workspace_agent_rendered_env_prompt", {
    workspaceRef,
    agentId,
    selectedModel: selectedModel ?? null,
  });
}

export function getAgentSystemPromptStats(agentId: string): Promise<AgentSystemPromptStats> {
  return ipcInvoke<AgentSystemPromptStats>("get_agent_system_prompt_stats", { agentId });
}

export function getWorkspaceAgentSystemPromptStats(
  workspaceRef: WorkspaceRef,
  agentId: string,
  selectedModel?: string | null,
): Promise<AgentSystemPromptStats> {
  return ipcInvoke<AgentSystemPromptStats>("get_workspace_agent_system_prompt_stats", {
    workspaceRef,
    agentId,
    selectedModel: selectedModel ?? null,
  });
}

export function listAgentInjectedItems(
  agentId: string,
  knowledgeMode?: KnowledgeAccessMode | null,
  selectedModel?: string | null,
  subagentModels?: Record<string, string> | null,
): Promise<InjectedPromptItem[]> {
  return ipcInvoke<InjectedPromptItem[]>("list_agent_injected_items", {
    agentId,
    knowledgeMode: knowledgeMode ?? null,
    selectedModel: selectedModel ?? null,
    subagentModels: subagentModels ?? null,
  });
}

export function listWorkspaceAgentInjectedItems(
  workspaceRef: WorkspaceRef,
  agentId: string,
  knowledgeMode?: KnowledgeAccessMode | null,
  selectedModel?: string | null,
  subagentModels?: Record<string, string> | null,
): Promise<InjectedPromptItem[]> {
  return ipcInvoke<InjectedPromptItem[]>("list_workspace_agent_injected_items", {
    workspaceRef,
    agentId,
    knowledgeMode: knowledgeMode ?? null,
    selectedModel: selectedModel ?? null,
    subagentModels: subagentModels ?? null,
  });
}

export function setAgentInjectionEnabled(
  workspaceRef: WorkspaceRef,
  agentId: string,
  injectionId: string,
  enabled: boolean,
): Promise<void> {
  return ipcInvoke("set_agent_injection_enabled", { workspaceRef, agentId, injectionId, enabled });
}

export function setAgentToolDirectLoad(workspaceRef: WorkspaceRef, agentId: string, toolName: string, directLoad: boolean): Promise<void> {
  return ipcInvoke("set_agent_tool_direct_load", { workspaceRef, agentId, toolName, directLoad });
}

export function setAgentToolEnabled(workspaceRef: WorkspaceRef, agentId: string, toolName: string, enabled: boolean): Promise<void> {
  return ipcInvoke("set_agent_tool_enabled", { workspaceRef, agentId, toolName, enabled });
}

export function listRules(workspaceRef: WorkspaceRef, agentId: string): Promise<RuleItem[]> {
  return ipcInvoke<RuleItem[]>("list_rules", { workspaceRef, agentId });
}

export function listAppRules(agentId: string): Promise<RuleItem[]> {
  return ipcInvoke<RuleItem[]>("list_app_rules", { agentId });
}

export function readRule(workspaceRef: WorkspaceRef, agentId: string, ruleKey: string): Promise<string> {
  return ipcInvoke<string>("read_rule", { workspaceRef, agentId, fileName: ruleKey });
}

export function readAppRule(agentId: string, ruleKey: string): Promise<string> {
  return ipcInvoke<string>("read_app_rule", { agentId, fileName: ruleKey });
}

export function saveRule(workspaceRef: WorkspaceRef, agentId: string, fileName: string, content: string): Promise<RuleItem> {
  return ipcInvoke<RuleItem>("save_rule", { workspaceRef, agentId, fileName, content });
}

export function deleteRule(workspaceRef: WorkspaceRef, agentId: string, fileName: string): Promise<void> {
  return ipcInvoke("delete_rule", { workspaceRef, agentId, fileName });
}

export function setRuleEnabled(workspaceRef: WorkspaceRef, agentId: string, ruleKey: string, enabled: boolean): Promise<void> {
  return ipcInvoke("set_rule_enabled", { workspaceRef, agentId, fileName: ruleKey, enabled });
}

export function setRuleOrder(workspaceRef: WorkspaceRef, agentId: string, ruleKeys: string[]): Promise<void> {
  return ipcInvoke("set_rule_order", { workspaceRef, agentId, fileNames: ruleKeys });
}
