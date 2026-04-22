import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("unity reference import window layout", () => {
  it("keeps only the close control in the titlebar", () => {
    const component = read("src/components/UnityReferenceImportProgressWindow.vue");

    expect(component).toContain('class="reference-import-window-titlebar-actions"');
    expect(component).toContain('class="reference-import-window-close"');
    expect(component).toContain(`:aria-label="t('common.close')"`);
  });

  it("uses a constrained scroll body and keeps the import action inside the window body", () => {
    const component = read("src/components/UnityReferenceImportProgressWindow.vue");
    const service = read("src/services/unityReferenceImportWindow.ts");

    expect(component).toContain('class="reference-import-window-scroll"');
    expect(component).toContain("display: flex;");
    expect(component).toContain("flex-direction: column;");
    expect(component).toContain("overflow: auto;");
    expect(component).toContain('class="reference-import-window-actions"');
    expect(component).toContain('variant="primary"');
    expect(service).toContain("width: 720");
    expect(service).toContain("height: 560");
  });
});
