import path from "node:path";
import process from "node:process";
import { mkdir } from "node:fs/promises";
import { Database } from "bun:sqlite";
import {
  CdpClient,
  findLocusWebViewTarget,
  sleep,
} from "./locus-webview2-stress-client";

const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_SPEED = 8;
const DEFAULT_MIN_EVENT_GAP_MS = 18;
const DEFAULT_SETTLE_MS = 800;
const DEFAULT_VIEWPORT_WIDTH = 1600;
const DEFAULT_VIEWPORT_HEIGHT = 900;

interface CliOptions {
  browserUrl: string;
  runtimeRoot: string;
  sourceDatabase: string;
  sourceSessionId: string;
  sourceRunId: string;
  fromSeq: number | null;
  untilSeq: number | null;
  speed: number;
  minEventGapMs: number;
  settleMs: number;
  timeoutMs: number;
  viewportWidth: number;
  viewportHeight: number;
  captureSeqs: Set<number>;
  expandLatest: boolean;
}

interface ReplayEventRecord {
  seq: number;
  eventType: string;
  payload: Record<string, unknown>;
  createdAt: number;
}

interface ReplayFrameSummary {
  frameCount: number;
  logicalMissingFrames: number;
  visibleMissingFrames: number;
  maxLogicalMissingStreak: number;
  maxVisibleMissingStreak: number;
  expansionCount: number;
  issueCount: number;
  issues: Array<Record<string, unknown>>;
  final: Record<string, unknown> | null;
}

const options = parseArgs(process.argv.slice(2));
assertIsolatedRuntime(options.runtimeRoot, options.sourceDatabase);
const replaySource = loadReplaySource(options);
const target = await findLocusWebViewTarget(options.browserUrl, options.timeoutMs);
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl!);
const artifactDirectory = path.join(options.runtimeRoot, "artifacts", "chat-session-replay");

try {
  await mkdir(artifactDirectory, { recursive: true });
  await cdp.send("Runtime.enable");
  await cdp.send("Page.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: options.viewportWidth,
    height: options.viewportHeight,
    deviceScaleFactor: 1,
    mobile: false,
  });

  const setup = await cdp.evaluate<{ sessionId: string; runId: string }>(
    replaySetupExpression(options),
  );
  const startedAt = performance.now();
  let previousTargetMs = 0;
  const captures: Array<{ seq: number; path: string }> = [];
  const checkpoints: Array<Record<string, unknown>> = [];

  for (let index = 0; index < replaySource.events.length; index += 1) {
    const event = replaySource.events[index]!;
    const sourceOffsetMs = Math.max(0, (event.createdAt - replaySource.firstCreatedAt) * 1_000);
    const scaledOffsetMs = sourceOffsetMs / options.speed;
    const minimumOffsetMs = index * options.minEventGapMs;
    const targetOffsetMs = Math.max(scaledOffsetMs, minimumOffsetMs, previousTargetMs);
    const remainingMs = targetOffsetMs - (performance.now() - startedAt);
    if (remainingMs > 0) await sleep(remainingMs);

    const eventSnapshot = await cdp.evaluate<Record<string, unknown>>(
      replayEventExpression(event, setup.sessionId, setup.runId),
    );
    previousTargetMs = targetOffsetMs;

    if (options.captureSeqs.has(event.seq)) {
      const captureSnapshot = await cdp.evaluate<Record<string, unknown>>(
        "window.__locusChatSessionReplay?.prepareCapture?.()",
      );
      await sleep(100);
      const capture = await cdp.send("Page.captureScreenshot", {
        format: "png",
        captureBeyondViewport: false,
        fromSurface: true,
      }) as { data?: string };
      const capturePath = path.join(artifactDirectory, `seq-${event.seq}.png`);
      await Bun.write(capturePath, Buffer.from(capture.data || "", "base64"));
      captures.push({ seq: event.seq, path: capturePath });
      checkpoints.push({
        seq: event.seq,
        eventType: event.eventType,
        afterEvent: eventSnapshot,
        atCapture: captureSnapshot,
      });
    }
  }

  await sleep(options.settleMs);
  const summary = await cdp.evaluate<ReplayFrameSummary>(
    "window.__locusChatSessionReplay?.finish?.()",
  );
  const result = {
    action: "chat-session-event-replay",
    runtimeRoot: options.runtimeRoot,
    sourceDatabase: options.sourceDatabase,
    sourceSessionId: options.sourceSessionId,
    sourceRunId: replaySource.runId,
    replaySessionId: setup.sessionId,
    replayRunId: setup.runId,
    sourceEventCount: replaySource.events.length,
    sourceSeqRange: [replaySource.events[0]!.seq, replaySource.events.at(-1)!.seq],
    sourceDurationMs: (replaySource.lastCreatedAt - replaySource.firstCreatedAt) * 1_000,
    replayDurationMs: Math.round(performance.now() - startedAt),
    speed: options.speed,
    minEventGapMs: options.minEventGapMs,
    captures,
    checkpoints,
    summary,
  };
  console.log(`LOCUS_CHAT_SESSION_REPLAY_JSON ${JSON.stringify(result)}`);
  const failedCheckpoint = checkpoints.some((checkpoint) => {
    const snapshot = checkpoint.atCapture as Record<string, unknown> | undefined;
    return snapshot?.logicalPresent !== true || snapshot?.visibleBlockPresent !== true;
  });
  if (summary.maxLogicalMissingStreak > 1 || failedCheckpoint) {
    process.exitCode = 1;
  }
} finally {
  cdp.close();
}

