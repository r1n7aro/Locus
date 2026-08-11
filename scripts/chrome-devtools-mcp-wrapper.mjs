import { createWriteStream, existsSync, mkdirSync, readFileSync, readdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const logPath = resolve(scriptDir, "..", ".tmp", "chrome-devtools-mcp-wrapper.log");
mkdirSync(dirname(logPath), { recursive: true });
const log = createWriteStream(logPath, { flags: "a" });

const bunPath = findBunExecutable();
const chromeDevtoolsMcpVersion = "0.23.0";
const packageName = "chrome-devtools-mcp";
const packageBin = join("build", "src", "bin", "chrome-devtools-mcp.js");
const mcpArgs = process.argv.slice(2);

function findBunExecutable() {
  const configured = process.env.BUN_EXE?.trim();
  if (configured) {
    return configured;
  }

  const executableName = process.platform === "win32" ? "bun.exe" : "bun";
  const bunInstall = process.env.BUN_INSTALL?.trim();
  if (bunInstall) {
    const installed = join(bunInstall, "bin", executableName);
    if (existsSync(installed)) {
      return installed;
    }
  }

  const pathEntries = process.env.PATH?.split(delimiter).filter(Boolean) ?? [];
  for (const pathEntry of pathEntries) {
    const candidate = join(pathEntry, executableName);
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  const home = process.env.USERPROFILE || process.env.HOME;
  if (home) {
    const userInstall = join(home, ".bun", "bin", executableName);
    if (existsSync(userInstall)) {
      return userInstall;
    }
  }

  return "bun";
}

function parseVersion(version) {
  return version.split(".").map((part) => Number.parseInt(part, 10) || 0);
}

function compareVersions(a, b) {
  const left = parseVersion(a);
  const right = parseVersion(b);

  for (let index = 0; index < Math.max(left.length, right.length); index += 1) {
    const diff = (left[index] ?? 0) - (right[index] ?? 0);

    if (diff !== 0) {
      return diff;
    }
  }

  return 0;
}

function getBunCacheDir() {
  if (process.env.BUN_INSTALL_CACHE_DIR) {
    return process.env.BUN_INSTALL_CACHE_DIR;
  }

  const home = process.env.USERPROFILE || process.env.HOME;

  return home ? join(home, ".bun", "install", "cache") : null;
}

function findCachedChromeDevtoolsMcpBin() {
  const cacheDir = getBunCacheDir();

  if (!cacheDir || !existsSync(cacheDir)) {
    return null;
  }

  return readdirSync(cacheDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && entry.name.startsWith(`${packageName}@`))
    .map((entry) => {
      const packageDir = join(cacheDir, entry.name);
      const packageJsonPath = join(packageDir, "package.json");
      const binPath = join(packageDir, packageBin);

      if (!existsSync(packageJsonPath) || !existsSync(binPath)) {
        return null;
      }

      try {
        const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"));

        return { binPath, version: packageJson.version || "0.0.0" };
      } catch {
        return null;
      }
    })
    .filter(Boolean)
    .sort((a, b) => compareVersions(b.version, a.version))[0]?.binPath ?? null;
}

const cachedMcpBin = findCachedChromeDevtoolsMcpBin();
const childCommand = cachedMcpBin ? process.execPath : bunPath;
const childArgs = cachedMcpBin
  ? [cachedMcpBin, ...mcpArgs]
  : ["x", `${packageName}@${chromeDevtoolsMcpVersion}`, ...mcpArgs];

const child = spawn(childCommand, childArgs, {
  cwd: process.cwd(),
  env: process.env,
  stdio: ["pipe", "pipe", "pipe"],
  windowsHide: true,
});

log.write(`[${new Date().toISOString()}] spawn ${childCommand} ${childArgs.join(" ")}\n`);

process.stdin.pipe(child.stdin);
child.stdout.pipe(process.stdout);
child.stderr.on("data", (chunk) => {
  log.write(chunk);
});

child.on("error", (error) => {
  log.write(`[${new Date().toISOString()}] child error: ${error.stack || error.message}\n`);
  process.exitCode = 1;
});

child.on("exit", (code, signal) => {
  log.write(`[${new Date().toISOString()}] exit code=${code ?? ""} signal=${signal ?? ""}\n`);
  process.exit(code ?? (signal ? 1 : 0));
});

process.on("exit", () => {
  log.end();
});
