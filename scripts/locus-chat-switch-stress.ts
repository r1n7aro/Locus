import path from "node:path";
import process from "node:process";
import { Database } from "bun:sqlite";
import { createLongSessionFixtures, type LongSessionFixture } from "./locus-chat-switch-stress-fixture";
import { CdpClient, findLocusWebViewTarget } from "./locus-webview2-stress-client";

const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_SETTLE_MS = 1_800;
const DEFAULT_SWITCH_COUNT = 15;
const DEFAULT_VIEWPORT_WIDTH = 1600;
const DEFAULT_VIEWPORT_HEIGHT = 900;

interface CliOptions {
  browserUrl: string;
  runtimeRoot: string;
  timeoutMs: number;
  settleMs: number;
  switchCount: number;
  viewportWidth: number;
  viewportHeight: number;
  cleanup: boolean;
  verify: boolean;
  profileOnly: boolean;
  forceMaterialize: boolean;
  contentVisibility: "production" | "auto" | "visible";
}

interface PerformanceMetricsResponse {
  metrics?: Array<{ name: string; value: number }>;
}

interface CreatedSessionsResult {
  sessionIds: string[];
}

const options = parseArgs(process.argv.slice(2));
const databasePath = resolveDatabasePath(options.runtimeRoot);
const fixtures = createLongSessionFixtures();
const target = await findLocusWebViewTarget(options.browserUrl, options.timeoutMs);
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl!);
let sessionIds: string[] = [];

try {
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: options.viewportWidth,
    height: options.viewportHeight,
    deviceScaleFactor: 1,
    mobile: false,
  });

  const created = await cdp.evaluate<CreatedSessionsResult>(createSessionsExpression(
    fixtures.map((fixture) => fixture.title),
    options.timeoutMs,
  ));
  sessionIds = created.sessionIds;
  seedLongSessions(databasePath, sessionIds, fixtures);

  await cdp.send("Performance.enable");
  await cdp.send("HeapProfiler.enable");
  await cdp.send("HeapProfiler.collectGarbage");
  const performanceBefore = await readPerformanceMetrics(cdp);
  const profileStartedAt = performance.now();
  const result = await cdp.evaluate<Record<string, unknown>>(switchStressExpression({
    fixtures: fixtures.map((fixture, index) => ({
      key: fixture.key,
      title: fixture.title,
      sessionId: sessionIds[index]!,
    })),
    settleMs: options.settleMs,
    switchCount: options.switchCount,
    timeoutMs: options.timeoutMs,
    forceMaterialize: options.forceMaterialize,
    contentVisibility: options.contentVisibility,
    lowOverheadProfile: options.profileOnly,
  }));
  const profileWallDurationMs = Math.round(performance.now() - profileStartedAt);
  const performanceAfter = await readPerformanceMetrics(cdp);
  await cdp.send("HeapProfiler.collectGarbage");
  const performanceAfterGc = await readPerformanceMetrics(cdp);
  const performanceProfile = diffPerformanceMetrics(
    performanceBefore,
    performanceAfter,
    performanceAfterGc,
    profileWallDurationMs,
  );
  const verification = verifySwitchStability(result);
  const reportedResult = options.profileOnly
    ? {
        aggregate: result.aggregate,
        contentVisibility: result.contentVisibility,
        lowOverheadProfile: result.lowOverheadProfile,
        forcedMaterialization: result.forcedMaterialization,
      }
    : result;
  printResult({
    action: "switch-stress",
    databasePath,
    runtimeRoot: options.runtimeRoot,
    fixtureCount: fixtures.length,
    messagesPerFixture: fixtures[0]?.messages.length ?? 0,
    sessionIds,
    performanceProfile,
    result: reportedResult,
    verification,
  });
  if (options.verify && !verification.passed) process.exitCode = 1;
} finally {
  if (options.cleanup && sessionIds.length > 0) {
    try {
      await cdp.evaluate(cleanupSessionsExpression(sessionIds));
    } catch (error) {
      console.error(`Failed to clean stress sessions: ${String(error)}`);
    }
  }
  cdp.close();
}

