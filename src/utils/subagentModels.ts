import type { SubagentModelChoices } from "../types";

/** Drops blank and duplicate ids while keeping the user's order. */
function uniqueModels(models: readonly unknown[]): string[] {
  const result: string[] = [];
  for (const model of models) {
    if (typeof model !== "string") continue;
    const id = model.trim();
    if (id && !result.includes(id)) result.push(id);
  }
  return result;
}

/**
 * Per-agent model allow-lists, default first. Settings saved before
 * multi-select hold one model string per agent; they become one-model lists.
 */
export function normalizeSubagentModelChoices(value: unknown): SubagentModelChoices {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const result: SubagentModelChoices = {};
  for (const [agentId, entry] of Object.entries(value as Record<string, unknown>)) {
    const models = uniqueModels(Array.isArray(entry) ? entry : [entry]);
    if (models.length > 0) result[agentId] = models;
  }
  return result;
}

/** Adds a model at the end, or removes it; removing the default promotes the next model. */
export function toggleSubagentModel(models: readonly string[], modelId: string): string[] {
  return models.includes(modelId)
    ? models.filter((model) => model !== modelId)
    : [...models, modelId];
}

/** Moves an allowed model to the front so it becomes the agent's default. */
export function makeDefaultSubagentModel(models: readonly string[], modelId: string): string[] {
  if (!models.includes(modelId)) return [...models];
  return [modelId, ...models.filter((model) => model !== modelId)];
}

/** Returns new choices with one agent's list replaced; an empty list inherits again. */
export function withSubagentModels(
  choices: SubagentModelChoices,
  agentId: string,
  models: readonly string[],
): SubagentModelChoices {
  const next = { ...choices };
  const normalized = uniqueModels(models);
  if (normalized.length > 0) next[agentId] = normalized;
  else delete next[agentId];
  return next;
}
