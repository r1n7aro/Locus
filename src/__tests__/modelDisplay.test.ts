import { describe, expect, it } from "vitest";
import {
  formatModelDisplayName,
  formatModelOptionDisplayName,
  modelSupportsFastMode,
} from "../utils/modelDisplay";

describe("formatModelDisplayName", () => {
  it("hides Claude 1m suffixes from frontend labels", () => {
    expect(formatModelDisplayName("Claude Fable 5[1m]")).toBe("Claude Fable 5");
    expect(formatModelDisplayName("Claude Opus 4.8 [1M]")).toBe("Claude Opus 4.8");
  });

  it("keeps labels without context suffixes unchanged", () => {
    expect(formatModelDisplayName("GPT-5.5")).toBe("GPT-5.5");
  });

  it("appends Fast only to Fast-capable Codex models while Fast mode is enabled", () => {
    const fastModel = {
      name: "GPT-5.6 Sol",
      provider: "openai_codex" as const,
      additionalSpeedTiers: ["fast"],
    };
    const standardModel = {
      name: "GPT-5.3 Codex Spark",
      provider: "openai_codex" as const,
      additionalSpeedTiers: [],
    };

    expect(modelSupportsFastMode(fastModel)).toBe(true);
    expect(modelSupportsFastMode(standardModel)).toBe(false);
    expect(formatModelOptionDisplayName(fastModel, true)).toBe("GPT-5.6 Sol Fast");
    expect(formatModelOptionDisplayName(fastModel, false)).toBe("GPT-5.6 Sol");
    expect(formatModelOptionDisplayName(standardModel, true)).toBe("GPT-5.3 Codex Spark");
  });

  it("does not treat non-Codex models as Fast-capable", () => {
    const model = {
      name: "Claude Opus 4.8[1m]",
      provider: "anthropic" as const,
      additionalSpeedTiers: ["fast"],
    };

    expect(modelSupportsFastMode(model)).toBe(false);
    expect(formatModelOptionDisplayName(model, true)).toBe("Claude Opus 4.8");
  });
});
