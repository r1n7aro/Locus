import path from "node:path";
import process from "node:process";
import { CdpClient, sleep } from "./locus-webview2-stress-client";

interface DevtoolsTarget {
  id: string;
  type: string;
  url: string;
  title?: string;
  webSocketDebuggerUrl?: string;
}

interface ScreenPoint {
  x: number;
  y: number;
}

interface DragPoint extends ScreenPoint {
  clientX: number;
  clientY: number;
  screenX: number;
  screenY: number;
  scale: number;
  anchorX: number;
  anchorY: number;
  tabWidth: number;
  tabHeight: number;
}

interface NativePreviewSnapshot {
  width: number;
  height: number;
  pointerOffsetX: number;
  pointerOffsetY: number;
  capturePath?: string;
  detection?: string;
}

const browserUrl = argument("--browser-url")?.replace(/\/$/, "") || "http://127.0.0.1:19222";
const devSourceUrl = argument("--dev-source-url")?.replace(/\/$/, "") || "http://localhost:14901";
const runtimeRoot = path.resolve(argument("--runtime-root") || "");
const verify = process.argv.includes("--verify");
const restoreOnly = process.argv.includes("--restore-only");
const verifyNativePreview = !process.argv.includes("--skip-native-preview");
const INTERNAL_DRAG_PREVIEW_WIDTH = 228;
const INTERNAL_DRAG_PREVIEW_HEIGHT = 34;
if (!runtimeRoot.toLocaleLowerCase().startsWith("e:\\locustemp\\")) {
  throw new Error("--runtime-root must point to the isolated E:\\LocusTemp instance.");
}

const mainTarget = await waitForTarget(
  isMainPageTarget,
  60_000,
);
const main = await CdpClient.connect(mainTarget.webSocketDebuggerUrl!);

if (restoreOnly) {
  const result = await verifyRestoredWorkbenchWindows(main, 30_000);
  console.log(`LOCUS_WORKBENCH_WINDOW_RESTORE_CDP_JSON ${JSON.stringify({ runtimeRoot, ...result })}`);
  main.close();
  if (verify && !result.restored) process.exit(1);
  process.exit(0);
}