function parseArgs(args: string[]): CliOptions {
  const parsed: CliOptions = {
    browserUrl: "",
    runtimeRoot: "",
    sourceDatabase: "",
    sourceSessionId: "",
    sourceRunId: "",
    fromSeq: null,
    untilSeq: null,
    speed: DEFAULT_SPEED,
    minEventGapMs: DEFAULT_MIN_EVENT_GAP_MS,
    settleMs: DEFAULT_SETTLE_MS,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    viewportWidth: DEFAULT_VIEWPORT_WIDTH,
    viewportHeight: DEFAULT_VIEWPORT_HEIGHT,
    captureSeqs: new Set<number>(),
    expandLatest: true,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    }
    if (arg === "--no-expand-latest") {
      parsed.expandLatest = false;
      continue;
    }
    const [name, inlineValue = ""] = arg.split(/=(.*)/s, 2);
    const value = inlineValue || args[index + 1];
    if (name === "--browser-url") {
      parsed.browserUrl = requireValue(name, value);
    } else if (name === "--runtime-root") {
      parsed.runtimeRoot = path.resolve(requireValue(name, value));
    } else if (name === "--source-db") {
      parsed.sourceDatabase = path.resolve(requireValue(name, value));
    } else if (name === "--session-id") {
      parsed.sourceSessionId = requireValue(name, value);
    } else if (name === "--run-id") {
      parsed.sourceRunId = requireValue(name, value);
    } else if (name === "--from-seq") {
      parsed.fromSeq = nonNegativeInteger(name, value);
    } else if (name === "--until-seq") {
      parsed.untilSeq = nonNegativeInteger(name, value);
    } else if (name === "--speed") {
      parsed.speed = positiveNumber(name, value);
    } else if (name === "--min-event-gap-ms") {
      parsed.minEventGapMs = nonNegativeInteger(name, value);
    } else if (name === "--settle-ms") {
      parsed.settleMs = nonNegativeInteger(name, value);
    } else if (name === "--timeout-ms") {
      parsed.timeoutMs = positiveInteger(name, value, 1_000);
    } else if (name === "--viewport-width") {
      parsed.viewportWidth = positiveInteger(name, value, 800);
    } else if (name === "--viewport-height") {
      parsed.viewportHeight = positiveInteger(name, value, 600);
    } else if (name === "--capture-seq") {
      for (const item of requireValue(name, value).split(",")) {
        parsed.captureSeqs.add(nonNegativeInteger(name, item.trim()));
      }
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
    if (!inlineValue) index += 1;
  }

  if (!parsed.runtimeRoot) throw new Error("--runtime-root is required.");
  if (!parsed.sourceDatabase) throw new Error("--source-db is required.");
  if (!parsed.sourceSessionId) throw new Error("--session-id is required.");
  if (parsed.fromSeq !== null && parsed.untilSeq !== null && parsed.fromSeq > parsed.untilSeq) {
    throw new Error("--from-seq must be <= --until-seq.");
  }
  return parsed;
}

