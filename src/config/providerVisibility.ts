/**
 * Claude Code CLI provider rollout switch. The backend is not yet stable, so
 * release builds keep the whole feature hidden (settings card, model list,
 * and the opt-in toggle in model configuration). Flip to true to expose it;
 * users still have to explicitly enable its models in model configuration.
 */
export const claudeCodeProviderReleased = false;

export const hiddenProviderIds = new Set<string>(
  claudeCodeProviderReleased ? [] : ["claude_code"],
);

export const visibleProviderOrder = [
  "openrouter",
  "anthropic",
  "claude_code",
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
