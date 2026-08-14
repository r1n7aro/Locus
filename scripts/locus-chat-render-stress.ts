import process from "node:process";
import { createChatRenderStressFixture } from "./locus-chat-render-stress-fixture";

const DEBUG_PORT_START = 19222;
const DEBUG_PORT_ATTEMPTS = 25;
const DEFAULT_DURATION_MS = 5 * 60_000;
const DEFAULT_DELTA_MS = 90;
const DEFAULT_TOOL_DELAY_MS = 180;
const DEFAULT_TIMEOUT_MS = 20_000;
const DEFAULT_VERIFY_MS = 12_000;
const POST_END_CAPTURE_MS = 1_200;
const DEFAULT_VIEWPORT_WIDTH = 1600;
const DEFAULT_VIEWPORT_HEIGHT = 900;

interface CliOptions {
  browserUrl: string;
  durationMs: number;
  deltaMs: number;
  toolDelayMs: number;
  timeoutMs: number;
  textPattern: "mixed" | "steady";
  stop: boolean;
  status: boolean;
  verify: boolean;
  verifyMs: number;
  viewportWidth: number;
  viewportHeight: number;
}

interface DevtoolsTarget {
  id: string;
  type: string;
  url: string;
  title?: string;
  webSocketDebuggerUrl?: string;
}

interface CdpMessage {
  id?: number;
  result?: unknown;
  error?: { message?: string };
}

const options = parseArgs(process.argv.slice(2));
const target = await findLocusTarget(options.browserUrl, options.timeoutMs);
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl!);

try {
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: options.viewportWidth,
    height: options.viewportHeight,
    deviceScaleFactor: 1,
    mobile: false,
  });

  if (options.stop) {
    const result = await cdp.evaluate(stopExpression("cli"));
    printResult({ action: "stop", browserUrl: target.url, ...result });
  } else if (options.status) {
    const result = await cdp.evaluate(statusExpression());
    printResult({ action: "status", browserUrl: target.url, ...result });
  } else if (options.verify) {
    const started = await cdp.evaluate(startExpression(options));
    await sleep(options.verifyMs);
    const stopped = await cdp.evaluate(stopExpression("verify"));
    const status = await cdp.evaluate(statusExpression());
    const verification = verifyStability(started, status);
    printResult({
      action: "verify",
      browserUrl: target.url,
      started,
      stopped,
      status,
      verification,
    });
    if (!verification.passed) process.exitCode = 1;
  } else {
    const result = await cdp.evaluate(startExpression(options));
    printResult({ action: "start", browserUrl: target.url, ...result });
  }
} finally {
  cdp.close();
}

function parseArgs(args: string[]): CliOptions {
  const parsed: CliOptions = {
    browserUrl: "",
    durationMs: DEFAULT_DURATION_MS,
    deltaMs: DEFAULT_DELTA_MS,
    toolDelayMs: DEFAULT_TOOL_DELAY_MS,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    textPattern: "mixed",
    stop: false,
    status: false,
    verify: false,
    verifyMs: DEFAULT_VERIFY_MS,
    viewportWidth: DEFAULT_VIEWPORT_WIDTH,
    viewportHeight: DEFAULT_VIEWPORT_HEIGHT,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    }
    if (arg === "--stop") {
      parsed.stop = true;
      continue;
    }
    if (arg === "--status") {
      parsed.status = true;
      continue;
    }
    if (arg === "--verify") {
      parsed.verify = true;
      continue;
    }

    const [name, inlineValue = ""] = arg.split(/=(.*)/s, 2);
    const value = inlineValue || args[index + 1];
    if (name === "--browser-url") {
      parsed.browserUrl = requireValue(name, value);
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--duration-ms") {
      parsed.durationMs = positiveInteger(name, value, 1_000);
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--delta-ms") {
      parsed.deltaMs = positiveInteger(name, value, 16);
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--tool-delay-ms") {
      parsed.toolDelayMs = positiveInteger(name, value, 0);
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--timeout-ms") {
      parsed.timeoutMs = positiveInteger(name, value, 1_000);
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--verify-ms") {
      parsed.verifyMs = positiveInteger(name, value, 2_000);
      parsed.verify = true;
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--viewport-width") {
      parsed.viewportWidth = positiveInteger(name, value, 800);
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--viewport-height") {
      parsed.viewportHeight = positiveInteger(name, value, 600);
      if (!inlineValue) index += 1;
      continue;
    }
    if (name === "--text-pattern") {
      const pattern = requireValue(name, value);
      if (pattern !== "mixed" && pattern !== "steady") {
        throw new Error("--text-pattern must be mixed or steady.");
      }
      parsed.textPattern = pattern;
      if (!inlineValue) index += 1;
      continue;
    }
    throw new Error(`Unknown option: ${arg}`);
  }

  if ([parsed.stop, parsed.status, parsed.verify].filter(Boolean).length > 1) {
    throw new Error("--stop, --status, and --verify cannot be used together.");
  }
  return parsed;
}

