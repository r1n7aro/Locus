import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("workspace tree knowledge auto-placement setting", () => {
  it("defaults to enabled and is exposed as a display checkbox", () => {
    const settings = read("src/composables/useDisplaySettings.ts");
    const panel = read("src/components/settings/DisplaySettings.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(settings).toContain("autoPlaceNewPlanDesignKnowledgeDocuments: boolean;");
    expect(settings).toContain("autoPlaceNewPlanDesignKnowledgeDocuments: true,");
    expect(panel).toContain(':model-value="display.autoPlaceNewPlanDesignKnowledgeDocuments"');
    expect(panel).toContain(
      "@update:model-value=\"setDisplay('autoPlaceNewPlanDesignKnowledgeDocuments', $event)\"",
    );
    expect(zh).toContain(
      '"settings.display.autoPlaceNewPlanDesignKnowledgeDocuments": "将新计划与设计添加到工作区树"',
    );
    expect(en).toContain(
      '"settings.display.autoPlaceNewPlanDesignKnowledgeDocuments": "Add new plans and designs to the workspace tree"',
    );
  });
});
