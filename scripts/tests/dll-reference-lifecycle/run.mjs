import { spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, writeFile, copyFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../..");
const root = await mkdtemp(path.join(tmpdir(), "locus-dll-lifecycle-"));
const source = path.join(repo, "locus_compile_server");
const xml = (s) => s.replaceAll("&", "&amp;").replaceAll('"', "&quot;");
const sourceHashes = {};
for (const name of ["ReferenceCache.cs", "CallerScan.cs", "CompileService.cs", "Program.cs", "ScopedCompileServiceRegistry.cs"]) {
  sourceHashes[name] = createHash("sha256").update(await readFile(path.join(source, name))).digest("hex");
}

function replaceOnce(text, before, after) {
  if (text.split(before).length !== 2) throw new Error(`Probe source anchor changed: ${before}`);
  return text.replace(before, after);
}

async function run(command, args, name) {
  let output = "";
  const child = spawn(command, args, { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
  const collect = (data) => { output += data; };
  child.stdout.on("data", collect);
  child.stderr.on("data", collect);
  const timeout = setTimeout(() => child.kill(), 180_000);
  try {
    const code = await new Promise((resolve, reject) => {
      child.on("error", reject);
      child.on("close", resolve);
    });
    await writeFile(path.join(root, `${name}.log`), output);
    if (code !== 0) throw new Error(`${name} exited ${code}:\n${output}`);
    return output;
  } finally { clearTimeout(timeout); }
}

console.log(`LOCUS_DLL_PROBE_ROOT ${root}`);
const project = await readFile(path.join(source, "LocusCompileServer.csproj"), "utf8");
const roslynVersion = project.match(/Include="Microsoft.CodeAnalysis.CSharp" Version="([^"]+)"/)[1];
await writeFile(path.join(root, "Probe.csproj"), `<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings><Nullable>enable</Nullable></PropertyGroup>
  <ItemGroup><PackageReference Include="Microsoft.CodeAnalysis.CSharp" Version="${roslynVersion}" /></ItemGroup>
  <ItemGroup><Compile Include="${xml(source)}/*.cs" Exclude="${xml(source)}/Program.cs" /></ItemGroup>
</Project>`);
await copyFile(path.join(here, "Probe.cs"), path.join(root, "Program.cs"));

// Instrument only the checkpoint; exercise the production file-reading code
// unchanged. Keep the hook out of the shipped assembly.
const scanner = await readFile(path.join(source, "CallerScan.cs"), "utf8");
let instrumented = replaceOnce(scanner, "namespace Locus.CompileServer;", "namespace InstrumentedScanner;");
instrumented = replaceOnce(instrumented, "MetadataReader pdbReader = pdbProvider.GetMetadataReader();", "MetadataReader pdbReader = pdbProvider.GetMetadataReader();\n        Checkpoint.AfterRead?.Invoke();");
instrumented += "\ninternal static class Checkpoint { public static Action? AfterRead; }\n";
await writeFile(path.join(root, "InstrumentedScanner.cs"), instrumented);
await run("dotnet", ["build", "Probe.csproj", "--nologo", "-v", "quiet"], "build");
await run("dotnet", [path.join(root, "bin/Debug/net10.0/Probe.dll"), root], "modern");

// Optional Unity Mono installations; no Editor, project, or shared process is touched.
const monoHomes = JSON.parse(process.env.LOCUS_DLL_PROBE_MONO_HOMES || "[]");
const monoResults = [];
if (monoHomes.length) {
  const deps = path.join(root, "mono-deps");
  await mkdir(deps);
  await writeFile(path.join(deps, "Deps.csproj"), `<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>netstandard2.0</TargetFramework><CopyLocalLockFileAssemblies>true</CopyLocalLockFileAssemblies></PropertyGroup><ItemGroup><PackageReference Include="System.Memory" Version="4.5.5" /></ItemGroup></Project>`);
  await run("dotnet", ["build", path.join(deps, "Deps.csproj"), "--nologo", "-v", "quiet"], "mono-deps");
}
for (let index = 0; index < monoHomes.length; index++) {
  const home = path.resolve(monoHomes[index]);
  const label = `mono-${index}`;
  const output = path.join(root, label);
  await mkdir(output);
  await copyFile(path.join(repo, "locus_unity/Editor/Roslyn/Locus.Roslyn.dll"), path.join(output, "Locus.Roslyn.dll"));
  // Standalone Mono does not inherit the Editor's assembly resolver. Supply
  // the same framework-compatible support libraries for every host tested.
  for (const name of ["System.Memory.dll", "System.Buffers.dll", "System.Runtime.CompilerServices.Unsafe.dll"]) {
    await copyFile(path.join(root, "mono-deps/bin/Debug/netstandard2.0", name), path.join(output, name));
  }
  await copyFile(path.join(here, "LegacyProbe.cs"), path.join(output, "LegacyProbe.cs"));
  const mono = path.join(home, "bin/mono.exe");
  const refs = path.join(home, "lib/mono/4.5");
  const sdk = (await run("dotnet", ["--list-sdks"], `sdks-${index}`)).trim().split(/\r?\n/).findLast((line) => line.startsWith("10."));
  const [, version, sdkRoot] = sdk.match(/^(\S+) \[(.+)\]$/);
  await run("dotnet", [path.join(sdkRoot, version, "Roslyn/bincore/csc.dll"), "/nologo", "/nostdlib+", "/target:exe", "/langversion:9", `/out:${path.join(output, "LegacyProbe.exe")}`, ...["mscorlib.dll", "System.dll", "System.Core.dll", "Facades/netstandard.dll"].map((f) => `/reference:${path.join(refs, f)}`), `/reference:${path.join(output, "Locus.Roslyn.dll")}`, path.join(output, "LegacyProbe.cs")], `build-${label}`);
  const versionText = await run(mono, ["--version"], `version-${label}`);
  const result = await run(mono, [path.join(output, "LegacyProbe.exe"), output], label);
  monoResults.push({ home, version: versionText.split(/\r?\n/)[0], observations: JSON.parse(result.trim()) });
}
const report = JSON.parse(await readFile(path.join(root, "report.json"), "utf8"));
report.mono = monoResults;
report.sourceHashes = sourceHashes;
await writeFile(path.join(root, "report.json"), JSON.stringify(report, null, 2));
console.log(`LOCUS_DLL_PROBE_REPORT ${JSON.stringify({ root, report })}`);
