import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("compile server dev ensure", () => {
  it("checks the published sidecar protocol before dev startup rebuilds it", () => {
    const pkg = read("package.json");
    const tauriConfig = read("src-tauri/tauri.conf.json");
    const tauriLauncher = read("scripts/run-tauri.mjs");
    const ensureScript = read("scripts/ensure-locus-compile-server.mjs");
    const unityTestLauncher = read("scripts/locus-unity-test.mjs");
    const buildScript = read("scripts/build-locus-compile-server.mjs");
    const csharp = read("locus_compile_server/CompileService.cs");
    const scopedRegistry = read("locus_compile_server/ScopedCompileServiceRegistry.cs");
    const program = read("locus_compile_server/Program.cs");
    const client = read("src-tauri/src/csharp_compile/client.rs");
    const compile = read("src-tauri/src/csharp_compile/mod.rs");
    const manager = read("src-tauri/src/csharp_compile/manager.rs");

    expect(pkg).toContain('"compile-server:bundle": "bun run scripts/build-locus-compile-server.mjs"');
    expect(pkg).toContain('"compile-server:ensure": "bun run scripts/ensure-locus-compile-server.mjs"');
    expect(pkg).toMatch(/"build:tauri:with_embed_python_git": "[^"]*compile-server:bundle[^"]*"/);
    expect(pkg).toMatch(/"build:tauri:without_embed_python_git": "[^"]*compile-server:bundle[^"]*"/);
    expect(tauriConfig).not.toContain('"beforeDevCommand"');
    expect(tauriLauncher).toContain('"compile-server:ensure"');
    expect(tauriLauncher).toContain("await runDevPrerequisites()");
    expect(ensureScript).toContain("CompileService.ProtocolVersion");
    expect(ensureScript).toContain("EXPECTED_PROTOCOL_VERSION");
    expect(ensureScript).toContain("inspectPublishedVersion");
    expect(ensureScript).toContain("latestCompileServerSourceMtimeMs");
    expect(ensureScript).toContain('entry.name === "bin" || entry.name === "obj"');
    expect(ensureScript).toContain('entry.name.endsWith(".cs") || entry.name.endsWith(".csproj")');
    expect(ensureScript).toContain("source changed after published DLL");
    expect(ensureScript).toContain("skipping publish");
    expect(ensureScript).toContain("publish required");
    expect(unityTestLauncher).toContain('await runRequired(bun, ["run", "compile-server:ensure"])');
    expect(buildScript).toContain("dotnet");
    expect(buildScript).toContain("publish");
    expect(buildScript).toContain("mkdtemp");
    expect(buildScript).toContain("directoriesMatch");
    expect(buildScript).toContain("restoreMissingUnchangedFiles");
    expect(buildScript).toContain("replacePublishedDirectory");
    expect(program).toContain('"index/schema" => scope.Service.HandleIndexSchema');
    expect(program).toContain('requestMethod == "scope/release"');
    expect(csharp).toContain("public const int ProtocolVersion = 10;");
    expect(manager).toContain("const EXPECTED_PROTOCOL_VERSION: i64 = 10;");
    expect(scopedRegistry).toContain('RequiredScopeGeneration(scope, "workspaceGeneration")');
    expect(scopedRegistry).toContain('RequiredScopeGeneration(scope, "serviceGeneration")');
    expect(client).toContain('scope.get("workspaceGeneration")?.as_u64()?');
    expect(client).toContain('scope.get("serviceGeneration")?.as_u64()?');
    expect(compile).toContain("publish_prevalidated_external_service");
    expect(compile).toContain("service_generation: Some(identity.service_generation)");
  });
});
