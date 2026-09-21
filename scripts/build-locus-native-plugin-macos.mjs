import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifest = path.join(root, "locus_native_plugin", "Cargo.toml");
// Both must ship even in a thin app: an arm64 Locus may control a Rosetta Editor.
for (const [target, directory, cpu] of [
  ["aarch64-apple-darwin", "macos-arm64", 0x0100000c],
  ["x86_64-apple-darwin", "macos-x86_64", 0x01000007],
]) {
  if (!process.argv.includes("--verify-only")) {
    execFileSync("cargo", ["build", "--release", "--manifest-path", manifest, "--target", target], {
      cwd: root, stdio: "inherit",
      env: { ...process.env, MACOSX_DEPLOYMENT_TARGET: process.env.MACOSX_DEPLOYMENT_TARGET ?? "14.0" },
    });
  }
  const built = path.join(root, "locus_native_plugin", "target", target, "release", "liblocus_native.dylib");
  const bytes = readFileSync(built);
  if (bytes.length < 32 || bytes.readUInt32LE(0) !== 0xfeedfacf || bytes.readUInt32LE(4) !== cpu || bytes.readUInt32LE(12) !== 6) {
    throw new Error(`Expected a linked ${target} Mach-O dylib: ${built}`);
  }
  const destination = path.join(root, "src-tauri", "gen", "macos", "native", directory);
  mkdirSync(destination, { recursive: true });
  copyFileSync(built, path.join(destination, "liblocus_native.dylib"));
  console.log(`[locus] macOS native broker: ${target} -> ${destination}`);
}
