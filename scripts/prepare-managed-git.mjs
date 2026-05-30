import { createHash } from "node:crypto";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import https from "node:https";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const GIT_RELEASE_API = "https://api.github.com/repos/git-for-windows/git/releases/latest";
const GIT_ASSET_PATTERN = /^PortableGit-.*-64-bit\.7z\.exe$/;
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const cacheDir = path.join(repoRoot, ".cache", "managed-git");
const outputRoot = path.join(repoRoot, "src-tauri", "gen", "managed-git");
const targetDir = path.join(outputRoot, "windows-x64");

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

function request(url, { json = false } = {}) {
  return new Promise((resolve, reject) => {
    const req = https.get(
      url,
      {
        headers: {
          "User-Agent": "Locus managed Git bundler",
          "Accept": json ? "application/vnd.github+json" : "application/octet-stream",
        },
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

async function resolveGitAsset() {
  const overrideUrl = process.env.LOCUS_MANAGED_GIT_URL?.trim();
  if (overrideUrl) {
    const name = path.basename(new URL(overrideUrl).pathname);
    return {
      tagName: process.env.LOCUS_MANAGED_GIT_VERSION?.trim() || name,
      name,
      url: overrideUrl,
    };
  }

  const release = await request(GIT_RELEASE_API, { json: true });
  const asset = release.assets?.find((entry) => GIT_ASSET_PATTERN.test(entry.name));
  if (!asset?.browser_download_url) {
    throw new Error("Unable to find a Git for Windows PortableGit 64-bit asset in the latest release.");
  }
  return {
    tagName: release.tag_name || asset.name,
    name: asset.name,
    url: asset.browser_download_url,
  };
}

function sha256(filePath) {
  const hash = createHash("sha256");
  hash.update(readFileSync(filePath));
  return hash.digest("hex");
}

function findCachedGitArchive() {
  if (!existsSync(cacheDir)) {
    return null;
  }

  const candidates = readdirSync(cacheDir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && GIT_ASSET_PATTERN.test(entry.name))
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

function formatError(error) {
  return error instanceof Error ? error.message : String(error);
}

function gitReleaseForManifest(asset, version) {
  return asset.tagName ?? `v${version}`;
}

function gitSourceUrlForManifest(asset, release) {
  return asset.url ?? `https://github.com/git-for-windows/git/releases/download/${release}/${asset.name}`;
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

function expandPortableGit(source, destination) {
  rmSync(destination, { recursive: true, force: true });
  mkdirSync(destination, { recursive: true });
  run(source, ["-y", `-o${destination}`]);
}

function verifyGit(gitExe) {
  const result = spawnSync(gitExe, ["--version"], { encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`managed Git verification failed: ${result.stderr || result.stdout}`);
  }
  const version = result.stdout.trim();
  if (!version.startsWith("git version ")) {
    throw new Error(`managed Git version output mismatch: ${version}`);
  }
  return version.replace(/^git version\s+/, "");
}

async function main() {
  if (process.platform !== "win32") {
    writeManifest(null);
    console.log("[locus] Managed Git skipped on non-Windows host.");
    return;
  }

  mkdirSync(cacheDir, { recursive: true });

  let asset;
  let archivePath;
  try {
    asset = await resolveGitAsset();
    archivePath = path.join(cacheDir, asset.name);

    if (!existsSync(archivePath)) {
      console.log(`[locus] Downloading managed Git ${asset.tagName}...`);
      await download(asset.url, archivePath);
    } else {
      console.log(`[locus] Using cached managed Git archive: ${path.relative(repoRoot, archivePath)}`);
    }
  } catch (error) {
    const cached = findCachedGitArchive();
    if (!cached) {
      throw error;
    }

    asset = {
      tagName: null,
      name: cached.name,
      url: null,
    };
    archivePath = cached.path;

    console.warn(`[locus] Managed Git metadata or download unavailable: ${formatError(error)}`);
    console.warn(`[locus] Falling back to cached managed Git archive: ${path.relative(repoRoot, archivePath)}`);
  }

  expandPortableGit(archivePath, targetDir);

  const gitExe = path.join(targetDir, "cmd", "git.exe");
  const version = verifyGit(gitExe);
  const release = gitReleaseForManifest(asset, version);
  writeManifest({
    id: "windows-x64",
    version,
    release,
    sourceUrl: gitSourceUrlForManifest(asset, release),
    archiveSha256: sha256(archivePath),
    executable: "windows-x64/cmd/git.exe",
    license: "windows-x64/LICENSE.txt",
  });

  console.log(`[locus] Prepared managed Git ${version}: ${path.relative(repoRoot, targetDir)}`);
}

main().catch((error) => {
  console.error(`[locus] Failed to prepare managed Git: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
});