try {
  await main.send("Runtime.enable");
  await waitForEvaluation(main, `!!document.querySelector('.development-workbench')`, 30_000);
  await waitForEvaluation(
    main,
    `!!window.__LOCUS_SHARED_WORKBENCH_STATE__?.poolLabel`,
    30_000,
  );
  await main.evaluate(`(async () => {
    const { setCurrentTauriWindowBounds } = await import(
      '${devSourceUrl}/src/services/tauriRuntime.ts'
    );
    await setCurrentTauriWindowBounds({ x: 320, y: 180, width: 1200, height: 800 });
    return true;
  })()`);
  await sleep(250);
  await main.evaluate(`window.__LOCUS_WORKBENCH_WINDOW_METRICS__ = []; true`);

  const prepared = await main.evaluate<{ firstId: string; secondId: string }>(`(async () => {
    const [{ useWorkbenchStore, createWorkbenchEditorInput }, { useWorkspaceContextStore }] = await Promise.all([
      import('${devSourceUrl}/src/stores/workbench.ts'),
      import('${devSourceUrl}/src/stores/workspaceContext.ts'),
    ]);
    const appContext = document.querySelector('#app')?.__vue_app__?._context;
    const provides = appContext?.provides || {};
    const pinia = Reflect.ownKeys(provides).map((key) => provides[key]).find((value) => (
      value && value._s instanceof Map && value.state
    ));
    if (!pinia) throw new Error('The active Pinia instance is unavailable.');
    const workbench = useWorkbenchStore(pinia);
    const contexts = useWorkspaceContextStore(pinia);
    const project = contexts.focusedProject || Object.values(contexts.projectsById)[0];
    if (!project) throw new Error('No isolated project context is available.');
    const checkout = contexts.focusedCheckout || project.checkouts[0];
    const state = workbench.ensureWindow('main');
    const paneId = state.focusedPaneId;
    const first = workbench.openEditor('main', createWorkbenchEditorInput({
      kind: 'project', projectId: project.projectId,
    }, 'CDP Project', {
      preview: false,
      pinned: true,
      checkoutBinding: checkout ? { checkoutId: checkout.checkoutId } : null,
    }), { paneId, preview: false, pinned: true, replacePreview: false });
    const second = workbench.openEditor('main', createWorkbenchEditorInput({
      kind: 'checkout', projectId: project.projectId, checkoutId: checkout.checkoutId,
    }, 'CDP Checkout', {
      preview: false,
      pinned: true,
      checkoutBinding: { checkoutId: checkout.checkoutId },
    }), { paneId, preview: false, pinned: true, replacePreview: false });
    return { firstId: first.editorId, secondId: second.editorId };
  })()`);

  await waitForEvaluation(
    main,
    `document.querySelectorAll('[data-workbench-tab-id]').length >= 2`,
    10_000,
  );
  await main.evaluate(`(() => {
    document.querySelectorAll('.banner-close').forEach((button) => button.click());
    window.__LOCUS_NATIVE_POINTER_TRACE_ABORT__?.abort();
    window.__LOCUS_NATIVE_POINTER_TRACE_ABORT__ = new AbortController();
    window.__LOCUS_NATIVE_POINTER_TRACE__ = [];
    for (const type of ['pointerdown', 'pointermove', 'pointerup', 'mousedown', 'mousemove', 'mouseup', 'click']) {
      window.addEventListener(type, (event) => {
        const trace = window.__LOCUS_NATIVE_POINTER_TRACE__;
        trace.push({
          type,
          x: event.clientX,
          y: event.clientY,
          buttons: event.buttons,
          target: event.target?.getAttribute?.('data-workbench-tab-id') || event.target?.className || event.target?.tagName,
        });
        if (trace.length > 80) trace.shift();
      }, { capture: true, signal: window.__LOCUS_NATIVE_POINTER_TRACE_ABORT__.signal });
    }
    return true;
  })()`);
  const firstStart = await elementDragPoint(main, `[data-workbench-tab-id="${prepared.firstId}"]`);
  const outside = await findFreeDetachPoint(main);
  const nativePreview = await cdpDrag(main, firstStart, outside, 360, verifyNativePreview);

  const auxiliary = await waitForTargetWithTab(prepared.firstId, 15_000);
  await waitForEvaluation(
    main,
    `!document.querySelector('[data-workbench-tab-id="${prepared.firstId}"]')`,
    10_000,
  );
  await main.evaluate(`(async () => {
    const { setSharedWorkbenchWindowBounds } = await import(
      '${devSourceUrl}/src/services/sharedWorkbenchWindow.ts'
    );
    return setSharedWorkbenchWindowBounds(
      ${JSON.stringify(sharedWorkbenchLabelFromUrl(auxiliary.target.url))},
      { x: 0, y: 0, width: 320, height: 240 },
    );
  })()`);
  await sleep(180);

  const secondStart = await elementDragPoint(main, `[data-workbench-tab-id="${prepared.secondId}"]`);
  const auxiliaryTabTarget = await elementScreenPoint(
    auxiliary.client,
    `[data-workbench-tab-id="${prepared.firstId}"]`,
    "tail",
  );
  await cdpDrag(main, secondStart, auxiliaryTabTarget, 360);
  await waitForEvaluation(
    auxiliary.client,
    `document.querySelectorAll('[data-workbench-tab-id]').length >= 2`,
    10_000,
  );

  const returnStart = await elementDragPoint(
    auxiliary.client,
    `[data-workbench-tab-id="${prepared.firstId}"]`,
  );
  const mainEditorTarget = await elementExposedScreenPoint(main, ".workbench-editor-group");
  await cdpDrag(auxiliary.client, returnStart, mainEditorTarget, 360);
  await waitForEvaluation(
    main,
    `!!document.querySelector('[data-workbench-tab-id="${prepared.firstId}"]')`,
    10_000,
  );

  const screenshots = {
    main: path.join(runtimeRoot, "logs", "workbench-window-main.png"),
    auxiliary: path.join(runtimeRoot, "logs", "workbench-window-auxiliary.png"),
  };
  await captureScreenshot(auxiliary.client, screenshots.auxiliary);
  const mergedIntoAuxiliary = await auxiliary.client.evaluate(
    `document.querySelectorAll('[data-workbench-tab-id]').length >= 1`,
  );
  const lastTabStart = await elementDragPoint(
    auxiliary.client,
    `[data-workbench-tab-id="${prepared.secondId}"]`,
  );
  const finalMainEditorTarget = await elementExposedScreenPoint(main, ".workbench-editor-group");
  await cdpDrag(auxiliary.client, lastTabStart, finalMainEditorTarget, 360);
  await waitForEvaluation(
    main,
    `!!document.querySelector('[data-workbench-tab-id="${prepared.secondId}"]')`,
    10_000,
  );
  const lastTabWindowClosed = await waitForTargetToDisappear(auxiliary.target.id, 10_000);

  const mainMetrics = await main.evaluate<Array<Record<string, unknown>>>(
    `window.__LOCUS_WORKBENCH_WINDOW_METRICS__ || []`,
  );
  const claimed = [...mainMetrics].reverse().find((metric) => metric.name === "shared-pool-claimed");
  const shown = mainMetrics.find((metric) => (
    metric.name === "shared-window-shown" && metric.token === claimed?.token
  ));
  const durationMs = typeof shown?.durationMs === "number" ? shown.durationMs : null;
  const contentReady = mainMetrics.find((metric) => (
    metric.name === "window-content-ready" && metric.token === claimed?.token
  ));
  const contentReadyDurationMs = typeof contentReady?.durationMs === "number"
    ? contentReady.durationMs
    : null;
  const dragSummaries = mainMetrics.filter(
    (metric) => metric.name === "externalized-drag-summary",
  );
  const maximumDecisionMs = dragSummaries.reduce((maximum, metric) => {
    const detail = metric.detail as Record<string, unknown> | undefined;
    const decision = typeof detail?.maxDecisionMs === "number" ? detail.maxDecisionMs : 0;
    return Math.max(maximum, decision);
  }, 0);
  const verification = {
    detached: true,
    mergedIntoAuxiliary,
    returnedToMain: true,
    lastTabReturnedToMain: true,
    lastTabWindowClosed,
    pooledWindowShownDurationMs: durationMs,
    pooledWindowContentReadyDurationMs: contentReadyDurationMs,
    performancePassed: durationMs !== null && durationMs <= 100,
    contentReadyPassed: contentReadyDurationMs !== null && contentReadyDurationMs <= 180,
    nativeDragMaximumDecisionMs: maximumDecisionMs,
    nativeDragDecisionPassed: dragSummaries.length >= 4 && maximumDecisionMs <= 4,
    nativePreviewSize: nativePreview
      ? { width: nativePreview.width, height: nativePreview.height }
      : null,
    nativePreviewPointerOffset: nativePreview
      ? { x: nativePreview.pointerOffsetX, y: nativePreview.pointerOffsetY }
      : null,
    nativePreviewVerification: verifyNativePreview ? "verified" : "skipped",
    nativePreviewPassed: !verifyNativePreview || (!!nativePreview
      && Math.abs(nativePreview.width - Math.round(INTERNAL_DRAG_PREVIEW_WIDTH * firstStart.scale)) <= 2
      && Math.abs(nativePreview.height - Math.round(INTERNAL_DRAG_PREVIEW_HEIGHT * firstStart.scale)) <= 2
      && Math.abs(nativePreview.pointerOffsetX - Math.round(firstStart.anchorX * firstStart.scale)) <= 2
      && Math.abs(nativePreview.pointerOffsetY - Math.round(firstStart.anchorY * firstStart.scale)) <= 2),
  };
  await sleep(500);
  await captureScreenshot(main, screenshots.main);
  console.log(`LOCUS_WORKBENCH_WINDOW_CDP_JSON ${JSON.stringify({
    runtimeRoot,
    auxiliaryTarget: auxiliary.target.url,
    mainMetrics,
    metrics: mainMetrics,
    screenshots,
    verification,
  })}`);
  if (verify && !Object.values(verification).every((value) => value !== false)) {
    process.exitCode = 1;
  }
  auxiliary.client.close();
} catch (error) {
  const targetDiagnostics: unknown[] = [];
  for (const target of await listTargets()) {
    if (!target.webSocketDebuggerUrl) continue;
    const client = await CdpClient.connect(target.webSocketDebuggerUrl).catch(() => null);
    if (!client) continue;
    const state = await client.evaluate(`({
      url: location.href,
      screen: { x: window.screenX, y: window.screenY, width: window.outerWidth, height: window.outerHeight, scale: window.devicePixelRatio },
      active: document.activeElement?.getAttribute?.('data-workbench-tab-id') || document.activeElement?.tagName || null,
      tabs: [...document.querySelectorAll('[data-workbench-tab-id]')].map((element) => ({
        id: element.getAttribute('data-workbench-tab-id'),
        selected: element.getAttribute('aria-selected'),
        rect: element.getBoundingClientRect().toJSON(),
      })),
      dragging: document.body.classList.contains('is-internal-dragging'),
      notices: [...document.querySelectorAll('.banner-msg')].map((element) => element.textContent),
      metrics: window.__LOCUS_WORKBENCH_WINDOW_METRICS__ || [],
      pointerTrace: window.__LOCUS_NATIVE_POINTER_TRACE__ || [],
    })`).catch((diagnosticError) => ({ error: String(diagnosticError) }));
    targetDiagnostics.push(state);
    client.close();
  }
  console.error(`LOCUS_WORKBENCH_WINDOW_CDP_DIAGNOSTICS ${JSON.stringify(targetDiagnostics)}`);
  throw error;
} finally {
  main.close();
}

