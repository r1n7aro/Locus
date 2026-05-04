import { spawn, spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  readdirSync,
  readFileSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
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
const NSIS_SCRIPT_NAME = "installer.nsi";
const NSIS_OUTPUT_NAME = "nsis-output.exe";
const NSIS_UPDATE_PAGE_MARKER = "; Locus: upgrade-compatible NSIS installs update in place.";
const NSIS_UPDATE_LEAVE_MARKER = "; Locus: keep upgrade path on update mode.";
const NSIS_LOCATION_MARKER = "; Locus: read install locations from historical metadata keys.";
const NSIS_UNITY_PACKAGE_MARKER = "; Locus: replace bundled Unity package resources.";
const NSIS_LEGACY_BUILTIN_SKILL_MARKER = "; Locus: remove legacy root-level bundled skill resources.";
const NSIS_MANAGED_RUNTIME_MARKER = "; Locus: replace optional bundled Python and Git resources.";

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

function splitBundleTargets(value) {
  return value
    .split(/[,\s]+/)
    .map((target) => target.trim().toLowerCase())
    .filter(Boolean);
}

function getBundleTargets(currentArgs) {
  const targets = [];

  for (let index = 0; index < currentArgs.length; index += 1) {
    const arg = currentArgs[index];

    if (arg === "--bundles" || arg === "-b") {
      const value = currentArgs[index + 1];

      if (value && !value.startsWith("-")) {
        targets.push(...splitBundleTargets(value));
        index += 1;
      }

      continue;
    }

    if (arg.startsWith("--bundles=")) {
      targets.push(...splitBundleTargets(arg.slice("--bundles=".length)));
      continue;
    }

    if (arg.startsWith("-b=")) {
      targets.push(...splitBundleTargets(arg.slice("-b=".length)));
    }
  }

  return targets;
}

function getTauriCommand(currentArgs) {
  for (const arg of currentArgs) {
    if (TAURI_TOP_LEVEL_COMMANDS.has(arg)) {
      return arg;
    }
  }

  return currentArgs.find((arg) => !arg.startsWith("-")) ?? "";
}

function shouldPatchNsisBuild(currentArgs) {
  if (process.platform !== "win32" || isHelpOrVersionCommand) {
    return false;
  }

  const command = getTauriCommand(currentArgs);

  if (command !== "build" && command !== "bundle") {
    return false;
  }

  const bundleTargets = getBundleTargets(currentArgs);

  return (
    bundleTargets.length === 0 ||
    bundleTargets.some((target) => target === "nsis" || target === "all")
  );
}

function readNsisDefine(content, name) {
  return content.match(new RegExp(`^!define\\s+${name}\\s+"([^"]*)"`, "m"))?.[1] ?? null;
}

function getLineEnding(content) {
  return content.includes("\r\n") ? "\r\n" : "\n";
}

function replaceOnce(content, pattern, replacementLines, missingMessage) {
  if (!pattern.test(content)) {
    throw new Error(missingMessage);
  }

  const lineEnding = getLineEnding(content);
  return content.replace(pattern, () => replacementLines.join(lineEnding));
}

function patchNsisUpdatePage(content) {
  if (content.includes(NSIS_UPDATE_PAGE_MARKER)) {
    return content;
  }

  return replaceOnce(
    content,
    /  ; Upgrading\r?\n  \$\{ElseIf\} \$R0 = 1\r?\n    StrCpy \$R1 "\$\(olderOrUnknownVersionInstalled\)"/,
    [
      "  ; Upgrading",
      "  ${ElseIf} $R0 = 1",
      "    ${If} $WixMode <> 1",
      `      ${NSIS_UPDATE_PAGE_MARKER}`,
      "      StrCpy $UpdateMode 1",
      "      Abort",
      "    ${EndIf}",
      '    StrCpy $R1 "$(olderOrUnknownVersionInstalled)"',
    ],
    "Unable to find the NSIS upgrade page branch to patch.",
  );
}

function patchNsisUpdateLeave(content) {
  if (content.includes(NSIS_UPDATE_LEAVE_MARKER)) {
    return content;
  }

  return replaceOnce(
    content,
    /  \$\{ElseIf\} \$R0 = 1 ; Upgrading\r?\n    \$\{If\} \$R1 = 1[^\r\n]*\r?\n      Goto reinst_uninstall\r?\n    \$\{Else\}\r?\n      Goto reinst_done[^\r\n]*\r?\n    \$\{EndIf\}/,
    [
      "  ${ElseIf} $R0 = 1 ; Upgrading",
      `    ${NSIS_UPDATE_LEAVE_MARKER}`,
      "    StrCpy $UpdateMode 1",
      "    Goto reinst_done",
    ],
    "Unable to find the NSIS upgrade leave branch to patch.",
  );
}

function patchNsisInstallLocation(content) {
  if (content.includes(NSIS_LOCATION_MARKER)) {
    return content;
  }

  return replaceOnce(
    content,
    /Function RestorePreviousInstallLocation\r?\n  ReadRegStr \$4 SHCTX "\$\{MANUPRODUCTKEY\}" ""\r?\n  StrCmp \$4 "" \+2 0\r?\n    StrCpy \$INSTDIR \$4\r?\nFunctionEnd/,
    [
      "Function RestorePreviousInstallLocation",
      '  ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""',
      `  ${NSIS_LOCATION_MARKER}`,
      '  ${If} $4 == ""',
      '    ReadRegStr $4 SHCTX "Software\\locus\\${PRODUCTNAME}" ""',
      "  ${EndIf}",
      '  ${If} $4 == ""',
      '    ReadRegStr $4 SHCTX "Software\\dot\\${PRODUCTNAME}" ""',
      "  ${EndIf}",
      '  ${If} $4 == ""',
      '    ReadRegStr $4 SHCTX "Software\\FarLocus\\${PRODUCTNAME}" ""',
      "  ${EndIf}",
      '  StrCmp $4 "" +2 0',
      "    StrCpy $INSTDIR $4",
      "FunctionEnd",
    ],
    "Unable to find the NSIS install location restore function to patch.",
  );
}

function patchNsisUnityPackageCleanup(content) {
  if (content.includes(NSIS_UNITY_PACKAGE_MARKER)) {
    return content;
  }

  return replaceOnce(
    content,
    /Section Install\r?\n  SetOutPath \$INSTDIR/,
    [
      "Section Install",
      "  SetOutPath $INSTDIR",
      `  ${NSIS_UNITY_PACKAGE_MARKER}`,
      '  RMDir /r "$INSTDIR\\locus_unity"',
    ],
    "Unable to find the NSIS install section to patch.",
  );
}

function patchNsisLegacyBuiltinSkillCleanup(content) {
  if (content.includes(NSIS_LEGACY_BUILTIN_SKILL_MARKER)) {
    return content;
  }

  return replaceOnce(
    content,
    /Section Install\r?\n  SetOutPath \$INSTDIR/,
    [
      "Section Install",
      "  SetOutPath $INSTDIR",
      `  ${NSIS_LEGACY_BUILTIN_SKILL_MARKER}`,
      '  Delete "$INSTDIR\\knowledge\\skill\\create-skill.md"',
      '  Delete "$INSTDIR\\knowledge\\skill\\unity-editor-tooling.md"',
      '  Delete "$INSTDIR\\knowledge\\skill\\unity-project-setup.md"',
    ],
    "Unable to find the NSIS install section to patch.",
  );
}

function patchNsisManagedRuntimeCleanup(content) {
  if (content.includes(NSIS_MANAGED_RUNTIME_MARKER)) {
    return content;
  }

  return replaceOnce(
    content,
    /Section Install\r?\n  SetOutPath \$INSTDIR/,
    [
      "Section Install",
      "  SetOutPath $INSTDIR",
      `  ${NSIS_MANAGED_RUNTIME_MARKER}`,
      '  RMDir /r "$INSTDIR\\managed-python"',
      '  RMDir /r "$INSTDIR\\managed-git"',
    ],
    "Unable to find the NSIS install section to patch.",
  );
}

function patchNsisInstallerScript(scriptPath) {
  const original = readFileSync(scriptPath, "utf8");
  const productName = readNsisDefine(original, "PRODUCTNAME");
  const version = readNsisDefine(original, "VERSION");
  const arch = readNsisDefine(original, "ARCH");
  const outFile = readNsisDefine(original, "OUTFILE") ?? NSIS_OUTPUT_NAME;

  if (!productName || !version || !arch) {
    throw new Error(`Unable to read NSIS installer metadata from ${scriptPath}.`);
  }

  const next = patchNsisManagedRuntimeCleanup(
    patchNsisUnityPackageCleanup(
      patchNsisLegacyBuiltinSkillCleanup(
        patchNsisInstallLocation(
          patchNsisUpdateLeave(patchNsisUpdatePage(original)),
        ),
      ),
    ),
  );

  if (next !== original) {
    writeFileSync(scriptPath, next);
  }

  return { productName, version, arch, outFile };
}

function findGeneratedNsisScripts(startedAtMs) {
  const profileNames = tauriArgs.includes("--debug") ? ["debug", "release"] : ["release", "debug"];
  const scripts = [];

  for (const profileName of profileNames) {
    const nsisRoot = path.join(srcTauriDir, "target", profileName, "nsis");

    if (!existsSync(nsisRoot)) {
      continue;
    }

    for (const entry of readdirSync(nsisRoot, { withFileTypes: true })) {
      if (!entry.isDirectory()) {
        continue;
      }

      const scriptPath = path.join(nsisRoot, entry.name, NSIS_SCRIPT_NAME);

      if (existsSync(scriptPath)) {
        scripts.push(scriptPath);
      }
    }
  }

  const freshScripts = scripts.filter((scriptPath) => statSync(scriptPath).mtimeMs >= startedAtMs - 1000);
  return freshScripts.length > 0 ? freshScripts : scripts;
}

function resolveMakensisExecutable() {
  const candidates = [
    env.NSIS_HOME ? path.join(env.NSIS_HOME, "makensis.exe") : null,
    env.LOCALAPPDATA ? path.join(env.LOCALAPPDATA, "tauri", "NSIS", "makensis.exe") : null,
    findExecutableInPath("makensis"),
  ].filter(Boolean);

  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  throw new Error("Unable to find makensis.exe for the patched NSIS rebuild.");
}

function getNsisBundleDir(scriptPath) {
  const targetProfileDir = path.dirname(path.dirname(path.dirname(scriptPath)));
  return path.join(targetProfileDir, "bundle", "nsis");
}

function findBundleInstaller(scriptPath, installerInfo, startedAtMs) {
  const bundleDir = getNsisBundleDir(scriptPath);
  const exactPath = path.join(
    bundleDir,
    `${installerInfo.productName}_${installerInfo.version}_${installerInfo.arch}-setup.exe`,
  );

  if (existsSync(exactPath)) {
    return exactPath;
  }

  if (!existsSync(bundleDir)) {
    throw new Error(`Unable to find NSIS bundle directory: ${bundleDir}`);
  }

  const prefix = `${installerInfo.productName}_${installerInfo.version}_`;
  const candidates = readdirSync(bundleDir)
    .filter((fileName) => fileName.startsWith(prefix) && fileName.endsWith(".exe"))
    .map((fileName) => {
      const filePath = path.join(bundleDir, fileName);
      return { filePath, modifiedAt: statSync(filePath).mtimeMs };
    })
    .filter(({ modifiedAt }) => modifiedAt >= startedAtMs - 1000)
    .sort((a, b) => b.modifiedAt - a.modifiedAt);

  if (candidates[0]) {
    return candidates[0].filePath;
  }

  throw new Error(`Unable to find generated NSIS installer for ${installerInfo.productName} ${installerInfo.version}.`);
}

function rebuildNsisInstaller(scriptPath, installerInfo, makensisExecutable) {
  const nsisDir = path.dirname(scriptPath);
  const outputPath = path.join(nsisDir, installerInfo.outFile);

  if (existsSync(outputPath)) {
    unlinkSync(outputPath);
  }

  const result = spawnSync(makensisExecutable, [path.basename(scriptPath)], {
    cwd: nsisDir,
    env,
    stdio: "inherit",
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    throw new Error(`makensis failed with exit code ${result.status ?? "unknown"}.`);
  }

  if (!existsSync(outputPath)) {
    throw new Error(`makensis did not produce ${outputPath}.`);
  }

  return outputPath;
}

function postProcessNsisInstallers(startedAtMs) {
  if (!shouldPatchNsisBuild(tauriArgs)) {
    return;
  }

  const scripts = findGeneratedNsisScripts(startedAtMs);

  if (scripts.length === 0) {
    return;
  }

  const makensisExecutable = resolveMakensisExecutable();

  for (const scriptPath of scripts) {
    const installerInfo = patchNsisInstallerScript(scriptPath);
    const rebuiltInstallerPath = rebuildNsisInstaller(scriptPath, installerInfo, makensisExecutable);
    const bundleInstallerPath = findBundleInstaller(scriptPath, installerInfo, startedAtMs);

    copyFileSync(rebuiltInstallerPath, bundleInstallerPath);
    console.log(`[locus] Patched NSIS updater compatibility: ${path.relative(repoRoot, bundleInstallerPath)}`);
  }
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

const tauriStartedAtMs = Date.now();
const tauriResult = await runTauriCli();

if (tauriResult.signal) {
  process.kill(process.pid, tauriResult.signal);
} else if (tauriResult.code !== 0) {
  process.exit(tauriResult.code ?? 1);
} else {
  try {
    postProcessNsisInstallers(tauriStartedAtMs);
    process.exit(0);
  } catch (error) {
    console.error(`[locus] Failed to patch NSIS installer compatibility: ${error.stack ?? error.message ?? error}`);
    process.exit(1);
  }
}
