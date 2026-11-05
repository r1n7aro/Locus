import type { ToolCallDisplay } from "../types";
import type { ClaudeAgentRoutes } from "../services/claudeAgentRoutes";

/** Delegation cards: Locus subagent tools and Claude Code's single Agent entry. */
export function isDelegationToolName(name: string): boolean {
  return name === "explore" || name === "subagent" || name === "task" || name === "Agent";
}

function parseArgs(argumentsJson: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(argumentsJson);
    return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
  } catch {
    return null;
  }
}

/** The agent type a delegation card runs: Locus-published types drop their
 *  `locus:` namespace; Claude's Agent defaults to general-purpose. */
export function delegationDisplayName(name: string, argumentsJson: string): string {
  const type = parseArgs(argumentsJson)?.subagent_type;
  const subagentType = typeof type === "string" && type.trim() ? type.trim() : "";
  if (name === "Agent") return subagentType.replace(/^locus:/, "") || "general-purpose";
  return subagentType || name;
}

export type AgentBackend = "claude" | "locus";

/** Progress title the host sets on an Agent card it runs as a Locus subagent. */
export const LOCUS_AGENT_PROGRESS_TITLE = "locus-agent";

/** Who runs a Claude Code Agent call: Locus types announce themselves while
 *  running and keep the Locus task id in their result; every other Agent is
 *  a native Claude subagent. Refused calls ran nowhere. */
export function agentBackend(toolCall: Pick<ToolCallDisplay, "name" | "status" | "output" | "progress">): AgentBackend | null {
  if (toolCall.name !== "Agent") return null;
  if (toolCall.progress?.title === LOCUS_AGENT_PROGRESS_TITLE || (toolCall.output ?? "").startsWith("Task id: ")) {
    return "locus";
  }
  return toolCall.status === "error" ? null : "claude";
}

/** A model id's short label, as the host labels subagent models. */
export function modelDisplayLabel(model: string): string {
  const id = model.trim();
  const prefix = ["openai/", "openrouter/", "claude_code/"].find((value) => id.startsWith(value));
  return prefix ? id.slice(prefix.length) : id;
}

export interface DelegationRoute {
  backend: AgentBackend;
  /** Short label and full id; `null` when the backend picks the model. */
  model: string | null;
  modelId: string | null;
  /** A Claude Code builtin type: Claude Code chooses its model. */
  builtin: boolean;
}

/** Backend and model a delegation card runs on. An Agent card follows its
 *  conversation's pinned route for the subagent_type (a builtin's model is
 *  Claude Code's choice); a Locus subagent card shows the model it asked
 *  for. Refused Agent calls ran nowhere. */
export function delegationRoute(
  toolCall: Pick<ToolCallDisplay, "name" | "arguments" | "status" | "output" | "progress">,
  routes: ClaudeAgentRoutes | null,
): DelegationRoute | null {
  if (toolCall.name === "subagent" || toolCall.name === "task") {
    const model = parseArgs(toolCall.arguments)?.model;
    const id = typeof model === "string" && model.trim() ? model.trim() : null;
    return { backend: "locus", model: id && modelDisplayLabel(id), modelId: id, builtin: false };
  }
  const ran = agentBackend(toolCall);
  if (!ran) return null;
  const type = parseArgs(toolCall.arguments)?.subagent_type;
  const subagentType = typeof type === "string" && type.trim() ? type.trim() : "general-purpose";
  const route = routes?.types.find((entry) => entry.subagentType === subagentType || entry.subagentType === `locus:${subagentType}`);
  if (route) {
    return {
      backend: route.backend === "locus" ? "locus" : "claude",
      model: route.modelLabel || modelDisplayLabel(route.model),
      modelId: route.model,
      builtin: false,
    };
  }
  // Before the routes load: a running Locus card reports its model.
  const progressModel = toolCall.progress?.title === LOCUS_AGENT_PROGRESS_TITLE ? toolCall.progress.info.trim() : "";
  return {
    backend: ran,
    model: progressModel ? modelDisplayLabel(progressModel) : null,
    modelId: progressModel || null,
    builtin: ran === "claude" && (routes?.builtins.includes(subagentType) ?? !subagentType.startsWith("locus:")),
  };
}
