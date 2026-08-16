import { spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
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
const ISOLATED_RUNTIME_BASE_ENV_KEY = "LOCUS_ISOLATED_RUNTIME_BASE";
const SKIP_ONBOARDING_ENV_KEY = "LOCUS_SKIP_ONBOARDING";
const LOCAL_DEV_CONFIG_FILE = ".locus-dev.local.json";
const DEV_WITH_MCP_COMMAND = "dev-mcp";
const DEV_ISOLATED_COMMAND = "dev-isolated";
const DEV_WITH_MCP_ISOLATED_COMMAND = "dev-mcp-isolated";
const DEV_SERVER_PORT = 14901;
const DEV_SERVER_START_TIMEOUT_MS = 30_000;
const DEV_PREREQUISITE_SCRIPTS = [
  "compile-server:ensure",
  "ort:bundle",
  "github-cli:bundle",
  "renderdoc:bundle",
];
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
const processTreeWatchdog = path.join(scriptDir, "process-tree-watchdog.mjs");
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
  : { enabled: false, paths: {}, remainingArgs: args.slice(1), skipOnboarding: false };
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
  let manifest;
  try {
    manifest = prepareIsolatedRuntime(isolatedRuntime.paths, isolatedRuntime.skipOnboarding);
  } catch (error) {
    console.error(`[locus] ${error instanceof Error ? error.message : String(error)}`);
    process.exit(2);
  }
  env.LOCUS_RUNTIME_ROOT = manifest.runtimeRoot;
  env.LOCUS_RUNTIME_DATA_DIR = manifest.databaseDir;
  env.LOCUS_RUNTIME_CONFIG_DIR = manifest.configDir;
  env.LOCUS_RUNTIME_LOG_DIR = manifest.logDir;
  env.LOCUS_RUNTIME_WORKSPACE_DIR = manifest.workspace;
  env.WEBVIEW2_USER_DATA_FOLDER = manifest.webviewDataDir;
  if (manifest.skipOnboarding) {
    env[SKIP_ONBOARDING_ENV_KEY] = "1";
  }
  env.TEMP = manifest.systemTempDir;
  env.TMP = manifest.systemTempDir;
  console.log(`LOCUS_RUNTIME_JSON ${JSON.stringify(manifest)}`);
}

