import { markRaw, nextTick, shallowReactive } from "vue";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import {
  detachedWindowGeometry,
  readWorkbenchAuxWindowRegistry,
  recordWorkbenchWindowMetric,
  registerWorkbenchAuxWindow,
  type WorkbenchAuxWindowRecord,
  type WorkbenchWindowScreenPoint,
} from "./workbenchWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import { waitForSharedWorkbenchTransferTarget } from "./sharedWorkbenchTransfer";

const SHARED_WINDOW_FRAGMENT_PREFIX = "locus-shared-workbench-";
const SHARED_POOL_LABEL_PREFIX = "workbench-shared-pool-";
const SHARED_WINDOW_LABEL_PREFIX = "workbench-shared-";
const STYLE_MARKER = "data-locus-shared-style";
const DEFAULT_WIDTH = 1120;
const DEFAULT_HEIGHT = 760;
const HANDLE_WAIT_MS = 2_000;

export interface SharedWorkbenchWindowHost {
  label: string;
  browserWindow: Window;
  appWindow: WebviewWindow;
  container: HTMLElement;
  transferToken: string;
  restoring: boolean;
  pooled: boolean;
  openedAt: number;
  claimedAt: number;
  disposeDocumentSync: () => void;
}

interface PreparedSharedWorkbenchWindow {
  label: string;
  browserWindow: Window;
  appWindow: WebviewWindow;
  container: HTMLElement;
  preparedAt: number;
  disposeDocumentSync: () => void;
}

export const sharedWorkbenchWindowHosts = shallowReactive<SharedWorkbenchWindowHost[]>([]);

let preparedWindow: PreparedSharedWorkbenchWindow | null = null;
let preparingWindow: Promise<PreparedSharedWorkbenchWindow | null> | null = null;
let poolSequence = 0;
let poolReplenishTimer: ReturnType<typeof window.setTimeout> | null = null;

function scheduleSharedWorkbenchWindowPoolReplenishment(): void {
  if (preparedWindow || preparingWindow || poolReplenishTimer) return;
  poolReplenishTimer = window.setTimeout(() => {
    poolReplenishTimer = null;
    const replenish = () => void prepareSharedWorkbenchWindowPool();
    if (typeof window.requestIdleCallback === "function") {
      window.requestIdleCallback(replenish, { timeout: 1_000 });
    } else {
      replenish();
    }
  }, 450);
}

function publishSharedWindowDebugState(): void {
  (window as Window & {
    __LOCUS_SHARED_WORKBENCH_STATE__?: {
      poolLabel: string | null;
      hostLabels: string[];
    };
  }).__LOCUS_SHARED_WORKBENCH_STATE__ = {
    poolLabel: preparedWindow?.label ?? null,
    hostLabels: sharedWorkbenchWindowHosts.map((host) => host.label),
  };
}

function randomLabel(prefix = SHARED_WINDOW_LABEL_PREFIX): string {
  const suffix = globalThis.crypto?.randomUUID?.()
    ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  return `${prefix}${suffix}`;
}

function sharedWindowUrl(label: string): string {
  return `about:blank#${SHARED_WINDOW_FRAGMENT_PREFIX}${label}`;
}

function featureString(geometry: { x: number; y: number; width: number; height: number }): string {
  return [
    "popup=yes",
    `left=${Math.round(geometry.x)}`,
    `top=${Math.round(geometry.y)}`,
    `width=${Math.round(geometry.width)}`,
    `height=${Math.round(geometry.height)}`,
  ].join(",");
}

function copyAttributes(source: Element, target: Element, retained: string[] = []): void {
  const retainedValues = new Map(retained.map((name) => [name, target.getAttribute(name)]));
  for (const attribute of [...target.attributes]) target.removeAttribute(attribute.name);
  for (const attribute of [...source.attributes]) target.setAttribute(attribute.name, attribute.value);
  for (const [name, value] of retainedValues) {
    if (value !== null) target.setAttribute(name, value);
  }
}

