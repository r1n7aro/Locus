// Reproducible source checks for platform dispatch added by the Mac port.
// This checks textual preservation; Windows/Unity runtime regression is separate.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { resolveMacosTarget } from "./macos-build-target.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const base = process.argv[2] ?? "cf70db9636ee61e74c779bec1472111ff389e00d";
const normalize = (value) => value.replaceAll("\r\n", "\n");
const before = (file) => normalize(execFileSync("git", ["show", `${base}:${file}`], { cwd: root, encoding: "utf8" }));
const after = (file) => normalize(readFileSync(path.join(root, file), "utf8"));
let count = 0;
function equal(file, transform = (value) => value) {
  assert.equal(transform(after(file)), before(file), `Original Windows implementation changed: ${file}`);
  count++;
  console.log(`PASS ${file}`);
}
for (const file of [
  "src-tauri/src/dotnet_runtime.rs", "src-tauri/tauri.conf.json",
  "src-tauri/tauri.with_embed_python_git.conf.json", "src-tauri/tauri.without_embed_python_git.conf.json",
  "scripts/build-locus-compile-server.mjs", "scripts/ensure-locus-compile-server.mjs",
  "scripts/build-locus-json-bundle.mjs", "scripts/build-locus-roslyn-bundle.mjs",
  "scripts/build-locus-detour-bundle.mjs", "scripts/build-locus-hotreload-runtime.mjs",
  "scripts/generate-third-party-bundle.mjs", "scripts/prepare-ort-runtime.mjs",
  "scripts/prepare-managed-python.mjs", "scripts/prepare-managed-git.mjs",
  "scripts/prepare-managed-github-cli.mjs", "scripts/build-release-installers.mjs",
]) equal(file);

const stripMacResources = (source) => source.replace(
  /^ *#\[cfg\(target_os = "macos"\)\]\n *if let Some\(root\) = (?:crate::)?macos_resources::resource_root\(\) \{\n *(?:candidates|app_agent_dir_candidates)\.push\(root\.join\([^\n]+\)\);\n *\}\n/gm, "",
);
for (const file of ["src-tauri/src/csharp_compile/manager.rs", "src-tauri/src/commands/knowledge.rs", "src-tauri/src/commands/skill.rs"]) {
  equal(file, stripMacResources);
}
equal("src-tauri/src/lib.rs", (source) => stripMacResources(source)
  .replace('#[cfg_attr(target_os = "macos", path = "dotnet_runtime_macos.rs")]\n', "")
  .replace('#[cfg(target_os = "macos")]\npub(crate) mod macos_resources;\n', ""));
equal("src-tauri/src/python_runtime.rs", (source) => stripMacResources(source)
  .replace(/    \/\/ Finder-launched apps do not inherit the user's shell PATH\.\n[\s\S]*?\n    \}\n/, ""));
equal("src-tauri/src/process_util.rs", (source) => source
  .replace('\n#[cfg(target_os = "macos")]\n#[path = "process_util_macos.rs"]\nmod macos;\n', "")
  .replace('#[cfg(not(target_os = "macos"))]\nfn discover_github_cli()', 'fn discover_github_cli()')
  .replace(/\n#\[cfg\(target_os = "macos"\)\]\nfn discover_github_cli\(\) -> Option<ResolvedGithubCli> \{\n[\s\S]*?\n\}\n/, "")
  .replace('#[cfg(target_os = "macos")]\nfn git_common_location_candidates() -> Vec<PathBuf> {\n    macos::git_candidates()\n}\n\n', "")
  .replace('#[cfg(not(any(target_os = "windows", target_os = "macos")))]\nfn git_common_location_candidates()', '#[cfg(not(target_os = "windows"))]\nfn git_common_location_candidates()'));

const importTarget = 'import { resolveMacosTarget } from "./macos-build-target.mjs";\n';
equal("scripts/build-locus-native-plugin.mjs", (source) => source.replace(importTarget, "")
  .replace('\nif (resolveMacosTarget()) {\n  await import("./build-locus-native-plugin-macos.mjs");\n  process.exit(0);\n}\n', ""));
equal("scripts/run-tauri.mjs", (source) => source.replace(importTarget, "")
  .replace(/const macosTarget = resolveMacosTarget\(tauriArgs\);\nif \(macosTarget\) \{\n[\s\S]*?\n\}\n/, "")
  .replace("if (!macosTarget && shouldInjectDefaultReleaseFlavor(tauriArgs))", "if (shouldInjectDefaultReleaseFlavor(tauriArgs))")
  .replace('for (const scriptName of macosTarget ? ["macos:prepare"] : DEV_PREREQUISITE_SCRIPTS)', 'for (const scriptName of DEV_PREREQUISITE_SCRIPTS)'));

const oldPackage = JSON.parse(before("package.json"));
const newPackage = JSON.parse(after("package.json"));
for (const [name, value] of Object.entries(oldPackage.scripts)) {
  if (name === "build:tauri") continue;
  assert.equal(newPackage.scripts[name], value, `Existing package command changed: ${name}`);
}
delete oldPackage.scripts;
delete newPackage.scripts;
assert.deepEqual(newPackage, oldPackage, "Package dependencies or release metadata changed");
assert.match(after("scripts/build-tauri-platform.mjs"), /resolveMacosTarget\(\) \? "build:tauri:macos" : "build:tauri:with_embed_python_git"/);
assert.equal(resolveMacosTarget([], {}, "win32", "x64"), null);
assert.equal(resolveMacosTarget(["--target", "x86_64-pc-windows-msvc"], {}, "win32", "x64"), null);
console.log("PASS all existing package commands and Windows default full installer flavor");

function between(source, begin, end) {
  const offset = source.indexOf(begin);
  assert.ok(offset >= 0);
  const stop = source.indexOf(end, offset + begin.length);
  assert.ok(stop > offset);
  return source.slice(offset, stop);
}
assert.equal(
  between(after("src/services/appUpdate.ts"), "function selectInstaller(", "function githubReleaseUrlFromUrl("),
  between(before("src/services/appUpdate.ts"), "function selectInstaller(", "function githubReleaseUrlFromUrl("),
  "Windows installer preference function changed",
);
assert.equal(
  between(after("src-tauri/src/knowledge_index/embedding.rs"), '#[cfg(windows)]\nfn ensure_ort_runtime_loaded()', "\n}\n"),
  between(before("src-tauri/src/knowledge_index/embedding.rs"), '#[cfg(windows)]\nfn ensure_ort_runtime_loaded()', "\n}\n"),
  "Windows ORT initialization changed",
);
console.log("PASS original Windows update selection and ORT initializer");
const rawNativeEntries = readdirSync(path.join(root, "locus_unity", "Editor", "Native"));
assert.ok(!rawNativeEntries.some((name) => name.startsWith("macos-")), "Mac native binaries/meta leaked into the Windows bundled Unity source");
assert.ok(!Object.keys(JSON.parse(after("src-tauri/tauri.conf.json")).bundle.resources).some((name) => name.includes("macos")));
console.log("PASS Windows bundle input excludes Mac native binaries and importer metadata");
console.log(`Windows platform preservation passed: ${count} source/config files plus commands, update selection and ORT initialization (baseline ${base}).`);
