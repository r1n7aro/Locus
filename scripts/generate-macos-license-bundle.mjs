// Independent Mac inventory: do not execute or rewrite the Windows generator.
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = path.resolve(root, "src-tauri", "gen", "macos", "licenses-bundle");
const targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
const normalize = (value) => value.replaceAll("\\", "/");
const relative = (value) => normalize(path.relative(output, value));
const folder = (name, version) => `${name}-${version}`.replace(/^@/, "at-").replace(/[<>:"/\\|?*\x00-\x1F]/g, "_");
const legalFile = (name) => /^(licen[cs]e|notices?|third[-_]?party[-_]?notices|copying)/i.test(name);
const knownLicenses = { "Apache-2.0": "Apache-2.0.txt", "BSD-3-Clause": "BSD-3-Clause.txt", MIT: "MIT.txt", "MPL-2.0": "MPL-2.0.txt" };

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(value, null, 2) + "\n");
}

function copyNotices(source, destination, license, subject) {
  fs.mkdirSync(destination, { recursive: true });
  const files = fs.readdirSync(source, { withFileTypes: true })
    .filter((entry) => entry.isFile() && legalFile(entry.name)).map((entry) => entry.name).sort();
  if (files.length) {
    for (const file of files) fs.copyFileSync(path.join(source, file), path.join(destination, file));
    return files.map((file) => relative(path.join(destination, file)));
  }
  const ids = [...new Set((String(license ?? "").match(/[A-Za-z0-9.+-]+/g) ?? []).filter((id) => id in knownLicenses))];
  if (!ids.length) throw new Error(`No license text for ${subject}: ${license}`);
  return ids.map((id) => {
    const file = path.join(destination, `LICENSE-${id}.txt`);
    fs.copyFileSync(path.join(root, "third_party", "spdx", knownLicenses[id]), file);
    return relative(file);
  });
}

// All writes stay within this generator's dedicated Mac output directory.
if (path.dirname(output) !== path.resolve(root, "src-tauri", "gen", "macos")) throw new Error("Invalid Mac license output");
if (fs.existsSync(output)) {
  if (fs.lstatSync(output).isSymbolicLink()) throw new Error("Mac license output must not be a symlink");
  fs.rmSync(output, { recursive: true });
}
fs.mkdirSync(output, { recursive: true });

const packageJson = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const frontend = [];
const seen = new Set();
function visitFrontend(name, from) {
  let manifest;
  try { manifest = createRequire(path.join(from, "package.json")).resolve(`${name}/package.json`); }
  catch { return; } // Optional packages absent from this install are not distributed.
  if (seen.has(manifest)) return;
  seen.add(manifest);
  const metadata = JSON.parse(fs.readFileSync(manifest, "utf8"));
  const source = path.dirname(manifest);
  frontend.push({ name: metadata.name, version: metadata.version, license: metadata.license ?? null,
    texts: copyNotices(source, path.join(output, "frontend", "packages", folder(metadata.name, metadata.version)), metadata.license, name) });
  for (const dependency of new Set([...Object.keys(metadata.dependencies ?? {}), ...Object.keys(metadata.optionalDependencies ?? {})])) visitFrontend(dependency, source);
}
for (const dependency of Object.keys(packageJson.dependencies ?? {}).sort()) visitFrontend(dependency, root);

const rustPackages = new Map();
for (const manifest of ["src-tauri/Cargo.toml", "locus_native_plugin/Cargo.toml"]) {
  for (const target of targets) {
    const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version", "1", "--manifest-path", path.join(root, manifest), "--filter-platform", target],
      { cwd: root, encoding: "utf8", maxBuffer: 128 * 1024 * 1024 }));
    const byId = new Map(metadata.packages.map((item) => [item.id, item]));
    const nodes = new Map(metadata.resolve.nodes.map((item) => [item.id, item]));
    const visited = new Set();
    function visit(id) {
      if (visited.has(id)) return;
      visited.add(id);
      const item = byId.get(id);
      if (item?.source) rustPackages.set(item.id, item);
      for (const dependency of nodes.get(id)?.deps ?? []) {
        if (dependency.dep_kinds.some((kind) => kind.kind === null)) visit(dependency.pkg);
      }
    }
    visit(metadata.resolve.root);
  }
}
const rust = [...rustPackages.values()].map((item) => ({ name: item.name, version: item.version, source: item.source, license: item.license,
  texts: copyNotices(path.dirname(item.manifest_path), path.join(output, "rust", "crates", folder(item.name, item.version)), item.license, item.name) }));
const compare = (a, b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`);
frontend.sort(compare);
rust.sort(compare);
writeJson(path.join(output, "frontend", "index.json"), { packageCount: frontend.length, packages: frontend });
writeJson(path.join(output, "rust", "index.json"), { targets, packageCount: rust.length, packages: rust });
// Python/Git/GitHub CLI are system tools; ORT/DirectML/RenderDoc are not shipped.
// Managed Unity assembly notices remain with the corresponding staged DLLs.
writeJson(path.join(output, "binaries", "index.json"), { packageCount: 0, packages: [] });
const manifestFiles = ["package.json", "bun.lock", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "locus_native_plugin/Cargo.toml", "locus_native_plugin/Cargo.lock"];
for (const file of manifestFiles) {
  const destination = path.join(output, "manifests", file);
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.copyFileSync(path.join(root, file), destination);
}
writeJson(path.join(output, "bundle-manifest.json"), { platform: "macos", targets,
  frontend: { packageCount: frontend.length }, rust: { packageCount: rust.length },
  binaries: { packageCount: 0 }, manifests: { fileCount: manifestFiles.length } });
fs.writeFileSync(path.join(output, "README.md"), "# macOS Third-Party License Bundle\n\nGenerated by scripts/generate-macos-license-bundle.mjs from both Darwin dependency graphs, including the Unity native broker. Unity managed assembly notices are kept beside the staged Json/Roslyn DLLs. Python, Git and GitHub CLI are system installations. ONNX Runtime, DirectML and RenderDoc binaries are not bundled on macOS.\n");
console.log(`[locus] macOS license bundle: ${frontend.length} frontend packages, ${rust.length} Rust crates, no Windows runtime binaries -> ${output}`);
