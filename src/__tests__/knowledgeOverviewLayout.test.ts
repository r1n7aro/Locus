import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function cssRuleBlock(source: string, selector: string): string {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = source.match(new RegExp(`${escapedSelector}\\s*\\{([\\s\\S]*?)\\n\\}`));
  return match?.[1] ?? "";
}

describe("KnowledgeOverviewPanel layout", () => {
  it("keeps the overview grid stable and moves reference import guidance into the folder flow", () => {
    const overview = read("src/components/knowledge/KnowledgeOverviewPanel.vue");
    const leftStackRule = cssRuleBlock(overview, ".overview-left-stack");
    const defaultGridRule = cssRuleBlock(overview, ".overview-grid-default");

    expect(overview).toContain("'overview-grid-default': true");
    expect(overview).toContain("class=\"overview-close-btn\"");
    expect(overview).toContain("@click=\"emit('close')\"");
    expect(defaultGridRule).toContain("\"documents token\"");
    expect(defaultGridRule).toContain("\"mode recent\"");
    expect(defaultGridRule).toContain("align-items: stretch;");
    expect(overview).toMatch(/\.overview-grid-default\s*>\s*\.overview-left-stack\s*>\s*\.overview-card-primary\s*\{[\s\S]*grid-area:\s*documents;/);
    expect(overview).toMatch(/\.overview-grid-default\s*>\s*\.overview-left-stack\s*>\s*\.overview-card-mode\s*\{[\s\S]*grid-area:\s*mode;/);
    expect(overview).toMatch(/\.overview-grid-default\s*>\s*\.overview-right-stack\s*>\s*\.overview-card-token\s*\{[\s\S]*grid-area:\s*token;/);
    expect(overview).toMatch(/\.overview-grid-default\s*>\s*\.overview-right-stack\s*>\s*\.overview-card-recent\s*\{[\s\S]*grid-area:\s*recent;/);
    expect(overview).toMatch(/\.overview-grid-default\s*>\s*\.overview-left-stack\s*>\s*\.overview-card-primary,\s*[\s\S]*\.overview-grid-default\s*>\s*\.overview-right-stack\s*>\s*\.overview-card-recent\s*\{[\s\S]*height:\s*100%;/);
    expect(overview).toContain('t("knowledge.referenceFolder.external.overviewHint")');
    expect(overview).toContain('t("knowledge.referenceFolder.external.createAction")');
    expect(overview).toContain('t("knowledge.referenceFolder.external.unityOverviewHint")');
    expect(overview).toContain('t("knowledge.referenceFolder.external.importUnityAction")');
    expect(overview).toContain("@click=\"emit('createExternalFolder')\"");
    expect(overview).toContain("@click=\"emit('createExternalFolder', 'unity')\"");
    expect(overview).toContain('class="overview-card overview-card-note"');
    expect(overview).toContain('class="overview-note-action-row"');
    expect(overview).not.toContain("source-stack");
    expect(overview).not.toContain("source-card");
    expect(leftStackRule).not.toContain("grid-area");
  });

  it("keeps the summary and mode statistics in a readable overview stack", () => {
    const overview = read("src/components/knowledge/KnowledgeOverviewPanel.vue");
    const leftStackRule = cssRuleBlock(overview, ".overview-left-stack");
    const modeStackRule = cssRuleBlock(overview, ".overview-mode-stack");

    expect(overview).toContain('t("knowledge.dashboard.totalSize")');
    expect(overview).toContain('t("knowledge.dashboard.maintenance")');
    expect(overview).toContain('t("knowledge.dashboard.retrieval")');
    expect(overview).toContain('t("knowledge.dashboard.injectMode")');
    expect(overview).toContain('t("knowledge.dashboard.tokenUsage")');
    expect(overview).toContain("overview-card-span-two");
    expect(overview).toContain("overview-card overview-card-mode overview-card-span-two");
    expect(overview).toContain("class=\"overview-right-stack\"");
    expect(overview).toContain("class=\"overview-card overview-card-token\"");
    expect(overview).toContain("overview-mode-row detail-row");
    expect(leftStackRule).toContain("grid-template-columns: repeat(2, minmax(0, 1fr));");
    expect(modeStackRule).toContain("flex-direction: column;");
    expect(overview).toMatch(/\.overview-right-stack\s*\{[\s\S]*flex-direction:\s*column;/);
    expect(overview).toContain("display: contents;");
    expect(overview).toMatch(/\.overview-mode-section \+ \.overview-mode-section\s*\{[\s\S]*border-top:/);
    expect(overview).toMatch(/\.overview-card-recent\s*\{[\s\S]*min-height:\s*220px;/);
  });

  it("responds to the right workspace width so external import actions stay visible", () => {
    const overview = read("src/components/knowledge/KnowledgeOverviewPanel.vue");

    expect(overview).toContain('class="overview-card-action"');
    expect(overview).toContain('class="overview-note-action"');
    expect(overview).toMatch(/\.overview-panel\s*\{[\s\S]*container-type:\s*inline-size;/);
    expect(overview).toMatch(/\.overview-grid-top\s*\{[\s\S]*grid-template-columns:\s*minmax\(340px,\s*0\.86fr\)\s*minmax\(300px,\s*1\.14fr\);/);
    expect(overview).toMatch(/\.card-title-row\s*\{[\s\S]*flex-wrap:\s*wrap;/);
    expect(overview).toMatch(/\.overview-note-action-row\s*\{[\s\S]*flex-wrap:\s*wrap;/);
    expect(overview).toContain("@container (max-width: 1040px)");
    expect(overview).toContain("@container (max-width: 720px)");
    expect(overview).toContain("@container (max-width: 760px)");
    expect(overview).toMatch(/@container \(max-width: 1040px\)\s*\{[\s\S]*\.stats-grid-three,\s*[\s\S]*\.stats-grid-four\s*\{[\s\S]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\);/);
    expect(overview).toMatch(/@container \(max-width: 720px\)\s*\{[\s\S]*\.overview-grid-top\s*\{[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\);/);
    expect(overview).toMatch(/@container \(max-width: 760px\)\s*\{[\s\S]*\.overview-card-action,\s*[\s\S]*\.overview-note-action\s*\{[\s\S]*width:\s*100%;/);
  });
});