function requireValue(name: string, value: string | undefined): string {
  if (!value || value.startsWith("--")) {
    throw new Error(`${name} requires a value.`);
  }
  return value;
}

function positiveInteger(name: string, value: string | undefined, minimum: number): number {
  const parsed = Number(requireValue(name, value));
  if (!Number.isInteger(parsed) || parsed < minimum) {
    throw new Error(`${name} must be an integer >= ${minimum}.`);
  }
  return parsed;
}

function printHelp() {
  console.log(`Usage:
  bun run locus:test:chat-render
  bun run locus:test:chat-render -- --verify
  bun run locus:test:chat-render -- --status
  bun run locus:test:chat-render -- --stop

Prerequisite:
  Start a dev instance with WebView2 DevTools and open an existing Chat session:
  bun run locus:test:app -- --skip-onboarding

Options:
  --browser-url <url>     WebView2 DevTools URL; auto-detected from ports 19222-19246
  --duration-ms <ms>      Synthetic response duration, default ${DEFAULT_DURATION_MS}
  --delta-ms <ms>         Text delta cadence, default ${DEFAULT_DELTA_MS}
  --tool-delay-ms <ms>    Delay between synthetic tool events, default ${DEFAULT_TOOL_DELAY_MS}
  --text-pattern <name>   mixed alternates Markdown block heights; steady emits uniform list rows
  --timeout-ms <ms>       DevTools/DOM wait timeout, default ${DEFAULT_TIMEOUT_MS}
  --verify                Run, sample, stop, and assert streaming layout stability unattended
  --verify-ms <ms>        Streaming sample time in verify mode, default ${DEFAULT_VERIFY_MS}
  --viewport-width <px>   Fixed CDP layout viewport width, default ${DEFAULT_VIEWPORT_WIDTH}
  --viewport-height <px>  Fixed CDP layout viewport height, default ${DEFAULT_VIEWPORT_HEIGHT}
  --status                Print the current scenario state
  --stop                  Finish the current synthetic response`);
}

async function findLocusTarget(requestedBrowserUrl: string, timeoutMs: number) {
  const deadline = Date.now() + timeoutMs;
  const browserUrls = requestedBrowserUrl
    ? [normalizeBrowserUrl(requestedBrowserUrl)]
    : Array.from(
        { length: DEBUG_PORT_ATTEMPTS },
        (_, offset) => `http://127.0.0.1:${DEBUG_PORT_START + offset}`,
      );

  while (Date.now() < deadline) {
    for (const browserUrl of browserUrls) {
      const targets = await readTargets(browserUrl);
      const target = targets.find((item) => (
        item.type === "page"
        && !!item.webSocketDebuggerUrl
        && /^http:\/\/localhost:\d+\/$/.test(item.url)
      ));
      if (target) return target;
    }
    await sleep(250);
  }

  throw new Error(
    requestedBrowserUrl
      ? `No Locus page found at ${requestedBrowserUrl}.`
      : "No Locus WebView2 page found on ports 19222-19246. Start bun run locus:test:app first.",
  );
}

function normalizeBrowserUrl(value: string): string {
  return value.replace(/\/$/, "");
}

async function readTargets(browserUrl: string): Promise<DevtoolsTarget[]> {
  try {
    const response = await fetch(`${browserUrl}/json/list`, {
      signal: AbortSignal.timeout(350),
    });
    if (!response.ok) return [];
    return await response.json() as DevtoolsTarget[];
  } catch {
    return [];
  }
}

function sleep(ms: number) {
  return new Promise<void>((resolve) => setTimeout(resolve, ms));
}

function printResult(result: Record<string, unknown>) {
  console.log(`LOCUS_CHAT_RENDER_STRESS_JSON ${JSON.stringify(result)}`);
}

class CdpClient {
  private nextId = 1;
  private pending = new Map<number, {
    resolve: (value: unknown) => void;
    reject: (reason: Error) => void;
  }>();

