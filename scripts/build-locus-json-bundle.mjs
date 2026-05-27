import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const ilRepackVersion = "2.0.44";
const jsonVersion = "13.0.3";
const ilRepackPackageUrl = `https://api.nuget.org/v3-flatcontainer/ilrepack/${ilRepackVersion}/ilrepack.${ilRepackVersion}.nupkg`;
const tmpRoot = path.join(repoRoot, ".tmp", "locus-json-bundle");
const packagePath = path.join(tmpRoot, `ilrepack.${ilRepackVersion}.nupkg`);
const ilRepackDir = path.join(tmpRoot, `ilrepack.${ilRepackVersion}`);
const ilRepackExe = path.join(ilRepackDir, "tools", "ILRepack.exe");
const stubDir = path.join(tmpRoot, "stub");
const stubProject = path.join(stubDir, "Locus.Json.csproj");
const stubDll = path.join(stubDir, "bin", "Release", "netstandard2.0", "Locus.Json.dll");
const bundleOutputDir = path.join(tmpRoot, "bundle-output");
const tmpOutputDll = path.join(bundleOutputDir, "Locus.Json.dll");
const sourceDir = path.join(repoRoot, "third_party", `newtonsoft-json-${jsonVersion}`, "assemblies");
const outputDir = path.join(repoRoot, "locus_unity", "Editor", "Json");
const outputDll = path.join(outputDir, "Locus.Json.dll");

const inputDlls = ["Newtonsoft.Json.dll"];

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
    run("powershell", [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      "& { param($archive, $destination) Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force }",
      packagePath,
      ilRepackDir,
    ]);
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
    <AssemblyName>Locus.Json</AssemblyName>
    <RootNamespace>Locus.Json</RootNamespace>
    <Version>${jsonVersion}</Version>
    <AssemblyVersion>${jsonVersion}.0</AssemblyVersion>
    <FileVersion>${jsonVersion}.0</FileVersion>
    <InformationalVersion>${jsonVersion}+newtonsoft-json-${jsonVersion}</InformationalVersion>
    <Deterministic>true</Deterministic>
    <GenerateAssemblyInfo>true</GenerateAssemblyInfo>
  </PropertyGroup>
  <ItemGroup>
    <Reference Include="Newtonsoft.Json">
      <HintPath>${path.join(sourceDir, "Newtonsoft.Json.dll")}</HintPath>
      <Private>false</Private>
    </Reference>
  </ItemGroup>
</Project>
`,
  );
  await writeFile(
    path.join(stubDir, "LocusJson.cs"),
    `using System;
using System.Globalization;

using Newtonsoft.Json;

namespace Locus.Json
{
    public static class LocusJson
    {
        private static readonly JsonSerializerSettings Settings = new JsonSerializerSettings
        {
            ConstructorHandling = ConstructorHandling.AllowNonPublicDefaultConstructor,
            Culture = CultureInfo.InvariantCulture,
            DateParseHandling = DateParseHandling.None,
            MetadataPropertyHandling = MetadataPropertyHandling.Ignore,
            MissingMemberHandling = MissingMemberHandling.Ignore,
            NullValueHandling = NullValueHandling.Include,
            ObjectCreationHandling = ObjectCreationHandling.Replace,
            TypeNameHandling = TypeNameHandling.None
        };

        public static object Deserialize(string json, Type type)
        {
            if (type == null)
                throw new ArgumentNullException("type");

            string source = string.IsNullOrWhiteSpace(json) ? "{}" : json;
            return JsonConvert.DeserializeObject(source, type, Settings);
        }

        public static T Deserialize<T>(string json)
        {
            object value = Deserialize(json, typeof(T));
            return value == null ? default(T) : (T)value;
        }

        public static string Serialize(object value)
        {
            return JsonConvert.SerializeObject(value, Formatting.None, Settings);
        }
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
    throw new Error(`missing JSON bundle inputs:\n${missing.join("\n")}`);
  }

  if (!existsSync(path.join(outputDir, "Locus.Json.dll.meta"))) {
    throw new Error("missing Unity meta file for Locus.Json.dll");
  }
}

async function cleanupOutputArtifacts() {
  const entries = await readdir(outputDir, { withFileTypes: true });

  await Promise.all(
    entries
      .filter(
        (entry) =>
          entry.name.startsWith("ILRepack-") ||
          (entry.name.startsWith("Locus.Json.dll.") && entry.name !== "Locus.Json.dll.meta"),
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
    "/internalize",
    "/renameinternalized",
    "/allowduplicateresources",
    `/out:${tmpOutputDll}`,
    `/lib:${sourceDir}`,
    stubDll,
    ...inputDlls.map((fileName) => path.join(sourceDir, fileName)),
  ]);

  const output = await stat(tmpOutputDll);
  if (output.size === 0) {
    throw new Error("Locus.Json.dll was generated as an empty file");
  }

  await rename(tmpOutputDll, outputDll);
  await cleanupOutputArtifacts();
}

await buildBundle();
