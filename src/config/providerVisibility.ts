export const hiddenProviderIds = new Set<string>(["anthropic_sdk"]);

export const visibleProviderOrder = [
  "openrouter",
  "anthropic",
  "openai_codex",
  "custom",
] as const;

export function isProviderVisible(providerId: string): boolean {
  return !hiddenProviderIds.has(providerId);
}

export function filterVisibleProviders<T extends { id: string }>(providers: T[]): T[] {
  return providers.filter((provider) => isProviderVisible(provider.id));
}

export function filterVisibleModels<T extends { provider: string }>(models: T[]): T[] {
  return models.filter((model) => isProviderVisible(model.provider));
}
