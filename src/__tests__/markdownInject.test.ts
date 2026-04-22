import { describe, expect, it } from "vitest";
import {
  walkHtmlText,
  injectAssetChips,
  injectFileRefs,
  injectWorkspaceMentions,
} from "../composables/markdownInject";

describe("walkHtmlText", () => {
  it("transforms plain text", () => {
    expect(walkHtmlText("hello world", (t) => t.toUpperCase())).toBe("HELLO WORLD");
  });

  it("skips text inside <code> tags", () => {
    const html = "before <code>inside</code> after";
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      "BEFORE <code>inside</code> AFTER",
    );
  });

  it("skips text inside <pre> tags", () => {
    const html = "before <pre>inside code</pre> after";
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      "BEFORE <pre>inside code</pre> AFTER",
    );
  });

  it("skips text inside <a> tags", () => {
    const html = 'click <a href="#">link text</a> here';
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      'CLICK <a href="#">link text</a> HERE',
    );
  });

  it("handles nested code inside pre", () => {
    const html = "text <pre><code>code</code></pre> more";
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      "TEXT <pre><code>code</code></pre> MORE",
    );
  });
});

describe("injectAssetChips", () => {
  it("converts @Assets/... references to chips", () => {
    const html = "See @Assets/Prefabs/Player.prefab for details";
    const result = injectAssetChips(html);
    expect(result).toContain("md-asset-chip");
    expect(result).toContain("ui-select-text");
    expect(result).toContain('data-asset-path="Assets/Prefabs/Player.prefab"');
    expect(result).toContain("Player");
  });

  it("does not convert inside code blocks", () => {
    const html = "<code>@Assets/Prefabs/Player.prefab</code>";
    const result = injectAssetChips(html);
    expect(result).not.toContain("md-asset-chip");
  });

  it("does not convert generic workspace mentions", () => {
    const html = "See @UIElementsSchema/UnityEditor.Overlays.xsd";
    const result = injectAssetChips(html);
    expect(result).not.toContain("md-asset-chip");
  });
});

describe("injectWorkspaceMentions", () => {
  it("converts generic workspace file mentions", () => {
    const html = "Inspect @UIElementsSchema/UnityEditor.Overlays.xsd";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-workspace-ref");
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-workspace-path="UIElementsSchema/UnityEditor.Overlays.xsd"');
    expect(result).toContain('data-entry-kind="file"');
    expect(result).toContain("@</span>UnityEditor.Overlays.xsd");
  });

  it("converts folder mentions with a trailing slash", () => {
    const html = "Inspect @UIElementsSchema/";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-workspace-path="UIElementsSchema"');
    expect(result).toContain('data-entry-kind="folder"');
    expect(result).toContain("@</span>UIElementsSchema/");
  });

  it("does not override asset-root mentions", () => {
    const html = "Inspect @Assets/Prefabs/Player.prefab";
    const chipped = injectAssetChips(html);
    const result = injectWorkspaceMentions(chipped);
    expect(result).toContain("md-asset-chip");
    expect(result).not.toContain("md-workspace-ref");
  });

  it("keeps asset-root folder mentions interactive", () => {
    const html = "Inspect @Assets/Scripts/";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-workspace-path="Assets/Scripts"');
  });
});

describe("injectFileRefs", () => {
  it("converts src/ relative paths to file refs", () => {
    const html = "Modified src/components/ChatView.vue to fix the bug";
    const result = injectFileRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain("ui-select-text");
    expect(result).toContain('data-file-path="src/components/ChatView.vue"');
    expect(result).toContain("ChatView.vue");
  });

  it("converts Assets/ paths to file refs", () => {
    const html = "Check Assets/Scripts/Player.cs for logic";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="Assets/Scripts/Player.cs"');
    expect(result).toContain("Player.cs");
  });

  it("converts src-tauri/ paths", () => {
    const html = "See src-tauri/src/commands/workspace.rs";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="src-tauri/src/commands/workspace.rs"');
  });

  it("converts generic dir/file.ext paths", () => {
    const html = "Update utils/helpers.ts";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="utils/helpers.ts"');
  });

  it("handles :line suffix", () => {
    const html = "Error at src/main.ts:42";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="src/main.ts"');
    expect(result).toContain('data-file-line="42"');
    expect(result).toContain("main.ts:42");
  });

  it("handles #Lline suffix", () => {
    const html = "See src/main.ts#L120";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="src/main.ts"');
    expect(result).toContain('data-file-line="120"');
    expect(result).toContain("main.ts:120");
  });

  it("does not match inside code blocks", () => {
    const html = "<pre><code>src/main.ts</code></pre>";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("does not match inside inline code", () => {
    const html = "<code>src/main.ts</code>";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("does not match inside anchor tags", () => {
    const html = '<a href="#">src/main.ts</a>';
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("does not match @Assets/ paths (handled by asset chips)", () => {
    // After injectAssetChips runs first, the @Assets path becomes a chip span.
    // injectFileRefs should not double-process it.
    const chipped = injectAssetChips("See @Assets/Prefabs/Player.prefab");
    const result = injectFileRefs(chipped);
    // Should have asset chip but not file ref
    expect(result).toContain("md-asset-chip");
    expect(result).not.toContain("md-file-ref");
  });

  it("does not double-process workspace mentions", () => {
    const mentioned = injectWorkspaceMentions("See @UIElementsSchema/UnityEditor.Overlays.xsd");
    const result = injectFileRefs(mentioned);
    const matches = result.match(/md-file-ref/g);
    expect(matches).toHaveLength(1);
  });

  it("does not match URLs", () => {
    const html = "Visit https://example.com/path/to/file.html for docs";
    const result = injectFileRefs(html);
    // The URL should not produce a file ref for path/to/file.html
    expect(result).not.toContain("md-file-ref");
  });

  it("does not match paths without slashes", () => {
    const html = "Run main.ts to start";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("handles multiple file refs in one text", () => {
    const html = "Changed src/a.ts and src/b.ts";
    const result = injectFileRefs(html);
    const matches = result.match(/md-file-ref/g);
    expect(matches).toHaveLength(2);
  });
});
