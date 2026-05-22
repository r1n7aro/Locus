import { describe, expect, it } from "vitest";
import { parseFragment } from "parse5";
import { highlightDiffHunk } from "../components/diff/fileDiffText";
import { langFromPath } from "../hljs";
import type { DiffHunk } from "../types";

type ParseNode = {
  nodeName: string;
  tagName?: string;
  childNodes?: ParseNode[];
  value?: string;
};

function hunkWithContent(content: string): DiffHunk {
  return {
    header: "@@ -1 +1 @@",
    oldStart: 1,
    oldCount: 1,
    newStart: 1,
    newCount: 1,
    lines: [
      {
        kind: "add",
        content,
        oldLineNo: null,
        newLineNo: 1,
      },
    ],
  };
}

function hasTag(node: ParseNode, tagName: string): boolean {
  if (node.tagName === tagName) return true;
  return (node.childNodes ?? []).some((child) => hasTag(child, tagName));
}

function textContent(node: ParseNode): string {
  if (node.nodeName === "#text") return node.value ?? "";
  return (node.childNodes ?? []).map((child) => textContent(child)).join("");
}

describe("file diff text highlighting", () => {
  it("maps Vue files to an escaped highlighter language", () => {
    const highlighted = highlightDiffHunk(hunkWithContent("<template>\n"), "src/App.vue");
    const fragment = parseFragment(`<span>${highlighted[0].content}</span>`) as ParseNode;

    expect(langFromPath("src/App.vue")).toBe("xml");
    expect(highlighted[0].content).toContain("&lt;");
    expect(highlighted[0].content).not.toContain("<template");
    expect(hasTag(fragment, "template")).toBe(false);
    expect(textContent(fragment)).toContain("<template");
  });

  it("escapes unknown file types before v-html rendering", () => {
    const highlighted = highlightDiffHunk(
      hunkWithContent('<button @click="save">Apply</button>\n'),
      "src/App.unknown",
    );
    const fragment = parseFragment(`<span>${highlighted[0].content}</span>`) as ParseNode;

    expect(highlighted[0].content).toBe("&lt;button @click=\"save\"&gt;Apply&lt;/button&gt;\n");
    expect(highlighted[0].content).not.toContain("<button");
    expect(hasTag(fragment, "button")).toBe(false);
    expect(textContent(fragment)).toContain('<button @click="save">Apply</button>');
  });
});
