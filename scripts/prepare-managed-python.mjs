import { createHash } from "node:crypto";
import { createWriteStream, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import https from "node:https";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const PYTHON_VERSION = "3.13.12";
const PYTHON_TAG = "python-3.13.12-embed-amd64";
const PYTHON_URL = `https://www.python.org/ftp/python/${PYTHON_VERSION}/${PYTHON_TAG}.zip`;
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const cacheDir = path.join(repoRoot, ".cache", "managed-python");
const outputRoot = path.join(repoRoot, "src-tauri", "gen", "managed-python");
const targetDir = path.join(outputRoot, "windows-x64");
const archivePath = path.join(cacheDir, `${PYTHON_TAG}.zip`);

function writeManifest(entry) {
  mkdirSync(outputRoot, { recursive: true });
  writeFileSync(
    path.join(outputRoot, "manifest.json"),
    `${JSON.stringify(
      {
        version: 1,
        generatedAt: new Date().toISOString(),
        runtimes: entry ? [entry] : [],
      },
      null,
      2,
    )}\n`,
  );
}

function download(url, destination) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode ?? 0)) {
        const location = response.headers.location;
        response.resume();
        if (!location) {
          reject(new Error(`redirect without location for ${url}`));
          return;
        }
        download(new URL(location, url).toString(), destination).then(resolve, reject);
        return;
      }
      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`download failed ${response.statusCode}: ${url}`));
        return;
      }
      const file = createWriteStream(destination);
      response.pipe(file);
      file.on("finish", () => file.close(resolve));
      file.on("error", reject);
    });
    request.on("error", reject);
  });
}

function sha256(filePath) {
  const hash = createHash("sha256");
  hash.update(readFileSync(filePath));
  return hash.digest("hex");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: "inherit",
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed with exit code ${result.status ?? "unknown"}`);
  }
}

function expandArchive(source, destination) {
  mkdirSync(destination, { recursive: true });
  if (process.platform === "win32") {
    const command = [
      "$ErrorActionPreference = 'Stop';",
      "Expand-Archive",
      "-LiteralPath",
      JSON.stringify(source),
      "-DestinationPath",
      JSON.stringify(destination),
      "-Force",
    ].join(" ");
    run("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command]);
    return;
  }
  run("unzip", ["-q", source, "-d", destination]);
}

function verifyPython(pythonExe) {
  const result = spawnSync(
    pythonExe,
    ["-c", "import json, pathlib, sys; print('{}.{}.{}'.format(*sys.version_info[:3]))"],
    { encoding: "utf8" },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`managed Python verification failed: ${result.stderr || result.stdout}`);
  }
  const version = result.stdout.trim();
  if (!version.startsWith(PYTHON_VERSION)) {
    throw new Error(`managed Python version mismatch: expected ${PYTHON_VERSION}, got ${version}`);
  }
  return version;
}

async function main() {
  if (process.platform !== "win32") {
    writeManifest(null);
    console.log("[locus] Managed Python skipped on non-Windows host.");
    return;
  }

  mkdirSync(cacheDir, { recursive: true });
  if (!existsSync(archivePath)) {
    console.log(`[locus] Downloading managed Python ${PYTHON_VERSION}...`);
    await download(PYTHON_URL, archivePath);
  } else {
    console.log(`[locus] Using cached managed Python archive: ${path.relative(repoRoot, archivePath)}`);
  }

  rmSync(targetDir, { recursive: true, force: true });
  expandArchive(archivePath, targetDir);

  const pythonExe = path.join(targetDir, "python.exe");
  const version = verifyPython(pythonExe);
  const digest = sha256(archivePath);
  writeManifest({
    id: "windows-x64",
    version,
    sourceUrl: PYTHON_URL,
    archiveSha256: digest,
    executable: "windows-x64/python.exe",
    license: "windows-x64/LICENSE.txt",
  });

  console.log(`[locus] Prepared managed Python ${version}: ${path.relative(repoRoot, targetDir)}`);
}

main().catch((error) => {
  console.error(`[locus] Failed to prepare managed Python: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
});