  private constructor(private readonly socket: WebSocket) {
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data)) as CdpMessage;
      if (!message.id) return;
      const request = this.pending.get(message.id);
      if (!request) return;
      this.pending.delete(message.id);
      if (message.error) {
        request.reject(new Error(message.error.message || "CDP request failed"));
      } else {
        request.resolve(message.result);
      }
    });
    socket.addEventListener("close", () => {
      for (const request of this.pending.values()) {
        request.reject(new Error("WebView2 DevTools connection closed."));
      }
      this.pending.clear();
    });
  }

  static connect(url: string): Promise<CdpClient> {
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(url);
      const onError = () => reject(new Error(`Failed to connect to ${url}`));
      socket.addEventListener("error", onError, { once: true });
      socket.addEventListener("open", () => {
        socket.removeEventListener("error", onError);
        resolve(new CdpClient(socket));
      }, { once: true });
    });
  }

  send(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression: string): Promise<Record<string, unknown>> {
    const response = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    }) as {
      result?: { value?: Record<string, unknown>; description?: string };
      exceptionDetails?: { text?: string; exception?: { description?: string } };
    };
    if (response.exceptionDetails) {
      throw new Error(
        response.exceptionDetails.exception?.description
        || response.exceptionDetails.text
        || "Browser evaluation failed.",
      );
    }
    return response.result?.value ?? {};
  }

  close() {
    this.socket.close();
  }
}