function requireValue(name: string, value: string | undefined) {
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value.`);
  return value;
}

function positiveInteger(name: string, value: string | undefined, minimum: number) {
  const parsed = Number(requireValue(name, value));
  if (!Number.isInteger(parsed) || parsed < minimum) {
    throw new Error(`${name} must be an integer >= ${minimum}.`);
  }
  return parsed;
}

function nonNegativeInteger(name: string, value: string | undefined) {
  return positiveInteger(name, value, 0);
}

function positiveNumber(name: string, value: string | undefined) {
  const parsed = Number(requireValue(name, value));
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a number > 0.`);
  }
  return parsed;
}

function assertIsolatedRuntime(runtimeRoot: string, sourceDatabase: string) {
  const root = path.resolve(runtimeRoot);
  const isolatedDatabase = path.resolve(root, "database", "locus.db");
  if (!isolatedDatabase.toLowerCase().startsWith(`${root}${path.sep}`.toLowerCase())) {
    throw new Error(`Refusing runtime database outside isolated root: ${isolatedDatabase}`);
  }
  if (!Bun.file(isolatedDatabase).size) {
    throw new Error(`Isolated Locus database is missing or empty: ${isolatedDatabase}`);
  }
  if (!Bun.file(sourceDatabase).size) {
    throw new Error(`Source database is missing or empty: ${sourceDatabase}`);
  }
  if (path.resolve(sourceDatabase).toLowerCase() === isolatedDatabase.toLowerCase()) {
    throw new Error("Source and isolated databases must be different files.");
  }
}

function loadReplaySource(options: CliOptions) {
  const database = new Database(options.sourceDatabase, { readonly: true, create: false });
  try {
    const run = options.sourceRunId
      ? database.query(
        "SELECT run_id FROM session_runs WHERE session_id = ? AND run_id = ?",
      ).get(options.sourceSessionId, options.sourceRunId) as { run_id?: string } | null
      : database.query(
        "SELECT run_id FROM session_runs WHERE session_id = ? ORDER BY started_at DESC LIMIT 1",
      ).get(options.sourceSessionId) as { run_id?: string } | null;
    const runId = run?.run_id;
    if (!runId) throw new Error("No matching session run found in the source database.");

    const conditions = ["session_id = ?", "run_id = ?"];
    const params: Array<string | number> = [options.sourceSessionId, runId];
    if (options.fromSeq !== null) {
      conditions.push("seq >= ?");
      params.push(options.fromSeq);
    }
    if (options.untilSeq !== null) {
      conditions.push("seq <= ?");
      params.push(options.untilSeq);
    }
    const rows = database.query(
      `SELECT seq, event_type, payload_json, created_at
       FROM session_events
       WHERE ${conditions.join(" AND ")}
       ORDER BY seq`,
    ).all(...params) as Array<{
      seq: number;
      event_type: string;
      payload_json: string;
      created_at: number;
    }>;
    const events = rows.map<ReplayEventRecord>((row) => ({
      seq: row.seq,
      eventType: row.event_type,
      payload: JSON.parse(row.payload_json) as Record<string, unknown>,
      createdAt: row.created_at,
    }));
    if (events.length === 0) throw new Error("No replay events matched the requested sequence range.");
    if (events[0]!.eventType !== "runStart") {
      throw new Error("Replay range must begin with runStart so the frontend owns the synthetic run.");
    }
    return {
      runId,
      events,
      firstCreatedAt: events[0]!.createdAt,
      lastCreatedAt: events.at(-1)!.createdAt,
    };
  } finally {
    database.close();
  }
}

