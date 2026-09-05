import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("workbench native drag preview", () => {
  it("uses the native file drag image without drawing a second floating card", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("const showWorkspaceDragFloatingPreview = computed(() => (");
    expect(workbench).toContain("&& !locusFileWorkspaceDragActive.value");
    expect(workbench).toContain(
      'v-if="showWorkspaceDragFloatingPreview && workspaceDragPreview"',
    );
  });
});
