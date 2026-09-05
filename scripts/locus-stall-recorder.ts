import { once } from "node:events";
import { createWriteStream, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { createGzip } from "node:zlib";
import { discoverLocusBrowserUrl } from "./locus-cdp-discovery.mjs";
import {
  CdpClient,
  findLocusWebViewTarget,
  sleep,
  type CdpEventMessage,
} from "./locus-webview2-stress-client";

const DEFAULT_TIMEOUT_MS = 20_000;
const DEFAULT_MAX_WAIT_MS = 60 * 60_000;
const DEFAULT_POST_CAPTURE_MS = 1_500;
const DEFAULT_ARM_DELAY_MS = 2_000;
const DEFAULT_TRACE_BUFFER_MB = 64;
const TRACE_COMPLETE_TIMEOUT_MS = 30_000;
const STREAM_CHUNK_BYTES = 1024 * 1024;
const STALL_CONSOLE_MODULE = "[RuntimePerformance]";
const STALL_CONSOLE_MESSAGE = "stall detected";
const TRACE_CATEGORIES = [
  "toplevel",
  "devtools.timeline",
  "disabled-by-default-devtools.timeline",
  "blink.user_timing",
  "v8.execute",
  "disabled-by-default-v8.cpu_profiler",
];

interface CliOptions {
  browserUrl: string;
  outputDir: string;
  timeoutMs: number;
  maxWaitMs: number;
  postCaptureMs: number;
  armDelayMs: number;
  traceBufferMb: number;
}

interface TraceCompleteParams {
  dataLossOccurred?: boolean;
  stream?: string;
  traceFormat?: string;
  streamCompression?: string;
}

interface CaptureTrigger {
  reason: "stall" | "manual" | "timeout";
  detectedAtMs: number;
}

interface PerformanceMetricsResponse {
  metrics?: Array<{ name: string; value: number }>;
}

const options = parseArgs(process.argv.slice(2));
const browserUrl = await discoverLocusBrowserUrl({ preferredUrl: options.browserUrl || null });
const target = await findLocusWebViewTarget(browserUrl, options.timeoutMs);
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl!);
let tracingStarted = false;
let tracingEnded = false;

