import { execFile } from "node:child_process";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { describe, expect, it } from "vitest";

// Opt-in integration regression: requires .NET, optionally Unity Mono runtimes.
describe.skipIf(process.env.LOCUS_DLL_LIFETIME_PROBE !== "1")("DLL reference lifecycle regression", () => {
  it("keeps old snapshots alive and permits replacement during compilation and analysis", async () => {
    const { stdout } = await promisify(execFile)("bun", [path.resolve("scripts/tests/dll-reference-lifecycle/run.mjs")], {
      timeout: 300_000,
      maxBuffer: 2 * 1024 * 1024,
      windowsHide: true,
    });
    const line = stdout.split(/\r?\n/).find((item) => item.startsWith("LOCUS_DLL_PROBE_REPORT "));
    expect(line, stdout).toBeDefined();
    const result = JSON.parse(line!.slice("LOCUS_DLL_PROBE_REPORT ".length));
    console.log(`DLL lifecycle report: ${result.root}/report.json`);
    expect(result.report.failures).toEqual([]);
    expect(result.report.observations.length).toBe(15);
    for (const runtime of result.report.mono) {
      expect(runtime.observations.failures, runtime.home).toBe(0);
    }
  }, 300_000);

  it("preserves the existing compile, index, scan and scope behavior", async () => {
    const output = await mkdtemp(path.join(tmpdir(), "locus-dll-server-tests-"));
    const { stdout, stderr } = await promisify(execFile)("dotnet", [
      "test", path.resolve("locus_compile_server/Tests/LocusCompileServer.Tests.csproj"),
      "--artifacts-path", path.join(output, "artifacts"),
      "--results-directory", output,
      "--logger", "trx",
      "--filter", "FullyQualifiedName~CompileServiceTests|FullyQualifiedName~CallerScanTests|FullyQualifiedName~ScopedCompileServiceRegistryTests|FullyQualifiedName~TypeIndexSourceTests|FullyQualifiedName~SerializedSchemaSourceTests",
      "--nologo", "--verbosity", "quiet",
    ], { timeout: 180_000, maxBuffer: 2 * 1024 * 1024, windowsHide: true });
    await writeFile(path.join(output, "test.log"), stdout + stderr);
    console.log(`Compile server regression results: ${output}`);
  }, 180_000);
});