function argument(name: string): string | null {
  const index = process.argv.indexOf(name);
  if (index >= 0) return process.argv[index + 1] ?? null;
  const inline = process.argv.find((value) => value.startsWith(`${name}=`));
  return inline?.slice(name.length + 1) ?? null;
}

async function listTargets(): Promise<DevtoolsTarget[]> {
  try {
    const response = await fetch(`${browserUrl}/json/list`, {
      signal: AbortSignal.timeout(600),
    });
    return response.ok ? await response.json() as DevtoolsTarget[] : [];
  } catch {
    return [];
  }
}

function isMainPageTarget(target: DevtoolsTarget): boolean {
  return target.type === "page" && (
    target.id === "main"
    || target.url === "http://tauri.localhost/"
    || /^http:\/\/(?:localhost|127\.0\.0\.1):\d+\/$/.test(target.url)
  );
}

function sharedWorkbenchLabelFromUrl(url: string): string {
  const marker = "#locus-shared-workbench-";
  const index = url.indexOf(marker);
  return index >= 0 ? decodeURIComponent(url.slice(index + marker.length)) : "";
}

async function verifyRestoredWorkbenchWindows(
  main: CdpClient,
  timeoutMs: number,
): Promise<{
  restored: boolean;
  registryLabels: string[];
  restoredTargets: Array<{ url: string; tabCount: number; tabIds: string[] }>;
}> {
  await main.send("Runtime.enable");
  const deadline = Date.now() + timeoutMs;
  let registryLabels: string[] = [];
  let restoredTargets: Array<{ url: string; tabCount: number; tabIds: string[] }> = [];
  while (Date.now() < deadline) {
    registryLabels = await main.evaluate<string[]>(`(() => {
      try {
        const records = JSON.parse(localStorage.getItem('locus:workbench-aux-windows:v1') || '[]');
        return Array.isArray(records) ? records.map((record) => record.label).filter(Boolean) : [];
      } catch { return []; }
    })()`).catch(() => []);
    restoredTargets = [];
    for (const target of await listTargets()) {
      if (!target.webSocketDebuggerUrl || isMainPageTarget(target)) continue;
      const client = await CdpClient.connect(target.webSocketDebuggerUrl).catch(() => null);
      if (!client) continue;
      const tabIds = await client.evaluate<string[]>(
        `[...document.querySelectorAll('[data-workbench-tab-id]')].map((element) => element.getAttribute('data-workbench-tab-id')).filter(Boolean)`,
      ).catch(() => []);
      client.close();
      if (tabIds.length) restoredTargets.push({ url: target.url, tabCount: tabIds.length, tabIds });
    }
    if (registryLabels.length && restoredTargets.length) {
      return { restored: true, registryLabels, restoredTargets };
    }
    await sleep(120);
  }
  return { restored: false, registryLabels, restoredTargets };
}

