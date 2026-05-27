import { describe, expect, it } from "vitest";
import { parseFragment } from "parse5";
import hljs from "../hljs";
import { renderHighlightedCodeLines, splitHighlightedHtmlLines } from "../composables/markdownCodeLines";

type ParseNode = {
  nodeName: string;
  tagName?: string;
  attrs?: Array<{ name: string; value: string }>;
  childNodes?: ParseNode[];
  value?: string;
};

function classNames(node: ParseNode): string[] {
  const classAttr = node.attrs?.find((attr) => attr.name === "class")?.value ?? "";
  return classAttr.split(/\s+/).filter(Boolean);
}

function hasClass(node: ParseNode, className: string): boolean {
  return classNames(node).includes(className);
}

function textContent(node: ParseNode | undefined): string {
  if (!node) return "";
  if (node.nodeName === "#text") return node.value ?? "";
  return (node.childNodes ?? []).map((child) => textContent(child)).join("");
}

function childByClass(node: ParseNode, className: string): ParseNode | undefined {
  return (node.childNodes ?? []).find((child) => hasClass(child, className));
}

describe("markdown code line rendering", () => {
  it("keeps code lines as siblings when highlight.js spans cross line breaks", () => {
    const code = `public static class Screenshot

{
    [FlutterBridgeMessageDoc("截取当前画面", Kind = FlutterBridgeMessageKind.Query,
        ParamsType = typeof(ScreenshotCaptureResult))]
    public const string Capture = "Screenshot.Capture";
}`;
    const highlighted = hljs.highlight(code, { language: "csharp" }).value;

    expect(highlighted).toContain('class="hljs-meta"');
    expect(highlighted).toContain('FlutterBridgeMessageKind.Query,\n        ParamsType');

    const rendered = renderHighlightedCodeLines(highlighted);
    const fragment = parseFragment(`<code>${rendered}</code>`) as ParseNode;
    const codeElement = fragment.childNodes?.find((child) => child.tagName === "code");
    const directCodeLines = codeElement?.childNodes?.filter((child) => hasClass(child, "code-line")) ?? [];

    expect(directCodeLines).toHaveLength(7);
    expect(directCodeLines.map((line) => textContent(childByClass(line, "line-number")).trim())).toEqual([
      "1",
      "2",
      "3",
      "4",
      "5",
      "6",
      "7",
    ]);
    expect(textContent(childByClass(directCodeLines[4], "line-content"))).toContain("ParamsType");
  });

  it("reopens active highlight spans on the next rendered line", () => {
    expect(splitHighlightedHtmlLines('<span class="hljs-meta">A\nB</span>')).toEqual([
      '<span class="hljs-meta">A</span>',
      '<span class="hljs-meta">B</span>',
    ]);
  });

  it("does not render separator newlines between visual code lines", () => {
    const rendered = renderHighlightedCodeLines("alpha\nbeta");

    expect(rendered).not.toContain("</span>\n<span");
    expect(rendered).toContain('</span></span><span class="code-line">');
  });
});
