import type { ModelOption } from "../types";

export interface ModelSelectorGroup {
  /** Unique v-for key: the provider id, or `custom:<provider id>`. */
  key: string;
  /** Underlying provider ("custom" for every custom-provider group). */
  provider: string;
  label: string;
  models: ModelOption[];
}

/**
 * Group models for the selector dropdowns. Built-in providers each form one
 * section; custom models form one section per custom provider (mirroring the
 * subscription-account groups) in configuration order.
 */
export function groupModelsForSelector(
  models: ModelOption[],
  providerOrder: readonly string[],
  providerLabels: Record<string, string>,
): ModelSelectorGroup[] {
  const byProvider = new Map<string, ModelOption[]>();
  for (const model of models) {
    const list = byProvider.get(model.provider) || [];
    list.push(model);
    byProvider.set(model.provider, list);
  }

  const groups: ModelSelectorGroup[] = [];
  for (const provider of providerOrder) {
    const providerModels = byProvider.get(provider);
    if (!providerModels || providerModels.length === 0) continue;

    if (provider !== "custom") {
      groups.push({
        key: provider,
        provider,
        label: providerLabels[provider] || provider,
        models: providerModels,
      });
      continue;
    }

    const customGroups = new Map<string, ModelSelectorGroup>();
    for (const model of providerModels) {
      const id = model.customProviderId || "";
      const key = `custom:${id}`;
      let group = customGroups.get(key);
      if (!group) {
        group = {
          key,
          provider,
          label: model.customProviderName || providerLabels[provider] || provider,
          models: [],
        };
        customGroups.set(key, group);
        groups.push(group);
      }
      group.models.push(model);
    }
  }
  return groups;
}

/** Name shown for a model inside its dropdown section: custom models drop
 *  the provider prefix because the section header already names it. */
export function modelListEntryName(model: ModelOption): string {
  if (model.provider === "custom" && model.customModelName) {
    return model.customModelName;
  }
  return model.name;
}