async function captureScreenshot(client: CdpClient, outputPath: string): Promise<void> {
  const response = await client.send("Page.captureScreenshot", {
    format: "png",
    fromSurface: true,
    captureBeyondViewport: false,
  }) as { data?: string };
  if (!response.data) throw new Error(`CDP screenshot is empty: ${outputPath}`);
  await Bun.write(outputPath, Buffer.from(response.data, "base64"));
}

async function waitForTarget(
  predicate: (target: DevtoolsTarget) => boolean,
  timeoutMs: number,
): Promise<DevtoolsTarget> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const target = (await listTargets()).find(
      (candidate) => !!candidate.webSocketDebuggerUrl && predicate(candidate),
    );
    if (target) return target;
    await sleep(120);
  }
  throw new Error(`CDP target did not appear within ${timeoutMs}ms.`);
}

async function waitForTargetToDisappear(targetId: string, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await listTargets()).some((target) => target.id === targetId)) return true;
    await sleep(100);
  }
  return false;
}

async function waitForEvaluation(
  client: CdpClient,
  expression: string,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await client.evaluate<boolean>(expression).catch(() => false)) return;
    await sleep(80);
  }
  throw new Error(`CDP condition timed out: ${expression}`);
}

async function findFreeDetachPoint(main: CdpClient): Promise<ScreenPoint> {
  const screen = await main.evaluate<{
    left: number;
    top: number;
    right: number;
    bottom: number;
    scale: number;
  }>(`(() => {
    const scale = window.devicePixelRatio || 1;
    return {
      left: Math.round((window.screen.availLeft || 0) * scale),
      top: Math.round((window.screen.availTop || 0) * scale),
      right: Math.round(((window.screen.availLeft || 0) + window.screen.availWidth) * scale),
      bottom: Math.round(((window.screen.availTop || 0) + window.screen.availHeight) * scale),
      scale,
    };
  })()`);
  const occupied: Array<{ left: number; top: number; right: number; bottom: number }> = [];
  for (const target of await listTargets()) {
    if (!target.webSocketDebuggerUrl) continue;
    const client = await CdpClient.connect(target.webSocketDebuggerUrl).catch(() => null);
    if (!client) continue;
    const rect = await client.evaluate<{
      tabCount: number;
      isMain: boolean;
      left: number;
      top: number;
      right: number;
      bottom: number;
    }>(`(async () => {
      const scale = window.devicePixelRatio || 1;
      const { getCurrentTauriWindowPhysicalBounds } = await import(
        '${devSourceUrl}/src/services/tauriRuntime.ts'
      );
      const bounds = await getCurrentTauriWindowPhysicalBounds();
      const contentInsetX = Math.max(0, (bounds.width - window.outerWidth * scale) / 2);
      return {
        tabCount: document.querySelectorAll('[data-workbench-tab-id]').length,
        isMain: /^http:\\/\\/localhost:\\d+\\/$/.test(location.href),
        left: Math.round(bounds.x + contentInsetX),
        top: Math.round(bounds.y),
        right: Math.round(bounds.x + contentInsetX + window.outerWidth * scale),
        bottom: Math.round(bounds.y + window.outerHeight * scale),
      };
    })()`).catch(() => null);
    client.close();
    if (rect && (rect.isMain || rect.tabCount > 0)) occupied.push(rect);
  }
  const deterministicCorner = { x: 160, y: 100 };
  if (!occupied.some((rect) => (
    deterministicCorner.x >= rect.left
    && deterministicCorner.x <= rect.right
    && deterministicCorner.y >= rect.top
    && deterministicCorner.y <= rect.bottom
  ))) return deterministicCorner;
  const margin = Math.max(12, Math.round(12 * screen.scale));
  const xs = [screen.left + margin, screen.right - margin];
  const ys = [screen.top + margin, screen.bottom - margin];
  for (const y of ys) {
    for (const x of xs) {
      if (!occupied.some((rect) => (
        x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
      ))) return { x, y };
    }
  }
  throw new Error("No free screen point is available for detaching a Workbench tab.");
}

