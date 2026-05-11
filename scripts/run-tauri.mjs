import { spawn, spawnSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const WEBVIEW2_ARGS_KEY = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
const REMOTE_DEBUG_FLAG = "--remote-debugging-port=";
const LOCUS_WEBVIEW2_DEBUG_START_PORT = 19222;
const LOCUS_WEBVIEW2_DEBUG_PORT_ATTEMPTS = 25;
const CODEX_MCP_SERVER_NAME = "locus_webview2_devtools";
const LEGACY_CODEX_MCP_SERVER_NAMES = ["locus-webview2-devtools"];
const CODEX_CLI_ENV_KEY = "LOCUS_CODEX_CLI";
const CODEX_NODE_ENV_KEY = "LOCUS_CODEX_NODE";
const DEV_WITH_MCP_COMMAND = "dev-mcp";
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const srcTauriDir = path.join(repoRoot, "src-tauri");
const DEFAULT_RELEASE_FLAVOR_CONFIG = path.relative(
  repoRoot,
  path.join(srcTauriDir, "tauri.with_embed_python_git.conf.json"),
);
const chromeDevtoolsMcpWrapper = path.join(scriptDir, "chrome-devtools-mcp-wrapper.mjs");
const TAURI_TOP_LEVEL_COMMANDS = new Set([
  "android",
  "build",
  "bundle",
  "completions",
  "dev",
  "icon",
  "info",
  "init",
  "ios",
  "migrate",
  "permission",
  "plugin",
  "signer",
]);

const args = process.argv.slice(2);
const shouldRunDevWithMcp = args[0] === DEV_WITH_MCP_COMMAND;
let tauriArgs = shouldRunDevWithMcp ? ["dev", ...args.slice(1)] : args;
const env = { ...process.env };

const isHelpOrVersionCommand =
  tauriArgs.includes("--help") ||
  tauriArgs.includes("-h") ||
  tauriArgs.includes("--version") ||
  tauriArgs.includes("-V");
const shouldExposeWebView2DebugPort =
  process.platform === "win32" && shouldRunDevWithMcp && !isHelpOrVersionCommand;

function hasConfigArg(currentArgs) {
  for (let index = 0; index < currentArgs.length; index += 1) {
    const arg = currentArgs[index];

    if (arg === "--config" || arg === "-c") {
      return true;
    }

    if (arg.startsWith("--config=") || arg.startsWith("-c=")) {
      return true;
    }
  }

  return false;
}

function shouldInjectDefaultReleaseFlavor(currentArgs) {
  if (isHelpOrVersionCommand || hasConfigArg(currentArgs)) {
    return false;
  }

  const command = getTauriCommand(currentArgs);
  return command === "build" || command === "bundle";
}

if (shouldInjectDefaultReleaseFlavor(tauriArgs)) {
  tauriArgs = [...tauriArgs, "--config", DEFAULT_RELEASE_FLAVOR_CONFIG];
}

function canListenOnPort(port) {
  return new Promise((resolve) => {
    const server = net.createServer();

    server.once("error", () => resolve(false));
    server.once("listening", () => {
      server.close(() => resolve(true));
    });
    server.listen(Number(port), "127.0.0.1");
  });
}

async function findAvailableDebugPort() {
  for (let offset = 0; offset < LOCUS_WEBVIEW2_DEBUG_PORT_ATTEMPTS; offset += 1) {
    const port = LOCUS_WEBVIEW2_DEBUG_START_PORT + offset;

    if (await canListenOnPort(port)) {
      return port;
    }
  }

  return null;
}

function findExecutableInPath(command) {
  const pathEntries = process.env.PATH?.split(path.delimiter) ?? [];
  const extensions =
    process.platform === "win32"
      ? (process.env.PATHEXT?.split(";") ?? [".EXE", ".CMD", ".BAT", ".COM"])
      : [""];

  for (const pathEntry of pathEntries) {
    for (const extension of extensions) {
      const candidate = path.join(pathEntry, `${command}${extension.toLowerCase()}`);

      if (existsSync(candidate)) {
        return candidate;
      }
    }
  }

  return null;
}

function findWindowsAppsCodexExecutable() {
  const windowsAppsDir = path.join(process.env.ProgramFiles ?? "C:\\Program Files", "WindowsApps");

  try {
    return readdirSync(windowsAppsDir, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && entry.name.startsWith("OpenAI.Codex_"))
      .map((entry) => {
        const candidate = path.join(windowsAppsDir, entry.name, "app", "resources", "codex.exe");
        const modifiedAt = existsSync(candidate) ? statSync(candidate).mtimeMs : 0;

        return { candidate, modifiedAt };
      })
      .filter(({ modifiedAt }) => modifiedAt > 0)
      .sort((a, b) => b.modifiedAt - a.modifiedAt)[0]?.candidate ?? null;
  } catch {
    return null;
  }
}

function findWindowsAppsCodexNodeExecutable() {
  const windowsAppsDir = path.join(process.env.ProgramFiles ?? "C:\\Program Files", "WindowsApps");

  try {
    return readdirSync(windowsAppsDir, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && entry.name.startsWith("OpenAI.Codex_"))
      .map((entry) => {
        const candidate = path.join(windowsAppsDir, entry.name, "app", "resources", "node.exe");
        const modifiedAt = existsSync(candidate) ? statSync(candidate).mtimeMs : 0;

        return { candidate, modifiedAt };
      })
      .filter(({ modifiedAt }) => modifiedAt > 0)
      .sort((a, b) => b.modifiedAt - a.modifiedAt)[0]?.candidate ?? null;
  } catch {
    return null;
  }
}

function resolveCodexExecutable() {
  const configuredCodexCli = process.env[CODEX_CLI_ENV_KEY]?.trim();

  if (configuredCodexCli && existsSync(configuredCodexCli)) {
    return configuredCodexCli;
  }

  return findExecutableInPath("codex") ?? findWindowsAppsCodexExecutable();
}

function resolveNodeExecutable() {
  const configuredNode = process.env[CODEX_NODE_ENV_KEY]?.trim();

  if (configuredNode && existsSync(configuredNode)) {
    return configuredNode;
  }

  return findExecutableInPath("node") ?? findWindowsAppsCodexNodeExecutable() ?? process.execPath;
}

function getTauriCommand(currentArgs) {
  for (const arg of currentArgs) {
    if (TAURI_TOP_LEVEL_COMMANDS.has(arg)) {
      return arg;
    }
  }

  return currentArgs.find((arg) => !arg.startsWith("-")) ?? "";
}

function runTauriCli() {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["x", "tauri", ...tauriArgs], {
      stdio: "inherit",
      env,
    });

    child.on("exit", (code, signal) => {
      resolve({ code, signal });
    });

    child.on("error", reject);
  });
}

