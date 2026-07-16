import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { copyFile, mkdir, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const ilRepackVersion = "2.0.44";
const roslynVersion = "3.8.0";
const ilRepackPackageUrl = `https://api.nuget.org/v3-flatcontainer/ilrepack/${ilRepackVersion}/ilrepack.${ilRepackVersion}.nupkg`;
const tmpRoot = path.join(repoRoot, ".tmp", "locus-roslyn-bundle");
const packagePath = path.join(tmpRoot, `ilrepack.${ilRepackVersion}.nupkg`);
const ilRepackDir = path.join(tmpRoot, `ilrepack.${ilRepackVersion}`);
const ilRepackExe = path.join(ilRepackDir, "tools", "ILRepack.exe");
const stubDir = path.join(tmpRoot, "stub");
const stubProject = path.join(stubDir, "Locus.Roslyn.csproj");
const stubDll = path.join(stubDir, "bin", "Release", "netstandard2.0", "Locus.Roslyn.dll");
const bundleOutputDir = path.join(tmpRoot, "bundle-output");
const tmpOutputDll = path.join(bundleOutputDir, "Locus.Roslyn.dll");
const sourceDir = path.join(repoRoot, "third_party", `roslyn-${roslynVersion}`, "assemblies");
const outputDir = path.join(repoRoot, "locus_unity", "Editor", "Roslyn");
const outputDll = path.join(outputDir, "Locus.Roslyn.dll");

const inputDlls = [
  "Microsoft.CodeAnalysis.dll",
  "Microsoft.CodeAnalysis.CSharp.dll",
  "Microsoft.CodeAnalysis.Scripting.dll",
  "Microsoft.CodeAnalysis.CSharp.Scripting.dll",
  "System.Collections.Immutable.dll",
  "System.Reflection.Metadata.dll",
  "System.Runtime.CompilerServices.Unsafe.dll",
  "Microsoft.CodeAnalysis.resources.dll",
  "Microsoft.CodeAnalysis.CSharp.resources.dll",
  "Microsoft.CodeAnalysis.Scripting.resources.dll",
  "Microsoft.CodeAnalysis.CSharp.Scripting.resources.dll",
];

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

async function ensureIlRepack() {
  await mkdir(tmpRoot, { recursive: true });
  await ensureDownloaded(ilRepackPackageUrl, packagePath);

  if (existsSync(ilRepackExe)) {
    return;
  }

  await rm(ilRepackDir, { recursive: true, force: true });
  await mkdir(ilRepackDir, { recursive: true });

  if (process.platform === "win32") {
    // Windows PowerShell's Expand-Archive only accepts the .zip extension.
    const zipCopy = `${packagePath}.zip`;
    await copyFile(packagePath, zipCopy);
    run("powershell", [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      "& { param($archive, $destination) Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force }",
      zipCopy,
      ilRepackDir,
    ]);
    await rm(zipCopy, { force: true });
  } else {
    run("unzip", ["-q", packagePath, "-d", ilRepackDir]);
  }
}

async function buildStub() {
  await rm(stubDir, { recursive: true, force: true });
  await mkdir(stubDir, { recursive: true });
  await writeFile(
    stubProject,
    `<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>netstandard2.0</TargetFramework>
    <AssemblyName>Locus.Roslyn</AssemblyName>
    <RootNamespace>Locus.Roslyn</RootNamespace>
    <Version>${roslynVersion}</Version>
    <AssemblyVersion>${roslynVersion}.0</AssemblyVersion>
    <FileVersion>${roslynVersion}.0</FileVersion>
    <InformationalVersion>${roslynVersion}+roslyn-${roslynVersion}</InformationalVersion>
    <Deterministic>true</Deterministic>
    <GenerateAssemblyInfo>true</GenerateAssemblyInfo>
  </PropertyGroup>
</Project>
`,
  );
  await writeFile(
    path.join(stubDir, "BundleMarker.cs"),
    `namespace Locus.Roslyn
{
    internal static class BundleMarker
    {
        internal const string Name = "Locus.Roslyn";
    }
}
`,
  );
  run("dotnet", ["build", stubProject, "-c", "Release", "-v", "minimal"]);
}

async function validateInputs() {
  const missing = [];
  for (const fileName of inputDlls) {
    const filePath = path.join(sourceDir, fileName);
    if (!existsSync(filePath)) {
      missing.push(filePath);
    }
  }

  if (missing.length > 0) {
    throw new Error(`missing Roslyn bundle inputs:\n${missing.join("\n")}`);
  }

  if (!existsSync(path.join(outputDir, "Locus.Roslyn.dll.meta"))) {
    throw new Error("missing Unity meta file for Locus.Roslyn.dll");
  }
}

async function cleanupOutputArtifacts() {
  const entries = await readdir(outputDir, { withFileTypes: true });

  await Promise.all(
    entries
      .filter(
        (entry) =>
          entry.name.startsWith("ILRepack-") ||
          (entry.name.startsWith("Locus.Roslyn.dll.") && entry.name !== "Locus.Roslyn.dll.meta"),
      )
      .map((entry) => rm(path.join(outputDir, entry.name), { recursive: true, force: true })),
  );
}

async function buildBundle() {
  await validateInputs();
  await ensureIlRepack();
  await buildStub();
  await rm(bundleOutputDir, { recursive: true, force: true });
  await mkdir(bundleOutputDir, { recursive: true });

  run(ilRepackExe, [
    "/target:library",
    "/ndebug",
    "/parallel",
    "/allowduplicateresources",
    `/out:${tmpOutputDll}`,
    `/lib:${sourceDir}`,
    stubDll,
    ...inputDlls.map((fileName) => path.join(sourceDir, fileName)),
  ]);

  const output = await stat(tmpOutputDll);
  if (output.size === 0) {
    throw new Error("Locus.Roslyn.dll was generated as an empty file");
  }

  await rename(tmpOutputDll, outputDll);
  await cleanupOutputArtifacts();
}

await buildBundle();
