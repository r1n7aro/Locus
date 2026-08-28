import { t } from "../i18n";
import type { AgentInfo } from "../types";

export function projectTypeLabel(projectType: string): string {
  if (projectType === "unity") return "Unity";
  if (projectType === "generic") return t("agent.projectType.generic");
  return projectType;
}

export function agentProjectTypesLabel(agent: AgentInfo): string {
  return agent.projectTypes.map(projectTypeLabel).join(" / ");
}
