import { describe, expect, it } from "vitest";
import {
  buildForwardPayload,
  formatDebugConsoleEntriesForLogExport,
} from "../services/debugConsole";
import type { DebugConsoleEntry } from "../types";

describe("debug console export", () => {
  it("formats entries as chronological .log text", () => {
    const entries: DebugConsoleEntry[] = [
      {
        id: "frontend-2",
        timestampMs: Date.parse("2026-05-14T08:01:02.000Z"),
        level: "warn",
        source: "frontend",
        module: "components/settings/ConsoleSettings",
        target: "components/settings/ConsoleSettings",
        message: "second line\r\ncontinued",
      },
      {
        id: "backend-1",
        timestampMs: Date.parse("2026-05-14T08:00:00.000Z"),
        level: "info",
        source: "backend",
        module: "commands::log",
        target: "commands::log",
        message: "first line",
      },
    ];

    const text = formatDebugConsoleEntriesForLogExport(
      entries,
      new Date("2026-05-14T08:02:00.000Z"),
    );

    expect(text).toBe([
      "# Locus Console Log Export",
      "# Exported At: 2026-05-14T08:02:00.000Z",
      "# Entries: 2",
      "",
      "[2026-05-14T08:00:00.000Z] [INFO] [backend] [commands::log] first line",
      "[2026-05-14T08:01:02.000Z] [WARN] [frontend] [components/settings/ConsoleSettings] second line",
      "    continued",
      "",
    ].join("\n"));
  });
});

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

  it("truncates oversized messages before forwarding", () => {
    const payload = buildForwardPayload(
      [{ ...baseEntry, message: "x".repeat(50) }],
      10,
    );
    expect(payload[0]?.message).toBe(`${"x".repeat(10)} …(truncated)`);
  });
});
