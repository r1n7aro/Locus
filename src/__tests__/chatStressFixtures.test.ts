import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createLongSessionFixtures } from "../../scripts/locus-chat-switch-stress-fixture";

describe("chat WebView stress fixtures", () => {
  it("builds five deterministic long sessions with complex render shapes", () => {
    const fixtures = createLongSessionFixtures(1_750_000_000);

    expect(fixtures).toHaveLength(5);
    expect(fixtures.map((fixture) => fixture.key)).toEqual([
      "unity-properties",
      "markdown-tables",
      "edit-diffs",
      "nested-tools",
      "mixed-layout",
    ]);
    expect(fixtures.every((fixture) => fixture.messages.length === 60)).toBe(true);
    expect(fixtures.every((fixture) => (
      fixture.messages.filter((message) => message.role === "assistant").length === 30
    ))).toBe(true);
  });

  it("covers Unity property fences, long tables, code, diffs, and nested tools", () => {
    const fixtures = createLongSessionFixtures();
    const byKey = new Map(fixtures.map((fixture) => [fixture.key, fixture]));
    const assistantMessages = (key: string) => byKey.get(key)!.messages
      .filter((message) => message.role === "assistant");

    expect(assistantMessages("unity-properties").every((message) => (
      message.content.includes("```unity_property")
      && message.content.includes("MeshRenderer:m_Materials.Array.data[0]")
    ))).toBe(true);
    expect(assistantMessages("markdown-tables")[0]!.content.match(/^\| property_/gm)?.length).toBe(28);
    expect(assistantMessages("edit-diffs")[0]!.content.match(/public float Sample/g)?.length).toBe(70);
    expect(assistantMessages("nested-tools")[0]!.toolCalls?.[0]?.nestedToolCalls).toHaveLength(3);
    expect(assistantMessages("mixed-layout")[0]!.content.match(/const layoutProbe/g)?.length).toBe(35);
  });

  it("persists canonical render parts and multiple tool calls for every assistant turn", () => {
    const fixtures = createLongSessionFixtures();
    const assistantMessages = fixtures.flatMap((fixture) =>
      fixture.messages.filter((message) => message.role === "assistant")
    );

    expect(assistantMessages).toHaveLength(150);
    expect(assistantMessages.every((message) => (message.toolCalls?.length ?? 0) >= 1)).toBe(true);
    expect(assistantMessages.every((message) => {
      const renderParts = message.metadata?.renderParts;
      return Array.isArray(renderParts)
        && renderParts.some((part) => part && typeof part === "object" && "kind" in part && part.kind === "text")
        && renderParts.some((part) => part && typeof part === "object" && "kind" in part && part.kind === "toolCall");
    })).toBe(true);
  });

  it("fails WebView verification on any deferred transcript height materialization", () => {
    const script = readFileSync(
      resolve(process.cwd(), "scripts/locus-chat-switch-stress.ts"),
      "utf8",
    );

    expect(script).toContain("noDeferredHeightMaterialization:");
    expect(script).toContain("Number(aggregate?.maxScrollHeightDelta) <= 0.5");
    expect(script).toContain("Number(aggregate?.maxContentHeightDelta) <= 0.5");
  });

  it("requires a layout-neutral stream-end handoff", () => {
    const script = readFileSync(
      resolve(process.cwd(), "scripts/locus-chat-render-stress.ts"),
      "utf8",
    );

    expect(script).toContain("Number(status.maxEndScrollDelta) <= 0.5");
    expect(script).toContain("Number(status.maxEndToolTopDelta) <= 0.5");
    expect(script).toContain("Number(status.maxEndTextHeightDelta) <= 0.5");
  });
});
