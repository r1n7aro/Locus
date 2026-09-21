// The existing ILRepack executables remain Windows-only. Export portable
// AnyCPU managed outputs there, then verify them against this checkout on Mac.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifestName = "managed-bundle.json";
const managedFiles = ["Editor/Json/Locus.Json.dll", "Editor/Roslyn/Locus.Roslyn.dll"];
const sourceInputs = [
  "scripts/build-locus-json-bundle.mjs", "scripts/build-locus-roslyn-bundle.mjs",
  "locus_json/LocusJson.cs", "third_party/newtonsoft-json-13.0.3/assemblies",
  "third_party/roslyn-3.8.0/assemblies",
];
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function sourceFingerprint() {
  const hash = createHash("sha256");
  function add(relative) {
    const absolute = path.join(root, relative);
    for (const entry of readdirSync(absolute, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const nested = `${relative}/${entry.name}`;
      if (entry.isDirectory()) add(nested);
      else if (entry.name.endsWith(".dll")) { hash.update(nested); hash.update(readFileSync(path.join(root, nested))); }
    }
  }
  for (const relative of sourceInputs) {
    hash.update(relative);
    if (relative.endsWith("assemblies")) add(relative);
    // Git's checkout newline conversion must not make equivalent source stale.
    else hash.update(readFileSync(path.join(root, relative), "utf8").replaceAll("\r\n", "\n"));
  }
  return hash.digest("hex");
}

export function assertPortableManagedAssembly(bytes, label) {
  if (bytes.length < 256 || bytes.toString("ascii", 0, 2) !== "MZ") throw new Error(`Invalid managed assembly: ${label}`);
  const pe = bytes.readUInt32LE(0x3c);
  if (pe + 24 >= bytes.length || bytes.readUInt32LE(pe) !== 0x4550 || bytes.readUInt16LE(pe + 4) !== 0x14c) {
    throw new Error(`Expected AnyCPU PE assembly: ${label}`);
  }
  const optional = pe + 24;
  const optionalSize = bytes.readUInt16LE(pe + 20);
  if (bytes.readUInt16LE(optional) !== 0x10b || optionalSize < 224) throw new Error(`Invalid CLR header: ${label}`);
  const clrRva = bytes.readUInt32LE(optional + 96 + 14 * 8);
  const sections = bytes.readUInt16LE(pe + 6);
  for (let i = 0; i < sections; i++) {
    const offset = optional + optionalSize + i * 40;
    if (offset + 40 > bytes.length) break;
    const address = bytes.readUInt32LE(offset + 12);
    const size = bytes.readUInt32LE(offset + 16);
    if (clrRva >= address && clrRva + 20 <= address + size) {
      const clr = bytes.readUInt32LE(offset + 20) + clrRva - address;
      if (clr + 20 > bytes.length) break;
      const flags = bytes.readUInt32LE(clr + 16);
      if ((flags & 1) !== 0 && (flags & 2) === 0) return;
    }
  }
  throw new Error(`Assembly is not portable IL-only AnyCPU: ${label}`);
}

export function verifyManagedBundle(directory) {
  const manifest = JSON.parse(readFileSync(path.join(directory, manifestName), "utf8"));
  if (manifest.schema !== 1 || manifest.sourceFingerprint !== sourceFingerprint()) {
    throw new Error("macOS managed bundle is stale; export it from this revision on Windows with bun run macos:managed:export");
  }
  for (const relative of managedFiles) {
    const bytes = readFileSync(path.join(directory, relative));
    assertPortableManagedAssembly(bytes, relative);
    if (manifest.files?.[relative] !== sha256(bytes)) throw new Error(`Managed bundle hash mismatch: ${relative}`);
  }
  return managedFiles;
}

if (process.argv.includes("--export")) {
  if (process.platform !== "win32") throw new Error("Managed bundle export uses the existing Windows build scripts; import the exported folder on macOS.");
  const directory = path.resolve(process.env.LOCUS_MACOS_MANAGED_BUNDLE ?? path.join(root, ".tmp", "macos-managed"));
  for (const script of ["build-locus-json-bundle.mjs", "build-locus-roslyn-bundle.mjs"]) {
    execFileSync(process.execPath, [path.join(root, "scripts", script)], { cwd: root, stdio: "inherit" });
  }
  const manifest = { schema: 1, sourceFingerprint: sourceFingerprint(), files: {} };
  for (const relative of managedFiles) {
    const source = path.join(root, "locus_unity", relative);
    const bytes = readFileSync(source);
    assertPortableManagedAssembly(bytes, relative);
    manifest.files[relative] = sha256(bytes);
    mkdirSync(path.dirname(path.join(directory, relative)), { recursive: true });
    copyFileSync(source, path.join(directory, relative));
  }
  writeFileSync(path.join(directory, manifestName), JSON.stringify(manifest, null, 2) + "\n");
  verifyManagedBundle(directory);
  console.log(`[locus] verified portable managed bundle exported to ${directory}`);
}
