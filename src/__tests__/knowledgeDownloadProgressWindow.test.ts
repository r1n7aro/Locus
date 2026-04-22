import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeDownloadProgressWindow", () => {
  it("shows a cancel action and tracks cancelling state", () => {
    const source = read("src/components/KnowledgeDownloadProgressWindow.vue");
    const serviceSource = read("src/services/knowledge.ts");

    expect(source).toContain("knowledgeCancelLocalEmbeddingModelDownload");
    expect(source).toContain("const cancelPending = ref(false)");
    expect(source).toContain("async function cancelDownload()");
    expect(source).toContain('t("knowledge.retrieval.downloadWindowCancelling")');
    expect(source).toContain('t("common.cancel")');
    expect(source).toContain('class="download-window-footer"');
    expect(source).toContain('@click="void cancelDownload()"');
    expect(serviceSource).toContain(
      'ipcInvoke<void>("knowledge_cancel_local_embedding_model_download")',
    );
  });

  it("shows download source and proxy routing details in the window", () => {
    const source = read("src/components/KnowledgeDownloadProgressWindow.vue");

    expect(source).toContain('statusSnapshot.value?.downloadNetwork');
    expect(source).toContain('t("knowledge.retrieval.downloadSource")');
    expect(source).toContain('t("knowledge.retrieval.downloadWindowProxy")');
    expect(source).toContain('downloadWindowProxyEnvironment');
    expect(source).toContain('download-window-row-value');
  });

  it("uses an expanded body layout and keeps the cancel action pinned to the footer", () => {
    const source = read("src/components/KnowledgeDownloadProgressWindow.vue");
    const serviceSource = read("src/services/knowledgeDownloadWindow.ts");

    expect(source).toContain('class="download-window-content"');
    expect(source).toContain("flex: 1;");
    expect(source).not.toContain('class="download-window-scroll"');
    expect(source).not.toContain("overflow: auto;");
    expect(source).toContain('class="download-window-footer"');
    expect(source).toContain("flex-shrink: 0;");
    expect(serviceSource).toContain("width: 620");
    expect(serviceSource).toContain("height: 560");
  });

  it("adds dedicated cancelled copy to both locales", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain(
      '"knowledge.retrieval.downloadWindowCancelledTitle": "模型下载已取消"',
    );
    expect(zh).toContain(
      '"knowledge.retrieval.downloadWindowAutoCloseCancelled": "下载已取消，窗口即将自动关闭。"',
    );
    expect(en).toContain(
      '"knowledge.retrieval.downloadWindowCancelledTitle": "Model Download Cancelled"',
    );
    expect(en).toContain(
      '"knowledge.retrieval.downloadWindowAutoCloseCancelled": "The download was cancelled. This window will close automatically."',
    );
    expect(zh).toContain('"knowledge.retrieval.downloadWindowProxy": "代理"');
    expect(zh).toContain(
      '"knowledge.retrieval.downloadWindowProxyUnsupported": "系统代理未接入"',
    );
    expect(en).toContain('"knowledge.retrieval.downloadWindowProxy": "Proxy"');
    expect(en).toContain(
      '"knowledge.retrieval.downloadWindowProxyUnsupported": "System Proxy Unavailable"',
    );
  });
});
