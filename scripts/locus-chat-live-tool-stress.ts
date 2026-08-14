import path from "node:path";
import process from "node:process";
import { CdpClient, findLocusWebViewTarget } from "./locus-webview2-stress-client";

const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_VIEWPORT_WIDTH = 1600;
const DEFAULT_VIEWPORT_HEIGHT = 900;

interface CliOptions {
  browserUrl: string;
  runtimeRoot: string;
  timeoutMs: number;
  viewportWidth: number;
  viewportHeight: number;
  cleanup: boolean;
  verify: boolean;
}

const options = parseArgs(process.argv.slice(2));
assertIsolatedDatabase(options.runtimeRoot);
const target = await findLocusWebViewTarget(options.browserUrl, options.timeoutMs);
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl!);

try {
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width: options.viewportWidth,
    height: options.viewportHeight,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const result = await cdp.evaluate<Record<string, unknown>>(stressExpression(options));
  const verification = verifyLiveToolStress(result);
  console.log(`LOCUS_CHAT_LIVE_TOOL_STRESS_JSON ${JSON.stringify({
    action: "live-tool-stress",
    runtimeRoot: options.runtimeRoot,
    result,
    verification,
  })}`);
  if (options.verify && !verification.passed) process.exitCode = 1;
} finally {
  cdp.close();
}

