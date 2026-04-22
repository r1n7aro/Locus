import type { ModelDefaults, ModelOption } from "../types";

export function pickPreferredModelId(
  models: ModelOption[],
  defaults: ModelDefaults,
  lastModelId: string,
): string {
  if (models.length === 0) return "";

  const ids = new Set(models.map((model) => model.id));

  if (defaults.mainModel && ids.has(defaults.mainModel)) {
    return defaults.mainModel;
  }

  if (lastModelId && ids.has(lastModelId)) {
    return lastModelId;
  }

  return models[0]?.id ?? "";
}
