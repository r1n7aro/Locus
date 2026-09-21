import { execFileSync } from "node:child_process";
import { cpSync, existsSync, lstatSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { root, verifyManagedBundle } from "./macos-managed-bundle.mjs";

const managed = path.resolve(process.env.LOCUS_MACOS_MANAGED_BUNDLE ?? path.join(root, ".tmp", "macos-managed"));
if (!existsSync(path.join(managed, "managed-bundle.json"))) {
  if (process.platform !== "win32") {
    throw new Error("Missing portable managed bundle. Export on Windows with bun run macos:managed:export, copy the folder to this Mac, and set LOCUS_MACOS_MANAGED_BUNDLE to that folder.");
  }
  execFileSync(process.execPath, [path.join(root, "scripts", "macos-managed-bundle.mjs"), "--export"], { cwd: root, stdio: "inherit" });
}
const managedFiles = verifyManagedBundle(managed);
const nativeArgs = process.argv.includes("--native-prebuilt") ? ["--verify-only"] : [];
execFileSync(process.execPath, [path.join(root, "scripts", "build-locus-native-plugin-macos.mjs"), ...nativeArgs], { cwd: root, stdio: "inherit" });
execFileSync(process.execPath, [path.join(root, "scripts", "ensure-locus-compile-server.mjs")], { cwd: root, stdio: "inherit" });

const source = path.join(root, "locus_unity");
const destination = path.join(root, "src-tauri", "gen", "macos", "locus_unity");
// This directory is owned exclusively by this generator. Clear stale staged
// files, checking its fixed absolute location before recursive deletion.
if (path.dirname(destination) !== path.resolve(root, "src-tauri", "gen", "macos")) throw new Error("Invalid macOS staging directory");
if (existsSync(destination)) {
  if (lstatSync(destination).isSymbolicLink()) throw new Error("macOS staging directory must not be a symlink");
  rmSync(destination, { recursive: true });
}
mkdirSync(destination, { recursive: true });
cpSync(source, destination, {
  recursive: true,
  filter: (file) => {
    const relative = path.relative(source, file).replaceAll("\\", "/");
    return !/^Editor\/(Detour|HotReload)(\/|\.meta$)/.test(relative)
      && !/^Editor\/Native\/(x86_64|arm64)(\/|\.meta$)/.test(relative)
      && !relative.endsWith(".dll")
      && !relative.includes("/ILRepack-");
  },
});
for (const relative of managedFiles) cpSync(path.join(managed, relative), path.join(destination, relative));
const asmdefFile = path.join(destination, "Editor", "Locus.Editor.asmdef");
const asmdef = JSON.parse(readFileSync(asmdefFile, "utf8"));
asmdef.precompiledReferences = asmdef.precompiledReferences.filter((file) => !["Locus.Detour.dll", "Locus.HotReload.Runtime.dll"].includes(file));
writeFileSync(asmdefFile, JSON.stringify(asmdef, null, 2) + "\n");
for (const arch of ["arm64", "x86_64"]) {
  const nativeRoot = path.join(destination, "Editor", "Native");
  const directory = `macos-${arch}`;
  mkdirSync(path.join(nativeRoot, directory), { recursive: true });
  cpSync(path.join(root, "src-tauri", "macos", "unity-native", `${directory}.meta`), path.join(nativeRoot, `${directory}.meta`));
  cpSync(path.join(root, "src-tauri", "macos", "unity-native", directory, "liblocus_native.dylib.meta"), path.join(nativeRoot, directory, "liblocus_native.dylib.meta"));
  cpSync(path.join(root, "src-tauri", "gen", "macos", "native", directory, "liblocus_native.dylib"), path.join(nativeRoot, directory, "liblocus_native.dylib"));
  for (const suffix of ["", ".meta"]) {
    const file = path.join(destination, "Editor", "Native", `macos-${arch}`, `liblocus_native.dylib${suffix}`);
    if (!existsSync(file)) throw new Error(`Missing macOS Unity resource: ${file}`);
  }
}
execFileSync(process.execPath, [path.join(root, "scripts", "generate-macos-license-bundle.mjs")], { cwd: root, stdio: "inherit" });
console.log(`[locus] macOS resources prepared in ${destination}; Python/Git use system installations, local embedding is unavailable.`);