function parseArgs(args: string[]): CliOptions {
  const parsed: CliOptions = {
    browserUrl: "",
    runtimeRoot: "",
    timeoutMs: DEFAULT_TIMEOUT_MS,
    viewportWidth: DEFAULT_VIEWPORT_WIDTH,
    viewportHeight: DEFAULT_VIEWPORT_HEIGHT,
    cleanup: false,
    verify: false,
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
    const [name, inlineValue = ""] = arg.split(/=(.*)/s, 2);
    const value = inlineValue || args[index + 1];
    if (name === "--runtime-root") {
      parsed.runtimeRoot = path.resolve(requireValue(name, value));
    } else if (name === "--browser-url") {
      parsed.browserUrl = requireValue(name, value);
    } else if (name === "--timeout-ms") {
      parsed.timeoutMs = positiveInteger(name, value, 1_000);
    } else if (name === "--viewport-width") {
      parsed.viewportWidth = positiveInteger(name, value, 800);
    } else if (name === "--viewport-height") {
      parsed.viewportHeight = positiveInteger(name, value, 600);
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
    if (!inlineValue) index += 1;
  }
  if (!parsed.runtimeRoot) {
    throw new Error("--runtime-root is required to guarantee an isolated Locus instance.");
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

function assertIsolatedDatabase(runtimeRoot: string) {
  const root = path.resolve(runtimeRoot);
  const databasePath = path.resolve(root, "database", "locus.db");
  if (!databasePath.toLowerCase().startsWith(`${root}${path.sep}`.toLowerCase())) {
    throw new Error(`Refusing database outside runtime root: ${databasePath}`);
  }
  if (!Bun.file(databasePath).size) {
    throw new Error(`Isolated Locus database is missing or empty: ${databasePath}`);
  }
}

function stressExpression(config: CliOptions) {
  return `(async () => {
    const config = ${JSON.stringify({ timeoutMs: config.timeoutMs, cleanup: config.cleanup })};
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
    const [{ useChatStore }, sessionService] = await Promise.all([
      import('/src/stores/chat.ts'),
      import('/src/services/session.ts'),
    ]);
    const store = useChatStore();
    const previousStress = window.__locusChatLiveToolStress;
    if (previousStress?.frameId) cancelAnimationFrame(previousStress.frameId);
    if (store.isStreaming && String(store.currentRunId || '').startsWith('live-tool-stress-')) {
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
    if (store.isStreaming) throw new Error('Stop the active response before running live-tool stress.');
    const sessionId = await sessionService.createSession({
      title: 'Stress · Live Tool Handoff',
      sessionType: 'chat',
    });
    await store.refreshSessions();
    await store.selectSession(sessionId, { persist: false });

    const runId = 'live-tool-stress-' + Date.now().toString(36);
    const state = {
      phase: 'setup',
      event: 'idle',
      frames: [],
      frameId: 0,
      startedAt: performance.now(),
      dynamicFirstToolId: null,
      targetExpandedToolId: null,
      sessionId,
    };
    window.__locusChatLiveToolStress = state;
    const emit = (event) => {
      state.event = event.type + (event.toolCallId ? ':' + event.toolCallId : '');
      const handled = store.handleStreamEvent({ runId, sessionId, ...event });
      if (!handled) throw new Error('Chat store rejected ' + event.type);
    };

    const readFrame = () => {
      const scroll = document.querySelector('.chat-transcript-scroll');
      const collection = state.dynamicFirstToolId
        ? [...document.querySelectorAll('.tool-call-collection')]
          .find((element) => (element.getAttribute('data-tool-layout-tool-call-ids') || '').includes(state.dynamicFirstToolId))
        : [...document.querySelectorAll('.tool-call-collection')].at(-1);
      const panel = collection?.querySelector('.tool-call-collection-panel');
      const blocks = collection ? [...collection.querySelectorAll('.tool-call-block')] : [];
      const firstBlockRect = blocks[0]?.getBoundingClientRect();
      const lastBlock = blocks.at(-1);
      const lastBlockRect = lastBlock?.getBoundingClientRect();
      const expandedBlocks = blocks.filter((block) => block.classList.contains('is-expanded'));
      const targetBlock = state.targetExpandedToolId
        ? blocks.find((block) => block.getAttribute('data-tool-layout-tool-call-ids') === state.targetExpandedToolId)
        : null;
      const collectionRect = collection?.getBoundingClientRect();
      const panelStyle = panel ? getComputedStyle(panel) : null;
      const runningToolSpinnerCount = document.querySelectorAll(
        '.tool-call-block.running .spinner-anim'
      ).length;
      const waitingIndicatorCount = document.querySelectorAll(
        '.tool-call-collection-waiting-status .chat-waiting-indicator, '
        + '.chat-transcript-tool-waiting-row .chat-waiting-indicator, '
        + '[data-render-part-kind="waiting"] .chat-waiting-indicator'
      ).length;
      return {
        t: Math.round(performance.now() - state.startedAt),
        phase: state.phase,
        event: state.event,
        isStreaming: store.isStreaming,
        activeToolCallCount: store.activeToolCalls.length,
        activeToolStatuses: store.activeToolCalls.slice(-3).map((tool) => tool.id + ':' + tool.status),
        liveRenderPartCount: store.liveRenderParts.length,
        messageCount: store.messages.length,
        toolCollectionCount: document.querySelectorAll('.tool-call-collection').length,
        toolBlockCount: blocks.length,
        firstToolExpanded: blocks[0]?.classList.contains('is-expanded') === true,
        expandedToolIds: expandedBlocks.map((block) => block.getAttribute('data-tool-layout-tool-call-ids')),
        targetToolExpanded: targetBlock?.classList.contains('is-expanded') === true,
        lastToolId: lastBlock?.getAttribute('data-tool-layout-tool-call-ids') ?? null,
        lastToolExpanded: lastBlock?.classList.contains('is-expanded') === true,
        lastToolHeight: lastBlockRect?.height ?? null,
        runningToolSpinnerCount,
        waitingIndicatorCount,
        doubleSpinner: runningToolSpinnerCount > 0 && waitingIndicatorCount > 0,
        collectionExpanded: collection?.getAttribute('data-tool-layout-expanded') ?? null,
        panelVisible: collection?.getAttribute('data-tool-layout-panel-visible') ?? null,
        panelLeaving: collection?.getAttribute('data-tool-layout-panel-leaving') ?? null,
        collectionHeight: collectionRect?.height ?? null,
        firstToolTop: firstBlockRect?.top ?? null,
        panelOpacity: panelStyle?.opacity ?? null,
        panelTransform: panelStyle?.transform ?? null,
        scrollTop: scroll?.scrollTop ?? null,
        scrollHeight: scroll?.scrollHeight ?? null,
        bottomDistance: scroll ? scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight : null,
      };
    };
    const sample = () => {
      state.frames.push(readFrame());
      state.frameId = requestAnimationFrame(sample);
    };
    state.frameId = requestAnimationFrame(sample);

    emit({ type: 'runStart' });
    emit({
      type: 'userMessage',
      message: {
        id: runId + '-user',
        role: 'user',
        content: '自动压测工具轮次交接与追加工具布局。',
        createdAt: Date.now() / 1000,
      },
    });

    const roundTool = (suffix, outcome) => ({
      id: runId + '-' + suffix,
      name: 'edit',
      arguments: JSON.stringify({
        filePath: 'Assets/Stress/LiveToolProbe.cs',
        oldString: 'private int value = 1;',
        newString: 'private int value = 2;',
      }),
      order: 10,
      outcome,
      recordedOutput: outcome ? 'Edited Assets/Stress/LiveToolProbe.cs [lines:12]' : undefined,
    });

    state.phase = 'control-completed-tool';
    const controlTool = roundTool('control', 'done');
    emit({
      type: 'toolCallStart',
      toolCallId: controlTool.id,
      toolName: controlTool.name,
      arguments: controlTool.arguments,
      order: 10,
      partId: controlTool.id,
      renderSeq: 10,
    });
    await sleep(180);
    emit({
      type: 'toolCallDone',
      toolCallId: controlTool.id,
      toolName: controlTool.name,
      output: controlTool.recordedOutput,
      outcome: 'done',
    });
    await sleep(100);
    const controlStart = state.frames.length;
    emit({
      type: 'toolCallRoundDone',
      messageId: runId + '-control-message',
      fullText: '',
      toolCalls: [controlTool],
    });
    await sleep(700);
    const controlEnd = state.frames.length;

    state.phase = 'missing-tool-done-handoff';
    const missingDoneTool = roundTool('missing-done', undefined);
    emit({
      type: 'toolCallStart',
      toolCallId: missingDoneTool.id,
      toolName: missingDoneTool.name,
      arguments: missingDoneTool.arguments,
      order: 20,
      partId: missingDoneTool.id,
      renderSeq: 20,
    });
    await sleep(180);
    const missingStart = state.frames.length;
    emit({
      type: 'toolCallRoundDone',
      messageId: runId + '-missing-message',
      fullText: '',
      toolCalls: [missingDoneTool],
    });
    await sleep(900);
    const missingEnd = state.frames.length;

    state.phase = 'background-tool-handoff';
    const backgroundTool = {
      ...roundTool('background', undefined),
      arguments: JSON.stringify({
        filePath: 'Assets/Stress/BackgroundProbe.cs',
        async: 'async',
      }),
      order: 25,
    };
    emit({
      type: 'toolCallStart',
      toolCallId: backgroundTool.id,
      toolName: backgroundTool.name,
      arguments: backgroundTool.arguments,
      order: backgroundTool.order,
      partId: backgroundTool.id,
      renderSeq: backgroundTool.order,
    });
    await sleep(180);
    const backgroundStart = state.frames.length;
    emit({
      type: 'toolCallRoundDone',
      messageId: runId + '-background-message',
      fullText: '',
      toolCalls: [backgroundTool],
    });
    await sleep(700);
    const backgroundEnd = state.frames.length;

    state.phase = 'append-tools-expanded';
    const dynamicTools = [];
    const initialDynamicToolCount = 16;
    const finalDynamicToolCount = 28;
    for (let index = 0; index < initialDynamicToolCount; index += 1) {
      const id = runId + '-dynamic-' + index;
      const name = index % 2 === 0 ? 'edit' : 'read';
      const args = JSON.stringify(name === 'edit' ? {
        filePath: 'Assets/Stress/DynamicProbe' + index + '.cs',
        oldString: 'value = ' + index + ';',
        newString: 'value = ' + (index + 1) + ';',
      } : {
        filePath: 'Assets/Stress/DynamicProbe' + index + '.cs',
        offset: 1,
        limit: 80,
      });
      dynamicTools.push({ id, name, arguments: args, order: 30 + index, outcome: 'done', recordedOutput: 'dynamic result ' + index });
      emit({ type: 'toolCallStart', toolCallId: id, toolName: name, arguments: args, order: 30 + index, partId: id, renderSeq: 30 + index });
      await sleep(55);
      emit({ type: 'toolCallDone', toolCallId: id, toolName: name, output: 'dynamic result ' + index, outcome: 'done' });
      await sleep(55);
    }

    const dynamicFirstToolId = runId + '-dynamic-0';
    state.dynamicFirstToolId = dynamicFirstToolId;
    const findDynamicCollection = () => [...document.querySelectorAll('.tool-call-collection')]
      .find((element) => (element.getAttribute('data-tool-layout-tool-call-ids') || '').includes(dynamicFirstToolId));
    const collection = await waitFor(
      () => findDynamicCollection(),
      'dynamic tool collection',
    );
    const targetExpandedToolId = runId + '-dynamic-' + (initialDynamicToolCount - 1);
    state.targetExpandedToolId = targetExpandedToolId;
    const targetBlock = [...collection.querySelectorAll('.tool-call-block')]
      .find((block) => block.getAttribute('data-tool-layout-tool-call-ids') === targetExpandedToolId);
    if (targetBlock && !targetBlock.classList.contains('is-expanded')) {
      targetBlock.querySelector('.tool-call-header')?.click();
    }
    await waitFor(
      () => [...collection.querySelectorAll('.tool-call-block')].find((block) =>
        block.getAttribute('data-tool-layout-tool-call-ids') === targetExpandedToolId
        && block.classList.contains('is-expanded')
      ),
      'expanded target dynamic tool',
    );
    await sleep(180);
    const appendStart = state.frames.length;
    let baselineCollection = collection;
    let collectionRemountCount = 0;
    const appendEvents = [];
    for (let index = initialDynamicToolCount; index < finalDynamicToolCount; index += 1) {
      const id = runId + '-dynamic-' + index;
      const name = index % 2 === 0 ? 'edit' : 'read';
      const args = JSON.stringify(name === 'edit' ? {
        filePath: 'Assets/Stress/DynamicProbe' + index + '.cs',
        oldString: 'value = ' + index + ';',
        newString: 'value = ' + (index + 1) + ';',
      } : {
        filePath: 'Assets/Stress/DynamicProbe' + index + '.cs', offset: 1, limit: 120,
      });
      const beforeFrame = state.frames.length;
      emit({ type: 'toolCallStart', toolCallId: id, toolName: name, arguments: args, order: 30 + index, partId: id, renderSeq: 30 + index });
      await sleep(120);
      const beforeDoneFrame = state.frames.length;
      const currentCollection = findDynamicCollection();
      if (currentCollection && currentCollection !== baselineCollection) {
        collectionRemountCount += 1;
        baselineCollection = currentCollection;
      }
      emit({
        type: 'toolCallDone',
        toolCallId: id,
        toolName: name,
        output: index % 3 === 0 ? 'Found multiple matches for oldString.' : 'dynamic result ' + index,
        outcome: index % 3 === 0 ? 'error' : 'done',
      });
      await sleep(180);
      appendEvents.push({ id, name, beforeFrame, beforeDoneFrame, afterFrame: state.frames.length });
    }
    const appendEnd = state.frames.length;
    await sleep(300);
    cancelAnimationFrame(state.frameId);

    const completionRowHeightDeltas = appendEvents.map((event) => {
      const runningFrame = state.frames
        .slice(event.beforeFrame, event.beforeDoneFrame)
        .filter((frame) => frame.lastToolId === event.id && typeof frame.lastToolHeight === 'number')
        .at(-1);
      const completedFrame = state.frames
        .slice(event.beforeDoneFrame, event.afterFrame)
        .filter((frame) => frame.lastToolId === event.id && typeof frame.lastToolHeight === 'number')
        .at(-1);
      return {
        id: event.id,
        runningHeight: runningFrame?.lastToolHeight ?? null,
        completedHeight: completedFrame?.lastToolHeight ?? null,
        delta: runningFrame && completedFrame
          ? Math.abs(completedFrame.lastToolHeight - runningFrame.lastToolHeight)
          : null,
      };
    });

    const summarizeWindow = (start, end) => {
      const frames = state.frames.slice(start, end);
      let maxScrollDelta = 0;
      let maxHeightDelta = 0;
      let maxToolTopDelta = 0;
      for (let index = 1; index < frames.length; index += 1) {
        const previous = frames[index - 1];
        const current = frames[index];
        maxScrollDelta = Math.max(maxScrollDelta, Math.abs((current.scrollTop ?? 0) - (previous.scrollTop ?? 0)));
        maxHeightDelta = Math.max(maxHeightDelta, Math.abs((current.collectionHeight ?? 0) - (previous.collectionHeight ?? 0)));
        maxToolTopDelta = Math.max(maxToolTopDelta, Math.abs((current.firstToolTop ?? 0) - (previous.firstToolTop ?? 0)));
      }
      return {
        frameCount: frames.length,
        doubleSpinnerFrameCount: frames.filter((frame) => frame.doubleSpinner).length,
        waitingFrameCount: frames.filter((frame) => frame.waitingIndicatorCount > 0).length,
        runningToolFrameCount: frames.filter((frame) => frame.runningToolSpinnerCount > 0).length,
        collectionMissingFrameCount: frames.filter((frame) => frame.toolCollectionCount === 0).length,
        collapsedFrameCount: frames.filter((frame) => frame.collectionExpanded === 'false').length,
        firstToolCollapsedFrameCount: frames.filter((frame) =>
          frame.toolBlockCount > 0 && !frame.targetToolExpanded
        ).length,
        panelHiddenFrameCount: frames.filter((frame) => frame.panelVisible === 'false').length,
        panelTransitionFrameCount: frames.filter((frame) =>
          frame.panelLeaving === 'true'
          || (frame.panelOpacity !== null && frame.panelOpacity !== '1')
          || (frame.panelTransform !== null && frame.panelTransform !== 'none')
        ).length,
        maxScrollDelta,
        maxHeightDelta,
        maxToolTopDelta,
        firstDoubleSpinnerFrame: frames.find((frame) => frame.doubleSpinner) ?? null,
        trace: frames.slice(0, 8),
      };
    };

    const result = {
      sessionId,
      control: summarizeWindow(controlStart, controlEnd),
      missingToolDone: summarizeWindow(missingStart, missingEnd),
      backgroundTool: summarizeWindow(backgroundStart, backgroundEnd),
      appendTools: {
        ...summarizeWindow(appendStart, appendEnd),
        collectionRemountCount,
        appendEvents,
        completionRowHeightDeltas,
        maxCompletionRowHeightDelta: Math.max(
          0,
          ...completionRowHeightDeltas.map((item) => item.delta ?? 0),
        ),
      },
      finalStoreState: {
        isStreaming: store.isStreaming,
        activeToolCallCount: store.activeToolCalls.length,
        messageCount: store.messages.length,
      },
    };

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
    if (config.cleanup) {
      await store.deleteSession(sessionId);
    }
    return result;
  })()`;
}

function verifyLiveToolStress(result: Record<string, unknown>) {
  const control = result.control as Record<string, number> | undefined;
  const missing = result.missingToolDone as Record<string, number> | undefined;
  const background = result.backgroundTool as Record<string, number> | undefined;
  const append = result.appendTools as Record<string, number> | undefined;
  const checks = {
    controlHasNoDoubleSpinner: Number(control?.doubleSpinnerFrameCount) === 0,
    missingDoneHasNoDoubleSpinner: Number(missing?.doubleSpinnerFrameCount) === 0,
    missingDoneShowsOnlyWaiting: Number(missing?.waitingFrameCount) > 0
      && Number(missing?.runningToolFrameCount) === 0,
    backgroundToolShowsOnlyToolSpinner: Number(background?.doubleSpinnerFrameCount) === 0
      && Number(background?.waitingFrameCount) === 0
      && Number(background?.runningToolFrameCount) > 0,
    expandedCollectionStaysMounted: Number(append?.collectionRemountCount) === 0,
    expandedToolStaysExpanded: Number(append?.firstToolCollapsedFrameCount) === 0,
    expandedCollectionDoesNotAnimateOnAppend: Number(append?.panelTransitionFrameCount) === 0,
    expandedToolAnchorStaysFixed: Number(append?.maxToolTopDelta) <= 0.5,
    completionRowHeightStaysFixed: Number(append?.maxCompletionRowHeightDelta) <= 0.5,
  };
  return { passed: Object.values(checks).every(Boolean), checks };
}

function printHelp() {
  console.log(`Usage:
  bun run locus:test:chat-live-tools -- --runtime-root <isolated-runtime-root>
  bun run locus:test:chat-live-tools -- --runtime-root <root> --verify --cleanup

Scenarios:
  1. completed tool -> tool round handoff (control)
  2. missing toolCallDone -> settled tool handoff plus waiting response
  3. background tool handoff -> running tool spinner without response waiting
  4. keep a tool batch expanded while twelve new tools are appended

Options:
  --runtime-root <dir>   Required isolated runtime root containing database/locus.db
  --browser-url <url>   WebView2 DevTools URL; auto-detected on ports 19222-19246
  --timeout-ms <ms>     WebView and DOM timeout, default ${DEFAULT_TIMEOUT_MS}
  --viewport-width <px> Fixed CDP viewport width, default ${DEFAULT_VIEWPORT_WIDTH}
  --viewport-height <px> Fixed CDP viewport height, default ${DEFAULT_VIEWPORT_HEIGHT}
  --verify              Exit non-zero when expected invariants fail
  --cleanup             Delete the generated session after capture`);
}
