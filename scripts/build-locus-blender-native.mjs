// Build the Blender native broker (`locus_blender_native`, a Rust cdylib on
// top of crates/locus-native-broker) and place it into the Blender host
// package at locus_blender/locus_host/bin/locus_blender_native_<hash>.dll.
//
// The content hash in the file name lets an installed host keep its loaded
// DLL (Windows locks it) while an update writes the new one next to it;
// `bin/native.json` names the DLL the host code loads. The exported symbol
// list must equal locus_blender_native/exports.txt, which contains no
// Unity-only entry points.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { readDllExports, readExportList } from "./native-dll-exports.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const crateDir = path.join(repoRoot, "locus_blender_native");
const manifest = path.join(crateDir, "Cargo.toml");
const builtDll = path.join(crateDir, "target", "release", "locus_blender_native.dll");
const exportList = path.join(crateDir, "exports.txt");
const destDir = path.join(repoRoot, "locus_blender", "locus_host", "bin");

const UNITY_ONLY = [/^locus_webview_/, /^locus_render_/, /^locus_overlay_/, /^locus_set_background_active$/, /^locus_set_compile_result$/];

execFileSync("cargo", ["build", "--release", "--manifest-path", manifest], { cwd: repoRoot, stdio: "inherit" });
if (!existsSync(builtDll)) throw new Error(`cargo build did not produce ${builtDll}`);

const actual = readDllExports(builtDll);
const expected = readExportList(exportList);
const missing = expected.filter((name) => !actual.includes(name));
const extra = actual.filter((name) => !expected.includes(name));
if (missing.length || extra.length) {
  throw new Error(
    `locus_blender_native.dll exports differ from exports.txt\n  missing: ${missing.join(", ") || "-"}\n  extra: ${extra.join(", ") || "-"}`,
  );
}
const leaked = actual.filter((name) => UNITY_ONLY.some((pattern) => pattern.test(name)));
if (leaked.length) throw new Error(`Unity-only exports leaked into the Blender DLL: ${leaked.join(", ")}`);

const bytes = readFileSync(builtDll);
const sha256 = createHash("sha256").update(bytes).digest("hex");
const fileName = `locus_blender_native_${sha256.slice(0, 12)}.dll`;
await mkdir(destDir, { recursive: true });
for (const entry of readdirSync(destDir)) {
  if (/^locus_blender_native_[0-9a-f]+\.dll$/.test(entry) && entry !== fileName) {
    rmSync(path.join(destDir, entry), { force: true });
  }
}
copyFileSync(builtDll, path.join(destDir, fileName));
writeFileSync(
  path.join(destDir, "native.json"),
  `${JSON.stringify({ dll: fileName, sha256, protocolVersion: 1, exports: actual.length }, null, 2)}\n`,
);

console.log(`[locus] Blender native broker built (${actual.length} exports) -> ${path.join(destDir, fileName)}`);
