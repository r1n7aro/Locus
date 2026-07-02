import { describe, expect, it } from "vitest";
import { formatModelDisplayName } from "../utils/modelDisplay";

describe("formatModelDisplayName", () => {
  it("hides Claude 1m suffixes from frontend labels", () => {
    expect(formatModelDisplayName("Claude Fable 5[1m]")).toBe("Claude Fable 5");
    expect(formatModelDisplayName("Claude Opus 4.8 [1M]")).toBe("Claude Opus 4.8");
  });

  it("keeps labels without context suffixes unchanged", () => {
    expect(formatModelDisplayName("GPT-5.5")).toBe("GPT-5.5");
  });
});
