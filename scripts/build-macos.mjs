import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { resolveMacosTarget } from "./macos-build-target.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const target = resolveMacosTarget([], { ...process.env, CARGO_BUILD_TARGET: process.env.LOCUS_MACOS_TARGET ?? process.env.CARGO_BUILD_TARGET });
if (!target) throw new Error("Specify a Darwin target with bun tauri build --target aarch64-apple-darwin or x86_64-apple-darwin.");
const env = { ...process.env, VITE_LOCUS_TARGET_OS: "macos", VITE_LOCUS_TARGET_ARCH: target.arch,
  MACOSX_DEPLOYMENT_TARGET: process.env.MACOSX_DEPLOYMENT_TARGET ?? "14.0" };
for (const script of ["macos:prepare", "view-runtime:export", "build"]) {
  execFileSync(process.execPath, ["run", script], { cwd: root, stdio: "inherit", env });
}
