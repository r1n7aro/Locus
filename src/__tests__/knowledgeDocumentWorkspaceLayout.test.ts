import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("workspace knowledge document layout", () => {
  it("uses the workspace tree as the only document list for a selected document", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const knowledgeView = read("src/components/KnowledgeView.vue");

    expect(workbench).toContain(':embedded="activeResource?.kind === \'knowledge\'"');
    expect(workbench).toContain(':selected-document-id="selectedKnowledge?.id ?? null"');
    expect(knowledgeView).toMatch(/v-if="!props\.embedded"\s+class="kx-side"/);
    expect(knowledgeView).toMatch(/v-if="!props\.embedded"\s+class="resize-handle"/);
  });
});
