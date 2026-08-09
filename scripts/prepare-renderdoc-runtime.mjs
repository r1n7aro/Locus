import { createHash } from "node:crypto";
import {
  cpSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import https from "node:https";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const RENDERDOC_VERSION = "1.45";
const ARCHIVE_NAME = `RenderDoc_${RENDERDOC_VERSION}_64.zip`;
const SOURCE_URL = `https://renderdoc.org/stable/${RENDERDOC_VERSION}/${ARCHIVE_NAME}`;
const ARCHIVE_BYTES = 97_805_614;
const ARCHIVE_SHA256 = "bd665c348a8245d10a1f513e35b83603edc1a78006277583d09ec0769286eea4";
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const archivePath = path.join(repoRoot, ".cache", "renderdoc-runtime", ARCHIVE_NAME);
const runtimeParent = path.join(repoRoot, "skills", "graphics-debugger", "runtime");
const targetDir = path.join(runtimeParent, "windows-x64");
const manifestPath = path.join(runtimeParent, "manifest.json");

function request(url, headers = {}, depth = 0) {
  if (depth > 8) return Promise.reject(new Error(`too many redirects for ${SOURCE_URL}`));
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { "User-Agent": "Locus RenderDoc bundler", ...headers } }, (response) => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode ?? 0)) {
        const location = response.headers.location;
        response.resume();
        if (!location) {
          reject(new Error(`redirect without location for ${url}`));
          return;
        }
        request(new URL(location, url).toString(), headers, depth + 1).then(resolve, reject);
        return;
      }
      resolve(response);
    });
    req.on("error", reject);
  });
}

async function downloadWithResume(url, destination) {
  mkdirSync(path.dirname(destination), { recursive: true });
  let offset = existsSync(destination) ? statSync(destination).size : 0;
  if (offset > ARCHIVE_BYTES) {
    rmSync(destination, { force: true });
    offset = 0;
  }
  if (offset === ARCHIVE_BYTES) return;

  const headers = offset > 0 ? { Range: `bytes=${offset}-` } : {};
  const response = await request(url, headers);
  const status = response.statusCode ?? 0;
  if (status !== 200 && status !== 206) {
    response.resume();
    throw new Error(`download failed ${status}: ${url}`);
  }
  const append = status === 206 && offset > 0;
  await new Promise((resolve, reject) => {
    const file = createWriteStream(destination, { flags: append ? "a" : "w" });
    response.pipe(file);
    response.on("error", reject);
    file.on("finish", () => file.close(resolve));
    file.on("error", reject);
  });
}

function sha256(filePath) {
  const hash = createHash("sha256");
  hash.update(readFileSync(filePath));
  return hash.digest("hex");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed with exit code ${result.status ?? "unknown"}`);
  }
}

function expandArchive(source, destination) {
  rmSync(destination, { recursive: true, force: true });
  mkdirSync(destination, { recursive: true });
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
}

function findRuntimeRoot(directory) {
  const queue = [directory];
  while (queue.length > 0) {
    const current = queue.shift();
    if (existsSync(path.join(current, "qrenderdoc.exe"))) return current;
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      if (entry.isDirectory()) queue.push(path.join(current, entry.name));
    }
  }
  throw new Error("RenderDoc archive does not contain qrenderdoc.exe");
}

function requireRuntimeFile(root, relativePath) {
  const resolved = path.join(root, relativePath);
  if (!existsSync(resolved) || !statSync(resolved).isFile()) {
    throw new Error(`RenderDoc runtime is missing ${relativePath}`);
  }
}

function verifyRuntime(root) {
  for (const relativePath of [
    "renderdoc.dll",
    "qrenderdoc.exe",
    "renderdoccmd.exe",
    "renderdoc_app.h",
    "LICENSE.md",
    "python36.dll",
  ]) {
    requireRuntimeFile(root, relativePath);
  }
  const result = spawnSync(path.join(root, "qrenderdoc.exe"), ["--version"], {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 || !result.stdout.includes(`v${RENDERDOC_VERSION}`)) {
    throw new Error(`RenderDoc runtime version check failed: ${result.stderr || result.stdout}`);
  }
  return result.stdout.trim();
}

function writeManifest(versionOutput) {
  mkdirSync(runtimeParent, { recursive: true });
  writeFileSync(
    manifestPath,
    `${JSON.stringify(
      {
        version: 1,
        renderDocVersion: RENDERDOC_VERSION,
        sourceUrl: SOURCE_URL,
        archiveBytes: ARCHIVE_BYTES,
        archiveSha256: ARCHIVE_SHA256,
        platform: "windows-x64",
        runtimeDirectory: "windows-x64",
        captureLibrary: "windows-x64/renderdoc.dll",
        pythonExecutable: "windows-x64/qrenderdoc.exe",
        versionOutput,
      },
      null,
      2,
    )}\n`,
  );
}

async function main() {
  if (process.platform !== "win32" || process.arch !== "x64") {
    console.log("[locus] RenderDoc runtime is currently bundled only on Windows x64.");
    return;
  }

  if (existsSync(targetDir) && existsSync(manifestPath)) {
    try {
      const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
      if (
        manifest.renderDocVersion === RENDERDOC_VERSION
        && manifest.archiveSha256 === ARCHIVE_SHA256
      ) {
        const versionOutput = verifyRuntime(targetDir);
        writeManifest(versionOutput);
        console.log(`[locus] Using prepared RenderDoc ${RENDERDOC_VERSION}: ${path.relative(repoRoot, targetDir)}`);
        return;
      }
    } catch (error) {
      console.warn(`[locus] Existing RenderDoc runtime will be replaced: ${error.message ?? error}`);
    }
  }

  await downloadWithResume(SOURCE_URL, archivePath);
  const archiveSize = statSync(archivePath).size;
  if (archiveSize !== ARCHIVE_BYTES) {
    throw new Error(`RenderDoc archive size mismatch: expected ${ARCHIVE_BYTES}, got ${archiveSize}`);
  }
  const digest = sha256(archivePath);
  if (digest !== ARCHIVE_SHA256) {
    throw new Error(`RenderDoc archive sha256 mismatch: expected ${ARCHIVE_SHA256}, got ${digest}`);
  }

  const extractRoot = path.join(repoRoot, ".tmp", `renderdoc-runtime-${process.pid}`);
  try {
    expandArchive(archivePath, extractRoot);
    const sourceRoot = findRuntimeRoot(extractRoot);
    verifyRuntime(sourceRoot);
    rmSync(targetDir, { recursive: true, force: true });
    mkdirSync(runtimeParent, { recursive: true });
    cpSync(sourceRoot, targetDir, { recursive: true });
    const versionOutput = verifyRuntime(targetDir);
    writeManifest(versionOutput);
    console.log(`[locus] Prepared RenderDoc ${RENDERDOC_VERSION}: ${path.relative(repoRoot, targetDir)}`);
  } finally {
    rmSync(extractRoot, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(`[locus] Failed to prepare RenderDoc: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
});