try {
  await cdp.send("Runtime.enable");
  const debugState = await cdp.evaluate<{ debugEnabled: boolean; href: string }>(`(() => ({
    debugEnabled: window.__LOCUS_DEBUG_ENABLED__ === true
      || localStorage.getItem("locus:webview-bridge:debug-enabled:v1") === "1",
    href: location.href,
  }))()`);
  if (!debugState.debugEnabled) {
    throw new Error("Locus debug mode is disabled. Enable Settings > General > Debug mode first.");
  }

  const traceComplete = waitForTraceComplete(cdp);
  await cdp.send("Tracing.start", {
    transferMode: "ReturnAsStream",
    streamFormat: "json",
    bufferUsageReportingInterval: 5_000,
    traceConfig: {
      recordMode: "recordContinuously",
      traceBufferSizeInKb: options.traceBufferMb * 1024,
      enableSampling: true,
      includedCategories: TRACE_CATEGORIES,
    },
  });
  tracingStarted = true;
  if (options.armDelayMs > 0) await sleep(options.armDelayMs);
  const beforeMetrics = await readPerformanceMetrics(cdp);
  const trigger = waitForCaptureTrigger(cdp, options.maxWaitMs);
  console.log(`LOCUS_STALL_RECORDER_READY ${JSON.stringify({
    browserUrl,
    pageUrl: target.url,
    traceBufferMb: options.traceBufferMb,
    maxWaitMs: options.maxWaitMs,
    armDelayMs: options.armDelayMs,
  })}`);

  const capturedTrigger = await trigger;
  const incident = capturedTrigger.reason === "stall"
    ? await readLatestIncident(cdp)
    : null;
  if (capturedTrigger.reason === "stall" && options.postCaptureMs > 0) {
    await sleep(options.postCaptureMs);
  }
  const afterMetrics = await readPerformanceMetrics(cdp);

  await cdp.send("Tracing.end");
  tracingEnded = true;
  const completed = await Promise.race([
    traceComplete,
    sleep(TRACE_COMPLETE_TIMEOUT_MS).then(() => {
      throw new Error("Timed out waiting for Tracing.tracingComplete.");
    }),
  ]);
  if (!completed.stream) {
    throw new Error("Tracing completed without an IO stream handle.");
  }

  const artifact = await saveTraceStream(cdp, completed.stream, options.outputDir);
  const manifest = {
    capturedAt: new Date().toISOString(),
    browserUrl,
    pageUrl: target.url,
    trigger: capturedTrigger,
    incident,
    trace: {
      path: artifact.tracePath,
      compressedBytes: artifact.compressedBytes,
      uncompressedBytes: artifact.uncompressedBytes,
      dataLossOccurred: completed.dataLossOccurred ?? false,
      format: completed.traceFormat ?? "json",
      categories: TRACE_CATEGORIES,
      bufferMb: options.traceBufferMb,
    },
    performance: diffPerformanceMetrics(beforeMetrics, afterMetrics),
  };
  writeFileSync(artifact.manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  console.log(`LOCUS_STALL_CAPTURE_JSON ${JSON.stringify({
    tracePath: artifact.tracePath,
    manifestPath: artifact.manifestPath,
    reason: capturedTrigger.reason,
    dataLossOccurred: completed.dataLossOccurred ?? false,
    compressedBytes: artifact.compressedBytes,
  })}`);
} finally {
  if (tracingStarted && !tracingEnded) {
    try {
      await cdp.send("Tracing.end");
    } catch {
      // The page or runtime may already be gone.
    }
  }
  cdp.close();
}

function parseArgs(args: string[]): CliOptions {
  const parsed: CliOptions = {
    browserUrl: "",
    outputDir: path.resolve(".tmp", "locus-stall-captures"),
    timeoutMs: DEFAULT_TIMEOUT_MS,
    maxWaitMs: DEFAULT_MAX_WAIT_MS,
    postCaptureMs: DEFAULT_POST_CAPTURE_MS,
    armDelayMs: DEFAULT_ARM_DELAY_MS,
    traceBufferMb: DEFAULT_TRACE_BUFFER_MB,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    }
    const [name, inlineValue = ""] = arg.split(/=(.*)/s, 2);
    const value = inlineValue || args[index + 1];
    if (name === "--browser-url") {
      parsed.browserUrl = requireValue(name, value).replace(/\/$/, "");
    } else if (name === "--output-dir") {
      parsed.outputDir = path.resolve(requireValue(name, value));
    } else if (name === "--timeout-ms") {
      parsed.timeoutMs = positiveInteger(name, value, 1_000);
    } else if (name === "--max-wait-ms") {
      parsed.maxWaitMs = positiveInteger(name, value, 1_000);
    } else if (name === "--post-capture-ms") {
      parsed.postCaptureMs = positiveInteger(name, value, 0);
    } else if (name === "--arm-delay-ms") {
      parsed.armDelayMs = positiveInteger(name, value, 0);
    } else if (name === "--trace-buffer-mb") {
      parsed.traceBufferMb = positiveInteger(name, value, 4, 256);
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
    if (!inlineValue) index += 1;
  }
  return parsed;
}

function requireValue(name: string, value: string | undefined): string {
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value.`);
  return value;
}

function positiveInteger(
  name: string,
  value: string | undefined,
  minimum: number,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  const parsed = Number(requireValue(name, value));
  if (!Number.isInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${name} must be an integer between ${minimum} and ${maximum}.`);
  }
  return parsed;
}

function printHelp(): void {
  console.log(`Usage:
  bun run locus:test:stall-capture
  bun run locus:test:stall-capture -- --browser-url http://127.0.0.1:19222

Prerequisite:
  Enable Settings > General > Debug mode before starting the recorder.

Options:
  --browser-url <url>       Locus CDP URL; auto-detected on ports 19222-19246
  --output-dir <dir>        Capture directory, default .tmp/locus-stall-captures
  --timeout-ms <ms>         CDP discovery timeout, default ${DEFAULT_TIMEOUT_MS}
  --max-wait-ms <ms>        Maximum wait for a stall, default ${DEFAULT_MAX_WAIT_MS}
  --post-capture-ms <ms>    Time retained after detection, default ${DEFAULT_POST_CAPTURE_MS}
  --arm-delay-ms <ms>       Trace startup stabilization, default ${DEFAULT_ARM_DELAY_MS}
  --trace-buffer-mb <mb>    Circular trace buffer, 4-256 MB, default ${DEFAULT_TRACE_BUFFER_MB}

Press Ctrl+C to save the current trace immediately.`);
}

function waitForCaptureTrigger(cdpClient: CdpClient, maxWaitMs: number): Promise<CaptureTrigger> {
  return new Promise((resolve) => {
    let settled = false;
    let unsubscribe = () => {};
    let timeout: ReturnType<typeof setTimeout> | null = null;
    const finish = (reason: CaptureTrigger["reason"]) => {
      if (settled) return;
      settled = true;
      unsubscribe();
      if (timeout !== null) clearTimeout(timeout);
      process.off("SIGINT", onInterrupt);
      resolve({ reason, detectedAtMs: Date.now() });
    };
    const onInterrupt = () => finish("manual");
    unsubscribe = cdpClient.subscribeEvents((event) => {
      if (isStallConsoleEvent(event)) finish("stall");
    });
    process.once("SIGINT", onInterrupt);
    timeout = setTimeout(() => finish("timeout"), maxWaitMs);
  });
}

