// Builds the signed Locus Remote APK (mobile/) that the `with_remote_app`
// installer flavor bundles, and stages it with its metadata:
//   src-tauri/gen/remote-app/locus-remote.apk
//   src-tauri/gen/remote-app/app.json   {"version", "sha256", "size", "signerSha256"}
//
//   bun run scripts/prepare-remote-app.mjs [--check]
//
// --check only validates the prerequisites (used before a release starts).
//
// Signing (required: an unsigned APK cannot be installed, and every release
// must use the same key or phones refuse to update the app):
//   LOCUS_REMOTE_KEYSTORE           path to the keystore (.jks), kept outside the repo
//   LOCUS_REMOTE_KEYSTORE_PASSWORD  keystore password
//   LOCUS_REMOTE_KEY_ALIAS          key alias
//   LOCUS_REMOTE_KEY_PASSWORD       key password (defaults to the keystore password)
// Android toolchain: ANDROID_HOME (or ANDROID_SDK_ROOT), NDK_HOME, JAVA_HOME,
// and the Rust target aarch64-linux-android.
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const mobileDir = path.join(repoRoot, "mobile");
const outputDir = path.join(repoRoot, "src-tauri", "gen", "remote-app");
const apkOutputDir = path.join(mobileDir, "src-tauri", "gen", "android", "app", "build", "outputs", "apk", "universal", "release");
const RUST_TARGET = "aarch64-linux-android";
const isWindows = process.platform === "win32";

function fail(message) {
  throw new Error(message);
}

function env(name) {
  const value = process.env[name]?.trim();
  return value ? value : null;
}

function androidSdk() {
  return env("ANDROID_HOME") ?? env("ANDROID_SDK_ROOT");
}

/** Every missing prerequisite at once, so a release fails before it starts. */
export function remoteAppPrerequisiteErrors() {
  const errors = [];
  for (const name of ["LOCUS_REMOTE_KEYSTORE", "LOCUS_REMOTE_KEYSTORE_PASSWORD", "LOCUS_REMOTE_KEY_ALIAS"]) {
    if (!env(name)) errors.push(`${name} is not set`);
  }
  const keystore = env("LOCUS_REMOTE_KEYSTORE");
  if (keystore && !existsSync(keystore)) errors.push(`LOCUS_REMOTE_KEYSTORE does not exist: ${keystore}`);
  if (keystore && path.resolve(keystore).startsWith(repoRoot + path.sep)) {
    errors.push("LOCUS_REMOTE_KEYSTORE must live outside the repository");
  }
  const sdk = androidSdk();
  if (!sdk || !existsSync(sdk)) errors.push("ANDROID_HOME (or ANDROID_SDK_ROOT) does not point to an Android SDK");
  const ndk = env("NDK_HOME");
  if (!ndk || !existsSync(ndk)) errors.push("NDK_HOME does not point to an Android NDK");
  const java = env("JAVA_HOME");
  if (!java || !existsSync(java)) errors.push("JAVA_HOME does not point to a JDK (17 or newer)");
  if (sdk && existsSync(sdk) && !apksignerPath(sdk)) errors.push("Android SDK build-tools (apksigner) are not installed");
  const targets = spawnSync("rustup", ["target", "list", "--installed"], { encoding: "utf8" });
  if (targets.status === 0 && !targets.stdout.split(/\r?\n/).includes(RUST_TARGET)) {
    errors.push(`Rust target ${RUST_TARGET} is not installed (rustup target add ${RUST_TARGET})`);
  }
  return errors;
}

