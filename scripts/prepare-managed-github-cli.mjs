import { createHash } from "node:crypto";
import {
  copyFileSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import https from "node:https";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const GH_LATEST_RELEASE_URL = "https://github.com/cli/cli/releases/latest";
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const cacheDir = path.join(repoRoot, ".cache", "gh-runtime");
const outputRoot = path.join(repoRoot, "src-tauri", "gen", "gh-runtime");
const target = resolveTarget();

function resolveTarget() {
  const matrix = {
    "win32:x64": {
      id: "windows-x64",
      pattern: /^gh_.*_windows_amd64\.zip$/i,
      assetSuffix: "windows_amd64.zip",
      executableName: "gh.exe",
      archiveKind: "zip",
    },
    "win32:arm64": {
      id: "windows-arm64",
      pattern: /^gh_.*_windows_arm64\.zip$/i,
      assetSuffix: "windows_arm64.zip",
      executableName: "gh.exe",
      archiveKind: "zip",
    },
    "darwin:x64": {
      id: "macos-x64",
      pattern: /^gh_.*_macOS_amd64\.zip$/i,
      assetSuffix: "macOS_amd64.zip",
      executableName: "gh",
      archiveKind: "zip",
    },
    "darwin:arm64": {
      id: "macos-arm64",
      pattern: /^gh_.*_macOS_arm64\.zip$/i,
      assetSuffix: "macOS_arm64.zip",
      executableName: "gh",
      archiveKind: "zip",
    },
    "linux:x64": {
      id: "linux-x64",
      pattern: /^gh_.*_linux_amd64\.tar\.gz$/i,
      assetSuffix: "linux_amd64.tar.gz",
      executableName: "gh",
      archiveKind: "tar.gz",
    },
    "linux:arm64": {
      id: "linux-arm64",
      pattern: /^gh_.*_linux_arm64\.tar\.gz$/i,
      assetSuffix: "linux_arm64.tar.gz",
      executableName: "gh",
      archiveKind: "tar.gz",
    },
  };
  return matrix[`${process.platform}:${process.arch}`] ?? null;
}

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

function requestHeaders(json) {
  const headers = {
    "User-Agent": "Locus GitHub CLI bundler",
    Accept: json ? "application/vnd.github+json" : "application/octet-stream",
  };
  const token = process.env.GITHUB_TOKEN?.trim() || process.env.GH_TOKEN?.trim();
  if (token) headers.Authorization = `Bearer ${token}`;
  return headers;
}

function request(url, { json = false } = {}) {
  return new Promise((resolve, reject) => {
    const req = https.get(
      url,
      {
        headers: requestHeaders(json),
      },
      (response) => {
        if ([301, 302, 303, 307, 308].includes(response.statusCode ?? 0)) {
          const location = response.headers.location;
          response.resume();
          if (!location) {
            reject(new Error(`redirect without location for ${url}`));
            return;
          }
          request(new URL(location, url).toString(), { json }).then(resolve, reject);
          return;
        }

        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`request failed ${response.statusCode}: ${url}`));
          return;
        }

        if (!json) {
          resolve(response);
          return;
        }

        let body = "";
        response.setEncoding("utf8");
        response.on("data", (chunk) => {
          body += chunk;
        });
        response.on("end", () => {
          try {
            resolve(JSON.parse(body));
          } catch (error) {
            reject(error);
          }
        });
      },
    );
    req.on("error", reject);
  });
}

async function download(url, destination) {
  const tempDestination = `${destination}.download`;
  rmSync(tempDestination, { force: true });

  try {
    const response = await request(url);
    await new Promise((resolve, reject) => {
      const file = createWriteStream(tempDestination);
      response.pipe(file);
      response.on("error", reject);
      file.on("finish", () => file.close(resolve));
      file.on("error", reject);
    });
    renameSync(tempDestination, destination);
  } catch (error) {
    rmSync(tempDestination, { force: true });
    throw error;
  }
}

async function resolveGithubCliAsset() {
  const overrideUrl = process.env.LOCUS_GITHUB_CLI_URL?.trim();
  if (overrideUrl) {
    const name = path.basename(new URL(overrideUrl).pathname);
    return {
      tagName: process.env.LOCUS_GITHUB_CLI_VERSION?.trim() || name,
      name,
      url: overrideUrl,
    };
  }

  const tagName = await resolveLatestReleaseTag();
  const name = githubCliAssetName(tagName);
  return {
    tagName,
    name,
    url: `https://github.com/cli/cli/releases/download/${tagName}/${name}`,
  };
}

