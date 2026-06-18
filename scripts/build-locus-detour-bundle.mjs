// Builds locus_unity/Editor/Detour/Locus.Detour.dll: MonoMod.RuntimeDetour
// (the hot-reload method redirection engine) merged with its dependencies
// into a single Unity-Editor plugin DLL, so the Locus Unity plugin never
// collides with a project's own Mono.Cecil / MonoMod copies.
//
// net452 / net40 variants are merged on purpose: the Unity Editor Mono
// profile loads them without any netstandard facade resolution.

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { copyFile, mkdir, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const ilRepackVersion = "2.0.44";
const monoModVersion = "21.12.13.1";
const cecilVersion = "0.11.4";

const packages = [
  {
    id: "monomod.runtimedetour",
    version: monoModVersion,
    libDir: "lib/net452",
    dlls: ["MonoMod.RuntimeDetour.dll"],
  },
  {
    id: "monomod.utils",
    version: monoModVersion,
    libDir: "lib/net452",
    dlls: ["MonoMod.Utils.dll"],
  },
  {
    id: "mono.cecil",
    version: cecilVersion,
    libDir: "lib/net40",
    dlls: ["Mono.Cecil.dll", "Mono.Cecil.Mdb.dll", "Mono.Cecil.Pdb.dll", "Mono.Cecil.Rocks.dll"],
  },
];

const ilRepackPackageUrl = `https://api.nuget.org/v3-flatcontainer/ilrepack/${ilRepackVersion}/ilrepack.${ilRepackVersion}.nupkg`;
const tmpRoot = path.join(repoRoot, ".tmp", "locus-detour-bundle");
const ilRepackNupkgPath = path.join(tmpRoot, `ilrepack.${ilRepackVersion}.nupkg`);
const ilRepackDir = path.join(tmpRoot, `ilrepack.${ilRepackVersion}`);
const ilRepackExe = path.join(ilRepackDir, "tools", "ILRepack.exe");
const inputsDir = path.join(tmpRoot, "inputs");
const bundleOutputDir = path.join(tmpRoot, "bundle-output");
const tmpOutputDll = path.join(bundleOutputDir, "Locus.Detour.dll");
const outputDir = path.join(repoRoot, "locus_unity", "Editor", "Detour");
const outputDll = path.join(outputDir, "Locus.Detour.dll");

function run(command, args, options = {}) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    ...options,
  });
}

async function ensureDownloaded(url, target) {
  if (existsSync(target)) {
    return;
  }

  try {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`download failed: ${url} (${response.status})`);
    }

    const bytes = new Uint8Array(await response.arrayBuffer());
    await writeFile(target, bytes);
    return;
  } catch (error) {
    if (process.platform !== "win32") {
      throw error;
    }
  }

  run("powershell", [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    "& { param($uri, $out) Invoke-WebRequest -Uri $uri -OutFile $out }",
    url,
    target,
  ]);
}

async function extractArchive(archive, destination) {
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });

  if (process.platform === "win32") {
    // Windows PowerShell's Expand-Archive only accepts the .zip extension.
    const zipCopy = archive.endsWith(".zip") ? archive : `${archive}.zip`;
    if (zipCopy !== archive) {
      await copyFile(archive, zipCopy);
    }
    run("powershell", [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      "& { param($archive, $destination) Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force }",
      zipCopy,
      destination,
    ]);
    if (zipCopy !== archive) {
      await rm(zipCopy, { force: true });
    }
  } else {
    run("unzip", ["-q", archive, "-d", destination]);
  }
}

async function ensureIlRepack() {
  await ensureDownloaded(ilRepackPackageUrl, ilRepackNupkgPath);

  if (existsSync(ilRepackExe)) {
    return;
  }

  await extractArchive(ilRepackNupkgPath, ilRepackDir);
}

async function collectInputDlls() {
  await rm(inputsDir, { recursive: true, force: true });
  await mkdir(inputsDir, { recursive: true });

  const inputDlls = [];
  for (const pkg of packages) {
    const nupkgPath = path.join(tmpRoot, `${pkg.id}.${pkg.version}.nupkg`);
    const nupkgUrl = `https://api.nuget.org/v3-flatcontainer/${pkg.id}/${pkg.version}/${pkg.id}.${pkg.version}.nupkg`;
    await ensureDownloaded(nupkgUrl, nupkgPath);

    const extractDir = path.join(tmpRoot, `${pkg.id}.${pkg.version}`);
    const needsExtract =
      !existsSync(path.join(extractDir, pkg.libDir)) ||
      pkg.dlls.some((dll) => !existsSync(path.join(extractDir, ...pkg.libDir.split("/"), dll)));

    if (needsExtract) {
      await extractArchive(nupkgPath, extractDir);
    }

    for (const dll of pkg.dlls) {
      const source = path.join(extractDir, ...pkg.libDir.split("/"), dll);
      if (!existsSync(source)) {
        throw new Error(`missing ${pkg.libDir}/${dll} in ${pkg.id}.${pkg.version}.nupkg`);
      }
      const target = path.join(inputsDir, dll);
      await rm(target, { force: true });
      await copyFile(source, target);
      inputDlls.push(target);
    }
  }

  return inputDlls;
}

async function validateOutputs() {
  if (!existsSync(path.join(outputDir, "Locus.Detour.dll.meta"))) {
    throw new Error("missing Unity meta file for Locus.Detour.dll");
  }
}

async function cleanupOutputArtifacts() {
  const entries = await readdir(outputDir, { withFileTypes: true });

  await Promise.all(
    entries
      .filter(
        (entry) =>
          entry.name.startsWith("ILRepack-") ||
          (entry.name.startsWith("Locus.Detour.dll.") && entry.name !== "Locus.Detour.dll.meta"),
      )
      .map((entry) => rm(path.join(outputDir, entry.name), { recursive: true, force: true })),
  );
}

async function buildBundle() {
  await mkdir(tmpRoot, { recursive: true });
  await validateOutputs();
  await ensureIlRepack();
  const inputDlls = await collectInputDlls();
  await rm(bundleOutputDir, { recursive: true, force: true });
  await mkdir(bundleOutputDir, { recursive: true });

  // MonoMod.RuntimeDetour is the primary assembly (keeps its public API);
  // ILRepack renames the merged assembly after the output file name.
  run(ilRepackExe, [
    "/target:library",
    "/ndebug",
    "/parallel",
    "/allowduplicateresources",
    `/ver:${monoModVersion}`,
    `/out:${tmpOutputDll}`,
    `/lib:${inputsDir}`,
    ...inputDlls,
  ]);

  const output = await stat(tmpOutputDll);
  if (output.size === 0) {
    throw new Error("Locus.Detour.dll was generated as an empty file");
  }

  await mkdir(outputDir, { recursive: true });
  await rename(tmpOutputDll, outputDll);
  await cleanupOutputArtifacts();
}

await buildBundle();