async function waitForTargetWithTab(
  editorId: string,
  timeoutMs: number,
): Promise<{ target: DevtoolsTarget; client: CdpClient }> {
  const deadline = Date.now() + timeoutMs;
  const visited = new Map<string, CdpClient>();
  while (Date.now() < deadline) {
    for (const target of await listTargets()) {
      if (!target.webSocketDebuggerUrl || isMainPageTarget(target)) continue;
      let client = visited.get(target.id);
      if (!client) {
        client = await CdpClient.connect(target.webSocketDebuggerUrl).catch(() => null as never);
        if (!client) continue;
        visited.set(target.id, client);
      }
      const found = await client.evaluate<boolean>(
        `!!document.querySelector('[data-workbench-tab-id="${editorId}"]')`,
      ).catch(() => false);
      if (found) {
        for (const [id, candidate] of visited) if (id !== target.id) candidate.close();
        return { target, client };
      }
    }
    await sleep(100);
  }
  for (const client of visited.values()) client.close();
  throw new Error("Detached Workbench target did not receive the editor tab.");
}

async function elementScreenPoint(
  client: CdpClient,
  selector: string,
  placement: "center" | "tail" | "right" = "center",
): Promise<ScreenPoint> {
  const point = await client.evaluate<ScreenPoint | null>(`(async () => {
    const element = document.querySelector(${JSON.stringify(selector)});
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    const placement = ${JSON.stringify(placement)};
    const clientX = placement === 'tail'
      ? rect.right - 8
      : placement === 'right'
        ? rect.right - 24
        : rect.left + rect.width / 2;
    const clientY = rect.top + rect.height / 2;
    const scale = window.devicePixelRatio || 1;
    const { getCurrentTauriWindowPhysicalBounds } = await import(
      '${devSourceUrl}/src/services/tauriRuntime.ts'
    );
    const bounds = await getCurrentTauriWindowPhysicalBounds();
    const contentInsetX = Math.max(0, (bounds.width - window.outerWidth * scale) / 2);
    return {
      x: Math.round(bounds.x + contentInsetX + clientX * scale),
      y: Math.round(bounds.y + clientY * scale),
    };
  })()`);
  if (!point) throw new Error(`Element is unavailable: ${selector}`);
  return point;
}