function replaySetupExpression(options: CliOptions) {
  return `(async () => {
    const config = ${JSON.stringify({
      timeoutMs: options.timeoutMs,
      expandLatest: options.expandLatest,
    })};
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));
    const waitFor = async (read, label) => {
      const deadline = performance.now() + config.timeoutMs;
      while (performance.now() < deadline) {
        const value = read();
        if (value) return value;
        await sleep(30);
      }
      throw new Error('Timed out waiting for ' + label);
    };
    const app = await waitFor(
      () => document.querySelector('#app')?.__vue_app__,
      'Vue app',
    );
    const pinia = Reflect.ownKeys(app._context.provides)
      .map((key) => app._context.provides[key])
      .find((value) => value?._s instanceof Map && value._s.has('chat'));
    if (!pinia) throw new Error('Unable to locate the Pinia store registry.');
    const store = pinia._s.get('chat');
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (typeof invoke !== 'function') throw new Error('Tauri invoke bridge is unavailable.');
    if (store.isStreaming && String(store.currentRunId || '').startsWith('session-replay-')) {
      store.$patch({
        activeToolCalls: [],
        liveRenderParts: [],
        streamingText: '',
        rawStreamText: '',
        streamingThinking: '',
        isStreaming: false,
        isThinking: false,
        currentRunId: null,
      });
      await nextFrame();
    }
    if (store.isStreaming) throw new Error('Stop the active isolated response before replaying.');
    const sessionId = await invoke('create_session', {
      title: 'Replay · Tool visibility',
      sessionType: 'chat',
      parentSessionId: null,
      agentId: null,
    });
    await store.refreshSessions();
    await store.selectSession(sessionId, { persist: false });
    await waitFor(() => document.querySelector('.chat-transcript-scroll'), 'chat transcript');

    const runId = 'session-replay-' + Date.now().toString(36);
    const state = {
      sessionId,
      runId,
      sourceSeq: 0,
      eventType: 'setup',
      expectedLatestToolId: null,
      expectedLatestToolName: null,
      expectedSince: 0,
      frames: [],
      issues: [],
      frameId: 0,
      logicalMissingStreak: 0,
      visibleMissingStreak: 0,
      maxLogicalMissingStreak: 0,
      maxVisibleMissingStreak: 0,
      logicalMissingFrames: 0,
      visibleMissingFrames: 0,
      expansionCount: 0,
      startedAt: performance.now(),
      finished: false,
    };

    const idsFor = (element) => (element?.getAttribute('data-tool-layout-tool-call-ids') || '')
      .split(',')
      .filter(Boolean);
    const isVisible = (element) => {
      if (!element) return false;
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.display !== 'none'
        && style.visibility !== 'hidden'
        && Number(style.opacity || '1') > 0.01
        && rect.width > 0
        && rect.height > 0;
    };
    const latestOwners = () => {
      if (!state.expectedLatestToolId) return [];
      return [...document.querySelectorAll('[data-tool-layout-tool-call-ids]')]
        .filter((element) => idsFor(element).includes(state.expectedLatestToolId));
    };
    const readSnapshot = () => {
      const owners = latestOwners();
      const blocks = owners.filter(
        (element) => element.getAttribute('data-tool-layout-kind') === 'block',
      );
      const collections = owners.filter((element) => element.classList.contains('tool-call-collection'));
      const visibleBlocks = blocks.filter(isVisible);
      const collection = collections.at(-1) || blocks.at(-1)?.closest('.tool-call-collection') || null;
      const summary = collection?.querySelector('.tool-call-batch-summary') || null;
      const scroll = document.querySelector('.chat-transcript-scroll');
      const domBlocks = [...document.querySelectorAll('[data-tool-layout-kind="block"]')];
      const lastBlock = domBlocks.at(-1) || null;
      return {
        t: Math.round(performance.now() - state.startedAt),
        seq: state.sourceSeq,
        eventType: state.eventType,
        expectedLatestToolId: state.expectedLatestToolId,
        expectedLatestToolName: state.expectedLatestToolName,
        logicalPresent: owners.length > 0,
        blockPresent: blocks.length > 0,
        visibleBlockPresent: visibleBlocks.length > 0,
        ownerCount: owners.length,
        ownerScopes: owners.map((owner) => owner.closest('[data-render-part-scope]')?.getAttribute('data-render-part-scope') || ''),
        collectionCanCollapse: collection?.getAttribute('data-tool-layout-can-collapse') || null,
        collectionExpanded: collection?.getAttribute('data-tool-layout-expanded') || null,
        collectionPanelVisible: collection?.getAttribute('data-tool-layout-panel-visible') || null,
        collectionPanelLeaving: collection?.getAttribute('data-tool-layout-panel-leaving') || null,
        summaryExpanded: summary?.getAttribute('aria-expanded') || null,
        lastDomToolId: idsFor(lastBlock)[0] || null,
        toolBlockCount: domBlocks.length,
        collectionCount: document.querySelectorAll('.tool-call-collection').length,
        transientToolGroupCount: document.querySelectorAll('[data-render-part-scope="transient"][data-render-part-kind="toolCall"]').length,
        historyToolGroupCount: document.querySelectorAll('[data-render-part-scope="history"][data-render-part-kind="toolCall"]').length,
        isStreaming: store.isStreaming,
        activeToolCallCount: store.activeToolCalls.length,
        liveRenderPartCount: store.liveRenderParts.length,
        messageCount: store.messages.length,
        scrollTop: scroll?.scrollTop ?? null,
        scrollHeight: scroll?.scrollHeight ?? null,
        bottomDistance: scroll ? scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight : null,
      };
    };
    const expandLatestCollection = () => {
      if (!config.expandLatest || !state.expectedLatestToolId) return false;
      const collection = latestOwners()
        .find((element) => element.classList.contains('tool-call-collection'));
      if (!collection) return false;
      if (collection.getAttribute('data-tool-layout-can-collapse') !== 'true') return false;
      if (collection.getAttribute('data-tool-layout-expanded') === 'true') return false;
      if (collection.getAttribute('data-tool-layout-panel-leaving') === 'true') return false;
      const summary = collection.querySelector('.tool-call-batch-summary');
      if (!summary || summary.getAttribute('aria-expanded') === 'true') return false;
      summary.click();
      state.expansionCount += 1;
      return true;
    };
    const sample = () => {
      if (state.finished) return;
      const snapshot = readSnapshot();
      if (state.expectedLatestToolId && performance.now() - state.expectedSince >= 34) {
        state.logicalMissingStreak = snapshot.logicalPresent ? 0 : state.logicalMissingStreak + 1;
        state.visibleMissingStreak = snapshot.visibleBlockPresent ? 0 : state.visibleMissingStreak + 1;
        if (!snapshot.logicalPresent) state.logicalMissingFrames += 1;
        if (!snapshot.visibleBlockPresent) state.visibleMissingFrames += 1;
        state.maxLogicalMissingStreak = Math.max(state.maxLogicalMissingStreak, state.logicalMissingStreak);
        state.maxVisibleMissingStreak = Math.max(state.maxVisibleMissingStreak, state.visibleMissingStreak);
        if (
          (state.logicalMissingStreak === 2 || state.visibleMissingStreak === 5)
          && state.issues.length < 40
        ) {
          state.issues.push({
            ...snapshot,
            logicalMissingStreak: state.logicalMissingStreak,
            visibleMissingStreak: state.visibleMissingStreak,
          });
        }
      }
      if (state.frames.length < 12_000) state.frames.push(snapshot);
      state.frameId = requestAnimationFrame(sample);
    };

    window.__locusChatSessionReplay = {
      state,
      async emit(event, meta) {
        state.sourceSeq = meta.seq;
        state.eventType = meta.eventType;
        if (meta.eventType === 'toolCallStart' && event.toolCallId) {
          if (state.expectedLatestToolId !== event.toolCallId) {
            state.expectedLatestToolId = event.toolCallId;
            state.expectedLatestToolName = event.toolName || null;
            state.expectedSince = performance.now();
            state.logicalMissingStreak = 0;
            state.visibleMissingStreak = 0;
          }
        } else if (meta.eventType === 'toolCallRoundDone' && Array.isArray(event.toolCalls) && event.toolCalls.length > 0) {
          const latest = event.toolCalls[event.toolCalls.length - 1];
          state.expectedLatestToolId = latest.id;
          state.expectedLatestToolName = latest.name || null;
          state.expectedSince = performance.now();
          state.logicalMissingStreak = 0;
          state.visibleMissingStreak = 0;
        }
        const handled = store.handleStreamEvent({ ...event, runId, sessionId });
        if (!handled) throw new Error('Chat store rejected ' + meta.eventType + ' at seq ' + meta.seq);
        await nextFrame();
        expandLatestCollection();
        await nextFrame();
        return readSnapshot();
      },
      async prepareCapture() {
        const scroll = document.querySelector('.chat-transcript-scroll');
        if (scroll) scroll.scrollTop = scroll.scrollHeight;
        await nextFrame();
        expandLatestCollection();
        await nextFrame();
        return readSnapshot();
      },
      finish() {
        state.finished = true;
        cancelAnimationFrame(state.frameId);
        const final = readSnapshot();
        return {
          frameCount: state.frames.length,
          logicalMissingFrames: state.logicalMissingFrames,
          visibleMissingFrames: state.visibleMissingFrames,
          maxLogicalMissingStreak: state.maxLogicalMissingStreak,
          maxVisibleMissingStreak: state.maxVisibleMissingStreak,
          expansionCount: state.expansionCount,
          issueCount: state.issues.length,
          issues: state.issues,
          final,
        };
      },
    };
    state.frameId = requestAnimationFrame(sample);
    return { sessionId, runId };
  })()`;
}

