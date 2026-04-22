import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeRetrievalPanel download popover", () => {
  it("keeps the popover open when only the download source changes", () => {
    const source = read("src/components/knowledge/KnowledgeRetrievalPanel.vue");

    expect(source).toContain("function handleDownloadSourceUpdate(value: string) {");
    expect(source).toContain('emit("setDownloadSource", nextValue);');
    expect(source).not.toContain("() => props.pending");
    expect(source).not.toContain("if (pending) closeDownloadPopover();");
  });

  it("closes the popover only after a local model download is started", () => {
    const source = read("src/components/knowledge/KnowledgeRetrievalPanel.vue");

    expect(source).toContain("function handleDownloadPreset() {");
    expect(source).toContain('emit("downloadLocalModel", modelId);');
    expect(source).toContain("closeDownloadPopover();");
  });

  it("uses official download copy as the default guidance", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain(
      '"knowledge.retrieval.downloadSourceOfficialHint": "官方站点是默认下载源，适合通用网络环境。"',
    );
    expect(zh).toContain(
      '"knowledge.retrieval.downloadSourceMirrorHint": "HF-Mirror 可作为手动切换的备选下载源。"',
    );
    expect(en).toContain(
      '"knowledge.retrieval.downloadSourceOfficialHint": "The official endpoint is the default and fits general network conditions."',
    );
    expect(en).toContain(
      '"knowledge.retrieval.downloadSourceMirrorHint": "HF-Mirror is available as a manually selected alternate endpoint."',
    );
  });
});
