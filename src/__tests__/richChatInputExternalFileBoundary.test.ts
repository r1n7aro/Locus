import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("RichChatInput external file boundary warning", () => {
  it("shows a global warning when external file refs are added while the file boundary is off", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(richInput).toContain("getCachedFileToolWorkspaceBoundary");
    expect(richInput).toContain("getFileToolWorkspaceBoundary");
    expect(richInput).toContain("useProjectStore");
    expect(richInput).toContain("LOCAL_FILE_BOUNDARY_WARNING_OPERATION");
    expect(richInput).toContain("function isPathInsideWorkspace");
    expect(richInput).toContain("function isExternalLocalFile");
    expect(richInput).toContain("function showLocalFileBoundaryWarning");
    expect(richInput).toContain("function warnIfFileBoundaryAllowsExternalFiles");
    expect(richInput).toContain("notificationStore.addNotice(\"warning\", t(\"chat.fileRefs.boundaryOffWarning\")");
    expect(richInput).toContain("replaceOperation: true");
    expect(richInput).toContain("void warnIfFileBoundaryAllowsExternalFiles(normalized);");
    expect(zh).toContain('"chat.fileRefs.boundaryOffWarning"');
    expect(en).toContain('"chat.fileRefs.boundaryOffWarning"');
  });
});
