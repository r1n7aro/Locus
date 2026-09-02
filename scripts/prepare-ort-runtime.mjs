import { createHash } from "node:crypto";
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
const ortPackage = nugetPackage(ortPackageId, ortPackageVersion, {
  bytes: 12_746_067,
  sha512: "+/4bdM1zm7mgLfmZKwUjipdxz7kMt++FSvNR6y07V+x9th/QHN7jOGuy7voU9qBTM5OaGE59WhogP+0gnTJfrQ==",
});
const directMlPackage = nugetPackage(directMlPackageId, directMlPackageVersion, {
  bytes: 202_292_617,
  sha512: "/edn9WkEq8kP1T9l2HKckYq39uPF4ezdR5kI/AK0U1zysIYPerKsubcx1suAm3LD1dTQKFP7j16gIqR7xE7yhQ==",
});
const runtimeOutputFiles = [
  {
    name: "onnxruntime.dll",
    bytes: 17_201_208,
    sha256: "f5131591edac6b0a8090d0e329040a49319d7a689cb5b465235fbf7030fa8027",
  },
  {
    name: "onnxruntime_providers_shared.dll",
    bytes: 22_048,
    sha256: "3b27e1417d12b73a6a34d80414c083e359e092d2f0ce572d7e67be8cdbe9e825",
  },
  {
    name: "DirectML.dll",
    bytes: 18_527_776,
    sha256: "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1",
  },
];
const ortRuntimeFiles = runtimeOutputFiles.slice(0, 2).map(({ name }) => name);

main().catch((error) => {
  console.error(`[locus] Failed to prepare ONNX Runtime DLLs: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
});

async function main() {
  ensureDirectory(cacheDir);
  if (await preparedOutputMatches()) {
    console.log(`[locus] ONNX Runtime DLLs already ready: ${path.relative(repoRoot, outputDir)}`);
    return;
  }

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
    files: runtimeOutputFiles.map(({ name }) => name),
  });

  if (!(await preparedOutputMatches())) {
    throw new Error(`Prepared ONNX Runtime DLLs failed integrity validation: ${outputDir}`);
  }
  console.log(`[locus] ONNX Runtime DLLs ready: ${path.relative(repoRoot, outputDir)}`);
}

async function preparedOutputMatches() {
  const manifestPath = path.join(outputDir, "manifest.json");
  if (!fs.existsSync(manifestPath)) {
    return false;
  }

  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  } catch {
    return false;
  }

  const expectedPackages = [ortPackage, directMlPackage];
  if (
    manifest.runtimeId !== runtimeId ||
    !Array.isArray(manifest.packages) ||
    !expectedPackages.every(
      (pkg, index) =>
        manifest.packages[index]?.id === pkg.id &&
        manifest.packages[index]?.version === pkg.version,
    )
  ) {
    return false;
  }

  for (const file of runtimeOutputFiles) {
    const filePath = path.join(outputDir, file.name);
    if (!fs.existsSync(filePath) || fs.statSync(filePath).size !== file.bytes) {
      return false;
    }
    if ((await hashFile(filePath, "sha256", "hex")) !== file.sha256) {
      return false;
    }
  }

  return true;
}

async function preparePackage(pkg) {
  await ensureDownloaded(pkg);
  ensureExtracted(pkg.packagePath, pkg.extractDir);
}

async function ensureDownloaded(pkg) {
  const { url, packagePath: target } = pkg;
  if (await packageMatches(pkg, target)) {
    return;
  }

  if (fs.existsSync(target)) {
    console.warn(`[locus] Removing invalid cached package ${path.basename(target)}.`);
    fs.rmSync(target, { force: true });
  }

  const tempTarget = `${target}.tmp`;
  if (await packageMatches(pkg, tempTarget)) {
    fs.renameSync(tempTarget, target);
    return;
  }
  if (fs.existsSync(tempTarget) && fs.statSync(tempTarget).size >= pkg.bytes) {
    console.warn(`[locus] Removing invalid completed download ${path.basename(tempTarget)}.`);
    fs.rmSync(tempTarget, { force: true });
  }

  const partialBytes = fs.existsSync(tempTarget) ? fs.statSync(tempTarget).size : 0;
  const action = partialBytes > 0 ? `Resuming at ${partialBytes} bytes` : "Downloading";
  console.log(`[locus] ${action} ${path.basename(target, ".nupkg")}...`);

  try {
    downloadWithCurl(url, tempTarget);
  } catch (curlError) {
    console.warn(`[locus] curl download failed, retrying direct HTTPS: ${errorMessage(curlError)}`);
    const fallbackTarget = `${target}.fallback.tmp`;
    fs.rmSync(fallbackTarget, { force: true });
    try {
      await download(url, fallbackTarget);
      fs.rmSync(tempTarget, { force: true });
      fs.renameSync(fallbackTarget, tempTarget);
    } catch (httpsError) {
      console.warn(
        `[locus] Direct HTTPS download failed, retrying with PowerShell: ${errorMessage(httpsError)}`,
      );
      fs.rmSync(fallbackTarget, { force: true });
      try {
        downloadWithPowerShell(url, fallbackTarget);
        fs.rmSync(tempTarget, { force: true });
        fs.renameSync(fallbackTarget, tempTarget);
      } finally {
        fs.rmSync(fallbackTarget, { force: true });
      }
    }
  }

  if (!(await packageMatches(pkg, tempTarget))) {
    fs.rmSync(tempTarget, { force: true });
    throw new Error(`Downloaded package failed integrity validation: ${path.basename(target)}`);
  }
  fs.renameSync(tempTarget, target);
}

async function packageMatches(pkg, filePath) {
  if (!fs.existsSync(filePath) || fs.statSync(filePath).size !== pkg.bytes) {
    return false;
  }

  return (await sha512(filePath)) === pkg.sha512;
}

function sha512(filePath) {
  return hashFile(filePath, "sha512", "base64");
}

function hashFile(filePath, algorithm, encoding) {
  return new Promise((resolve, reject) => {
    const hash = createHash(algorithm);
    const stream = fs.createReadStream(filePath);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest(encoding)));
    stream.on("error", reject);
  });
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

function nugetPackage(id, version, integrity) {
  return {
    id,
    version,
    ...integrity,
    url: `https://api.nuget.org/v3-flatcontainer/${id}/${version}/${id}.${version}.nupkg`,
    packagePath: path.join(cacheDir, `${id}.${version}.nupkg`),
    extractDir: path.join(cacheDir, `${id}-${version}`),
  };
}

function downloadWithCurl(url, destination) {
  const command = process.platform === "win32" ? "curl.exe" : "curl";
  const result = spawnSync(
    command,
    [
      "--fail",
      "--location",
      "--retry",
      "5",
      "--retry-all-errors",
      "--retry-delay",
      "2",
      "--connect-timeout",
      "30",
      "--continue-at",
      "-",
      "--output",
      destination,
      url,
    ],
    { cwd: repoRoot, stdio: "inherit" },
  );

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`curl failed with exit code ${result.status ?? "unknown"}`);
  }
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
        "$ErrorActionPreference = 'Stop'",
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

function errorMessage(error) {
  return error?.message ?? String(error);
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
