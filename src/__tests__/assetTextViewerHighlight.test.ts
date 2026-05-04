import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AssetTextViewer highlighting", () => {
  it("renders text previews with editor monospace typography and hljs language highlighting", () => {
    const source = read("src/components/asset/AssetTextViewer.vue");

    expect(source).toContain('import hljs from "../../hljs";');
    expect(source).toContain("hljs.highlight(props.snippet, { language }).value");
    expect(source).toContain('class="atv-pre hljs"');
    expect(source).toContain('v-html="line || \' \'"');
    expect(source).toContain("font-family: var(--font-mono-editor);");
  });
});
