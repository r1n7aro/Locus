import { createWriteStream, existsSync, mkdirSync, readFileSync, readdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { resolveLocusBrowserArgs } from "./locus-cdp-discovery.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const logPath = resolve(scriptDir, "..", ".tmp", "chrome-devtools-mcp-wrapper.log");
mkdirSync(dirname(logPath), { recursive: true });
const log = createWriteStream(logPath, { flags: "a" });

const bunPath = findBunExecutable();
const chromeDevtoolsMcpVersion = "0.23.0";
const packageName = "chrome-devtools-mcp";
const packageBin = join("build", "src", "bin", "chrome-devtools-mcp.js");
const configuredMcpArgs = process.argv.slice(2);

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
const {
  args: mcpArgs,
  browserUrl: resolvedBrowserUrl,
  preferredUrl: configuredBrowserUrl,
} = await resolveLocusBrowserArgs(configuredMcpArgs);
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

let shutdownReason = null;
let forceKillTimer = null;

function shutdownChild(reason) {
  if (shutdownReason || child.exitCode !== null) {
    return;
  }
  shutdownReason = reason;
  log.write(`[${new Date().toISOString()}] shutdown reason=${reason}\n`);
  process.stdin.unpipe(child.stdin);
  if (!child.stdin.destroyed && !child.stdin.writableEnded) {
    child.stdin.end();
  }
  forceKillTimer = setTimeout(() => {
    if (child.exitCode === null && !child.killed) {
      log.write(`[${new Date().toISOString()}] force terminate child after stdin closed\n`);
      child.kill();
    }
  }, 1_000);
}

log.write(
  `[${new Date().toISOString()}] browserUrl configured=${configuredBrowserUrl ?? ""} resolved=${resolvedBrowserUrl}\n`,
);
log.write(`[${new Date().toISOString()}] spawn ${childCommand} ${childArgs.join(" ")}\n`);

process.stdin.pipe(child.stdin, { end: false });
child.stdout.pipe(process.stdout);
process.stdin.once("end", () => shutdownChild("stdin-end"));
process.stdin.once("close", () => shutdownChild("stdin-close"));
process.once("disconnect", () => shutdownChild("parent-disconnect"));
process.once("SIGINT", () => shutdownChild("SIGINT"));
process.once("SIGTERM", () => shutdownChild("SIGTERM"));

child.stdin.on("error", (error) => {
  if (error?.code !== "EPIPE") {
    log.write(`[${new Date().toISOString()}] child stdin error: ${error.stack || error.message}\n`);
  }
});

process.stdout.on("error", (error) => {
  if (error?.code === "EPIPE") {
    shutdownChild("stdout-closed");
    return;
  }
  log.write(`[${new Date().toISOString()}] stdout error: ${error.stack || error.message}\n`);
});

child.stderr.on("data", (chunk) => {
  log.write(chunk);
});

child.on("error", (error) => {
  log.write(`[${new Date().toISOString()}] child error: ${error.stack || error.message}\n`);
  process.exitCode = 1;
});

child.on("exit", (code, signal) => {
  if (forceKillTimer) {
    clearTimeout(forceKillTimer);
    forceKillTimer = null;
  }
  log.write(`[${new Date().toISOString()}] exit code=${code ?? ""} signal=${signal ?? ""}\n`);
  const exitCode = code ?? (signal && !shutdownReason ? 1 : 0);
  log.end(() => process.exit(exitCode));
});

process.on("exit", () => {
  if (child.exitCode === null && !child.killed) {
    child.kill();
  }
  log.end();
});