function isStallConsoleEvent(event: CdpEventMessage): boolean {
  if (event.method !== "Runtime.consoleAPICalled") return false;
  const args = Array.isArray(event.params.args) ? event.params.args : [];
  const values = args.map((arg) => (
    arg && typeof arg === "object" && "value" in arg
      ? (arg as { value?: unknown }).value
      : undefined
  ));
  return values.includes(STALL_CONSOLE_MODULE) && values.includes(STALL_CONSOLE_MESSAGE);
}

function waitForTraceComplete(cdpClient: CdpClient): Promise<TraceCompleteParams> {
  return new Promise((resolve) => {
    const unsubscribe = cdpClient.subscribeEvents((event) => {
      if (event.method !== "Tracing.tracingComplete") return;
      unsubscribe();
      resolve(event.params as TraceCompleteParams);
    });
  });
}

async function readLatestIncident(cdpClient: CdpClient): Promise<Record<string, unknown> | null> {
  try {
    return await cdpClient.evaluate<Record<string, unknown> | null>(
      "window.__LOCUS_RUNTIME_PERFORMANCE_INCIDENT__ ?? null",
    );
  } catch {
    return null;
  }
}

async function readPerformanceMetrics(cdpClient: CdpClient): Promise<Record<string, number>> {
  try {
    await cdpClient.send("Performance.enable");
    const response = await cdpClient.send("Performance.getMetrics") as PerformanceMetricsResponse;
    return Object.fromEntries((response.metrics ?? []).map(({ name, value }) => [name, value]));
  } catch {
    return {};
  }
}

function diffPerformanceMetrics(
  before: Record<string, number>,
  after: Record<string, number>,
): Record<string, number> {
  const delta = (name: string) => (after[name] ?? 0) - (before[name] ?? 0);
  const milliseconds = (name: string) => Math.round(delta(name) * 100_000) / 100;
  const count = (name: string) => Math.round(delta(name));
  return {
    taskDurationMs: milliseconds("TaskDuration"),
    scriptDurationMs: milliseconds("ScriptDuration"),
    layoutDurationMs: milliseconds("LayoutDuration"),
    recalcStyleDurationMs: milliseconds("RecalcStyleDuration"),
    layoutCount: count("LayoutCount"),
    recalcStyleCount: count("RecalcStyleCount"),
    jsHeapUsedDeltaBytes: Math.round(delta("JSHeapUsedSize")),
    nodesDelta: count("Nodes"),
    documentsDelta: count("Documents"),
    eventListenersDelta: count("JSEventListeners"),
  };
}

async function saveTraceStream(
  cdpClient: CdpClient,
  streamHandle: string,
  outputDir: string,
): Promise<{
  tracePath: string;
  manifestPath: string;
  compressedBytes: number;
  uncompressedBytes: number;
}> {
  mkdirSync(outputDir, { recursive: true });
  const captureName = new Date().toISOString().replace(/[:.]/g, "-");
  const tracePath = path.join(outputDir, `locus-stall-${captureName}.trace.json.gz`);
  const manifestPath = path.join(outputDir, `locus-stall-${captureName}.json`);
  const output = createWriteStream(tracePath);
  const gzip = createGzip({ level: 6 });
  gzip.pipe(output);
  let uncompressedBytes = 0;

  try {
    for (;;) {
      const chunk = await cdpClient.send("IO.read", {
        handle: streamHandle,
        size: STREAM_CHUNK_BYTES,
      }) as { data?: string; base64Encoded?: boolean; eof?: boolean };
      const data = chunk.base64Encoded
        ? Buffer.from(chunk.data ?? "", "base64")
        : Buffer.from(chunk.data ?? "", "utf8");
      uncompressedBytes += data.byteLength;
      if (data.byteLength > 0 && !gzip.write(data)) await once(gzip, "drain");
      if (chunk.eof) break;
    }
  } finally {
    try {
      await cdpClient.send("IO.close", { handle: streamHandle });
    } catch {
      // The stream is already exhausted or the runtime has closed it.
    }
    gzip.end();
  }
  await once(output, "finish");
  const compressedBytes = Number(output.bytesWritten);
  return { tracePath, manifestPath, compressedBytes, uncompressedBytes };
}