function runCodexMcp(args) {
  const codexExecutable = resolveCodexExecutable();

  if (!codexExecutable) {
    return {
      status: 1,
      stdout: "",
      stderr: `Codex CLI not found. Set ${CODEX_CLI_ENV_KEY} to the full codex.exe path to enable automatic MCP registration.`,
    };
  }

  return spawnSync(codexExecutable, ["mcp", ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function commandOutput(result) {
  return [result.stdout, result.stderr, result.error?.message]
    .filter(Boolean)
    .join("\n")
    .trim();
}

function getDebugUrl(port) {
  return `http://127.0.0.1:${port}`;
}

function withRemoteDebugPort(currentArgs, port) {
  const debugArg = `${REMOTE_DEBUG_FLAG}${port}`;

  if (!currentArgs?.trim()) {
    return debugArg;
  }

  const argsWithoutDebugPort = currentArgs
    .trim()
    .split(/\s+/)
    .filter((arg) => !arg.startsWith(REMOTE_DEBUG_FLAG));

  return [...argsWithoutDebugPort, debugArg].join(" ");
}

function ensureCodexDevtoolsMcp(port) {
  const debugUrl = getDebugUrl(port);
  const nodeExecutable = resolveNodeExecutable();
  const expectedFragments = [chromeDevtoolsMcpWrapper, debugUrl];

  for (const legacyServerName of LEGACY_CODEX_MCP_SERVER_NAMES) {
    const legacy = runCodexMcp(["get", legacyServerName]);

    if (legacy.status === 0) {
      runCodexMcp(["remove", legacyServerName]);
    }
  }

  const current = runCodexMcp(["get", CODEX_MCP_SERVER_NAME]);
  const currentOutput = commandOutput(current);

  if (current.error) {
    console.warn(`[locus] Failed to inspect Codex MCP config. ${currentOutput}`);
    return;
  }

  if (current.status === 0) {
    if (expectedFragments.every((fragment) => currentOutput.includes(fragment))) {
      return;
    }

    const remove = runCodexMcp(["remove", CODEX_MCP_SERVER_NAME]);

    if (remove.status !== 0) {
      console.warn(
        `[locus] Failed to update Codex MCP server "${CODEX_MCP_SERVER_NAME}". ${commandOutput(remove)}`,
      );
      return;
    }
  } else if (!currentOutput.includes("No MCP server named")) {
    console.warn(`[locus] Failed to inspect Codex MCP config. ${currentOutput}`);
    return;
  }

  const add = runCodexMcp([
    "add",
    CODEX_MCP_SERVER_NAME,
    "--",
    nodeExecutable,
    chromeDevtoolsMcpWrapper,
    "--browserUrl",
    debugUrl,
    "--no-usage-statistics",
  ]);

  if (add.status !== 0) {
    console.warn(
      `[locus] Failed to register Codex MCP server "${CODEX_MCP_SERVER_NAME}". ${commandOutput(add)}`,
    );
    return;
  }

  console.log(
    `[locus] Codex MCP server "${CODEX_MCP_SERVER_NAME}" registered for ${debugUrl}. Restart Codex Desktop to load new MCP tools if it is already running.`,
  );
}

if (shouldExposeWebView2DebugPort) {
  const debugPort = await findAvailableDebugPort();

  if (debugPort === null) {
    console.error(
      `[locus] No available WebView2 debug port found in ${LOCUS_WEBVIEW2_DEBUG_START_PORT}-${LOCUS_WEBVIEW2_DEBUG_START_PORT + LOCUS_WEBVIEW2_DEBUG_PORT_ATTEMPTS - 1}.`,
    );
    process.exit(1);
  }

  if (debugPort !== LOCUS_WEBVIEW2_DEBUG_START_PORT) {
    console.log(
      `[locus] WebView2 debug port ${LOCUS_WEBVIEW2_DEBUG_START_PORT} is in use; using ${debugPort}.`,
    );
  }

  ensureCodexDevtoolsMcp(debugPort);

  env[WEBVIEW2_ARGS_KEY] = withRemoteDebugPort(env[WEBVIEW2_ARGS_KEY], debugPort);
}

const tauriResult = await runTauriCli();

if (tauriResult.signal) {
  process.kill(process.pid, tauriResult.signal);
} else if (tauriResult.code !== 0) {
  process.exit(tauriResult.code ?? 1);
} else {
  process.exit(0);
}
