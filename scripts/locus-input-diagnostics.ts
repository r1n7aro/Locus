import { spawn, execFileSync } from "node:child_process";
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { discoverLocusBrowserUrl, isLocusPageTarget } from "./locus-cdp-discovery.mjs";
import { CdpClient, sleep } from "./locus-webview2-stress-client";
import { installInputProbe } from "./locus-input-probe.mjs";

const args = process.argv.slice(2);
if (args.includes("--help")) {
  console.log(`Usage: bun run scripts/locus-input-diagnostics.ts [--browser-url http://127.0.0.1:<port>] [--duration-ms 15000] [--output-dir .tmp/locus-input-captures]
Requires Locus debug mode. During capture, move/click the mouse in the affected window.
Records native window state and DOM event metadata; never types, clicks, focuses, reloads, or repairs the app.`);
  process.exit(0);
}
if (process.platform !== "win32") throw new Error("Native input capture requires Windows.");
const options = new Map<string, string>();
for (let i = 0; i < args.length; i += 2) {
  const name = args[i]!;
  if (!["--browser-url", "--duration-ms", "--output-dir"].includes(name) || !args[i + 1]) {
    throw new Error(`Invalid option: ${name}. Use --help.`);
  }
  options.set(name, args[i + 1]!);
}
const durationMs = Number(options.get("--duration-ms") ?? 15000);
if (!Number.isInteger(durationMs) || durationMs < 1000 || durationMs > 300000) {
  throw new Error("--duration-ms must be an integer between 1000 and 300000.");
}
const preferredUrl = options.get("--browser-url");
function localBrowserUrl(value: string): string {
  const parsed = new URL(value);
  if (parsed.protocol !== "http:" || !["localhost", "127.0.0.1"].includes(parsed.hostname)
    || !parsed.port || parsed.username || parsed.password || parsed.pathname !== "/" || parsed.search || parsed.hash) {
    throw new Error("The browser URL must be an explicit local HTTP port.");
  }
  return parsed.origin;
}
// An explicit selection must never silently fall back to another instance.
const browserUrl = preferredUrl ? localBrowserUrl(preferredUrl)
  : localBrowserUrl(await discoverLocusBrowserUrl());