function statusExpression() {
  return `(() => {
    const state = window.__locusChatRenderStress;
    if (!state) return { running: false, reason: "not-started" };
    const latest = state.samples?.[state.samples.length - 1] ?? null;
    const allSamples = state.samples ?? [];
    const steadyStateCutoff = (allSamples[0]?.t ?? 0) + 500;
    const samples = allSamples.filter((sample) => sample.t >= steadyStateCutoff);
    const respondingSamples = samples.filter((sample) => sample.responding);
    const postEndSamples = samples.filter((sample) => !sample.responding && state.stoppedAt !== null
      && sample.t >= state.stoppedAt);
    const lastRespondingSample = respondingSamples[respondingSamples.length - 1] ?? null;
    const firstPostEndSample = postEndSamples[0] ?? null;
    let maxScrollDelta = 0;
    let maxToolTopDelta = 0;
    let toolTopDirectionChanges = 0;
    let maxTextHeightDelta = 0;
    let textHeightShrinkCount = 0;
    let maxBottomDistance = 0;
    let bottomDetachedFrameCount = 0;
    let scrollHeightShrinkCount = 0;
    let maxTextHeightTransition = null;
    let maxEndScrollDelta = 0;
    let maxEndToolTopDelta = 0;
    let maxEndTextHeightDelta = 0;
    let maxEndBottomDistance = 0;
    let textScopeTransitionCount = 0;
    const tailTagStats = {};
    let previousDirection = 0;
    for (let index = 1; index < samples.length; index += 1) {
      const previous = samples[index - 1];
      const current = samples[index];
      const bothResponding = previous.responding && current.responding;
      const inEndWindow = state.stoppedAt !== null && current.t >= state.stoppedAt;
      if (bothResponding && typeof previous.scrollTop === 'number' && typeof current.scrollTop === 'number') {
        maxScrollDelta = Math.max(maxScrollDelta, Math.abs(current.scrollTop - previous.scrollTop));
      }
      if (current.responding && typeof current.bottomDistance === 'number') {
        maxBottomDistance = Math.max(maxBottomDistance, current.bottomDistance);
        if (current.bottomDistance > 1) bottomDetachedFrameCount += 1;
      }
      if (bothResponding && typeof previous.scrollHeight === 'number' && typeof current.scrollHeight === 'number'
        && current.scrollHeight < previous.scrollHeight - 0.5) {
        scrollHeightShrinkCount += 1;
      }
      if (bothResponding && typeof previous.toolTop === 'number' && typeof current.toolTop === 'number') {
        const delta = current.toolTop - previous.toolTop;
        maxToolTopDelta = Math.max(maxToolTopDelta, Math.abs(delta));
        const direction = Math.abs(delta) < 0.5 ? 0 : Math.sign(delta);
        if (direction && previousDirection && direction !== previousDirection) toolTopDirectionChanges += 1;
        if (direction) previousDirection = direction;
      }
      if (bothResponding && typeof previous.textHeight === 'number' && typeof current.textHeight === 'number') {
        const delta = current.textHeight - previous.textHeight;
        if (Math.abs(delta) > maxTextHeightDelta) {
          maxTextHeightDelta = Math.abs(delta);
          maxTextHeightTransition = {
            delta,
            fromTag: previous.tailTag,
            toTag: current.tailTag,
            fromLineHeight: previous.tailLineHeight,
            toLineHeight: current.tailLineHeight,
          };
        }
        if (delta < -0.5) textHeightShrinkCount += 1;
      }
      if (current.responding && current.tailTag) {
        const key = current.tailTag + ':' + (current.tailLineHeight || 'unknown');
        tailTagStats[key] = (tailTagStats[key] || 0) + 1;
      }
      if (inEndWindow) {
        if (typeof previous.scrollTop === 'number' && typeof current.scrollTop === 'number') {
          maxEndScrollDelta = Math.max(maxEndScrollDelta, Math.abs(current.scrollTop - previous.scrollTop));
        }
        if (typeof previous.toolTop === 'number' && typeof current.toolTop === 'number') {
          maxEndToolTopDelta = Math.max(maxEndToolTopDelta, Math.abs(current.toolTop - previous.toolTop));
        }
        if (typeof previous.textHeight === 'number' && typeof current.textHeight === 'number') {
          maxEndTextHeightDelta = Math.max(maxEndTextHeightDelta, Math.abs(current.textHeight - previous.textHeight));
        }
        if (typeof current.bottomDistance === 'number') {
          maxEndBottomDistance = Math.max(maxEndBottomDistance, current.bottomDistance);
        }
        if (previous.textScope && current.textScope && previous.textScope !== current.textScope) {
          textScopeTransitionCount += 1;
        }
      }
    }
    return {
      running: state.running === true,
      runId: state.runId,
      sessionId: state.sessionId,
      elapsedMs: Math.round(performance.now() - state.startedAt),
      emittedSegments: state.emittedSegments,
      expandedToolId: state.expandedToolId,
      collectionReopenCount: state.collectionReopenCount ?? 0,
      toolReopenCount: state.toolReopenCount ?? 0,
      sampleCount: state.samples?.length ?? 0,
      analyzedSampleCount: samples.length,
      collapsedFrameCount: respondingSamples.filter((sample) => !sample.collectionExpanded || !sample.toolExpanded).length,
      respondingFrameCount: respondingSamples.length,
      postEndSampleCount: postEndSamples.length,
      historyFrameCount: postEndSamples.filter((sample) => sample.textScope === 'history').length,
      postEndStateTrace: postEndSamples.slice(0, 8).map((sample) => ({
        t: sample.t,
        activeSessionId: sample.activeSessionId,
        messageCount: sample.messageCount,
        historyTextCount: sample.historyTextCount,
        textScope: sample.textScope,
        scrollHeight: sample.scrollHeight,
        scrollTop: sample.scrollTop,
      })),
      endTransition: lastRespondingSample && firstPostEndSample ? {
        before: {
          messageHeight: lastRespondingSample.messageHeight,
          stackHeight: lastRespondingSample.stackHeight,
          stackGap: lastRespondingSample.stackGap,
          textTop: lastRespondingSample.textTop,
          textHeight: lastRespondingSample.textHeight,
          collectionTop: lastRespondingSample.collectionTop,
          collectionHeight: lastRespondingSample.collectionHeight,
          messageLayouts: lastRespondingSample.messageLayouts,
          scrollHeight: lastRespondingSample.scrollHeight,
          scrollTop: lastRespondingSample.scrollTop,
        },
        after: {
          messageHeight: firstPostEndSample.messageHeight,
          stackHeight: firstPostEndSample.stackHeight,
          stackGap: firstPostEndSample.stackGap,
          textTop: firstPostEndSample.textTop,
          textHeight: firstPostEndSample.textHeight,
          collectionTop: firstPostEndSample.collectionTop,
          collectionHeight: firstPostEndSample.collectionHeight,
          messageLayouts: firstPostEndSample.messageLayouts,
          scrollHeight: firstPostEndSample.scrollHeight,
          scrollTop: firstPostEndSample.scrollTop,
        },
      } : null,
      maxScrollDelta,
      maxToolTopDelta,
      toolTopDirectionChanges,
      maxTextHeightDelta,
      maxTextHeightTransition,
      textHeightShrinkCount,
      maxBottomDistance,
      bottomDetachedFrameCount,
      scrollHeightShrinkCount,
      maxEndScrollDelta,
      maxEndToolTopDelta,
      maxEndTextHeightDelta,
      maxEndBottomDistance,
      textScopeTransitionCount,
      captureComplete: state.captureComplete === true,
      endBoundaryProbeFound: state.endBoundaryProbeFound === true,
      endBoundaryHeightDelta: state.endBoundaryHeightDelta ?? 0,
      cursorLayoutWidth: state.cursorLayoutWidth ?? null,
      tailTagStats,
      latest,
    };
  })()`;
}

function stopExpression(reason: "cli" | "verify") {
  return `(async () => {
    const state = window.__locusChatRenderStress;
    if (!state?.stop) return { stopped: false, reason: "not-running" };
    await state.stop(${JSON.stringify(reason)});
    return {
      stopped: true,
      runId: state.runId,
      captureComplete: state.captureComplete === true,
      endBoundaryProbeFound: state.endBoundaryProbeFound === true,
      endBoundaryHeightDelta: state.endBoundaryHeightDelta ?? 0,
      cursorLayoutWidth: state.cursorLayoutWidth ?? null,
    };
  })()`;
}

