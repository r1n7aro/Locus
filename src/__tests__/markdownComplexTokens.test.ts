import { describe, expect, it } from "vitest";
import {
  findMarkdownMathTokens,
  findPlainMarkdownReferences,
  isUnityPropertyFenceLanguage,
  isUnityReferenceFenceLanguage,
  parseInlineMarkdownReference,
  parseMarkdownTable,
} from "../components/ui/markdown-editor/markdownComplexTokens";

describe("Markdown complex Live Preview tokens", () => {
  it("parses GFM tables without interpreting user content as HTML", () => {
    expect(parseMarkdownTable([
      "| Name | Value | Code |",
      "| :--- | ---: | :---: |",
      "| Hero | 42 | `a|b` |",
      "| Escaped | 7 | a\\|b |",
    ].join("\n"))).toEqual({
      header: ["Name", "Value", "Code"],
      rows: [
        ["Hero", "42", "a|b"],
        ["Escaped", "7", "a|b"],
      ],
      alignments: ["left", "right", "center"],
    });
    expect(parseMarkdownTable("| A | B |\n| -- | --- |\n| 1 | 2 |")).toBeNull();
    expect(parseMarkdownTable("ordinary text")).toBeNull();
  });

  it("recognizes the four math delimiter families and rejects incomplete literals", () => {
    const source = [
      "inline $x^2$ and \\(y + 1\\)",
      "",
      "$$\\frac{a}{b}$$",
      "",
      "\\[z_1 + 2\\]",
    ].join("\n");
    const tokens = findMarkdownMathTokens(source);
    expect(tokens.map((token) => [token.latex, token.display])).toEqual([
      ["x^2", false],
      ["y + 1", false],
      ["\\frac{a}{b}", true],
      ["z_1 + 2", true],
    ]);
    expect(findMarkdownMathTokens("\\$x\\$ and $ open")).toHaveLength(0);
    expect(findMarkdownMathTokens("citation \\[1\\]")).toHaveLength(0);
    expect(findMarkdownMathTokens(`$${"x".repeat(121)}$`)).toHaveLength(0);
  });

  it("parses inline knowledge, workspace, file, View, and Unity references", () => {
    expect(parseInlineMarkdownReference("`Locus/knowledge/design/editor.md`")).toMatchObject({
      kind: "knowledge",
      path: "design/editor.md",
      label: "editor.md",
    });
    expect(parseInlineMarkdownReference("`src/components/App.vue:42`")).toMatchObject({
      kind: "workspace",
      path: "src/components/App.vue",
      line: 42,
    });
    expect(parseInlineMarkdownReference("`unity:preview Assets/Prefabs/Hero.prefab`")).toMatchObject({
      kind: "unity-asset",
      path: "Assets/Prefabs/Hero.prefab",
      level: "preview",
    });
    expect(parseInlineMarkdownReference("`Assets/Scenes/Main.unity/Root/Camera`")).toMatchObject({
      kind: "unity-scene-object",
      path: "Assets/Scenes/Main.unity/Root/Camera",
      label: "Camera",
    });
    expect(parseInlineMarkdownReference("`view:tools/material-inspector`")).toMatchObject({
      kind: "view",
      path: "tools/material-inspector",
    });
    expect(parseInlineMarkdownReference("`ordinary code()`")).toBeNull();
  });

  it("finds conservative plain references and preserves prose-shaped View text", () => {
    const source = [
      "See design/editor.md and @src/main.ts.",
      "Use {@Assets/My Folder/Hero.prefab}.",
      "view:tools/dashboard",
      "The value view:inside-prose stays text.",
    ].join("\n");
    const refs = findPlainMarkdownReferences(source);
    expect(refs.map((ref) => [ref.kind, ref.path])).toEqual([
      ["knowledge", "design/editor.md"],
      ["workspace", "src/main.ts"],
      ["unity-asset", "Assets/My Folder/Hero.prefab"],
      ["view", "tools/dashboard"],
    ]);
  });

  it("recognizes Unity object and property fence aliases", () => {
    expect(isUnityReferenceFenceLanguage("unity:preview")).toBe(true);
    expect(isUnityReferenceFenceLanguage("asset-editor")).toBe(true);
    expect(isUnityReferenceFenceLanguage("typescript")).toBe(false);
    expect(isUnityPropertyFenceLanguage("unity_property")).toBe(true);
    expect(isUnityPropertyFenceLanguage("property:unity")).toBe(true);
    expect(isUnityPropertyFenceLanguage("json")).toBe(false);
  });
});
