import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const repoRoot = resolve(import.meta.dirname, "../..");

function source(path: string): string {
  return readFileSync(resolve(repoRoot, path), "utf8");
}

describe("deferred C# diagnostics", () => {
  it("keeps edit/write feedback free of Roslyn diagnostics", () => {
    const filesystem = source("src-tauri/src/tool/builtins/filesystem.rs");

    expect(filesystem).not.toContain("append_unity_csharp_write_feedback");
    expect(filesystem).not.toContain("EDIT_WRITE_DIAGNOSTIC_MAX_RESULTS");
    expect(filesystem).toContain("append_unity_csharp_status(");
  });

  it("adds semantic warnings at hot reload and every public recompile entry", () => {
    const codeTools = source("src-tauri/src/code_tools.rs");
    const unityTools = source("src-tauri/src/tool/builtins/unity.rs");
    const agent = source("src-tauri/src/agent/instance/mod.rs");
    const mcp = source("src-tauri/src/mcp/server/tools.rs");

    expect(codeTools).toContain("diagnostics.retain(|diagnostic| diagnostic.severity == 2)");
    expect(unityTools).toContain("hot_reload_with_semantic_warnings");
    expect(unityTools).toContain("recompile_with_semantic_warnings");
    expect(agent).toContain("recompile_with_semantic_warnings");
    expect(mcp).toContain("recompile_with_semantic_warnings");
  });
});
