import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope } from "vue";
import { useCopyFeedback } from "../composables/useCopyFeedback";

describe("useCopyFeedback", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("marks copied for a short period after clipboard write succeeds", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText,
      },
    });

    const scope = effectScope();
    const state = scope.run(() => useCopyFeedback({ durationMs: 1200 }));
    if (!state) throw new Error("scope did not initialize");

    await expect(state.copyText("GFMG-GH800")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("GFMG-GH800");
    expect(state.copied.value).toBe(true);

    vi.advanceTimersByTime(1199);
    expect(state.copied.value).toBe(true);

    vi.advanceTimersByTime(1);
    expect(state.copied.value).toBe(false);

    scope.stop();
  });

  it("keeps copied false when clipboard write fails", async () => {
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText: vi.fn().mockRejectedValue(new Error("clipboard unavailable")),
      },
    });

    const scope = effectScope();
    const state = scope.run(() => useCopyFeedback());
    if (!state) throw new Error("scope did not initialize");

    await expect(state.copyText("GFMG-GH800")).resolves.toBe(false);
    expect(state.copied.value).toBe(false);

    scope.stop();
  });
});
