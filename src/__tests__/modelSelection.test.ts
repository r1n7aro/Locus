import { describe, expect, it } from "vitest";
import { pickPreferredModelId } from "../stores/modelSelection";
import type { ModelDefaults, ModelOption } from "../types";

const models: ModelOption[] = [
  { id: "openrouter/claude-fable-5", name: "Claude Fable 5", provider: "openrouter" },
  { id: "claude-sonnet-5", name: "Claude Sonnet 5", provider: "anthropic" },
  { id: "openai/gpt-5.5", name: "GPT-5.5", provider: "openai_codex" },
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
        defaults({ mainModel: "openai/gpt-5.5" }),
        "claude-sonnet-5",
      ),
    ).toBe("openai/gpt-5.5");
  });

  it("falls back to last remembered model when mainModel is unavailable", () => {
    expect(
      pickPreferredModelId(
        models,
        defaults({ mainModel: "custom/missing" }),
        "claude-sonnet-5",
      ),
    ).toBe("claude-sonnet-5");
  });

  it("uses the first available model when nothing is remembered", () => {
    expect(
      pickPreferredModelId(models, defaults(), ""),
    ).toBe("openrouter/claude-fable-5");
  });

  it("prefers catalog default over display order when nothing is remembered", () => {
    expect(
      pickPreferredModelId(
        [
          { id: "openrouter/claude-fable-5", name: "Claude Fable 5", provider: "openrouter" },
          {
            id: "openrouter/claude-opus-4.8",
            name: "Claude Opus 4.8",
            provider: "openrouter",
            isDefault: true,
          },
        ],
        defaults(),
        "",
      ),
    ).toBe("openrouter/claude-opus-4.8");
  });

  it("returns empty when there are no available models", () => {
    expect(
      pickPreferredModelId([], defaults({ mainModel: "openai/gpt-5.5" }), "claude-sonnet-5"),
    ).toBe("");
  });
});
