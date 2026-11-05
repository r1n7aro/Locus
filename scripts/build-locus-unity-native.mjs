// Build the Unity native broker (`locus_unity_native`, a Rust cdylib on top of
// crates/locus-native-broker) and place the DLL into the Unity package at
// locus_unity/Editor/Native/x86_64/locus_unity_native.dll, where the editor
// plugin loads it from (bundled wholesale via tauri.conf.json's
// `../locus_unity` resource). The crate is standalone (its own workspace
// root), so it builds with --manifest-path and keeps its own target dir.
//
// The exported symbol list must equal locus_unity_native/exports.txt (the
// original `locus_native.dll` surface): Unity plugins of every version bind
// these names.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, rmSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { readDllExports, readExportList } from "./native-dll-exports.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const crateDir = path.join(repoRoot, "locus_unity_native");
const manifest = path.join(crateDir, "Cargo.toml");
const builtDll = path.join(crateDir, "target", "release", "locus_unity_native.dll");
const exportList = path.join(crateDir, "exports.txt");
const destDir = path.join(repoRoot, "locus_unity", "Editor", "Native", "x86_64");
const destDll = path.join(destDir, "locus_unity_native.dll");
// Pre-split name. Remove it so the package never ships two brokers.
const legacyDll = path.join(destDir, "locus_native.dll");

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

const actual = readDllExports(builtDll);
const expected = readExportList(exportList);
const missing = expected.filter((name) => !actual.includes(name));
const extra = actual.filter((name) => !expected.includes(name));
if (missing.length || extra.length) {
  throw new Error(
    `locus_unity_native.dll exports differ from exports.txt\n  missing: ${missing.join(", ") || "-"}\n  extra: ${extra.join(", ") || "-"}`,
  );
}

await mkdir(destDir, { recursive: true });
copyFileSync(builtDll, destDll);
rmSync(legacyDll, { force: true });

console.log(`[locus] native plugin built (${actual.length} exports) and copied to ${destDll}`);