function synchronizeDocument(targetWindow: Window, label: string): {
  container: HTMLElement;
  dispose: () => void;
} {
  const sourceDocument = window.document;
  const targetDocument = targetWindow.document;
  targetDocument.title = "Locus";
  const sourceBackground = getComputedStyle(sourceDocument.documentElement).getPropertyValue("--bg-color").trim()
    || getComputedStyle(sourceDocument.body).backgroundColor
    || "#111318";
  targetDocument.documentElement.style.background = sourceBackground;
  targetDocument.body.style.background = sourceBackground;
  targetDocument.body.style.margin = "0";

  let styleSyncQueued = false;
  const syncStyles = () => {
    styleSyncQueued = false;
    targetDocument.head.querySelectorAll(`[${STYLE_MARKER}]`).forEach((node) => node.remove());
    for (const source of sourceDocument.head.querySelectorAll<HTMLLinkElement | HTMLStyleElement>(
      'link[rel="stylesheet"], style',
    )) {
      const clone = source.cloneNode(true) as HTMLLinkElement | HTMLStyleElement;
      clone.setAttribute(STYLE_MARKER, "");
      if (clone instanceof HTMLLinkElement && source instanceof HTMLLinkElement) {
        clone.href = source.href;
      }
      targetDocument.head.appendChild(clone);
    }
  };
  const scheduleStyleSync = () => {
    if (styleSyncQueued) return;
    styleSyncQueued = true;
    queueMicrotask(syncStyles);
  };

  const syncRootAttributes = () => {
    copyAttributes(sourceDocument.documentElement, targetDocument.documentElement, [
      "data-locus-shared-window",
    ]);
    copyAttributes(sourceDocument.body, targetDocument.body, ["data-locus-shared-window"]);
    targetDocument.documentElement.setAttribute("data-locus-shared-window", label);
    targetDocument.body.setAttribute("data-locus-shared-window", label);
  };

  syncStyles();
  syncRootAttributes();

  const container = sourceDocument.createElement("div");
  container.className = "locus-shared-workbench-host";
  container.dataset.locusSharedWorkbenchHost = label;
  targetDocument.body.replaceChildren(container);

  const styleObserver = new MutationObserver(scheduleStyleSync);
  styleObserver.observe(sourceDocument.head, {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true,
    attributeFilter: ["href", "media", "disabled"],
  });
  const attributeObserver = new MutationObserver(syncRootAttributes);
  attributeObserver.observe(sourceDocument.documentElement, { attributes: true });
  attributeObserver.observe(sourceDocument.body, { attributes: true });

  const preventNavigationDrop = (event: DragEvent) => event.preventDefault();
  targetDocument.body.addEventListener("dragover", preventNavigationDrop);
  targetDocument.body.addEventListener("drop", preventNavigationDrop);

  return {
    container,
    dispose: () => {
      styleObserver.disconnect();
      attributeObserver.disconnect();
      targetDocument.body.removeEventListener("dragover", preventNavigationDrop);
      targetDocument.body.removeEventListener("drop", preventNavigationDrop);
    },
  };
}

async function waitForTauriWindow(label: string): Promise<WebviewWindow> {
  const deadline = performance.now() + HANDLE_WAIT_MS;
  do {
    const handle = await WebviewWindow.getByLabel(label).catch(() => null);
    if (handle) return handle;
    await new Promise<void>((resolve) => window.setTimeout(resolve, 12));
  } while (performance.now() < deadline);
  throw new Error(`Shared Workbench native window did not appear: ${label}`);
}