function replayEventExpression(
  event: ReplayEventRecord,
  sessionId: string,
  runId: string,
) {
  return `window.__locusChatSessionReplay.emit(
    ${JSON.stringify(event.payload)},
    ${JSON.stringify({ seq: event.seq, eventType: event.eventType, sessionId, runId })}
  )`;
}

function printHelp() {
  console.log(`Usage:
  bun run scripts/locus-chat-session-replay.ts \\
    --runtime-root <isolated-runtime-root> \\
    --source-db <read-only-source-locus.db> \\
    --session-id <source-session-id> [options]

Options:
  --run-id <id>             Replay a specific run; defaults to the newest run
  --from-seq <n>            First persisted event sequence; must begin at runStart
  --until-seq <n>           Stop after this persisted event sequence
  --speed <factor>          Relative-time acceleration, default ${DEFAULT_SPEED}
  --min-event-gap-ms <ms>   Minimum visible spacing between events, default ${DEFAULT_MIN_EVENT_GAP_MS}
  --settle-ms <ms>          Frames sampled after the final event, default ${DEFAULT_SETTLE_MS}
  --capture-seq <a,b,c>     Save private viewport PNGs below the isolated runtime root
  --no-expand-latest        Preserve natural batch collapse state
  --browser-url <url>       WebView2 DevTools URL; auto-detected on ports 19222-19246
  --viewport-width <px>     Fixed CDP viewport width, default ${DEFAULT_VIEWPORT_WIDTH}
  --viewport-height <px>    Fixed CDP viewport height, default ${DEFAULT_VIEWPORT_HEIGHT}
  --timeout-ms <ms>         Connection/setup timeout, default ${DEFAULT_TIMEOUT_MS}`);
}
