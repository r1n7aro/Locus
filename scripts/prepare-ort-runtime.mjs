import fs from "node:fs";
import https from "node:https";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const ortPackageId = "microsoft.ml.onnxruntime.directml";
const ortPackageVersion = "1.23.0";
const directMlPackageId = "microsoft.ai.directml";
const directMlPackageVersion = "1.15.4";
const runtimeId = "win-x64";
const cacheDir = path.join(repoRoot, ".cache", "ort-runtime");
const outputDir = path.join(repoRoot, "src-tauri", "gen", "ort-runtime", "windows-x64");
const ortPackage = nugetPackage(ortPackageId, ortPackageVersion);
const directMlPackage = nugetPackage(directMlPackageId, directMlPackageVersion);
const ortRuntimeFiles = [
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
];

main().catch((error) => {
  console.error(`[locus] Failed to prepare ONNX Runtime DLLs: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
});

async function main() {
  ensureDirectory(cacheDir);
  await preparePackage(ortPackage);
  await preparePackage(directMlPackage);
  resetDirectory(outputDir);

  const nativeDir = path.join(ortPackage.extractDir, "runtimes", runtimeId, "native");
  for (const fileName of ortRuntimeFiles) {
    const source = path.join(nativeDir, fileName);
    if (!fs.existsSync(source)) {
      throw new Error(`Missing ${fileName} in ${nativeDir}`);
    }
    fs.copyFileSync(source, path.join(outputDir, fileName));
  }
  const directMlDll = findRuntimeFile(directMlPackage.extractDir, "DirectML.dll");
  if (!directMlDll) {
    throw new Error(`Missing DirectML.dll in ${directMlPackage.extractDir}`);
  }
  fs.copyFileSync(directMlDll, path.join(outputDir, "DirectML.dll"));

  writeJson(path.join(outputDir, "manifest.json"), {
    runtimeId,
    packages: [
      {
        id: ortPackage.id,
        version: ortPackage.version,
        source: ortPackage.url,
      },
      {
        id: directMlPackage.id,
        version: directMlPackage.version,
        source: directMlPackage.url,
      },
    ],
    files: [...ortRuntimeFiles, "DirectML.dll"],
  });

  console.log(`[locus] ONNX Runtime DLLs ready: ${path.relative(repoRoot, outputDir)}`);
}

async function preparePackage(pkg) {
  await ensureDownloaded(pkg.url, pkg.packagePath);
  ensureExtracted(pkg.packagePath, pkg.extractDir);
}

async function ensureDownloaded(url, target) {
  if (fs.existsSync(target) && fs.statSync(target).size > 0) {
    return;
  }

  const tempTarget = `${target}.tmp`;
  if (fs.existsSync(tempTarget)) {
    fs.rmSync(tempTarget, { force: true });
  }

  console.log(`[locus] Downloading ${path.basename(target, ".nupkg")}...`);
  try {
    await download(url, tempTarget);
  } catch (error) {
    console.warn(`[locus] Direct HTTPS download failed, retrying with PowerShell: ${error.message ?? error}`);
    downloadWithPowerShell(url, tempTarget);
  }
  fs.renameSync(tempTarget, target);
}

function ensureExtracted(source, target) {
  const marker = path.join(target, ".locus-extracted");
  if (fs.existsSync(marker)) {
    return;
  }

  resetDirectory(target);
  if (!extractWithTar(source, target)) {
    extractWithPowerShell(source, target);
  }

  fs.writeFileSync(marker, new Date().toISOString(), "utf8");
}

function extractWithTar(source, target) {
  const result = spawnSync("tar", ["-xf", source, "-C", target], {
    cwd: repoRoot,
    stdio: "inherit",
  });

  return !result.error && result.status === 0;
}

function extractWithPowerShell(source, target) {
  const result = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      `Expand-Archive -LiteralPath ${quotePowerShell(source)} -DestinationPath ${quotePowerShell(target)} -Force`,
    ],
    { cwd: repoRoot, stdio: "inherit" },
  );

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`Expand-Archive failed with exit code ${result.status ?? "unknown"}`);
  }
}

function nugetPackage(id, version) {
  return {
    id,
    version,
    url: `https://api.nuget.org/v3-flatcontainer/${id}/${version}/${id}.${version}.nupkg`,
    packagePath: path.join(cacheDir, `${id}.${version}.nupkg`),
    extractDir: path.join(cacheDir, `${id}-${version}`),
  };
}

function findRuntimeFile(root, fileName) {
  const matches = [];
  walkFiles(root, (filePath) => {
    if (path.basename(filePath).toLowerCase() === fileName.toLowerCase()) {
      matches.push(filePath);
    }
  });
  matches.sort((left, right) => runtimeFileScore(right) - runtimeFileScore(left));
  return matches[0] ?? null;
}

function runtimeFileScore(filePath) {
  const normalized = filePath.toLowerCase().replaceAll("\\", "/");
  let score = 0;
  if (normalized.includes("/win-x64/")) score += 4;
  if (normalized.includes("/x64-win/")) score += 4;
  if (normalized.includes("x64")) score += 2;
  if (normalized.includes("/native/")) score += 1;
  return score;
}

function walkFiles(dir, visit) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkFiles(entryPath, visit);
    } else if (entry.isFile()) {
      visit(entryPath);
    }
  }
}

function download(url, destination) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
        response.resume();
        download(new URL(response.headers.location, url).toString(), destination).then(resolve, reject);
        return;
      }

      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`download failed ${response.statusCode}: ${url}`));
        return;
      }

      const file = fs.createWriteStream(destination);
      response.pipe(file);
      file.on("finish", () => file.close(resolve));
      file.on("error", reject);
    });

    request.on("error", reject);
  });
}

function downloadWithPowerShell(url, destination) {
  const result = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      [
        "$ProgressPreference = 'SilentlyContinue'",
        `Invoke-WebRequest -UseBasicParsing -Uri ${quotePowerShell(url)} -OutFile ${quotePowerShell(destination)}`,
      ].join("; "),
    ],
    { cwd: repoRoot, stdio: "inherit" },
  );

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`Invoke-WebRequest failed with exit code ${result.status ?? "unknown"}`);
  }
}

function ensureDirectory(dir) {
  fs.mkdirSync(dir, { recursive: true });
}

function resetDirectory(dir) {
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function quotePowerShell(value) {
  return `'${value.replaceAll("'", "''")}'`;
}