async function openBlankSharedWindow(
  label: string,
  geometry: { x: number; y: number; width: number; height: number },
): Promise<PreparedSharedWorkbenchWindow> {
  const openedAt = Date.now();
  const browserWindow = window.open(sharedWindowUrl(label), "", featureString(geometry));
  if (!browserWindow) throw new Error("Chromium rejected the shared Workbench window request.");
  try {
    const appWindow = await waitForTauriWindow(label);
    const { container, dispose } = synchronizeDocument(browserWindow, label);
    recordWorkbenchWindowMetric("shared-window-proxy-ready", {
      startedAt: openedAt,
      detail: { label },
    });
    return {
      label,
      browserWindow: markRaw(browserWindow),
      appWindow: markRaw(appWindow),
      container: markRaw(container),
      preparedAt: Date.now(),
      disposeDocumentSync: dispose,
    };
  } catch (error) {
    browserWindow.close();
    throw error;
  }
}

export async function prepareSharedWorkbenchWindowPool(): Promise<void> {
  if (!hasTauriWindowRuntime() || preparedWindow || preparingWindow) return;
  const label = `${SHARED_POOL_LABEL_PREFIX}${++poolSequence}`;
  preparingWindow = openBlankSharedWindow(label, {
    x: 80,
    y: 80,
    width: DEFAULT_WIDTH,
    height: DEFAULT_HEIGHT,
  }).then(async (prepared) => {
    addSharedHost({
      prepared,
      transferToken: "",
      restoring: false,
      pooled: true,
      claimedAt: 0,
    });
    await nextTick();
    const target = await waitForSharedWorkbenchTransferTarget(label, HANDLE_WAIT_MS);
    if (!target) throw new Error(`Shared Workbench pool host did not mount: ${label}`);
    preparedWindow = prepared;
    publishSharedWindowDebugState();
    recordWorkbenchWindowMetric("shared-pool-ready", { detail: { label } });
    return prepared;
  }).catch((error) => {
    void removeSharedWorkbenchWindowHost(label);
    console.warn("[workbench-window] shared pool warmup failed", error);
    return null;
  }).finally(() => {
    preparingWindow = null;
  });
  await preparingWindow;
}

async function claimOrCreateSharedWindow(
  geometry: { x: number; y: number; width: number; height: number },
): Promise<{ prepared: PreparedSharedWorkbenchWindow; pooled: boolean }> {
  const pooled = preparedWindow;
  preparedWindow = null;
  publishSharedWindowDebugState();
  if (pooled && !pooled.browserWindow.closed) {
    await Promise.all([
      pooled.appWindow.setPosition(new LogicalPosition(geometry.x, geometry.y)),
      pooled.appWindow.setSize(new LogicalSize(geometry.width, geometry.height)),
    ]);
    return { prepared: pooled, pooled: true };
  }
  pooled?.disposeDocumentSync();
  const label = randomLabel();
  const prepared = await openBlankSharedWindow(label, geometry);
  await Promise.all([
    prepared.appWindow.setPosition(new LogicalPosition(geometry.x, geometry.y)),
    prepared.appWindow.setSize(new LogicalSize(geometry.width, geometry.height)),
  ]);
  return {
    prepared,
    pooled: false,
  };
}

function addSharedHost(options: {
  prepared: PreparedSharedWorkbenchWindow;
  transferToken: string;
  restoring: boolean;
  pooled: boolean;
  claimedAt: number;
}): SharedWorkbenchWindowHost {
  const host = shallowReactive<SharedWorkbenchWindowHost>({
    label: options.prepared.label,
    browserWindow: options.prepared.browserWindow,
    appWindow: options.prepared.appWindow,
    container: options.prepared.container,
    transferToken: options.transferToken,
    restoring: options.restoring,
    pooled: options.pooled,
    openedAt: options.prepared.preparedAt,
    claimedAt: options.claimedAt,
    disposeDocumentSync: options.prepared.disposeDocumentSync,
  });
  sharedWorkbenchWindowHosts.push(host);
  publishSharedWindowDebugState();
  const handleUnload = () => void removeSharedWorkbenchWindowHost(host.label, false);
  host.browserWindow.addEventListener("unload", handleUnload, { once: true });
  return host;
}