function parseArgs(args: string[]): CliOptions {
  const parsed: CliOptions = {
    browserUrl: "",
    runtimeRoot: "",
    timeoutMs: DEFAULT_TIMEOUT_MS,
    settleMs: DEFAULT_SETTLE_MS,
    switchCount: DEFAULT_SWITCH_COUNT,
    viewportWidth: DEFAULT_VIEWPORT_WIDTH,
    viewportHeight: DEFAULT_VIEWPORT_HEIGHT,
    cleanup: false,
    verify: false,
    profileOnly: false,
    forceMaterialize: true,
    contentVisibility: "production",
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    }
    if (arg === "--cleanup") {
      parsed.cleanup = true;
      continue;
    }
    if (arg === "--verify") {
      parsed.verify = true;
      continue;
    }
    if (arg === "--profile-only") {
      parsed.profileOnly = true;
      continue;
    }
    if (arg === "--no-force-materialize") {
      parsed.forceMaterialize = false;
      continue;
    }
    const [name, inlineValue = ""] = arg.split(/=(.*)/s, 2);
    const value = inlineValue || args[index + 1];
    if (name === "--browser-url") {
      parsed.browserUrl = requireValue(name, value);
    } else if (name === "--runtime-root") {
      parsed.runtimeRoot = path.resolve(requireValue(name, value));
    } else if (name === "--timeout-ms") {
      parsed.timeoutMs = positiveInteger(name, value, 1_000);
    } else if (name === "--settle-ms") {
      parsed.settleMs = positiveInteger(name, value, 500);
    } else if (name === "--switch-count") {
      parsed.switchCount = positiveInteger(name, value, 5);
    } else if (name === "--viewport-width") {
      parsed.viewportWidth = positiveInteger(name, value, 800);
    } else if (name === "--viewport-height") {
      parsed.viewportHeight = positiveInteger(name, value, 600);
    } else if (name === "--content-visibility") {
      const mode = requireValue(name, value);
      if (mode !== "production" && mode !== "auto" && mode !== "visible") {
        throw new Error("--content-visibility must be production, auto, or visible.");
      }
      parsed.contentVisibility = mode;
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
    if (!inlineValue) index += 1;
  }

  if (!parsed.runtimeRoot) {
    throw new Error("--runtime-root is required so the stress test can only write to an isolated database.");
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

function resolveDatabasePath(runtimeRoot: string) {
  const root = path.resolve(runtimeRoot);
  const databasePath = path.resolve(root, "database", "locus.db");
  const expectedPrefix = `${root}${path.sep}`.toLowerCase();
  if (!databasePath.toLowerCase().startsWith(expectedPrefix)) {
    throw new Error(`Refusing database path outside isolated runtime root: ${databasePath}`);
  }
  if (!Bun.file(databasePath).size) {
    throw new Error(`Isolated Locus database is missing or empty: ${databasePath}`);
  }
  return databasePath;
}

async function readPerformanceMetrics(cdp: CdpClient) {
  const response = await cdp.send("Performance.getMetrics") as PerformanceMetricsResponse;
  return Object.fromEntries((response.metrics ?? []).map(({ name, value }) => [name, value]));
}

function diffPerformanceMetrics(
  before: Record<string, number>,
  after: Record<string, number>,
  afterGc: Record<string, number>,
  wallDurationMs: number,
) {
  const delta = (name: string) => (after[name] ?? 0) - (before[name] ?? 0);
  const milliseconds = (name: string) => Math.round(delta(name) * 1_000 * 100) / 100;
  const count = (name: string) => Math.round(delta(name));
  return {
    wallDurationMs,
    taskDurationMs: milliseconds("TaskDuration"),
    scriptDurationMs: milliseconds("ScriptDuration"),
    layoutDurationMs: milliseconds("LayoutDuration"),
    recalcStyleDurationMs: milliseconds("RecalcStyleDuration"),
    layoutCount: count("LayoutCount"),
    recalcStyleCount: count("RecalcStyleCount"),
    jsHeapUsedBeforeBytes: Math.round(before.JSHeapUsedSize ?? 0),
    jsHeapUsedAfterBytes: Math.round(afterGc.JSHeapUsedSize ?? 0),
    jsHeapUsedDeltaBytes: Math.round((afterGc.JSHeapUsedSize ?? 0) - (before.JSHeapUsedSize ?? 0)),
    nodesBefore: Math.round(before.Nodes ?? 0),
    nodesAfter: Math.round(afterGc.Nodes ?? 0),
    nodesDelta: Math.round((afterGc.Nodes ?? 0) - (before.Nodes ?? 0)),
    documentsDelta: Math.round((afterGc.Documents ?? 0) - (before.Documents ?? 0)),
    eventListenersDelta: Math.round(
      (afterGc.JSEventListeners ?? 0) - (before.JSEventListeners ?? 0),
    ),
  };
}

function seedLongSessions(
  databasePath: string,
  sessionIdsToSeed: string[],
  fixtureList: LongSessionFixture[],
) {
  if (sessionIdsToSeed.length !== fixtureList.length) {
    throw new Error("Created session count does not match fixture count.");
  }
  const db = new Database(databasePath, { create: false, readwrite: true });
  db.run("PRAGMA busy_timeout = 10000");
  db.run("PRAGMA foreign_keys = ON");
  const insert = db.prepare(`
    INSERT INTO messages (
      id, session_id, role, content, created_at, tool_calls,
      thinking_content, thinking_duration, metadata_json
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
  `);
  const clearMessages = db.prepare("DELETE FROM messages WHERE session_id = ?");
  const updateSession = db.prepare("UPDATE sessions SET title = ?, updated_at = ? WHERE id = ?");
  const transaction = db.transaction(() => {
    fixtureList.forEach((fixture, fixtureIndex) => {
      const sessionId = sessionIdsToSeed[fixtureIndex]!;
      clearMessages.run(sessionId);
      for (const message of fixture.messages) {
        insert.run(
          `${sessionId}:${message.id}`,
          sessionId,
          message.role,
          message.content,
          message.createdAt,
          message.toolCalls ? JSON.stringify(message.toolCalls) : null,
          message.thinkingContent ?? null,
          message.thinkingDuration ?? null,
          message.metadata ? JSON.stringify(message.metadata) : null,
        );
      }
      updateSession.run(
        fixture.title,
        fixture.messages[fixture.messages.length - 1]?.createdAt ?? Math.floor(Date.now() / 1000),
        sessionId,
      );
    });
  });
  transaction();
  db.close();
}

function createSessionsExpression(titles: string[], timeoutMs: number) {
  return `(async () => {
    const titles = ${JSON.stringify(titles)};
    const timeoutMs = ${timeoutMs};
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const [{ useChatStore }, sessionService] = await Promise.all([
      import('/src/stores/chat.ts'),
      import('/src/services/session.ts'),
    ]);
    const store = useChatStore();
    if (store.isStreaming) throw new Error('Stop the active response before running switch stress.');
    const deadline = performance.now() + timeoutMs;
    while (!document.querySelector('.chat-transcript-scroll')) {
      if (performance.now() >= deadline) throw new Error('Timed out waiting for Chat transcript.');
      await sleep(40);
    }
    const sessionIds = [];
    for (const title of titles) {
      sessionIds.push(await sessionService.createSession({ title, sessionType: 'chat' }));
    }
    return { sessionIds };
  })()`;
}

function switchStressExpression(config: {
  fixtures: Array<{ key: string; title: string; sessionId: string }>;
  settleMs: number;
  switchCount: number;
  timeoutMs: number;
  forceMaterialize: boolean;
  contentVisibility: "production" | "auto" | "visible";
  lowOverheadProfile: boolean;
}) {
  return `(async () => {
    const config = ${JSON.stringify(config)};
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));
    const { useChatStore } = await import('/src/stores/chat.ts');
    const store = useChatStore();
    await store.refreshSessions();

    document.getElementById('stress-content-visibility-override')?.remove();
    if (config.contentVisibility !== 'production') {
      const style = document.createElement('style');
      style.id = 'stress-content-visibility-override';
      style.textContent = config.contentVisibility === 'auto'
        ? '.chat-transcript-message.is-session{content-visibility:auto!important;contain-intrinsic-size:auto 180px!important;}'
        : '.chat-transcript-message.is-session{content-visibility:visible!important;contain-intrinsic-size:auto!important;}';
      document.head.appendChild(style);
    }

    const layoutShifts = [];
    const collectLayoutShifts = (entries) => {
      for (const entry of entries) {
        layoutShifts.push({
          t: Math.round(performance.now()),
          value: entry.value,
          hadRecentInput: entry.hadRecentInput,
          sources: (entry.sources || []).slice(0, 5).map((source) => {
            const node = source.node;
            return node ? {
              tag: node.tagName || null,
              className: typeof node.className === 'string' ? node.className.slice(0, 160) : '',
            } : null;
          }).filter(Boolean),
        });
      }
    };
    let shiftObserver = null;
    if (typeof PerformanceObserver !== 'undefined'
      && PerformanceObserver.supportedEntryTypes?.includes('layout-shift')) {
      shiftObserver = new PerformanceObserver((list) => collectLayoutShifts(list.getEntries()));
      shiftObserver.observe({ type: 'layout-shift', buffered: false });
    }

    const readFrame = (startedAt, expectedSessionId) => {
      const scroll = document.querySelector('.chat-transcript-scroll');
      const content = document.querySelector('.chat-transcript-content');
      const messages = [...document.querySelectorAll('.chat-transcript-message')];
      const scrollRect = scroll?.getBoundingClientRect();
      const contentRect = content?.getBoundingClientRect();
      const firstMessage = messages[0];
      const lastMessage = messages.at(-1);
      const firstRect = firstMessage?.getBoundingClientRect();
      const lastRect = lastMessage?.getBoundingClientRect();
      const unityFences = [...document.querySelectorAll('.unity-property-fence')];
      return {
        t: Math.round(performance.now() - startedAt),
        expectedSessionId,
        activeSessionId: store.activeSessionId,
        pendingSelectionSessionId: store.pendingSelectionSessionId ?? null,
        storeMessageCount: store.messages.length,
        domMessageCount: messages.length,
        scrollTop: scroll?.scrollTop ?? null,
        scrollHeight: scroll?.scrollHeight ?? null,
        clientHeight: scroll?.clientHeight ?? null,
        bottomDistance: scroll ? scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight : null,
        viewportTop: scrollRect?.top ?? null,
        viewportBottom: scrollRect?.bottom ?? null,
        contentTop: contentRect?.top ?? null,
        contentHeight: contentRect?.height ?? null,
        firstMessageTop: firstRect?.top ?? null,
        lastMessageTop: lastRect?.top ?? null,
        lastMessageBottom: lastRect?.bottom ?? null,
        firstMessageId: firstMessage?.getAttribute('data-scroll-anchor-id') ?? null,
        lastMessageId: lastMessage?.getAttribute('data-scroll-anchor-id') ?? null,
        markdownCount: document.querySelectorAll('.markdown-body').length,
        tableCount: document.querySelectorAll('.markdown-body table').length,
        preCount: document.querySelectorAll('.markdown-body pre').length,
        toolCollectionCount: document.querySelectorAll('.tool-call-collection').length,
        toolBlockCount: document.querySelectorAll('.tool-call-block').length,
        unityFenceCount: unityFences.length,
        unityLoadingCount: document.querySelectorAll('.unity-property-fence .unity-property-state:not(.error)').length,
        unityErrorCount: document.querySelectorAll('.unity-property-fence .unity-property-state.error').length,
      };
    };

    const analyze = (frames, selectedAt, shiftStartIndex, shiftEndIndex, fixture) => {
      const targetFrames = frames.filter((frame) => frame.activeSessionId === fixture.sessionId);
      const firstTargetAt = targetFrames[0]?.t ?? selectedAt;
      const after500 = targetFrames.filter((frame) => frame.t >= firstTargetAt + 500);
      const initialWindow = targetFrames.filter((frame) => frame.t <= firstTargetAt + 500);
      let maxScrollTopDelta = 0;
      let maxScrollHeightDelta = 0;
      let maxContentHeightDelta = 0;
      let maxLastBottomDelta = 0;
      let resizeFrameCount = 0;
      let lastUnstableAt = 0;
      let maxFrameGapMs = 0;
      let longFrameCount50 = 0;
      let longFrameCount100 = 0;
      for (let index = 1; index < frames.length; index += 1) {
        const gap = frames[index].t - frames[index - 1].t;
        maxFrameGapMs = Math.max(maxFrameGapMs, gap);
        if (gap > 50) longFrameCount50 += 1;
        if (gap > 100) longFrameCount100 += 1;
      }
      for (let index = 1; index < targetFrames.length; index += 1) {
        const previous = targetFrames[index - 1];
        const current = targetFrames[index];
        const scrollTopDelta = Math.abs((current.scrollTop ?? 0) - (previous.scrollTop ?? 0));
        const scrollHeightDelta = Math.abs((current.scrollHeight ?? 0) - (previous.scrollHeight ?? 0));
        const contentHeightDelta = Math.abs((current.contentHeight ?? 0) - (previous.contentHeight ?? 0));
        const lastBottomDelta = Math.abs((current.lastMessageBottom ?? 0) - (previous.lastMessageBottom ?? 0));
        maxScrollTopDelta = Math.max(maxScrollTopDelta, scrollTopDelta);
        maxScrollHeightDelta = Math.max(maxScrollHeightDelta, scrollHeightDelta);
        maxContentHeightDelta = Math.max(maxContentHeightDelta, contentHeightDelta);
        maxLastBottomDelta = Math.max(maxLastBottomDelta, lastBottomDelta);
        if (scrollHeightDelta > 0.5 || contentHeightDelta > 0.5) resizeFrameCount += 1;
        if (current.t >= firstTargetAt + 250 && (
          scrollHeightDelta > 0.5
          || contentHeightDelta > 0.5
          || lastBottomDelta > 1
          || Math.abs(current.bottomDistance ?? 0) > 1
        )) lastUnstableAt = current.t;
      }
      const finalFrames = targetFrames.slice(-12);
      let bottomStableAt = 0;
      for (let index = 0; index < targetFrames.length; index += 1) {
        const window = targetFrames.slice(index, index + 3);
        if (window.length === 3 && window.every((frame) => Math.abs(frame.bottomDistance ?? 0) <= 1)) {
          bottomStableAt = targetFrames[index].t;
          break;
        }
      }
      return {
        fixture,
        selectedAt,
        firstTargetAt,
        firstTargetBottomDistance: targetFrames[0]?.bottomDistance ?? null,
        maxInitialBottomDistance: Math.max(0, ...initialWindow.map((frame) => Math.abs(frame.bottomDistance ?? 0))),
        bottomRestoreDurationMs: bottomStableAt ? Math.max(0, bottomStableAt - firstTargetAt) : null,
        frameCount: frames.length,
        targetFrameCount: targetFrames.length,
        maxFrameGapMs,
        longFrameCount50,
        longFrameCount100,
        blankTargetFrameCount: targetFrames.filter((frame) => frame.domMessageCount === 0).length,
        wrongMessagePrefixFrameCount: targetFrames.filter((frame) => {
          const id = frame.lastMessageId || '';
          return id && !id.includes(fixture.sessionId + ':') && id !== '__transient__';
        }).length,
        post500BottomDetachFrames: after500.filter((frame) => Math.abs(frame.bottomDistance ?? 0) > 1).length,
        maxPost500BottomDistance: Math.max(0, ...after500.map((frame) => Math.abs(frame.bottomDistance ?? 0))),
        maxScrollTopDelta,
        maxScrollHeightDelta,
        maxContentHeightDelta,
        maxLastBottomDelta,
        resizeFrameCount,
        lastUnstableAt,
        settleDurationMs: lastUnstableAt ? Math.max(0, lastUnstableAt - firstTargetAt) : 0,
        layoutShiftCount: shiftEndIndex - shiftStartIndex,
        layoutShiftScore: layoutShifts.slice(shiftStartIndex, shiftEndIndex)
          .reduce((sum, entry) => sum + entry.value, 0),
        final: finalFrames.at(-1) ?? null,
        finalMetricRanges: {
          scrollTop: range(finalFrames.map((frame) => frame.scrollTop)),
          scrollHeight: range(finalFrames.map((frame) => frame.scrollHeight)),
          contentHeight: range(finalFrames.map((frame) => frame.contentHeight)),
          lastMessageBottom: range(finalFrames.map((frame) => frame.lastMessageBottom)),
        },
        transitionTrace: targetFrames.slice(0, 4),
      };
    };

    const range = (values) => {
      const numbers = values.filter((value) => typeof value === 'number');
      return numbers.length ? Math.max(...numbers) - Math.min(...numbers) : 0;
    };

    const average = (values) => values.length
      ? values.reduce((sum, value) => sum + value, 0) / values.length
      : 0;
    const percentile = (values, quantile) => {
      if (!values.length) return 0;
      const sorted = [...values].sort((a, b) => a - b);
      return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * quantile) - 1)];
    };

    const results = [];
    for (let switchIndex = 0; switchIndex < config.switchCount; switchIndex += 1) {
      const fixture = config.fixtures[switchIndex % config.fixtures.length];
      const startedAt = performance.now();
      const shiftStartIndex = layoutShifts.length;
      const frames = [];
      let sampling = true;
      const sample = () => {
        frames.push(config.lowOverheadProfile ? {
          t: Math.round(performance.now() - startedAt),
          expectedSessionId: fixture.sessionId,
          activeSessionId: store.activeSessionId,
          pendingSelectionSessionId: store.pendingSelectionSessionId ?? null,
          storeMessageCount: store.messages.length,
        } : readFrame(startedAt, fixture.sessionId));
        if (sampling) requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
      await store.selectSession(fixture.sessionId, { persist: false });
      const selectedAt = Math.round(performance.now() - startedAt);
      await nextFrame();
      const scroll = document.querySelector('.chat-transcript-scroll');
      store.rememberSessionScrollState(fixture.sessionId, { mode: 'bottom' });
      if (scroll) scroll.scrollTop = scroll.scrollHeight;
      if (config.forceMaterialize) {
        await sleep(250);
        for (const message of document.querySelectorAll('.chat-transcript-message.is-session')) {
          message.style.contentVisibility = 'visible';
          message.style.containIntrinsicSize = 'auto';
          void message.getBoundingClientRect().height;
          for (const block of message.querySelectorAll(
            '.markdown-body, .tool-call-collection, .unity-property-fence, pre, table'
          )) {
            void block.getBoundingClientRect().height;
            void block.scrollHeight;
          }
        }
      }
      await sleep(config.settleMs);
      sampling = false;
      await nextFrame();
      if (shiftObserver) collectLayoutShifts(shiftObserver.takeRecords());
      results.push(analyze(
        frames,
        selectedAt,
        shiftStartIndex,
        layoutShifts.length,
        fixture,
      ));
    }

    shiftObserver?.disconnect();
    const aggregate = {
      switchCount: results.length,
      averageSelectionDurationMs: Math.round(average(results.map((result) => result.selectedAt))),
      p95SelectionDurationMs: percentile(results.map((result) => result.selectedAt), 0.95),
      averageFirstTargetMs: Math.round(average(results.map((result) => result.firstTargetAt))),
      p95FirstTargetMs: percentile(results.map((result) => result.firstTargetAt), 0.95),
      maxFrameGapMs: Math.max(0, ...results.map((result) => result.maxFrameGapMs)),
      longFrameCount50: results.reduce((sum, result) => sum + result.longFrameCount50, 0),
      longFrameCount100: results.reduce((sum, result) => sum + result.longFrameCount100, 0),
      totalFrameCount: results.reduce((sum, result) => sum + result.frameCount, 0),
      maxSettleDurationMs: Math.max(0, ...results.map((result) => result.settleDurationMs)),
      maxPost500BottomDistance: Math.max(0, ...results.map((result) => result.maxPost500BottomDistance)),
      maxScrollTopDelta: Math.max(0, ...results.map((result) => result.maxScrollTopDelta)),
      maxScrollHeightDelta: Math.max(0, ...results.map((result) => result.maxScrollHeightDelta)),
      maxContentHeightDelta: Math.max(0, ...results.map((result) => result.maxContentHeightDelta)),
      maxLastBottomDelta: Math.max(0, ...results.map((result) => result.maxLastBottomDelta)),
      blankTargetFrameCount: results.reduce((sum, result) => sum + result.blankTargetFrameCount, 0),
      wrongMessagePrefixFrameCount: results.reduce((sum, result) => sum + result.wrongMessagePrefixFrameCount, 0),
      post500BottomDetachFrames: results.reduce((sum, result) => sum + result.post500BottomDetachFrames, 0),
      layoutShiftCount: layoutShifts.length,
      layoutShiftScore: layoutShifts.reduce((sum, entry) => sum + entry.value, 0),
    };
    const output = {
      aggregate,
      results,
      layoutShiftSupported: !!shiftObserver,
      contentVisibility: config.contentVisibility,
      lowOverheadProfile: config.lowOverheadProfile,
      forcedMaterialization: config.forceMaterialize,
      layoutShifts: layoutShifts.slice(0, 30),
    };
    window.__locusChatSwitchStress = output;
    return output;
  })()`;
}

function cleanupSessionsExpression(ids: string[]) {
  return `(async () => {
    const ids = ${JSON.stringify(ids)};
    const { useChatStore } = await import('/src/stores/chat.ts');
    const store = useChatStore();
    for (const id of ids) {
      try { await store.deleteSession(id); } catch {}
    }
    await store.refreshSessions();
    return { deleted: ids.length };
  })()`;
}

function verifySwitchStability(result: Record<string, unknown>) {
  const aggregate = result.aggregate as Record<string, number> | undefined;
  const checks = {
    allSwitchesCaptured: Number(aggregate?.switchCount) >= 5,
    noBlankTargetFrames: Number(aggregate?.blankTargetFrameCount) === 0,
    noStaleSessionFrames: Number(aggregate?.wrongMessagePrefixFrameCount) === 0,
    noDeferredHeightMaterialization: Number(aggregate?.maxScrollHeightDelta) <= 0.5
      && Number(aggregate?.maxContentHeightDelta) <= 0.5,
    bottomStableAfter500ms: Number(aggregate?.maxPost500BottomDistance) <= 1
      && Number(aggregate?.post500BottomDetachFrames) === 0,
    settlesWithinWindow: Number(aggregate?.maxSettleDurationMs) <= options.settleMs - 100,
  };
  return { passed: Object.values(checks).every(Boolean), checks };
}

function printResult(result: Record<string, unknown>) {
  console.log(`LOCUS_CHAT_SWITCH_STRESS_JSON ${JSON.stringify(result)}`);
}

function printHelp() {
  console.log(`Usage:
  bun run locus:test:chat-switch -- --runtime-root <isolated-runtime-root>
  bun run locus:test:chat-switch -- --runtime-root <root> --verify --cleanup

The script creates five persisted long sessions in the isolated database, switches
between them through the real Chat store, and samples the WebView layout every frame.

Options:
  --runtime-root <dir>   Required isolated runtime root containing database/locus.db
  --browser-url <url>   WebView2 DevTools URL; auto-detected on ports 19222-19246
  --switch-count <n>    Number of session switches, default ${DEFAULT_SWITCH_COUNT}
  --settle-ms <ms>      Capture duration after each switch, default ${DEFAULT_SETTLE_MS}
  --timeout-ms <ms>     WebView and DOM timeout, default ${DEFAULT_TIMEOUT_MS}
  --viewport-width <px> Fixed CDP viewport width, default ${DEFAULT_VIEWPORT_WIDTH}
  --viewport-height <px> Fixed CDP viewport height, default ${DEFAULT_VIEWPORT_HEIGHT}
  --verify              Exit non-zero when the stability checks fail
  --profile-only        Print aggregate and CDP performance metrics without frame traces
  --content-visibility <production|auto|visible> Select production CSS or an A/B override
  --no-force-materialize Keep natural WebView2 content-visibility scheduling
  --cleanup             Delete generated sessions after capture`);
}
