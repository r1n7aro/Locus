import { describe, expect, it } from "vitest";
import {
  filterUnityConsoleErrorPayload,
  isUnityConsoleErrorLevel,
  type UnityConsoleTextPayload,
} from "../services/unity";

describe("Unity Console error filtering", () => {
  it("recognizes Unity error-like levels", () => {
    expect(isUnityConsoleErrorLevel("Error")).toBe(true);
    expect(isUnityConsoleErrorLevel("ScriptingException")).toBe(true);
    expect(isUnityConsoleErrorLevel("Assert")).toBe(true);
    expect(isUnityConsoleErrorLevel("Fatal")).toBe(true);
    expect(isUnityConsoleErrorLevel("Warning")).toBe(false);
    expect(isUnityConsoleErrorLevel("Log")).toBe(false);
  });

  it("keeps only error entries and clears the unfiltered aggregate text", () => {
    const payload: UnityConsoleTextPayload = {
      text: "all console output",
      title: "Unity Console",
      source: "unity-console",
      entries: [
        { level: "Log", text: "started" },
        { level: "Error", text: "missing reference" },
        { level: "Warning", text: "slow import" },
        { level: "ScriptingException", text: "invalid operation" },
      ],
    };

    const filtered = filterUnityConsoleErrorPayload(payload);

    expect(filtered.text).toBe("");
    expect(filtered.entries).toEqual([
      { level: "Error", text: "missing reference" },
      { level: "ScriptingException", text: "invalid operation" },
    ]);
    expect(payload.entries).toHaveLength(4);
  });
});