function parseIsolatedRuntimeArgs(values, enabledByCommand) {
  const paths = {};
  const remainingArgs = [];
  let enabled = enabledByCommand;
  let skipOnboarding = false;

  for (let index = 0; index < values.length; index += 1) {
    const arg = values[index];
    if (arg === "--isolated") {
      enabled = true;
      continue;
    }
    if (arg === "--skip-onboarding") {
      skipOnboarding = true;
      continue;
    }

    const [name, inlineValue = ""] = arg.split(/=(.*)/s, 2);
    const key = {
      "--runtime-root": "runtimeRoot",
      "--runtime-base": "runtimeBase",
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

  if (skipOnboarding && !enabled) {
    console.error("[locus] --skip-onboarding requires an isolated runtime.");
    process.exit(2);
  }

  return { enabled, paths, remainingArgs, skipOnboarding };
}

function prepareIsolatedRuntime(requestedPaths, skipOnboarding) {
  const runtimeRoot = requestedPaths.runtimeRoot
    ? path.resolve(requestedPaths.runtimeRoot)
    : createGeneratedRuntimeRoot(requestedPaths.runtimeBase);
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
    skipOnboarding,
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

function createGeneratedRuntimeRoot(requestedBase) {
  const base = resolveIsolatedRuntimeBase(requestedBase);
  mkdirSync(base, { recursive: true });
  return mkdtempSync(path.join(base, "locus-app-test-"));
}

function resolveIsolatedRuntimeBase(requestedBase) {
  if (requestedBase) {
    return path.resolve(requestedBase);
  }

  const environmentBase = process.env[ISOLATED_RUNTIME_BASE_ENV_KEY]?.trim();
  if (environmentBase) {
    return path.resolve(environmentBase);
  }

  const localConfigPath = path.join(repoRoot, LOCAL_DEV_CONFIG_FILE);
  if (!existsSync(localConfigPath)) {
    return tmpdir();
  }

  let localConfig;
  try {
    localConfig = JSON.parse(readFileSync(localConfigPath, "utf8"));
  } catch (error) {
    throw new Error(
      `Failed to read ${LOCAL_DEV_CONFIG_FILE}: ${error instanceof Error ? error.message : String(error)}`,
    );
  }

  const localBase = localConfig?.isolatedRuntimeBase;
  if (localBase == null || localBase === "") {
    return tmpdir();
  }
  if (typeof localBase !== "string" || !path.isAbsolute(localBase)) {
    throw new Error(`${LOCAL_DEV_CONFIG_FILE} isolatedRuntimeBase must be an absolute path.`);
  }
  return path.normalize(localBase);
}

function printIsolatedDevHelp() {
  console.log(`Usage:
  bun tauri dev-mcp --isolated [options]
  bun tauri dev-mcp-isolated [options]
  bun run locus:test:app -- [options]

Options:
  --runtime-root <dir>       Root used for every unspecified isolated directory
  --runtime-base <dir>       Parent for an automatically named isolated runtime
  --database-dir <dir>      Directory containing locus.db (alias: --data-dir)
  --config-dir <dir>        Persistent application configuration directory
  --log-dir <dir>           Directory containing locus.log
  --workspace <dir>         Initial Locus workspace; created when missing
  --webview-data-dir <dir>  Isolated WebView2 profile and local storage
  --skip-onboarding         Open the isolated instance directly in Chat

Generated runtime root precedence: --runtime-base,
${ISOLATED_RUNTIME_BASE_ENV_KEY}, ${LOCAL_DEV_CONFIG_FILE}, then the system
temporary directory. Locus prints the result as LOCUS_RUNTIME_JSON.`);
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

const managedChildren = new Set();
let shutdownPromise = null;

function startProcessTreeWatchdog(child) {
  if (!child.pid || !existsSync(processTreeWatchdog)) {
    return null;
  }

  const watchdog = spawn(process.execPath, [processTreeWatchdog, String(child.pid)], {
    stdio: ["pipe", "ignore", "ignore"],
    windowsHide: true,
  });
  watchdog.on("error", (error) => {
    console.warn(`[locus] Process watchdog failed: ${error.message}`);
  });
  watchdog.stdin?.on("error", () => {
    // The watchdog can exit as soon as the target process exits.
  });
  watchdog.unref();
  watchdog.stdin?.unref?.();
  return watchdog;
}

function closeProcessTreeWatchdog(managed) {
  const watchdog = managed.watchdog;
  if (!watchdog) {
    return;
  }
  managed.watchdog = null;
  if (!watchdog.stdin?.destroyed) {
    watchdog.stdin.end();
  }
}

function spawnManagedChild(command, commandArgs, options) {
  const { label = path.basename(command), ...spawnOptions } = options;
  const child = spawn(command, commandArgs, spawnOptions);
  const managed = {
    child,
    label,
    watchdog: null,
    result: null,
  };
  managed.watchdog = startProcessTreeWatchdog(child);
  managed.result = new Promise((resolve) => {
    let settled = false;
    const settle = (result) => {
      if (settled) {
        return;
      }
      settled = true;
      managedChildren.delete(managed);
      closeProcessTreeWatchdog(managed);
      resolve(result);
    };
    child.once("error", (error) => settle({ code: 1, signal: null, error }));
    child.once("exit", (code, signal) => settle({ code, signal, error: null }));
  });
  managedChildren.add(managed);
  return managed;
}

function isManagedChildRunning(managed) {
  return (
    managed?.child?.pid &&
    managed.child.exitCode === null &&
    managed.child.signalCode === null
  );
}

function terminateProcessTree(pid) {
  if (!pid) {
    return;
  }

  if (process.platform === "win32") {
    spawnSync("taskkill", ["/pid", String(pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
    return;
  }

  try {
    process.kill(pid, "SIGTERM");
  } catch {
    // The process can finish before the termination request reaches it.
  }
}

async function terminateManagedChild(managed) {
  if (!managed) {
    return;
  }
  if (isManagedChildRunning(managed)) {
    terminateProcessTree(managed.child.pid);
  }
  closeProcessTreeWatchdog(managed);
  await managed.result;
}

async function terminateAllManagedChildren() {
  const active = [...managedChildren].reverse();
  await Promise.allSettled(active.map((managed) => terminateManagedChild(managed)));
}

function signalExitCode(signal) {
  return signal === "SIGINT" ? 130 : signal === "SIGTERM" ? 143 : 1;
}

function installShutdownHandlers() {
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.once(signal, () => {
      shutdownPromise ??= terminateAllManagedChildren().finally(() => {
        process.exit(signalExitCode(signal));
      });
    });
  }
}

function formatChildFailure(label, result) {
  if (result.error) {
    return `${label} failed to start: ${result.error.message}`;
  }
  if (result.signal) {
    return `${label} exited from ${result.signal}`;
  }
  return `${label} exited with code ${result.code ?? 1}`;
}

async function runDevPrerequisites() {
  for (const scriptName of DEV_PREREQUISITE_SCRIPTS) {
    const managed = spawnManagedChild(process.execPath, ["run", scriptName], {
      cwd: repoRoot,
      stdio: "inherit",
      env,
      label: `bun run ${scriptName}`,
    });
    const result = await managed.result;
    if (result.error || result.signal || result.code !== 0) {
      throw new Error(formatChildFailure(managed.label, result));
    }
  }
}

function canConnectToPort(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port: Number(port) });
    socket.setTimeout(500);
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    const unavailable = () => {
      socket.destroy();
      resolve(false);
    };
    socket.once("error", unavailable);
    socket.once("timeout", unavailable);
  });
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForDevServer(port) {
  const deadline = Date.now() + DEV_SERVER_START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await canConnectToPort(port)) {
      return;
    }
    await delay(100);
  }
  throw new Error(`Dev server did not listen on port ${port} within ${DEV_SERVER_START_TIMEOUT_MS}ms.`);
}

async function startManagedDevServer() {
  if (!(await canListenOnPort(DEV_SERVER_PORT))) {
    throw new Error(
      `Dev server port ${DEV_SERVER_PORT} is already in use. Close the owning process before starting another Locus dev instance.`,
    );
  }

  await runDevPrerequisites();

  if (!(await canListenOnPort(DEV_SERVER_PORT))) {
    throw new Error(`Dev server port ${DEV_SERVER_PORT} became unavailable during preparation.`);
  }

  const managed = spawnManagedChild(process.execPath, ["run", "dev"], {
    cwd: repoRoot,
    stdio: "inherit",
    env,
    label: "Vite dev server",
  });
  const startup = await Promise.race([
    waitForDevServer(DEV_SERVER_PORT).then(() => ({ ready: true })),
    managed.result.then((result) => ({ ready: false, result })),
  ]);

  if (!startup.ready) {
    throw new Error(formatChildFailure(managed.label, startup.result));
  }
  return managed;
}

function runTauriCli() {
  if (!existsSync(tauriCliScript)) {
    console.error(`[locus] Tauri CLI not found at ${tauriCliScript}. Run "bun install" first.`);
    return null;
  }

  return spawnManagedChild(process.execPath, [tauriCliScript, ...tauriArgs], {
    cwd: repoRoot,
    stdio: "inherit",
    env,
    label: "Tauri CLI",
  });
}

async function superviseTauriDev(devServer, tauri) {
  const firstExit = await Promise.race([
    tauri.result.then((result) => ({ source: "tauri", result })),
    devServer.result.then((result) => ({ source: "dev-server", result })),
  ]);

  if (firstExit.source === "tauri") {
    await terminateManagedChild(devServer);
    return firstExit.result;
  }

  console.error(`[locus] ${formatChildFailure(devServer.label, firstExit.result)}.`);
  await terminateManagedChild(tauri);
  return {
    ...firstExit.result,
    code: firstExit.result.code || 1,
  };
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

installShutdownHandlers();

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

let tauriResult = { code: 1, signal: null, error: null };

try {
  const shouldManageDevServer = getTauriCommand(tauriArgs) === "dev" && !isHelpOrVersionCommand;
  const devServer = shouldManageDevServer ? await startManagedDevServer() : null;
  const tauri = runTauriCli();

  if (tauri) {
    tauriResult = devServer
      ? await superviseTauriDev(devServer, tauri)
      : await tauri.result;
  } else if (devServer) {
    await terminateManagedChild(devServer);
  }
} catch (error) {
  console.error(`[locus] ${error instanceof Error ? error.message : String(error)}`);
} finally {
  await terminateAllManagedChildren();
}

if (tauriResult.signal) {
  process.kill(process.pid, tauriResult.signal);
} else if (tauriResult.error || tauriResult.code !== 0) {
  process.exit(tauriResult.code ?? 1);
} else {
  process.exit(0);
}
