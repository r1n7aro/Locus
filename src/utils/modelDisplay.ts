import type { ModelOption } from "../types";

export function formatModelDisplayName(name: string): string {
  const display = name
    .replace(/\s*\[1m\]\s*/gi, " ")
    .replace(/\s+/g, " ")
    .trim();
  return display || name.trim();
}

export function modelSupportsFastMode(
  model: Pick<ModelOption, "provider" | "additionalSpeedTiers">,
): boolean {
  return model.provider === "openai_codex"
    && model.additionalSpeedTiers?.some((tier) => tier.toLowerCase() === "fast") === true;
}

export function formatModelOptionDisplayName(
  model: Pick<ModelOption, "name" | "provider" | "additionalSpeedTiers">,
  fastModeEnabled = false,
): string {
  const name = formatModelDisplayName(model.name);
  return fastModeEnabled && modelSupportsFastMode(model) ? `${name} Fast` : name;
}
