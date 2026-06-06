import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View package compile validation", () => {
  it("validates a View package on selection or open instead of tree refresh", () => {
    const page = read("src/components/ViewPackageView.vue");

    expect(page).toContain("pruneViewCompileDiagnostics(snapshot.views);");
    expect(page).not.toContain("refreshViewCompileErrors");
    expect(page).not.toContain("Promise.all(");
    expect(page).toContain("function selectView(view: ViewPackageSummary)");
    expect(page).toContain("selectView(row.node.view);");
    expect(page).toContain("const compileError = await ensureViewCompileValidated(view);");
    expect(page).toContain("operation: \"viewCompile\"");
  });
});