async function readJson(suffix: string) {
  const response = await fetch(`${browserUrl}${suffix}`, { signal: AbortSignal.timeout(2000) });
  if (!response.ok) throw new Error(`${suffix}: HTTP ${response.status}`);
  return response.json();
}
const version = await readJson("/json/version");
const targets = (await readJson("/json/list")).filter(isLocusPageTarget);
if (targets.length !== 1) throw new Error("Expected exactly one main Locus page at the selected port.");
const target = targets[0];
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl);
async function bounded<T>(operation: Promise<T>, timeoutMs = 1500): Promise<T> {
  let timer: ReturnType<typeof setTimeout>;
  try {
    return await Promise.race([operation, new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`Probe timed out after ${timeoutMs}ms`)), timeoutMs);
    })]);
  } finally { clearTimeout(timer!); }
}
const key = `__LOCUS_INPUT_CAPTURE_${randomUUID().replaceAll("-", "")}`;
let native: ReturnType<typeof spawn> | undefined;
let interrupted = false;
const interrupt = () => { interrupted = true; };
process.on("SIGINT", interrupt);
process.on("SIGTERM", interrupt);
let outputDir: string | undefined;
try {
  const enabled = await bounded(cdp.evaluate<boolean>(`localStorage.getItem("locus:webview-bridge:debug-enabled:v1") === "1"`));
  if (!enabled) throw new Error("Locus debug mode is disabled. Enable Settings > General > Debug mode before capturing.");

  // Resolve the process from this exact CDP listener, including a development
  // WebView2 browser's parent chain. Never guess between running Locus instances.
  const port = Number(new URL(browserUrl).port);
  const processInfo = JSON.parse(execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `
    $ErrorActionPreference = 'Stop'
    $captureOwners = @(Get-NetTCPConnection -LocalPort ${port} -State Listen | Select-Object -ExpandProperty OwningProcess -Unique)
    if ($captureOwners.Count -ne 1) { throw 'Ambiguous CDP listener process.' }
    $captureProcess = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + $captureOwners[0])
    for ($depth = 0; $depth -lt 12 -and $captureProcess; $depth++) {
      if ($captureProcess.Name -eq 'locus.exe') {
        $captureProcess | Select-Object ProcessId,ParentProcessId,CreationDate,ExecutablePath | ConvertTo-Json -Compress
        exit 0
      }
      $captureProcess = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + $captureProcess.ParentProcessId)
    }
    throw 'The selected CDP listener does not belong to Locus.'
  `], { encoding: "utf8", windowsHide: true, timeout: 10000 }));

  const installed = await bounded(cdp.evaluate<{ enabled: boolean }>(
    `(${installInputProbe.toString()})(${JSON.stringify(key)}, ${durationMs + 10000})`,
  ));
  if (!installed.enabled) throw new Error("Debug mode was disabled before capture started.");
  outputDir = path.resolve(options.get("--output-dir") ?? ".tmp/locus-input-captures", `${new Date().toISOString().replaceAll(":", "-")}-${randomUUID().slice(0, 8)}`);
  mkdirSync(outputDir, { recursive: true });
  writeFileSync(path.join(outputDir, "manifest.json"), JSON.stringify({
    startedAt: new Date().toISOString(), durationMs, browserUrl, processInfo,
    target: { id: target.id, url: target.url }, browserVersion: version.Browser,
    note: "No DOM events alone is not proof of failure. Compare real mouse activity, native hit target, focus, captures and thread probes. WM_NULL response does not prove normal input dispatch.",
  }, null, 2));
  native = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-File",
    path.join(import.meta.dirname, "locus-native-input-snapshot.ps1"),
    "-LocusProcessId", String(processInfo.ProcessId), "-DurationMs", String(durationMs),
  ], { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
  let nativeOutputBytes = 0;
  let nativeError: string | null = null;
  native.stdout!.on("data", (data) => {
    nativeOutputBytes += data.length;
    appendFileSync(path.join(outputDir!, "native.ndjson"), data);
  });
  native.stderr!.on("data", (data) => appendFileSync(path.join(outputDir!, "native-errors.log"), data));
  native.on("error", (error) => {
    nativeError = String(error);
    appendFileSync(path.join(outputDir!, "native-errors.log"), nativeError);
  });
  const nativeDone = new Promise<void>((resolve) => native!.once("close", (code, signal) => {
    if (code !== null && code !== 0) nativeError = `Native helper exited with code ${code}`;
    writeFileSync(path.join(outputDir!, "native-exit.json"), JSON.stringify({ code, signal }));
    resolve();
  }));
  console.log(`LOCUS_INPUT_CAPTURE_READY ${JSON.stringify({ outputDir, pid: processInfo.ProcessId, durationMs })}`);
  const deadline = Date.now() + durationMs;
  let reason = "duration-elapsed";
  while (!interrupted && Date.now() < deadline) {
    try {
      const sample = await bounded(cdp.evaluate<{ enabled: boolean; stopped?: boolean }>(
        `window[${JSON.stringify(key)}]?.sample() ?? { enabled: false }`,
      ));
      appendFileSync(path.join(outputDir, "dom.ndjson"), `${JSON.stringify(sample)}\n`);
      if (!sample.enabled || sample.stopped) { reason = "debug-disabled-or-probe-stopped"; break; }
    } catch (error) {
      // Keep native evidence when renderer evaluation fails. Do not stack
      // unanswered CDP requests or rely on the stalled renderer for cleanup.
      appendFileSync(path.join(outputDir, "dom.ndjson"), `${JSON.stringify({ atMs: Date.now(), error: String(error) })}\n`);
      reason = "dom-probe-failed";
      break;
    }
    await sleep(500);
  }
  // Stop only the helper created above. No Locus/WebView2 process is terminated.
  native.kill();
  await bounded(nativeDone, 3000).catch(() => undefined);
  writeFileSync(path.join(outputDir, "result.json"), JSON.stringify({
    finishedAt: new Date().toISOString(), reason: interrupted ? "interrupted" : reason,
    nativeOutputBytes, nativeError,
  }, null, 2));
  const complete = nativeOutputBytes > 0 && !nativeError && reason === "duration-elapsed" && !interrupted;
  console.log(`LOCUS_INPUT_CAPTURE_FINISHED ${JSON.stringify({ outputDir, reason, complete })}`);
  if (nativeError || nativeOutputBytes === 0) process.exitCode = 1;
} finally {
  native?.kill();
  await bounded(cdp.evaluate(`(() => { const p=window[${JSON.stringify(key)}]; p?.stop(); delete window[${JSON.stringify(key)}]; })()`), 1000).catch(() => undefined);
  cdp.close();
  process.off("SIGINT", interrupt);
  process.off("SIGTERM", interrupt);
}
