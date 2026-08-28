import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function read(path: string) {
  return readFileSync(resolve(process.cwd(), path), "utf8");
}

describe("plan workspace scope", () => {
  it("resolves plan content from session identity", () => {
    const backend = read("src-tauri/src/commands/plan.rs");
    const service = read("src/services/session.ts");
    const windowService = read("src/services/planViewWindow.ts");
    const window = read("src/components/PlanViewWindow.vue");

    expect(backend).toContain("session_id: String");
    expect(backend).toContain("resolve_session_workspace_scope(");
    expect(backend).not.toContain("pub async fn get_plan_file_content(\n    path: String");
    expect(service).toContain('getPlanFileContent(sessionId: string)');
    expect(service).toContain('{ sessionId }');
    expect(windowService).toContain("sessionId: string");
    expect(window).toContain("getPlanFileContent(sessionId)");
  });
});