async function elementDragPoint(
  client: CdpClient,
  selector: string,
  placement: "center" | "tail" = "center",
): Promise<DragPoint> {
  const point = await client.evaluate<DragPoint | null>(`(async () => {
    const element = document.querySelector(${JSON.stringify(selector)});
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    const clientX = ${JSON.stringify(placement)} === 'tail' ? rect.right - 8 : rect.left + rect.width / 2;
    const clientY = rect.top + rect.height / 2;
    const scale = window.devicePixelRatio || 1;
    const { getCurrentTauriWindowPhysicalBounds } = await import(
      '${devSourceUrl}/src/services/tauriRuntime.ts'
    );
    const bounds = await getCurrentTauriWindowPhysicalBounds();
    const contentInsetX = Math.max(0, (bounds.width - window.outerWidth * scale) / 2);
    return {
      clientX,
      clientY,
      screenX: window.screenX,
      screenY: window.screenY,
      scale,
      anchorX: Math.max(8, Math.min(220, clientX - rect.left)),
      anchorY: Math.max(6, Math.min(28, clientY - rect.top)),
      tabWidth: element.closest('[data-locus-tab-shell]')?.getBoundingClientRect().width || rect.width,
      tabHeight: element.closest('[data-locus-tab-shell]')?.getBoundingClientRect().height || rect.height,
      x: Math.round(bounds.x + contentInsetX + clientX * scale),
      y: Math.round(bounds.y + clientY * scale),
    };
  })()`);
  if (!point) throw new Error(`Element is unavailable: ${selector}`);
  return point;
}

