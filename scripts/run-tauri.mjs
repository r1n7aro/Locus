import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, statSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
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
const DEV_ISOLATED_COMMAND = "dev-isolated";
const DEV_WITH_MCP_ISOLATED_COMMAND = "dev-mcp-isolated";
const ISOLATED_DEV_COMMANDS = new Set([
  DEV_ISOLATED_COMMAND,
  DEV_WITH_MCP_ISOLATED_COMMAND,
]);
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const srcTauriDir = path.join(repoRoot, "src-tauri");
const tauriCliScript = path.join(repoRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");
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
const requestedCommand = args[0] ?? "";
const supportsIsolatedRuntime = new Set([
  "dev",
  DEV_WITH_MCP_COMMAND,
  DEV_ISOLATED_COMMAND,
  DEV_WITH_MCP_ISOLATED_COMMAND,
]).has(requestedCommand);
const shouldRunDevWithMcp =
  requestedCommand === DEV_WITH_MCP_COMMAND ||
  requestedCommand === DEV_WITH_MCP_ISOLATED_COMMAND;
const isolatedRuntime = supportsIsolatedRuntime
  ? parseIsolatedRuntimeArgs(args.slice(1), ISOLATED_DEV_COMMANDS.has(requestedCommand))
  : { enabled: false, paths: {}, remainingArgs: args.slice(1) };
const isCustomDevCommand =
  requestedCommand === "dev" ||
  requestedCommand === DEV_WITH_MCP_COMMAND ||
  ISOLATED_DEV_COMMANDS.has(requestedCommand);
let tauriArgs = isCustomDevCommand
  ? ["dev", ...isolatedRuntime.remainingArgs]
  : args;
const env = { ...process.env };

const isHelpOrVersionCommand =
  tauriArgs.includes("--help") ||
  tauriArgs.includes("-h") ||
  tauriArgs.includes("--version") ||
  tauriArgs.includes("-V");
const shouldExposeWebView2DebugPort =
  process.platform === "win32" && shouldRunDevWithMcp && !isHelpOrVersionCommand;

if (isolatedRuntime.enabled && isHelpOrVersionCommand) {
  printIsolatedDevHelp();
  process.exit(0);
}

if (isolatedRuntime.enabled) {
  const manifest = prepareIsolatedRuntime(isolatedRuntime.paths);
  env.LOCUS_RUNTIME_ROOT = manifest.runtimeRoot;
  env.LOCUS_RUNTIME_DATA_DIR = manifest.databaseDir;
  env.LOCUS_RUNTIME_CONFIG_DIR = manifest.configDir;
  env.LOCUS_RUNTIME_LOG_DIR = manifest.logDir;
  env.LOCUS_RUNTIME_WORKSPACE_DIR = manifest.workspace;
  env.WEBVIEW2_USER_DATA_FOLDER = manifest.webviewDataDir;
  env.TEMP = manifest.systemTempDir;
  env.TMP = manifest.systemTempDir;
  console.log(`LOCUS_RUNTIME_JSON ${JSON.stringify(manifest)}`);
}

function parseIsolatedRuntimeArgs(values, enabledByCommand) {
  const paths = {};
  const remainingArgs = [];
  let enabled = enabledByCommand;

  for (let index = 0; index < values.length; index += 1) {
    const arg = values[index];
    if (arg === "--isolated") {
      enabled = true;
      continue;
    }

    const [name, inlineValue = ""] = arg.split(/=(.*)/s, 2);
    const key = {
      "--runtime-root": "runtimeRoot",
      "--database-dir": "databaseDir",
      "--data-dir": "databaseDir",
      "--config-dir": "configDir",
      "--log-dir": "logDir",
      "--workspace": "workspace",
      "--webview-data-dir": "webviewDataDir",
    }[name];
    if (!key) {
      remainingArgs.push(arg);
      continue;
    }

    enabled = true;
    const value = inlineValue || values[index + 1];
    if (!value || (!inlineValue && value.startsWith("--"))) {
      console.error(`[locus] ${name} requires a directory.`);
      process.exit(2);
    }
    paths[key] = path.resolve(value);
    if (!inlineValue) index += 1;
  }

  return { enabled, paths, remainingArgs };
}

function prepareIsolatedRuntime(requestedPaths) {
  const runtimeRoot = requestedPaths.runtimeRoot
    ? path.resolve(requestedPaths.runtimeRoot)
    : mkdtempSync(path.join(tmpdir(), "locus-app-test-"));
  const manifest = {
    runtimeRoot,
    databaseDir:
      requestedPaths.databaseDir ?? path.join(runtimeRoot, "database"),
    databaseFile: "",
    configDir: requestedPaths.configDir ?? path.join(runtimeRoot, "config"),
    logDir: requestedPaths.logDir ?? path.join(runtimeRoot, "logs"),
    logFile: "",
    workspace: requestedPaths.workspace ?? path.join(runtimeRoot, "workspace"),
    webviewDataDir:
      requestedPaths.webviewDataDir ?? path.join(runtimeRoot, "webview"),
    systemTempDir: path.join(runtimeRoot, "system-temp"),
  };
  manifest.databaseFile = path.join(manifest.databaseDir, "locus.db");
  manifest.logFile = path.join(manifest.logDir, "locus.log");

  for (const directory of [
    manifest.runtimeRoot,
    manifest.databaseDir,
    manifest.configDir,
    manifest.logDir,
    manifest.workspace,
    manifest.webviewDataDir,
    manifest.systemTempDir,
  ]) {
    mkdirSync(directory, { recursive: true });
  }
  return manifest;
}

function printIsolatedDevHelp() {
  console.log(`Usage:
  bun tauri dev-mcp --isolated [options]
  bun tauri dev-mcp-isolated [options]
  bun run locus:test:app -- [options]

Options:
  --runtime-root <dir>       Root used for every unspecified isolated directory
  --database-dir <dir>      Directory containing locus.db (alias: --data-dir)
  --config-dir <dir>        Persistent application configuration directory
  --log-dir <dir>           Directory containing locus.log
  --workspace <dir>         Initial Locus workspace; created when missing
  --webview-data-dir <dir>  Isolated WebView2 profile and local storage

When no directory is supplied, Locus creates a complete environment under
the system temporary directory and prints it as LOCUS_RUNTIME_JSON.`);
}

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
    if (!existsSync(tauriCliScript)) {
      console.error(`[locus] Tauri CLI not found at ${tauriCliScript}. Run "bun install" first.`);
      resolve({ code: 1, signal: null });
      return;
    }

    const child = spawn(process.execPath, [tauriCliScript, ...tauriArgs], {
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
