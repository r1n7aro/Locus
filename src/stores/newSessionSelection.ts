import type { AgentInfo, AgentModelPreference, EffortLevel, ModelOption } from "../types";
import type { WorkspaceSelectionSnapshot } from "../services/selectionMemory";

/**
 * How a new session picks its Agent:
 * - remember: the workspace's last explicit choice, then its defaults
 * - workspace: always the workspace default
 * - fixed: always the configured Agent when it is available
 */
export type NewSessionAgentMode = "remember" | "workspace" | "fixed";

const EFFORT_LEVELS: readonly EffortLevel[] = ["none", "low", "medium", "high", "xhigh", "max"];

export function isEffortLevel(value: unknown): value is EffortLevel {
  return typeof value === "string" && (EFFORT_LEVELS as readonly string[]).includes(value);
}

/**
 * Maps a requested effort onto the levels a model accepts: the highest level
 * not above the request, else the lowest level above it. Never jumps to the
 * first listed level, which for Claude would silently turn thinking off.
 */
export function clampEffortToLevels(levels: readonly EffortLevel[], level: EffortLevel): EffortLevel {
  if (levels.length === 0 || levels.includes(level)) return level;
  const rank = EFFORT_LEVELS.indexOf(level);
  const supported = EFFORT_LEVELS.filter((candidate) => levels.includes(candidate));
  const below = supported.filter((candidate) => EFFORT_LEVELS.indexOf(candidate) < rank);
  if (below.length > 0) return below[below.length - 1]!;
  return supported[0] ?? levels[0]!;
}

export interface NewSessionAgentInput {
  agents: readonly Pick<AgentInfo, "id" | "isDefault">[];
  mode: NewSessionAgentMode;
  fixedAgentId?: string;
  snapshot?: WorkspaceSelectionSnapshot | null;
  /** A pre-migration global default, used until any explicit choice exists. */
  legacyAgentId?: string;
}

export function resolveNewSessionAgentId(input: NewSessionAgentInput): string {
  const { agents, mode, snapshot } = input;
  const candidates: Array<string | null | undefined> = [];
  if (mode === "fixed") candidates.push(input.fixedAgentId);
  if (mode === "remember") {
    candidates.push(
      snapshot?.memory.agentId,
      snapshot?.workspaceDefaultAgentId,
      snapshot?.lastAgentId || input.legacyAgentId,
    );
  }
  for (const candidate of candidates) {
    const id = candidate?.trim();
    if (id && agents.some((agent) => agent.id === id)) return id;
  }
  // `isDefault` is the workspace default, or the best Agent for the project type.
  return agents.find((agent) => agent.isDefault)?.id ?? agents[0]?.id ?? "";
}

export interface NewSessionModelInput {
  models: readonly Pick<ModelOption, "id" | "isDefault">[];
  agentId: string;
  /** Settings' pinned main model. Empty means "remember the last choice". */
  pinnedModelId?: string;
  snapshot?: WorkspaceSelectionSnapshot | null;
  agentPreferences?: Readonly<Record<string, AgentModelPreference>>;
  lastModelId?: string;
}

export interface ResolvedModelSelection {
  modelId: string;
  /** Remembered effort for the chosen model; null lets the caller apply its default. */
  effort: EffortLevel | null;
}

export function resolveNewSessionModel(input: NewSessionModelInput): ResolvedModelSelection {
  const ids = new Set(input.models.map((model) => model.id));
  const memory = input.snapshot?.memory;
  const agentId = input.agentId.trim();
  const records = [
    agentId ? memory?.agents?.[agentId] : undefined,
    memory ? { modelId: memory.modelId, effort: memory.effort } : undefined,
    agentId ? input.agentPreferences?.[agentId] : undefined,
  ].filter((record): record is { modelId: string; effort: string } => !!record?.modelId?.trim());
  const effortFor = (modelId: string): EffortLevel | null => {
    const record = records.find((candidate) => candidate.modelId === modelId && isEffortLevel(candidate.effort));
    return record && isEffortLevel(record.effort) ? record.effort : null;
  };
  const pick = (modelId: string | undefined): ResolvedModelSelection | null => {
    const id = modelId?.trim();
    return id && ids.has(id) ? { modelId: id, effort: effortFor(id) } : null;
  };

  return pick(input.pinnedModelId)
    ?? records.map((record) => pick(record.modelId)).find(Boolean)
    ?? pick(input.lastModelId)
    ?? pick(input.models.find((model) => model.isDefault)?.id)
    ?? pick(input.models[0]?.id)
    ?? { modelId: "", effort: null };
}
