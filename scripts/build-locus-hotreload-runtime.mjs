// Builds locus_unity/Editor/HotReload/Locus.HotReload.Runtime.dll: the
// hot-reload field-store runtime (M4) shared by every patch assembly of a
// session. Single-source by design — per-patch copies would split the
// ConditionalWeakTable state (see locus_hotreload_runtime/LocusFieldStore.cs).

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { copyFile, mkdir, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const projectDir = path.join(repoRoot, "locus_hotreload_runtime");
const projectFile = path.join(projectDir, "LocusHotReloadRuntime.csproj");
const builtDll = path.join(
  projectDir,
  "bin",
  "Release",
  "netstandard2.0",
  "Locus.HotReload.Runtime.dll",
);
const outputDir = path.join(repoRoot, "locus_unity", "Editor", "HotReload");
const outputDll = path.join(outputDir, "Locus.HotReload.Runtime.dll");

function run(command, args, options = {}) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    ...options,
  });
}

async function validateOutputs() {
  if (!existsSync(path.join(outputDir, "Locus.HotReload.Runtime.dll.meta"))) {
    throw new Error("missing Unity meta file for Locus.HotReload.Runtime.dll");
  }
}

async function buildBundle() {
  await validateOutputs();

  run("dotnet", ["build", projectFile, "-c", "Release", "--nologo", "-v", "q"]);

  const output = await stat(builtDll);
  if (output.size === 0) {
    throw new Error("Locus.HotReload.Runtime.dll was generated as an empty file");
  }

  await mkdir(outputDir, { recursive: true });
  await copyFile(builtDll, outputDll);
  console.log(`Locus.HotReload.Runtime.dll -> ${outputDll} (${output.size} bytes)`);
}

await buildBundle();
