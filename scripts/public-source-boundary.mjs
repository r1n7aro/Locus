import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";

function git(repoRoot, args) {
  const result = spawnSync("git", args, { cwd: repoRoot, encoding: "utf8", windowsHide: true });
  if (result.status !== 0) throw new Error("Cannot verify the public source boundary with Git");
  return result.stdout;
}

function containsPath(parent, child) {
  const relative = path.relative(parent, child);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative));
}

/** Fail a public build before it can package a nested private repository. */
export function assertPublicSourceBoundary(repoRoot) {
  repoRoot = path.resolve(repoRoot);
  const privateRoot = path.join(repoRoot, "crates", "locus-server");
  if (git(repoRoot, ["ls-files", "--stage", "-z", "--", "crates/locus-server"]).length) {
    throw new Error("Private server files or a gitlink are tracked by the public repository");
  }
  const ignored = spawnSync("git", ["check-ignore", "--no-index", "-q", "--", "crates/locus-server/Cargo.toml"], { cwd: repoRoot, windowsHide: true });
  if (ignored.status !== 0) throw new Error("The private server directory must be excluded from the public repository");

  const manifests = new Set(git(repoRoot, ["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "Cargo.toml", "**/Cargo.toml", "package.json", "src-tauri/Cargo.lock"]).split("\0").filter(Boolean));
  manifests.add("package.json");
  manifests.add("src-tauri/Cargo.toml");
  for (const relative of manifests) {
    const file = path.join(repoRoot, relative);
    if (existsSync(file) && /locus-server/.test(readFileSync(file, "utf8"))) {
      throw new Error(`Public build manifest references the private server: ${relative}`);
    }
  }

  const tauriDir = path.join(repoRoot, "src-tauri");
  if (!existsSync(tauriDir)) return;
  for (const name of readdirSync(tauriDir).filter(name => /^tauri.*\.json$/.test(name) && !name.includes(".internal."))) {
    const config = JSON.parse(readFileSync(path.join(tauriDir, name), "utf8"));
    const resources = config.bundle?.resources || [];
    const sources = Array.isArray(resources) ? resources : Object.keys(resources);
    sources.push(...(config.bundle?.externalBin || []));
    for (const source of sources) {
      // A wildcard's static directory prefix can include the whole private tree.
      const normalized = source.replaceAll("\\", "/");
      const wildcard = normalized.search(/[?*[{]/);
      const prefix = wildcard < 0 ? normalized : normalized.slice(0, normalized.lastIndexOf("/", wildcard) + 1);
      const absolute = path.resolve(tauriDir, prefix || ".");
      if (containsPath(absolute, privateRoot) || containsPath(privateRoot, absolute)) {
        throw new Error(`Public Tauri resource includes the private server: ${name}: ${source}`);
      }
    }
  }
}
