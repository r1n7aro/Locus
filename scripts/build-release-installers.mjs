import { existsSync, readFileSync, readdirSync, renameSync, statSync, unlinkSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const srcTauriDir = path.join(repoRoot, "src-tauri");
const nsisBundleDir = path.join(srcTauriDir, "target", "release", "bundle", "nsis");
const withoutEmbedConfig = path.join(srcTauriDir, "tauri.without_embed_python_git.conf.json");
const flavors = new Map([
  [
    "default",
    {
      label: "Windows x64",
      suffix: "",
      buildArgs: ["build"],
    },
  ],
  [
    "with_embed_python_git",
    {
      label: "Windows x64",
      suffix: "",
      buildArgs: ["build"],
    },
  ],
  [
    "without_embed_python_git",
    {
      label: "Windows x64 - without_embed_python_git",
      suffix: "without_embed_python_git",
      buildArgs: [
        "build",
        "--config",
        path.relative(repoRoot, withoutEmbedConfig),
      ],
    },
  ],
]);

function readJson(filePath) {
  return JSON.parse(readFileSync(filePath, "utf8"));
}

function usage() {
  const names = [...flavors.keys()].join(", ");
  return [
    "Usage: bun run release:installers [flavor...] [-- tauri args...]",
    "",
    `Flavors: ${names}`,
    "Default: without_embed_python_git default",
  ].join("\n");
}

function parseArgs(rawArgs) {
  const separatorIndex = rawArgs.indexOf("--");
  const flavorArgs = separatorIndex >= 0 ? rawArgs.slice(0, separatorIndex) : rawArgs;
  const tauriArgs = separatorIndex >= 0 ? rawArgs.slice(separatorIndex + 1) : [];
  const requestedFlavors = flavorArgs.length > 0 ? flavorArgs : ["without_embed_python_git", "default"];

  for (const flavor of requestedFlavors) {
    if (!flavors.has(flavor)) {
      throw new Error(`Unknown release installer flavor "${flavor}".\n\n${usage()}`);
    }
  }

  return { requestedFlavors, tauriArgs };
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env: process.env,
    stdio: "inherit",
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status ?? "unknown"}`);
  }
}

function expectedInstallerBaseName() {
  const tauriConfig = readJson(path.join(srcTauriDir, "tauri.conf.json"));
  const productName = tauriConfig.productName;
  const version = tauriConfig.version;

  if (!productName || !version) {
    throw new Error("Unable to resolve productName/version from src-tauri/tauri.conf.json.");
  }

  return `${productName}_${version}_x64-setup.exe`;
}

function installerNameForFlavor(baseName, suffix) {
  if (!suffix) {
    return baseName;
  }

  return baseName.replace(/-setup\.exe$/i, `-${suffix}-setup.exe`);
}

function findGeneratedInstaller(baseName, startedAtMs) {
  const exactPath = path.join(nsisBundleDir, baseName);

  if (existsSync(exactPath) && statSync(exactPath).mtimeMs >= startedAtMs - 1000) {
    return exactPath;
  }

  if (!existsSync(nsisBundleDir)) {
    throw new Error(`Unable to find NSIS bundle directory: ${nsisBundleDir}`);
  }

  const candidates = readdirSync(nsisBundleDir)
    .filter((fileName) => fileName.endsWith("-setup.exe"))
    .map((fileName) => {
      const filePath = path.join(nsisBundleDir, fileName);
      return { filePath, modifiedAt: statSync(filePath).mtimeMs };
    })
    .filter(({ modifiedAt }) => modifiedAt >= startedAtMs - 1000)
    .sort((left, right) => right.modifiedAt - left.modifiedAt);

  if (candidates[0]) {
    return candidates[0].filePath;
  }

  throw new Error(`Unable to find generated NSIS installer ${baseName}.`);
}

function finalizeInstaller(flavor, baseName, startedAtMs) {
  const sourcePath = findGeneratedInstaller(baseName, startedAtMs);
  const finalName = installerNameForFlavor(baseName, flavor.suffix);
  const finalPath = path.join(nsisBundleDir, finalName);

  if (sourcePath !== finalPath) {
    if (existsSync(finalPath)) {
      unlinkSync(finalPath);
    }
    renameSync(sourcePath, finalPath);
  }

  return finalPath;
}

function buildFlavor(flavorName, tauriArgs, baseName) {
  const flavor = flavors.get(flavorName);
  const startedAtMs = Date.now();
  console.log(`[locus] Building release installer flavor: ${flavorName}`);
  run("bun", ["tauri", ...flavor.buildArgs, ...tauriArgs]);
  const finalPath = finalizeInstaller(flavor, baseName, startedAtMs);

  return {
    flavor: flavorName,
    label: flavor.label,
    path: finalPath,
  };
}

try {
  const { requestedFlavors, tauriArgs } = parseArgs(process.argv.slice(2));
  const baseName = expectedInstallerBaseName();
  const results = requestedFlavors.map((flavorName) => buildFlavor(flavorName, tauriArgs, baseName));

  console.log("[locus] Release installers ready:");
  for (const result of results) {
    console.log(`- ${result.label}: ${path.relative(repoRoot, result.path)}`);
  }
} catch (error) {
  console.error(`[locus] Failed to build release installers: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
}