export async function createSharedDetachedWorkbenchWindow(
  token: string,
  point: WorkbenchWindowScreenPoint,
  startedAt = Date.now(),
  tabAnchor?: { x: number; y: number },
): Promise<{ label: string; pooled: boolean }> {
  const geometry = await detachedWindowGeometry(point, tabAnchor);
  const claimedAt = Date.now();
  const { prepared, pooled } = await claimOrCreateSharedWindow(geometry);
  registerWorkbenchAuxWindow(prepared.label, geometry);
  const existingHost = sharedWorkbenchWindowHosts.find((host) => host.label === prepared.label);
  if (existingHost) {
    existingHost.transferToken = token;
    existingHost.restoring = false;
    existingHost.pooled = false;
    existingHost.claimedAt = claimedAt;
  } else {
    addSharedHost({ prepared, transferToken: token, restoring: false, pooled, claimedAt });
  }
  await nextTick();
  recordWorkbenchWindowMetric(pooled ? "shared-pool-claimed" : "shared-window-created", {
    token,
    startedAt,
    detail: { label: prepared.label, ...geometry },
  });
  scheduleSharedWorkbenchWindowPoolReplenishment();
  return { label: prepared.label, pooled };
}

async function restoreSharedWorkbenchWindow(record: WorkbenchAuxWindowRecord): Promise<void> {
  const existing = sharedWorkbenchWindowHosts.find((host) => host.label === record.label);
  if (existing || preparedWindow?.label === record.label) return;
  const geometry = {
    x: record.x ?? 80,
    y: record.y ?? 80,
    width: record.width ?? DEFAULT_WIDTH,
    height: record.height ?? DEFAULT_HEIGHT,
  };
  const prepared = await openBlankSharedWindow(record.label, geometry);
  addSharedHost({
    prepared,
    transferToken: "",
    restoring: true,
    pooled: false,
    claimedAt: Date.now(),
  });
  await nextTick();
}

export async function restoreSharedWorkbenchWindows(): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  for (const record of readWorkbenchAuxWindowRegistry()) {
    await restoreSharedWorkbenchWindow(record).catch((error) => {
      console.warn(`[workbench-window] failed to restore shared window ${record.label}`, error);
    });
  }
}

export async function removeSharedWorkbenchWindowHost(
  label: string,
  closeWindow = true,
): Promise<void> {
  const host = sharedWorkbenchWindowHosts.find((candidate) => candidate.label === label);
  if (!host) return;
  if (preparedWindow?.label === label) preparedWindow = null;
  if (closeWindow && !host.browserWindow.closed) {
    await host.appWindow.close().catch(() => {
      host.browserWindow.close();
    });
  }
  const index = sharedWorkbenchWindowHosts.findIndex((candidate) => candidate === host);
  if (index >= 0) sharedWorkbenchWindowHosts.splice(index, 1);
  publishSharedWindowDebugState();
  host.disposeDocumentSync();
  host.container.remove();
}

export function sharedWorkbenchWindowAtScreenPoint(
  point: WorkbenchWindowScreenPoint,
): SharedWorkbenchWindowHost | null {
  for (let index = sharedWorkbenchWindowHosts.length - 1; index >= 0; index -= 1) {
    const host = sharedWorkbenchWindowHosts[index]!;
    const target = host.browserWindow;
    if (
      point.x >= target.screenX
      && point.x <= target.screenX + target.outerWidth
      && point.y >= target.screenY
      && point.y <= target.screenY + target.outerHeight
    ) return host;
  }
  return null;
}

export async function setSharedWorkbenchWindowBounds(
  label: string,
  geometry: { x: number; y: number; width: number; height: number },
): Promise<boolean> {
  const host = sharedWorkbenchWindowHosts.find((candidate) => candidate.label === label);
  if (!host) return false;
  await Promise.all([
    host.appWindow.setPosition(new LogicalPosition(geometry.x, geometry.y)),
    host.appWindow.setSize(new LogicalSize(geometry.width, geometry.height)),
  ]);
  return true;
}
