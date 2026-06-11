// Publish the Locus compile-server sidecar (framework-dependent .NET DLL)
// into src-tauri/gen/compile-server/, where dev builds resolve it from and
// `tauri.conf.json` bundles it from (resources -> compile-server/).
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const project = path.join(repoRoot, "locus_compile_server", "LocusCompileServer.csproj");
const outputDir = path.join(repoRoot, "src-tauri", "gen", "compile-server");
const outputDll = path.join(outputDir, "LocusCompileServer.dll");

function run(command, args, options = {}) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    ...options,
  });
}

if (!existsSync(project)) {
  throw new Error(`missing compile server project: ${project}`);
}

await rm(outputDir, { recursive: true, force: true });
await mkdir(outputDir, { recursive: true });

run("dotnet", [
  "publish",
  project,
  "-c",
  "Release",
  "--nologo",
  "-v",
  "minimal",
  "-o",
  outputDir,
]);

if (!existsSync(outputDll)) {
  throw new Error(`compile server publish did not produce ${outputDll}`);
}

console.log(`[locus] compile server published to ${outputDir}`);
