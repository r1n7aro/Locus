import { describe, expect, it } from "vitest";
import { pickPreferredModelId } from "../stores/modelSelection";
import type { ModelDefaults, ModelOption } from "../types";

const models: ModelOption[] = [
  { id: "openrouter/claude-sonnet-4.6", name: "Claude Sonnet 4.6", provider: "openrouter" },
  { id: "claude-sonnet-4.6", name: "Claude Sonnet 4.6", provider: "anthropic" },
  { id: "openai/gpt-5.4", name: "GPT-5.4", provider: "openai_codex" },
];

function defaults(partial?: Partial<ModelDefaults>): ModelDefaults {
  return {
    mainModel: "",
    planModel: "",
    subagentModels: {},
    ...partial,
  };
}

describe("pickPreferredModelId", () => {
  it("prefers mainModel when it is available", () => {
    expect(
      pickPreferredModelId(
        models,
        defaults({ mainModel: "openai/gpt-5.4" }),
        "claude-sonnet-4.6",
      ),
    ).toBe("openai/gpt-5.4");
  });

  it("falls back to last remembered model when mainModel is unavailable", () => {
    expect(
      pickPreferredModelId(
        models,
        defaults({ mainModel: "custom/missing" }),
        "claude-sonnet-4.6",
      ),
    ).toBe("claude-sonnet-4.6");
  });

  it("uses the first available model when nothing is remembered", () => {
    expect(
      pickPreferredModelId(models, defaults(), ""),
    ).toBe("openrouter/claude-sonnet-4.6");
  });

  it("returns empty when there are no available models", () => {
    expect(
      pickPreferredModelId([], defaults({ mainModel: "openai/gpt-5.4" }), "claude-sonnet-4.6"),
    ).toBe("");
  });
});