async function elementExposedScreenPoint(
  client: CdpClient,
  selector: string,
): Promise<ScreenPoint> {
  const candidates = await client.evaluate<ScreenPoint[] | null>(`(async () => {
    const element = document.querySelector(${JSON.stringify(selector)});
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    const scale = window.devicePixelRatio || 1;
    const { getCurrentTauriWindowPhysicalBounds } = await import(
      '${devSourceUrl}/src/services/tauriRuntime.ts'
    );
    const bounds = await getCurrentTauriWindowPhysicalBounds();
    const contentInsetX = Math.max(0, (bounds.width - window.outerWidth * scale) / 2);
    const fractions = [0.5, 0.85, 0.15, 0.7, 0.3];
    return fractions.flatMap((yFraction) => fractions.map((xFraction) => ({
      x: Math.round(bounds.x + contentInsetX + (rect.left + rect.width * xFraction) * scale),
      y: Math.round(bounds.y + (rect.top + rect.height * yFraction) * scale),
    })));
  })()`);
  if (!candidates?.length) throw new Error(`Element is unavailable: ${selector}`);
  const occupied: Array<{ left: number; top: number; right: number; bottom: number }> = [];
  for (const target of await listTargets()) {
    if (!target.webSocketDebuggerUrl || isMainPageTarget(target)) continue;
    const targetClient = await CdpClient.connect(target.webSocketDebuggerUrl).catch(() => null);
    if (!targetClient) continue;
    const rect = await targetClient.evaluate<{
      tabCount: number;
      left: number;
      top: number;
      right: number;
      bottom: number;
    }>(`(async () => {
      const scale = window.devicePixelRatio || 1;
      const { getCurrentTauriWindowPhysicalBounds } = await import(
        '${devSourceUrl}/src/services/tauriRuntime.ts'
      );
      const bounds = await getCurrentTauriWindowPhysicalBounds();
      const contentInsetX = Math.max(0, (bounds.width - window.outerWidth * scale) / 2);
      return {
        tabCount: document.querySelectorAll('[data-workbench-tab-id]').length,
        left: Math.round(bounds.x + contentInsetX),
        top: Math.round(bounds.y),
        right: Math.round(bounds.x + contentInsetX + window.outerWidth * scale),
        bottom: Math.round(bounds.y + window.outerHeight * scale),
      };
    })()`).catch(() => null);
    targetClient.close();
    if (rect?.tabCount) occupied.push(rect);
  }
  const exposed = candidates.find((point) => !occupied.some((rect) => (
    point.x >= rect.left && point.x <= rect.right
      && point.y >= rect.top && point.y <= rect.bottom
  )));
  if (!exposed) throw new Error("The main Workbench has no exposed editor area for the return drag.");
  return exposed;
}

