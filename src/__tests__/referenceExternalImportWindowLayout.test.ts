import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("ReferenceExternalImportWindow layout", () => {
  it("uses the existing frameless child-window shell", () => {
    const component = read("src/components/ReferenceExternalImportWindow.vue");
    const app = read("src/App.vue");
    const agents = read("AGENTS.md");

    expect(component).toContain('class="external-import-window-titlebar-actions"');
    expect(component).toContain('class="external-import-window-close"');
    expect(component).toContain('class="external-import-window-scroll"');
    expect(component).toContain('@close="void requestWindowClose()"');
    expect(component).toContain("border: 1px solid var(--border-strong);");
    expect(component).toContain("position: relative;");
    expect(component).toContain("inset 0 1px 0 color-mix(in srgb, white 8%, transparent)");
    expect(component).toContain("box-shadow: inset 0 1px 0 color-mix(in srgb, white 6%, transparent);");
    expect(component).toContain('background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);');
    expect(component).not.toContain('class="external-import-window-shell"');
    expect(component).not.toContain('class="external-import-window-header"');
    expect(component).not.toContain('class="external-import-window-footer"');
    expect(app).toContain("const isReferenceExternalImportWindow = isReferenceExternalImportWindowLocation();");
    expect(app).toContain("<ReferenceExternalImportWindow v-else-if=\"isReferenceExternalImportWindow\" />");
    expect(agents).toContain("独立窗口默认采用标题栏下直接 `header / scroll body / footer` 的连续布局");
  });

  it("keeps the external import window free of the extra bottom-right resize control", () => {
    const component = read("src/components/ReferenceExternalImportWindow.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(component).not.toContain("async function startWindowResize(");
    expect(component).not.toContain("appWindow.startResizeDragging(direction)");
    expect(component).not.toContain('class="external-import-window-resize-handle"');
    expect(component).not.toContain("cursor: nwse-resize;");
    expect(zh).toContain('"knowledge.referenceFolder.external.windowHint": "在这个独立窗口里完成来源配置和导入，导入过程会持续显示在这里。"');
    expect(en).toContain('"knowledge.referenceFolder.external.windowHint": "Finish source configuration and import in this independent window. Import progress stays visible here."');
  });
});