function verifyStability(
  started: Record<string, unknown>,
  status: Record<string, unknown>,
) {
  const tailTagStats = status.tailTagStats as Record<string, number> | undefined;
  const checks = {
    sixToolsCreated: started.toolCallCount === 6,
    longToolExpanded: started.expandedToolName === "bash"
      && started.expandedToolOutputLines === 160,
    respondingWhileExpanded: started.respondingWhileExpanded === true,
    sustainedStreamingSamples: Number(status.respondingFrameCount) >= 120,
    expansionStayedOpen: Number(status.collapsedFrameCount) === 0,
    mixedMarkdownLayouts: Object.keys(tailTagStats ?? {}).length >= 5,
    streamingBottomStable: Number(status.maxBottomDistance) <= 0.5
      && Number(status.bottomDetachedFrameCount) === 0,
    streamingHeightMonotonic: Number(status.textHeightShrinkCount) === 0
      && Number(status.scrollHeightShrinkCount) === 0,
    cursorLayoutNeutral: Number(status.cursorLayoutWidth) <= 0.5
      && status.endBoundaryProbeFound === false
      && Number(status.endBoundaryHeightDelta) <= 0.5,
    endHandoffCaptured: Number(status.postEndSampleCount) >= 10
      && Number(status.historyFrameCount) >= 1,
    endTextHeightStable: Number(status.maxEndTextHeightDelta) <= 0.5,
    endViewportStable: Number(status.maxEndBottomDistance) <= 0.5
      && Number(status.maxEndScrollDelta) <= 0.5
      && Number(status.maxEndToolTopDelta) <= 0.5,
    expansionPreservedAfterEnd: (status.latest as Record<string, unknown> | null)?.collectionExpanded === true
      && (status.latest as Record<string, unknown> | null)?.toolExpanded === true,
  };
  return {
    passed: Object.values(checks).every(Boolean),
    checks,
  };
}