async function cdpDrag(
  _client: CdpClient,
  start: DragPoint,
  end: ScreenPoint,
  durationMs: number,
  verifyNativePreview = false,
): Promise<NativePreviewSnapshot | null> {
  const helper = path.resolve("scripts/locus-native-pointer-drag.ps1");
  const baselinePath = verifyNativePreview
    ? await captureDesktopBaseline()
    : null;
  const command = [
    "powershell.exe",
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", helper,
    "-StartX", String(start.x),
    "-StartY", String(start.y),
    "-EndX", String(end.x),
    "-EndY", String(end.y),
    "-DurationMs", String(durationMs),
    "-RuntimeRoot", runtimeRoot,
  ];
  if (verifyNativePreview) command.push("-HoldBeforeReleaseMs", "3000");
  const child = Bun.spawn(command, { stdout: "pipe", stderr: "pipe" });
  let nativePreview: NativePreviewSnapshot | null = null;
  if (verifyNativePreview) {
    await sleep(durationMs + 500);
    nativePreview = await inspectNativePreview(
      Math.round(INTERNAL_DRAG_PREVIEW_WIDTH * start.scale),
      Math.round(INTERNAL_DRAG_PREVIEW_HEIGHT * start.scale),
      baselinePath!,
    );
  }
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ]);
  if (exitCode !== 0) throw new Error(`Native pointer drag failed: ${stderr}`);
  console.log(`LOCUS_NATIVE_POINTER_DRAG_JSON ${stdout.trim()}`);
  await sleep(180);
  return nativePreview;
}

async function inspectNativePreview(
  expectedWidth: number,
  expectedHeight: number,
  baselinePath: string,
): Promise<NativePreviewSnapshot> {
  const helper = path.resolve("scripts/locus-native-drag-preview-inspect.ps1");
  const child = Bun.spawn([
    "powershell.exe",
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", helper,
    "-RuntimeRoot", runtimeRoot,
    "-ExpectedWidth", String(expectedWidth),
    "-ExpectedHeight", String(expectedHeight),
    "-BaselinePath", baselinePath,
    "-TimeoutMs", "2500",
  ], { stdout: "pipe", stderr: "pipe" });
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ]);
  if (exitCode !== 0) throw new Error(`Native drag preview inspection failed: ${stderr}`);
  const snapshot = JSON.parse(stdout.trim()) as NativePreviewSnapshot;
  console.log(`LOCUS_NATIVE_DRAG_PREVIEW_JSON ${JSON.stringify(snapshot)}`);
  return snapshot;
}

async function captureDesktopBaseline(): Promise<string> {
  const outputPath = path.join(runtimeRoot, "logs", "chromium-native-drag-baseline.png");
  const helper = path.resolve("scripts/locus-screen-capture.ps1");
  const child = Bun.spawn([
    "powershell.exe",
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", helper,
    "-OutputPath", outputPath,
  ], { stdout: "pipe", stderr: "pipe" });
  const [exitCode, stderr] = await Promise.all([
    child.exited,
    new Response(child.stderr).text(),
  ]);
  if (exitCode !== 0) throw new Error(`Desktop baseline capture failed: ${stderr}`);
  return outputPath;
}

async function moveNativeCursor(end: ScreenPoint): Promise<void> {
  const helper = path.resolve("scripts/locus-native-pointer-drag.ps1");
  const child = Bun.spawn([
    "powershell.exe",
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", helper,
    "-StartX", String(end.x),
    "-StartY", String(end.y),
    "-EndX", String(end.x),
    "-EndY", String(end.y),
    "-RuntimeRoot", runtimeRoot,
    "-MoveOnly",
  ], { stdout: "pipe", stderr: "pipe" });
  const exitCode = await child.exited;
  const stdout = await new Response(child.stdout).text();
  if (exitCode !== 0) {
    throw new Error(`Native pointer drag failed: ${await new Response(child.stderr).text()}`);
  }
  console.log(`LOCUS_NATIVE_CURSOR_JSON ${stdout.trim()}`);
}
