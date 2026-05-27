import { describe, expect, it } from "vitest";
import {
  normalizeSemanticCodeForDisplay,
  renderSemanticCodeHtml,
  semanticCodeLanguageFromPath,
} from "../composables/semanticCodeRendering";

describe("semantic code rendering", () => {
  it("recognizes python and json knowledge files from their paths", () => {
    expect(semanticCodeLanguageFromPath("skills/demo/scripts/parse_psd.py")).toBe("python");
    expect(semanticCodeLanguageFromPath("skills/demo/skill.json")).toBe("json");
    expect(semanticCodeLanguageFromPath("skills/demo/SKILL.md")).toBeNull();
  });

  it("formats valid json before highlighting", () => {
    const display = normalizeSemanticCodeForDisplay('{"id":"demo","enabled":true}', "json");

    expect(display.parseError).toBeNull();
    expect(display.content).toBe('{\n  "id": "demo",\n  "enabled": true\n}');
  });

  it("keeps invalid json unchanged for display", () => {
    const display = normalizeSemanticCodeForDisplay('{"id":', "json");

    expect(display.content).toBe('{"id":');
    expect(display.parseError).toBeTruthy();
  });

  it("renders highlighted code lines for python and json", () => {
    const pythonHtml = renderSemanticCodeHtml("def parse_psd():\n    return True", "python");
    const jsonHtml = renderSemanticCodeHtml('{"id":"demo"}', "json");

    expect(pythonHtml).toContain('class="code-line"');
    expect(pythonHtml).toContain("hljs-keyword");
    expect(jsonHtml).toContain('class="line-number"');
    expect(jsonHtml).toContain("hljs-attr");
  });
});