function startExpression(options: CliOptions) {
  const config = JSON.stringify({
    durationMs: options.durationMs,
    deltaMs: options.deltaMs,
    toolDelayMs: options.toolDelayMs,
    timeoutMs: options.timeoutMs,
    textPattern: options.textPattern,
    postEndCaptureMs: POST_END_CAPTURE_MS,
  });
  const fixture = JSON.stringify(createChatRenderStressFixture());

  return `(async () => {
    const config = ${config};
    const fixture = ${fixture};
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const waitFor = async (read, label) => {
      const deadline = performance.now() + config.timeoutMs;
      while (performance.now() < deadline) {
        const value = read();
        if (value) return value;
        await sleep(40);
      }
      throw new Error('Timed out waiting for ' + label);
    };

    const previous = window.__locusChatRenderStress;
    if (previous?.running && previous.stop) {
      await previous.stop('replaced');
      await sleep(100);
    }

    const [{ useChatStore }, { createSession }] = await Promise.all([
      import('/src/stores/chat.ts'),
      import('/src/services/session.ts'),
    ]);
    const store = useChatStore();
    if (store.isStreaming) {
      throw new Error('The active Chat session is already responding. Stop it before starting the synthetic stream.');
    }
    const sessionId = await createSession({
      title: 'TS Render Stress ' + new Date().toLocaleTimeString(),
      sessionType: 'chat',
    });
    await store.refreshSessions();
    await store.selectSession(sessionId, { persist: false });

    const stamp = Date.now().toString(36);
    const runId = 'ts-render-stress-' + stamp;
    const userMessageId = runId + '-user';
    const assistantMessageId = runId + '-assistant';
    const textPartId = runId + '-text';
    const toolSpecs = fixture.toolSpecs;
    const toolCalls = toolSpecs.map((tool, index) => ({
      id: runId + '-tool-' + (index + 1),
      name: tool.name,
      arguments: JSON.stringify(tool.arguments),
      order: index + 1,
      outcome: 'done',
      recordedOutput: tool.output,
    }));
    const longToolId = toolCalls[toolCalls.length - 1].id;
    const state = {
      running: true,
      runId,
      sessionId,
      startedAt: performance.now(),
      emittedSegments: 0,
      expandedToolId: null,
      samples: [],
      intervalId: 0,
      expandIntervalId: 0,
      timeoutId: 0,
      frameId: 0,
      fullText: '',
      stop: null,
      finishing: false,
      stoppedAt: null,
      captureUntil: 0,
      captureComplete: false,
      endBoundaryProbeFound: false,
      endBoundaryHeightDelta: 0,
      endBoundaryProbeChars: 0,
      cursorLayoutWidth: null,
      cleanupComplete: false,
    };
    window.__locusChatRenderStress = state;

    const emit = (event) => {
      const handled = store.handleStreamEvent({ runId, sessionId, ...event });
      if (!handled) throw new Error('Chat store rejected synthetic event: ' + event.type);
    };
    const waitForTwoFrames = () => new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(resolve));
    });
    const measureCursorHeightDelta = () => {
      const cursor = [...document.querySelectorAll(
        '.chat-transcript-message.assistant.transient .streaming-cursor'
      )].at(-1);
      const surface = cursor?.closest('.streaming-markdown') ?? cursor?.closest('.markdown-body');
      if (!cursor || !surface) return 0;
      const withCursor = surface.getBoundingClientRect().height;
      const previousDisplay = cursor.style.display;
      cursor.style.display = 'none';
      const withoutCursor = surface.getBoundingClientRect().height;
      cursor.style.display = previousDisplay;
      return withCursor - withoutCursor;
    };
    const measureCursorLayoutWidth = () => {
      const cursor = [...document.querySelectorAll(
        '.chat-transcript-message.assistant.transient .streaming-cursor'
      )].at(-1);
      return cursor?.getBoundingClientRect().width ?? null;
    };
    const prepareEndBoundary = async () => {
      const remainingPieces = (fixture.mixedPieces.length
        - (state.emittedSegments % fixture.mixedPieces.length)) % fixture.mixedPieces.length;
      for (let offset = 0; offset < remainingPieces; offset += 1) {
        state.emittedSegments += 1;
        const text = fixture.mixedPieces[(state.emittedSegments - 1) % fixture.mixedPieces.length];
        state.fullText += text;
        emit({ type: 'textDelta', text, order: 100, partId: textPartId, renderSeq: 100 });
      }
      if (remainingPieces > 0) {
        await sleep(320);
        await waitForTwoFrames();
      }
      const prefix = '\\n\\n响应结束布局边界：';
      state.fullText += prefix;
      emit({ type: 'textDelta', text: prefix, order: 100, partId: textPartId, renderSeq: 100 });
      for (let index = 0; index < 128; index += 1) {
        if (index > 0) {
          state.fullText += '测';
          emit({ type: 'textDelta', text: '测', order: 100, partId: textPartId, renderSeq: 100 });
        }
        await sleep(Math.max(100, config.deltaMs + 20));
        await waitForTwoFrames();
        const cursorLayoutWidth = measureCursorLayoutWidth();
        state.cursorLayoutWidth = cursorLayoutWidth;
        if (typeof cursorLayoutWidth === 'number' && cursorLayoutWidth <= 0.5) {
          return;
        }
        const heightDelta = measureCursorHeightDelta();
        if (heightDelta > 0.5) {
          state.endBoundaryProbeFound = true;
          state.endBoundaryHeightDelta = heightDelta;
          state.endBoundaryProbeChars = index;
          return;
        }
      }
    };
    const finishFrontendResponse = () => {
      const existingAssistant = store.messages.find((message) => message.id === assistantMessageId);
      const finalAssistant = {
        ...existingAssistant,
        id: assistantMessageId,
        role: 'assistant',
        content: state.fullText,
        createdAt: existingAssistant?.createdAt ?? Date.now() / 1000,
        thinkingContent: existingAssistant?.thinkingContent
          ?? '正在合成多轮工具调用与长时间模型响应。',
        thinkingDuration: existingAssistant?.thinkingDuration ?? 1,
        thinkingOrder: existingAssistant?.thinkingOrder ?? 1,
        contentOrder: 100,
        toolCalls,
        renderParts: undefined,
      };
      const existingIndex = store.messages.findIndex((message) => message.id === assistantMessageId);
      const finalMessages = existingIndex >= 0
        ? store.messages.map((message, index) => index === existingIndex ? finalAssistant : message)
        : [...store.messages, finalAssistant];
      store.$patch({
        messages: finalMessages,
        streamingText: '',
        rawStreamText: '',
        streamingThinking: '',
        liveRenderParts: [],
        activeToolCalls: [],
        isStreaming: false,
        isThinking: false,
        currentRunId: null,
      });
    };
    let finished = false;
    const finish = async (reason) => {
      if (finished) return;
      finished = true;
      state.finishing = true;
      clearInterval(state.intervalId);
      clearTimeout(state.timeoutId);
      if (reason === 'verify') await prepareEndBoundary();
      clearInterval(state.expandIntervalId);
      if (store.currentRunId === runId) finishFrontendResponse();
      state.stoppedAt = Math.round(performance.now() - state.startedAt);
      state.captureUntil = performance.now() + config.postEndCaptureMs;
      state.running = false;
      state.stopReason = reason;
      await sleep(config.postEndCaptureMs + 80);
      cancelAnimationFrame(state.frameId);
      state.frameId = 0;
      state.captureComplete = true;
      state.finishing = false;
      if (reason === 'verify') {
        try {
          await store.deleteSession(sessionId);
          state.cleanupComplete = true;
        } catch (error) {
          state.cleanupError = String(error);
        }
      }
    };
    state.stop = finish;

    emit({ type: 'runStart' });
    emit({
      type: 'userMessage',
      message: {
        id: userMessageId,
        role: 'user',
        content: 'TS 自动复现：模型正在响应时展开长工具调用',
        createdAt: Date.now() / 1000,
      },
    });
    emit({
      type: 'thinkingDelta',
      text: '正在合成多轮工具调用与长时间模型响应。',
      order: 1,
      partId: runId + '-thinking',
      renderSeq: 1,
    });

    for (let index = 0; index < toolCalls.length; index += 1) {
      const toolCall = toolCalls[index];
      const spec = toolSpecs[index];
      emit({
        type: 'toolCallStart',
        toolCallId: toolCall.id,
        toolName: toolCall.name,
        arguments: toolCall.arguments,
        order: index + 2,
        partId: toolCall.id,
        renderSeq: index + 2,
      });
      if (toolCall.id === longToolId) {
        for (const chunk of spec.output.match(/.{1,480}(?:\\n|$)/gs) ?? []) {
          emit({ type: 'toolCallDelta', toolCallId: toolCall.id, delta: chunk });
          await sleep(Math.max(16, Math.floor(config.toolDelayMs / 3)));
        }
      } else {
        await sleep(config.toolDelayMs);
      }
      emit({
        type: 'toolCallDone',
        toolCallId: toolCall.id,
        toolName: toolCall.name,
        output: spec.output,
        outcome: 'done',
      });
    }

    emit({
      type: 'toolCallRoundDone',
      messageId: assistantMessageId,
      fullText: '',
      toolCalls,
    });
    const intro = '工具调用已完成。模型将保持长时间响应，脚本会自动展开最后一个长工具块。\\n\\n';
    state.fullText = intro;
    emit({
      type: 'textDelta',
      text: intro,
      order: 100,
      partId: textPartId,
      renderSeq: 100,
    });

    const mixedPieces = fixture.mixedPieces;
    const emitNextSegment = () => {
      state.emittedSegments += 1;
      const text = config.textPattern === 'mixed'
        ? mixedPieces[(state.emittedSegments - 1) % mixedPieces.length]
        : '- 流式片段 ' + String(state.emittedSegments).padStart(4, '0')
          + '：持续改变回复高度，用于观察滚动锚点与工具详情布局。\\n';
      state.fullText += text;
      emit({
        type: 'textDelta',
        text,
        order: 100,
        partId: textPartId,
        renderSeq: 100,
      });
    };
    const startTextInterval = () => {
      clearInterval(state.intervalId);
      state.intervalId = setInterval(emitNextSegment, config.deltaMs);
    };
    startTextInterval();
    state.timeoutId = setTimeout(() => void finish('duration'), config.durationMs);

    await sleep(650);
    clearInterval(state.intervalId);
    state.intervalId = 0;
    const findCollection = () => [...document.querySelectorAll('.tool-call-collection')]
      .find((element) => (element.getAttribute('data-tool-layout-tool-call-ids') || '').includes(longToolId));
    const findLongBlock = () => {
      const collection = findCollection();
      return collection
        ? [...collection.querySelectorAll('.tool-call-block')]
          .find((element) => (element.getAttribute('data-tool-layout-tool-call-ids') || '').includes(longToolId))
        : null;
    };
    const ensureExpanded = () => {
      const collection = findCollection();
      if (!collection) return false;
      if (collection.getAttribute('data-tool-layout-expanded') !== 'true') {
        collection.querySelector('.tool-call-batch-summary')?.click();
        state.collectionReopenCount = (state.collectionReopenCount || 0) + 1;
        return false;
      }
      const longBlock = findLongBlock();
      if (!longBlock) return false;
      if (!longBlock.classList.contains('is-expanded')) {
        longBlock.querySelector('.tool-call-header')?.click();
        state.toolReopenCount = (state.toolReopenCount || 0) + 1;
        return false;
      }
      return true;
    };
    state.expandIntervalId = setInterval(ensureExpanded, 80);
    await waitFor(() => ensureExpanded(), 'automatically expanded long tool block');
    state.expandedToolId = longToolId;
    const baselineScrollElement = findLongBlock()?.closest('.chat-transcript-scroll')
      ?? document.querySelector('.chat-transcript-scroll');
    if (baselineScrollElement) {
      store.rememberSessionScrollState(sessionId, { mode: 'bottom' });
      baselineScrollElement.scrollTop = baselineScrollElement.scrollHeight;
      await sleep(120);
    }
    startTextInterval();

    const sampleFrame = () => {
      const longBlock = findLongBlock();
      const scrollElement = longBlock?.closest('.chat-transcript-scroll') ?? document.querySelector('.chat-transcript-scroll');
      const scrollRect = scrollElement?.getBoundingClientRect();
      const toolRect = longBlock?.getBoundingClientRect();
      const transientMessage = [...document.querySelectorAll('.chat-transcript-message.assistant.transient')].at(-1);
      const streamingMarkdown = transientMessage?.querySelector('.streaming-markdown');
      const historyMarkdown = [...document.querySelectorAll(
        '[data-render-part-kind="text"][data-render-part-scope="history"]'
      )].at(-1);
      const textSurface = streamingMarkdown ?? historyMarkdown;
      const tailElement = streamingMarkdown
        ? streamingMarkdown.querySelector('.streaming-markdown-block:last-child > :last-child')
        : historyMarkdown?.querySelector(':scope > :last-child');
      const textRect = textSurface?.getBoundingClientRect();
      const tailRect = tailElement?.getBoundingClientRect();
      const messageSurface = streamingMarkdown
        ? transientMessage
        : historyMarkdown?.closest('.chat-transcript-message');
      const stack = textSurface?.closest('.chat-transcript-item-stack');
      const messageRect = messageSurface?.getBoundingClientRect();
      const stackRect = stack?.getBoundingClientRect();
      const collectionRect = findCollection()?.getBoundingClientRect();
      const messageLayouts = [...document.querySelectorAll('.chat-transcript-message.is-session')]
        .slice(-3)
        .map((message) => {
          const rect = message.getBoundingClientRect();
          const style = getComputedStyle(message);
          const role = message.querySelector(':scope > .chat-transcript-message-role');
          const content = message.querySelector(':scope > .chat-transcript-message-content');
          return {
            className: message.className,
            top: rect.top,
            height: rect.height,
            paddingTop: style.paddingTop,
            paddingBottom: style.paddingBottom,
            roleHeight: role?.getBoundingClientRect().height ?? null,
            contentHeight: content?.getBoundingClientRect().height ?? null,
          };
        });
      state.samples.push({
        t: Math.round(performance.now() - state.startedAt),
        responding: store.isStreaming === true && store.currentRunId === runId,
        activeSessionId: store.activeSessionId,
        messageCount: store.messages.length,
        historyTextCount: document.querySelectorAll(
          '[data-render-part-kind="text"][data-render-part-scope="history"]'
        ).length,
        scrollTop: scrollElement?.scrollTop ?? null,
        scrollHeight: scrollElement?.scrollHeight ?? null,
        clientHeight: scrollElement?.clientHeight ?? null,
        bottomDistance: scrollElement
          ? scrollElement.scrollHeight - scrollElement.scrollTop - scrollElement.clientHeight
          : null,
        viewportTop: scrollRect?.top ?? null,
        viewportBottom: scrollRect?.bottom ?? null,
        collectionExpanded: findCollection()?.getAttribute('data-tool-layout-expanded') === 'true',
        toolExpanded: longBlock?.classList.contains('is-expanded') === true,
        collectionReopenCount: state.collectionReopenCount || 0,
        toolReopenCount: state.toolReopenCount || 0,
        toolTop: toolRect?.top ?? null,
        toolHeight: toolRect?.height ?? null,
        textHeight: textRect?.height ?? null,
        textTop: textRect?.top ?? null,
        textScope: streamingMarkdown ? 'transient' : historyMarkdown ? 'history' : null,
        messageHeight: messageRect?.height ?? null,
        stackHeight: stackRect?.height ?? null,
        stackGap: stack ? getComputedStyle(stack).gap : null,
        collectionTop: collectionRect?.top ?? null,
        collectionHeight: collectionRect?.height ?? null,
        messageLayouts,
        tailHeight: tailRect?.height ?? null,
        tailTag: tailElement?.tagName ?? null,
        tailLineHeight: tailElement ? getComputedStyle(tailElement).lineHeight : null,
      });
      if (state.samples.length > 6000) state.samples.splice(0, 1000);
      if (state.running || performance.now() <= state.captureUntil) {
        state.frameId = requestAnimationFrame(sampleFrame);
      }
    };
    state.frameId = requestAnimationFrame(sampleFrame);

    return {
      running: state.running,
      runId,
      sessionId,
      durationMs: config.durationMs,
      deltaMs: config.deltaMs,
      textPattern: config.textPattern,
      toolCallCount: toolCalls.length,
      expandedToolId: state.expandedToolId,
      expandedToolName: toolSpecs[toolSpecs.length - 1].name,
      expandedToolOutputLines: fixture.longOutputLines,
      collectionReopenCount: state.collectionReopenCount || 0,
      toolReopenCount: state.toolReopenCount || 0,
      respondingWhileExpanded: store.isStreaming === true && store.currentRunId === runId,
    };
  })()`;
}