function compareVersions(left, right) {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const diff = (a[index] ?? 0) - (b[index] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

function apksignerPath(sdk) {
  const buildTools = path.join(sdk, "build-tools");
  if (!existsSync(buildTools)) return null;
  const versions = readdirSync(buildTools)
    .filter((name) => /^\d+(\.\d+)*$/.test(name))
    .sort(compareVersions)
    .reverse();
  for (const version of versions) {
    const candidate = path.join(buildTools, version, isWindows ? "apksigner.bat" : "apksigner");
    if (existsSync(candidate)) return candidate;
  }
  return null;
}

/** The nested Android build must not see the desktop build's Tauri or
 * profile overrides (it is often started from inside a desktop release). */
function androidBuildEnv() {
  const next = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (key.startsWith("TAURI_") || key.startsWith("CARGO_PROFILE_RELEASE_")) continue;
    next[key] = value;
  }
  next.LOCUS_REMOTE_KEY_PASSWORD = env("LOCUS_REMOTE_KEY_PASSWORD") ?? env("LOCUS_REMOTE_KEYSTORE_PASSWORD");
  return next;
}

function run(command, args, options) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) fail(`${command} ${args.join(" ")} failed with exit code ${result.status ?? "unknown"}`);
}

function desktopVersion() {
  const config = JSON.parse(readFileSync(path.join(repoRoot, "src-tauri", "tauri.conf.json"), "utf8"));
  if (typeof config.version !== "string" || !/^\d+\.\d+\.\d+$/.test(config.version)) {
    fail(`Unexpected desktop version in src-tauri/tauri.conf.json: ${config.version}`);
  }
  return config.version;
}

function signerDigest(apk) {
  const apksigner = apksignerPath(androidSdk());
  // apksigner is a .bat on Windows, which only runs through the shell.
  const result = isWindows
    ? spawnSync(`"${apksigner}" verify --print-certs "${apk}"`, { encoding: "utf8", shell: true })
    : spawnSync(apksigner, ["verify", "--print-certs", apk], { encoding: "utf8" });
  if (result.status !== 0) fail(`apksigner rejected ${apk}:\n${result.stdout}\n${result.stderr}`);
  const match = /certificate SHA-256 digest:\s*([0-9a-f]{64})/i.exec(result.stdout);
  if (!match) fail(`apksigner did not report a signer digest for ${apk}`);
  return match[1].toLowerCase();
}

function buildApk(version) {
  if (!existsSync(path.join(mobileDir, "node_modules"))) {
    run("bun", ["install", "--frozen-lockfile"], { cwd: mobileDir });
  }
  rmSync(apkOutputDir, { recursive: true, force: true });
  // The app shares the desktop version, so each release has a higher
  // versionCode and installs over the previous one.
  run(
    "bun",
    ["tauri", "android", "build", "--apk", "--target", "aarch64", "--config", JSON.stringify({ version })],
    { cwd: mobileDir, env: androidBuildEnv() },
  );
  const signed = path.join(apkOutputDir, "app-universal-release.apk");
  if (!existsSync(signed)) {
    fail(`Signed APK not found at ${signed}. Check the LOCUS_REMOTE_* signing variables.`);
  }
  return signed;
}

function stage(apk, version) {
  const bytes = readFileSync(apk);
  const metadata = {
    version,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    size: statSync(apk).size,
    signerSha256: signerDigest(apk),
  };
  rmSync(outputDir, { recursive: true, force: true });
  mkdirSync(outputDir, { recursive: true });
  copyFileSync(apk, path.join(outputDir, "locus-remote.apk"));
  writeFileSync(path.join(outputDir, "app.json"), `${JSON.stringify(metadata, null, 2)}\n`);
  return metadata;
}

function main() {
  const errors = remoteAppPrerequisiteErrors();
  if (errors.length > 0) {
    fail(`Cannot build the Locus Remote APK:\n- ${errors.join("\n- ")}\nSee the header of scripts/prepare-remote-app.mjs.`);
  }
  if (process.argv.includes("--check")) {
    console.log("[remote-app] prerequisites OK");
    return;
  }
  const version = desktopVersion();
  console.log(`[remote-app] building Locus Remote ${version} (${RUST_TARGET})`);
  const metadata = stage(buildApk(version), version);
  console.log(
    `[remote-app] staged ${path.relative(repoRoot, outputDir)}: ${(metadata.size / 1048576).toFixed(1)} MB, signer ${metadata.signerSha256}`,
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`[remote-app] ${error.message ?? error}`);
    process.exit(1);
  }
}