function githubCliAssetName(tagName) {
  const version = tagName.replace(/^v/i, "");
  return `gh_${version}_${target.assetSuffix}`;
}

function tagFromReleaseUrl(url) {
  const parsed = new URL(url);
  const match = parsed.pathname.match(/\/cli\/cli\/releases\/tag\/([^/]+)/);
  return match ? decodeURIComponent(match[1]) : null;
}

function resolveLatestReleaseTag(url = GH_LATEST_RELEASE_URL, depth = 0) {
  if (depth > 8) {
    return Promise.reject(new Error(`too many redirects while resolving ${GH_LATEST_RELEASE_URL}`));
  }

  return new Promise((resolve, reject) => {
    const req = https.get(
      url,
      {
        headers: requestHeaders(false),
      },
      (response) => {
        if ([301, 302, 303, 307, 308].includes(response.statusCode ?? 0)) {
          const location = response.headers.location;
          response.resume();
          if (!location) {
            reject(new Error(`redirect without location for ${url}`));
            return;
          }
          const nextUrl = new URL(location, url).toString();
          const tag = tagFromReleaseUrl(nextUrl);
          if (tag) {
            resolve(tag);
            return;
          }
          resolveLatestReleaseTag(nextUrl, depth + 1).then(resolve, reject);
          return;
        }

        response.resume();
        const tag = tagFromReleaseUrl(url);
        if (tag) {
          resolve(tag);
          return;
        }
        reject(new Error(`unable to resolve latest GitHub CLI release tag: HTTP ${response.statusCode ?? "unknown"}`));
      },
    );
    req.on("error", reject);
  });
}

function findCachedGithubCliArchive() {
  if (!existsSync(cacheDir)) {
    return null;
  }

  const candidates = readdirSync(cacheDir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && target.pattern.test(entry.name))
    .map((entry) => {
      const archivePath = path.join(cacheDir, entry.name);
      const stat = statSync(archivePath);
      return {
        name: entry.name,
        path: archivePath,
        mtimeMs: stat.mtimeMs,
        size: stat.size,
      };
    })
    .filter((entry) => entry.size > 0)
    .sort((left, right) => right.mtimeMs - left.mtimeMs || right.name.localeCompare(left.name));

  return candidates[0] ?? null;
}

function sha256(filePath) {
  const hash = createHash("sha256");
  hash.update(readFileSync(filePath));
  return hash.digest("hex");
}

function formatError(error) {
  return error instanceof Error ? error.message : String(error);
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

function powershellSingleQuoted(value) {
  return `'${String(value).replace(/'/g, "''")}'`;
}

function expandArchive(source, destination) {
  rmSync(destination, { recursive: true, force: true });
  mkdirSync(destination, { recursive: true });

  if (target.archiveKind === "zip") {
    if (process.platform === "win32") {
      const command = `Expand-Archive -LiteralPath ${powershellSingleQuoted(source)} -DestinationPath ${powershellSingleQuoted(destination)} -Force`;
      run("powershell.exe", [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        command,
      ]);
    } else {
      run("unzip", ["-q", source, "-d", destination]);
    }
    return;
  }

  run("tar", ["-xzf", source, "-C", destination]);
}

function walkFiles(root) {
  if (!existsSync(root)) return [];
  const files = [];
  const stack = [root];
  while (stack.length > 0) {
    const dir = stack.pop();
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const entryPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(entryPath);
      } else if (entry.isFile()) {
        files.push(entryPath);
      }
    }
  }
  return files;
}

function isLegalFileName(filePath) {
  const normalized = path.basename(filePath).toLowerCase();
  return (
    normalized.startsWith("license") ||
    normalized.startsWith("licence") ||
    normalized.startsWith("notice") ||
    normalized.startsWith("notices") ||
    normalized.startsWith("copying")
  );
}

function findGithubCliExecutable(extractedDir) {
  const candidates = walkFiles(extractedDir).filter((filePath) => path.basename(filePath) === target.executableName);
  candidates.sort((left, right) => {
    const leftScore = left.includes(`${path.sep}bin${path.sep}`) ? 0 : 1;
    const rightScore = right.includes(`${path.sep}bin${path.sep}`) ? 0 : 1;
    return leftScore - rightScore || left.localeCompare(right);
  });
  return candidates[0] ?? null;
}

