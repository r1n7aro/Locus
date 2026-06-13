// Build the Locus native broker (`locus_native`, a Rust cdylib) and place the
// resulting DLL into the Unity package at
// locus_unity/Editor/Native/x86_64/locus_native.dll, where the editor plugin
// loads it from (committed alongside the managed DLLs, then bundled wholesale
// via tauri.conf.json's `../locus_unity` resource). The crate is standalone
// (its own workspace root), so it builds with --manifest-path and keeps its own
// target dir out of the main app build.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const manifest = path.join(repoRoot, "locus_native_plugin", "Cargo.toml");
const builtDll = path.join(
  repoRoot,
  "locus_native_plugin",
  "target",
  "release",
  "locus_native.dll"
);
const destDir = path.join(repoRoot, "locus_unity", "Editor", "Native", "x86_64");
const destDll = path.join(destDir, "locus_native.dll");

function run(command, args, options = {}) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    ...options,
  });
}

if (!existsSync(manifest)) {
  throw new Error(`missing native plugin crate: ${manifest}`);
}

run("cargo", ["build", "--release", "--manifest-path", manifest]);

if (!existsSync(builtDll)) {
  throw new Error(`cargo build did not produce ${builtDll}`);
}

await mkdir(destDir, { recursive: true });
copyFileSync(builtDll, destDll);

console.log(`[locus] native plugin built and copied to ${destDll}`);
