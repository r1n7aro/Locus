import { describe, expect, it, vi } from "vitest";

vi.mock("../i18n", () => ({ t: (key: string, ...args: unknown[]) => [key, ...args].join("|") }));

import { blenderErrorText } from "../services/blenderErrors";

function appError(code: string, message: string, detail?: string) {
  return { code, message, detail, retryable: false, severity: "error" };
}

describe("blenderErrorText", () => {
  it("localizes the summary and keeps the backend's cause", () => {
    expect(blenderErrorText(appError("blender.launch_failed", "Blender could not be started.", "Blender exited before it connected")))
      .toBe("blender.error.withDetail|blender.error.launchFailed|Blender exited before it connected");
    expect(blenderErrorText(appError("blender.not_running", "The Blender service is not running.")))
      .toBe("blender.error.notRunning");
  });

  it("explains manual add refusals on their own", () => {
    expect(blenderErrorText(appError("blender.add.too_old", "Could not add this Blender.", "4.1.1")))
      .toBe("settings.blender.addError.too_old|4.1.1");
  });

  it("falls back to the backend message for unknown codes", () => {
    expect(blenderErrorText(appError("other.code", "Something failed.", "why")))
      .toBe("blender.error.withDetail|Something failed.|why");
  });
});