function copyRuntimeFiles(extractedDir, destination) {
  rmSync(destination, { recursive: true, force: true });
  mkdirSync(path.join(destination, "bin"), { recursive: true });

  const executable = findGithubCliExecutable(extractedDir);
  if (!executable) {
    throw new Error(`Unable to find ${target.executableName} in extracted GitHub CLI archive.`);
  }
  const executableDestination = path.join(destination, "bin", target.executableName);
  copyFileSync(executable, executableDestination);
  if (process.platform !== "win32") {
    run("chmod", ["755", executableDestination]);
  }

  const legalFiles = walkFiles(extractedDir).filter(isLegalFileName);
  if (legalFiles.length === 0) {
    throw new Error("No GitHub CLI license or notice files found in the downloaded archive.");
  }
  for (const source of legalFiles) {
    copyFileSync(source, path.join(destination, path.basename(source)));
  }

  return {
    executable: executableDestination,
    legalFiles: legalFiles.map((filePath) => path.basename(filePath)).sort((left, right) => left.localeCompare(right)),
  };
}

function verifyGithubCli(executable) {
  const result = spawnSync(executable, ["--version"], {
    encoding: "utf8",
    env: {
      ...process.env,
      GH_TELEMETRY: "false",
      DO_NOT_TRACK: "true",
      GH_NO_UPDATE_NOTIFIER: "1",
      GH_NO_EXTENSION_UPDATE_NOTIFIER: "1",
    },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`GitHub CLI verification failed: ${result.stderr || result.stdout}`);
  }
  const versionLine = result.stdout.trim().split(/\r?\n/)[0] ?? "";
  if (!versionLine.startsWith("gh version ")) {
    throw new Error(`GitHub CLI version output mismatch: ${versionLine}`);
  }
  return versionLine.replace(/^gh version\s+/, "").split(/\s+/)[0];
}

function releaseForManifest(asset, version) {
  return asset.tagName ?? `v${version}`;
}

function sourceUrlForManifest(asset, release) {
  return asset.url ?? `https://github.com/cli/cli/releases/download/${release}/${asset.name}`;
}

async function main() {
  if (!target) {
    writeManifest(null);
    console.log(`[locus] GitHub CLI skipped on unsupported host: ${process.platform}/${process.arch}`);
    return;
  }

  mkdirSync(cacheDir, { recursive: true });

  let asset;
  let archivePath;
  try {
    asset = await resolveGithubCliAsset();
    archivePath = path.join(cacheDir, asset.name);

    if (!existsSync(archivePath)) {
      console.log(`[locus] Downloading GitHub CLI ${asset.tagName}...`);
      await download(asset.url, archivePath);
    } else {
      console.log(`[locus] Using cached GitHub CLI archive: ${path.relative(repoRoot, archivePath)}`);
    }
  } catch (error) {
    const cached = findCachedGithubCliArchive();
    if (!cached) {
      throw error;
    }

    asset = {
      tagName: null,
      name: cached.name,
      url: null,
    };
    archivePath = cached.path;

    console.warn(`[locus] GitHub CLI metadata or download unavailable: ${formatError(error)}`);
    console.warn(`[locus] Falling back to cached GitHub CLI archive: ${path.relative(repoRoot, archivePath)}`);
  }

  const extractDir = path.join(outputRoot, ".extract", target.id);
  const targetDir = path.join(outputRoot, target.id);
  expandArchive(archivePath, extractDir);
  const runtime = copyRuntimeFiles(extractDir, targetDir);
  rmSync(path.join(outputRoot, ".extract"), { recursive: true, force: true });

  const version = verifyGithubCli(runtime.executable);
  const release = releaseForManifest(asset, version);
  const license = runtime.legalFiles.find((fileName) => fileName.toLowerCase().startsWith("license")) ?? runtime.legalFiles[0];
  writeManifest({
    id: target.id,
    version,
    release,
    sourceUrl: sourceUrlForManifest(asset, release),
    archiveSha256: sha256(archivePath),
    executable: `${target.id}/bin/${target.executableName}`,
    license: `${target.id}/${license}`,
    legalFiles: runtime.legalFiles.map((fileName) => `${target.id}/${fileName}`),
  });

  console.log(`[locus] Prepared GitHub CLI ${version}: ${path.relative(repoRoot, targetDir)}`);
}

main().catch((error) => {
  console.error(`[locus] Failed to prepare GitHub CLI: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
});
