import { copyFileSync, existsSync, readFileSync, readdirSync, renameSync, statSync, unlinkSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertPublicArguments, assertClientBundle } from "./client-edition.mjs";
import { assertPublicSourceBoundary } from "./public-source-boundary.mjs";
import { remoteAppPrerequisiteErrors } from "./prepare-remote-app.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const srcTauriDir = path.join(repoRoot, "src-tauri");
const nsisBundleDir = path.join(srcTauriDir, "target", "release", "bundle", "nsis");
const withEmbedConfig = path.join(srcTauriDir, "tauri.with_embed_python_git.conf.json");
const withoutEmbedConfig = path.join(srcTauriDir, "tauri.without_embed_python_git.conf.json");
// Adds the signed Locus Remote APK (src-tauri/gen/remote-app) on top of a flavor.
const remoteAppConfig = path.join(srcTauriDir, "tauri.with_remote_app.conf.json");
// NSIS compression: the base tauri.conf.json uses zlib (fast packaging, used by
// the official release flow). Passing --compression=lzma layers this overlay on
// top for a smaller but much slower-to-build installer.
const nsisLzmaConfig = path.join(srcTauriDir, "tauri.nsis-lzma.conf.json");
const buildModes = new Map([
  [
    "development",
    {
      label: "development",
      outputSuffix: "dev",
      defaultFlavors: ["without_embed_python_git"],
      cargoEnv: {
        CARGO_PROFILE_RELEASE_INCREMENTAL: "true",
        CARGO_PROFILE_RELEASE_LTO: "off",
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS: "256",
      },
    },
  ],
  [
    "release",
    {
      label: "release",
      outputSuffix: "",
      defaultFlavors: ["without_embed_python_git", "default", "with_remote_app"],
      cargoEnv: {
        CARGO_PROFILE_RELEASE_INCREMENTAL: "false",
        CARGO_PROFILE_RELEASE_LTO: "thin",
        // One unit ran the ~450k-line locus crate through LLVM on a single
        // thread (~26 min per flavor); 16 units parallelize it under ThinLTO.
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS: "16",
      },
    },
  ],
]);
const compressions = new Set(["zlib", "lzma"]);
const flavors = new Map([
  [
    "default",
    {
      label: "Windows x64",
      suffix: "",
      buildArgs: [
        "build",
        "--config",
        path.relative(repoRoot, withEmbedConfig),
      ],
    },
  ],
  [
    "with_embed_python_git",
    {
      label: "Windows x64",
      suffix: "",
      buildArgs: [
        "build",
        "--config",
        path.relative(repoRoot, withEmbedConfig),
      ],
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
  [
    // The default installer plus the Locus Remote Android app, which phones
    // on the LAN can download while pairing.
    "with_remote_app",
    {
      label: "Windows x64 - with Locus Remote (Android)",
      suffix: "with_remote_app",
      remoteApp: true,
      buildArgs: [
        "build",
        "--config",
        path.relative(repoRoot, withEmbedConfig),
        "--config",
        path.relative(repoRoot, remoteAppConfig),
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
    "Usage: bun run scripts/build-release-installers.mjs [--mode=development|release] [--compression=zlib|lzma] [flavor...] [-- tauri args...]",
    "",
    `Flavors: ${names}`,
    "Mode: release (default)",
    "Default flavors: development = without_embed_python_git; release = without_embed_python_git default with_remote_app",
    "with_remote_app builds and signs the Android APK first (see scripts/prepare-remote-app.mjs for the required variables)",
    "Compression: zlib (default, fast packaging) or lzma (smaller installer, much slower)",
  ].join("\n");
}

function parseArgs(rawArgs) {
  const separatorIndex = rawArgs.indexOf("--");
  const ownArgs = separatorIndex >= 0 ? rawArgs.slice(0, separatorIndex) : rawArgs;
  const tauriArgs = separatorIndex >= 0 ? rawArgs.slice(separatorIndex + 1) : [];
  const flavorArgs = [];
  let buildMode = "release";
  let compression = "zlib";
  let showHelp = false;

  for (const arg of ownArgs) {
    if (arg === "-h" || arg === "--help") {
      showHelp = true;
      continue;
    }

    if (arg.startsWith("--mode=")) {
      buildMode = arg.slice("--mode=".length);
      continue;
    }

    if (arg.startsWith("--compression=")) {
      compression = arg.slice("--compression=".length);
      continue;
    }

    flavorArgs.push(arg);
  }

  if (!buildModes.has(buildMode)) {
    throw new Error(`Unknown installer build mode "${buildMode}".\n\n${usage()}`);
  }

  if (!compressions.has(compression)) {
    throw new Error(`Unknown NSIS compression "${compression}".\n\n${usage()}`);
  }

  const mode = buildModes.get(buildMode);
  const requestedFlavors = flavorArgs.length > 0 ? flavorArgs : mode.defaultFlavors;

  for (const flavor of requestedFlavors) {
    if (!flavors.has(flavor)) {
      throw new Error(`Unknown installer flavor "${flavor}".\n\n${usage()}`);
    }
  }

  return { buildMode, requestedFlavors, tauriArgs, compression, showHelp };
}

function run(command, args, extraEnv) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env: extraEnv ? { ...process.env, ...extraEnv } : process.env,
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

function installerNameForFlavor(baseName, suffixes) {
  const suffix = suffixes.filter(Boolean).join("-");

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

function finalizeInstaller(flavor, mode, baseName, startedAtMs) {
  const sourcePath = findGeneratedInstaller(baseName, startedAtMs);
  const finalName = installerNameForFlavor(baseName, [mode.outputSuffix, flavor.suffix]);
  const finalPath = path.join(nsisBundleDir, finalName);

  if (sourcePath !== finalPath) {
    if (existsSync(finalPath)) {
      unlinkSync(finalPath);
    }
    renameSync(sourcePath, finalPath);
  }

  return finalPath;
}

function buildFlavor(flavorName, mode, compression, tauriArgs, baseName) {
  assertPublicArguments(tauriArgs);
  assertPublicSourceBoundary(repoRoot);
  const flavor = flavors.get(flavorName);
  const startedAtMs = Date.now();
  // Config overlays merge in order, so the compression overlay must come after
  // the flavor config to win.
  const compressionArgs =
    compression === "lzma" ? ["--config", path.relative(repoRoot, nsisLzmaConfig)] : [];
  if (flavor.remoteApp) {
    run("bun", ["run", "scripts/prepare-remote-app.mjs"]);
  }
  console.log(
    `[locus] Building ${mode.label} installer flavor: ${flavorName} (nsis compression: ${compression})`,
  );
  // Tauri writes every flavor under the base name before it is renamed, so a
  // suffixed flavor would overwrite an installer that already finished under
  // that name (the default flavor). Keep it aside until this build is done.
  const basePath = path.join(nsisBundleDir, baseName);
  const keepsBaseName = installerNameForFlavor(baseName, [mode.outputSuffix, flavor.suffix]) === baseName;
  const stashedBasePath = !keepsBaseName && existsSync(basePath) ? `${basePath}.pending` : null;
  if (stashedBasePath) {
    if (existsSync(stashedBasePath)) unlinkSync(stashedBasePath);
    renameSync(basePath, stashedBasePath);
  }
  let finalPath;
  try {
    run("bun", ["tauri", ...flavor.buildArgs, ...compressionArgs, ...tauriArgs], { ...mode.cargoEnv, LOCUS_BUILD_EDITION: "public" });
    assertClientBundle(repoRoot, "public");
    finalPath = finalizeInstaller(flavor, mode, baseName, startedAtMs);
  } finally {
    if (stashedBasePath && existsSync(stashedBasePath)) {
      if (existsSync(basePath)) unlinkSync(basePath);
      renameSync(stashedBasePath, basePath);
    }
  }
  const results = [{ flavor: flavorName, label: flavor.label, path: finalPath }];
  if (flavor.remoteApp) {
    // The same signed APK, published on its own for phones that cannot use
    // the LAN download.
    const version = readJson(path.join(srcTauriDir, "tauri.conf.json")).version;
    const apkPath = path.join(nsisBundleDir, `locus-remote_${version}_android-arm64.apk`);
    copyFileSync(path.join(srcTauriDir, "gen", "remote-app", "locus-remote.apk"), apkPath);
    results.push({ flavor: "remote_app_apk", label: "Android (Locus Remote)", path: apkPath });
  }

  return results;
}

try {
  const { buildMode, requestedFlavors, tauriArgs, compression, showHelp } = parseArgs(
    process.argv.slice(2),
  );

  if (showHelp) {
    console.log(usage());
    process.exit(0);
  }

  const mode = buildModes.get(buildMode);
  // Fail before any installer is built rather than after the first ones.
  if (requestedFlavors.some((name) => flavors.get(name).remoteApp)) {
    const errors = remoteAppPrerequisiteErrors();
    if (errors.length > 0) {
      throw new Error(
        `The with_remote_app flavor cannot be built:\n- ${errors.join("\n- ")}\n`
          + "Set the variables, or pass the flavors to build explicitly (e.g. without_embed_python_git default).",
      );
    }
  }
  const baseName = expectedInstallerBaseName();
  const results = requestedFlavors.flatMap((flavorName) =>
    buildFlavor(flavorName, mode, compression, tauriArgs, baseName),
  );

  console.log(`[locus] ${mode.label} installers ready:`);
  for (const result of results) {
    console.log(`- ${result.label}: ${path.relative(repoRoot, result.path)}`);
  }
} catch (error) {
  console.error(`[locus] Failed to build release installers: ${error.stack ?? error.message ?? error}`);
  process.exit(1);
}
