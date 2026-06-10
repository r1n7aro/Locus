import { describe, expect, it } from "vitest";
import { buildKnowledgeSnippetSegments } from "../components/knowledge/knowledgeSearchSnippet";

describe("buildKnowledgeSnippetSegments", () => {
  it("highlights backend matched terms case-insensitively", () => {
    const segments = buildKnowledgeSnippetSegments(
      "Parse PSD files for Unity UI workflows",
      ["psd", "unity"],
      "psd unity",
    );

    expect(segments).toEqual([
      { text: "Parse ", highlighted: false },
      { text: "PSD", highlighted: true },
      { text: " files for ", highlighted: false },
      { text: "Unity", highlighted: true },
      { text: " UI workflows", highlighted: false },
    ]);
  });

  it("falls back to query tokens when no matched terms are provided", () => {
    const segments = buildKnowledgeSnippetSegments(
      "层级面板里的预制体映射",
      null,
      "预制体",
    );

    expect(segments.some((segment) => segment.highlighted && segment.text === "预制体")).toBe(
      true,
    );
  });

  it("ignores single-letter latin tokens to avoid noisy highlights", () => {
    const segments = buildKnowledgeSnippetSegments(
      "a tool that maps a layer",
      [],
      "a tool",
    );

    expect(segments.filter((segment) => segment.highlighted).map((s) => s.text)).toEqual([
      "tool",
    ]);
  });

  it("returns the plain snippet when nothing matches", () => {
    expect(buildKnowledgeSnippetSegments("plain text", [], "")).toEqual([
      { text: "plain text", highlighted: false },
    ]);
  });

  it("returns no segments for an empty snippet", () => {
    expect(buildKnowledgeSnippetSegments("", ["term"], "term")).toEqual([]);
    expect(buildKnowledgeSnippetSegments(null, ["term"], "term")).toEqual([]);
  });

  it("escapes regex metacharacters in terms", () => {
    const segments = buildKnowledgeSnippetSegments(
      "call foo(bar) twice",
      ["foo(bar)"],
      "",
    );

    expect(segments).toEqual([
      { text: "call ", highlighted: false },
      { text: "foo(bar)", highlighted: true },
      { text: " twice", highlighted: false },
    ]);
  });
});
