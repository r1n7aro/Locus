import { describe, expect, it } from "vitest";
import {
  buildForwardPayload,
  buildLiveLogDroppedEntry,
} from "../services/debugConsole";
import type { DebugConsoleEntry } from "../types";

describe("debug console file forwarding", () => {
  const baseEntry: DebugConsoleEntry = {
    id: "frontend-1",
    timestampMs: 1750000000000,
    level: "warn",
    source: "frontend",
    module: "stores/chat",
    target: "stores/chat",
    message: "boom",
  };

  it("maps entries to the backend payload shape", () => {
    expect(buildForwardPayload([baseEntry])).toEqual([
      {
        timestampMs: 1750000000000,
        level: "warn",
        module: "stores/chat",
        message: "boom",
      },
    ]);
  });

  it("preserves complete messages before forwarding to disk", () => {
    const message = `${"日志".repeat(700_000)}完整尾部`;
    const payload = buildForwardPayload(
      [{ ...baseEntry, message }],
    );
    expect(payload[0]?.message).toBe(message);
  });
});

describe("debug console live log backpressure", () => {
  it("creates a stable warning when a live batch drops entries", () => {
    expect(buildLiveLogDroppedEntry(42, 1750000000000, 7)).toEqual({
      id: "backend-live-dropped-7",
      timestampMs: 1750000000000,
      level: "warn",
      source: "backend",
      module: "logging",
      target: "logging",
      message: "Live console dropped 42 entries during a log burst. Refresh to load the latest backend snapshot.",
    });
  });
});
