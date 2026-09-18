import { describe, expect, it } from "vitest";
import { unityDriverExitCode } from "../../scripts/locus-unity-driver-result.mjs";

describe("Unity acceptance driver completion", () => {
  it("rejects a zero executable exit after an error or without a successful finished event", () => {
    expect(unityDriverExitCode({ code: 0, finishedOk: false, driverError: "suite failed" })).toBe(1);
    expect(unityDriverExitCode({ code: 0, finishedOk: false, driverError: "" })).toBe(1);
    expect(unityDriverExitCode({ code: 0 })).toBe(1);
    expect(unityDriverExitCode({ code: 0, finishedOk: true, driverError: "an earlier stage failed" })).toBe(1);
  });

  it("accepts intentional termination only after verified driver success", () => {
    expect(unityDriverExitCode({ code: 0, finishedOk: true, driverError: "" })).toBe(0);
    expect(unityDriverExitCode({ code: 1, finishedOk: true, driverError: "" })).toBe(0);
    expect(unityDriverExitCode({ code: null, signal: "SIGTERM", finishedOk: true, driverError: "" })).toBe(0);
    expect(unityDriverExitCode({ code: null, signal: "SIGTERM", finishedOk: false, driverError: "" })).toBe(1);
    expect(unityDriverExitCode({ code: 77, finishedOk: false, driverError: "" })).toBe(77);
  });
});
