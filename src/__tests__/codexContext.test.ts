import { describe, expect, it } from "vitest";
import {
  CODEX_DEFAULT_CONTEXT_WINDOW,
  codexEffectiveContextWindow,
  normalizeCodexContextWindow,
} from "../config/codexContext";

describe("Codex context window", () => {
  it("defaults to 272K and migrates the legacy extended switch", () => {
    expect(normalizeCodexContextWindow(undefined)).toBe(CODEX_DEFAULT_CONTEXT_WINDOW);
    expect(normalizeCodexContextWindow(undefined, true)).toBe(372_000);
  });

  it("accepts custom values and clamps them to the supported range", () => {
    expect(normalizeCodexContextWindow(500_000)).toBe(500_000);
    expect(normalizeCodexContextWindow(1)).toBe(16_000);
    expect(normalizeCodexContextWindow(2_000_000)).toBe(1_000_000);
    expect(codexEffectiveContextWindow(1_000_000)).toBe(950_000);
  });
});
