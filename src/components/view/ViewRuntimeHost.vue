<script setup lang="ts">
import { computed, markRaw, nextTick, onMounted, onBeforeUnmount, ref, shallowRef, toRef, watch, type Component } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getLocusRuntime } from "../../services/locusRuntime";
import { WORKSPACE_EVENT_NAME, workspaceMaterializationMatches, type RoutedWorkspaceEvent, type WorkspaceRef } from "../../services/project";
import { viewRead, viewWatch, viewUnwatch, viewAppendFrontendLogs, viewRequiresUnityConnection, type ViewPackageDetail, type ViewFrontendLogEntry, type ViewFrontendLogLevel } from "../../services/view";
import { checkUnityConnectionStatus } from "../../services/unity";
import { hasTauriWindowRuntime } from "../../services/tauriRuntime";
import { normalizeAppError } from "../../services/errors";
import { t } from "../../i18n";
import { useWorkspaceEventScope } from "../../composables/useWorkspaceEventScope";
import { compileViewPackage } from "./viewCompilation";
import type { CompiledViewPackage } from "./viewCompilationTypes";
import { createViewRuntimeComponent } from "./viewRuntime";
import { createViewExecutionScope, type ViewExecutionScope } from "./viewExecutionScope";
import { createViewHostApi } from "./viewHostApi";
import { createViewAutomation } from "./viewAutomation";
import { registerNativeViewHost } from "./viewHostRegistry";
import { acquireViewEditorState } from "./viewEditorState";
import { getFrontendWorkbench } from "../../services/frontendWorkbench";

const props = withDefaults(defineProps<{ viewId: string; workspaceRef: WorkspaceRef; instanceId: string; active?: boolean; windowLabel?: string; ownerWindow?: Window }>(), { active: true, windowLabel: "main" });
const emit = defineEmits<{ activate: []; ready: [] }>();
const root = ref<HTMLElement | null>(null);
const detail = shallowRef<ViewPackageDetail | null>(null);
const runtimeComponent = shallowRef<Component | null>(null);
const compilation = shallowRef<CompiledViewPackage | null>(null);
const loading = ref(false);
const error = ref("");
const latestFrontendLog = shallowRef<ViewFrontendLogEntry | null>(null);
const active = toRef(props, "active");
const manifest = computed(() => detail.value?.manifest ?? null);
const lifetime = useWorkspaceEventScope();
const stateKey = () => JSON.stringify([props.instanceId, props.viewId, props.workspaceRef.checkoutId, props.workspaceRef.expectedGeneration, props.workspaceRef.expectedMaterializationEpoch]);
let stateOwner = acquireViewEditorState(stateKey());
let state = stateOwner.state;
let execution: ViewExecutionScope | null = null;
let revision = 0;
let stale = true;
let disposed = false;
let pending: Promise<void> | null = null;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;
let style: HTMLStyleElement | null = null;
let watcherRelease: (() => void) | null = null;
let watcherRevision = 0;
let unregisterHost: (() => void) | null = null;
let logTimer: ReturnType<typeof setTimeout> | null = null;
let logs: Array<{ workspaceRef: WorkspaceRef; viewId: string; level: ViewFrontendLogLevel; message: string }> = [];

function matches(ref: WorkspaceRef, event: { checkoutId: string; workspaceGeneration?: number; materializationEpoch?: number | null }) {
  return event.checkoutId === ref.checkoutId && event.workspaceGeneration === ref.expectedGeneration && workspaceMaterializationMatches(ref.expectedMaterializationEpoch, event.materializationEpoch);
}
function flushLogs() {
  if (logTimer) clearTimeout(logTimer); logTimer = null;
  const batch = logs; logs = [];
  for (let i = 0; i < batch.length;) {
    const first = batch[i]!;
    const group = [];
    while (i < batch.length && batch[i]!.viewId === first.viewId && batch[i]!.workspaceRef === first.workspaceRef) group.push(batch[i++]!);
    void viewAppendFrontendLogs(first.workspaceRef, group.map(({ viewId, level, message }) => ({ viewId, level, message }))).catch(() => undefined);
  }
}
function log(workspaceRef: WorkspaceRef, viewId: string, level: ViewFrontendLogLevel, args: unknown[]) {
  const seen = new WeakSet<object>();
  const message = args.map((value) => {
    if (value instanceof Error) return value.stack || value.message;
    if (typeof value === "string") return value;
    try { return JSON.stringify(value, (_key, nested) => {
      if (typeof nested === "bigint") return String(nested);
      if (nested && typeof nested === "object") { if (seen.has(nested)) return "[Circular]"; seen.add(nested); }
      return nested;
    }) ?? String(value); } catch { return String(value); }
  }).join(" ").slice(0, 16_384);
  latestFrontendLog.value = { time: Date.now(), level, message };
  logs.push({ workspaceRef, viewId, level, message });
  if (logs.length >= 64) flushLogs(); else logTimer ??= setTimeout(flushLogs, 100);
}
function applyStyles(compiled: CompiledViewPackage) {
  if (!style) { const owner = root.value?.ownerDocument ?? props.ownerWindow?.document ?? document; style = owner.createElement("style"); style.dataset.locusViewInstance = props.instanceId; owner.head.appendChild(style); }
  style.textContent = [...compiled.styles, ...Object.values(compiled.modules).flatMap((module) => module.styles)].join("\n");
}
async function reload(): Promise<void> {
  const requested = ++revision;
  stale = true;
  if (!props.active && runtimeComponent.value) return;
  const workspaceRef = { ...props.workspaceRef };
  const viewId = props.viewId;
  loading.value = true;
  const task = (async () => {
    let candidate: ViewExecutionScope | null = null;
    try {
      const next = await viewRead(workspaceRef, viewId);
      if (disposed || requested !== revision) return;
      if (viewRequiresUnityConnection(next.manifest) && !(await checkUnityConnectionStatus(workspaceRef)).connected) throw new Error(t("view.host.unityConnectionRequired"));
      const compiled = await compileViewPackage(next);
      if (disposed || requested !== revision) return;
      if (compilation.value?.scriptKey === compiled.scriptKey && runtimeComponent.value) {
        compilation.value = compiled; detail.value = next; applyStyles(compiled); stale = false; error.value = ""; return;
      }
      candidate = createViewExecutionScope({ viewId, editorId: props.instanceId, windowLabel: props.windowLabel, ownerWindow: props.ownerWindow ?? root.value?.ownerDocument.defaultView ?? window, workspaceRef, active, state, log: (level, args) => log(workspaceRef, viewId, level, args) });
      const api = createViewHostApi({ viewId, workspaceRef, scope: candidate, title: () => next.manifest.name, reload });
      const component = createViewRuntimeComponent({ detail: next, api, compilation: compiled, scope: candidate, onError: (failure) => {
        error.value = normalizeAppError(failure).message; log(workspaceRef, viewId, "error", [failure]);
      } });
      if (disposed || requested !== revision) { candidate.dispose(); return; }
      const previous = execution;
      execution = candidate; candidate = null;
      detail.value = next; compilation.value = compiled; error.value = ""; stale = false;
      applyStyles(compiled); runtimeComponent.value = markRaw(component);
      await nextTick(); previous?.dispose();
      if (!disposed && requested === revision) emit("ready");
    } catch (failure) {
      candidate?.dispose();
      if (disposed || requested !== revision) return;
      error.value = normalizeAppError(failure).message; log(workspaceRef, viewId, "error", [failure]);
    } finally { if (requested === revision) loading.value = false; }
  })();
  pending = task;
  try { await task; } finally { if (pending === task) pending = null; }
}
function scheduleReload() {
  stale = true; ++revision;
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = setTimeout(() => { reloadTimer = null; void reload(); }, 80);
}
async function ensureMounted() {
  if (pending) await pending;
  if (stale || !runtimeComponent.value) await reload();
  await nextTick();
}
function exportState(): Record<string, unknown> { return Object.fromEntries(state); }
function restoreState(snapshot: Record<string, unknown>) { for (const [key, value] of Object.entries(snapshot)) state.set(key, value); }
defineExpose({ ensureMounted, reload, exportState, restoreState });

async function attachWatcher() {
  const attachment = ++watcherRevision;
  watcherRelease?.(); watcherRelease = null;
  if (!hasTauriWindowRuntime()) return;
  const workspaceRef = { ...props.workspaceRef };
  const viewId = props.viewId;
  const hostLabel = props.windowLabel || getCurrentWindow().label;
  const token = `${hostLabel}:${props.instanceId}:${crypto.randomUUID()}`;
  await viewWatch(workspaceRef, viewId, token, hostLabel);
  const release = () => { void viewUnwatch(workspaceRef, viewId, token, hostLabel).catch(() => undefined); };
  if (disposed || attachment !== watcherRevision || viewId !== props.viewId || !matches(props.workspaceRef, { checkoutId: workspaceRef.checkoutId, workspaceGeneration: workspaceRef.expectedGeneration ?? undefined, materializationEpoch: workspaceRef.expectedMaterializationEpoch })) release();
  else watcherRelease = release;
}
function registerHost() {
  unregisterHost?.();
  unregisterHost = registerNativeViewHost({ instanceId: props.instanceId, windowLabel: props.windowLabel, viewId: props.viewId, workspaceRef: { ...props.workspaceRef }, active: () => active.value, root: () => root.value,
    ready: ensureMounted, reload, activate: async () => {
      let workbench: ReturnType<typeof getFrontendWorkbench> | null = null;
      try { workbench = getFrontendWorkbench(props.windowLabel); } catch { /* Unity wrapper has no Workbench. */ }
      if (workbench) await workbench.activate(props.instanceId); else emit("activate");
      await nextTick(); await ensureMounted();
    },
    execute: async (kind, payload) => {
      if (!root.value) throw new Error("View is not mounted.");
      const automation = createViewAutomation({ root: () => root.value!, globals: () => execution?.globals({}) ?? { window, globalThis: window }, signal: lifetime,
        activeViewId: toRef(props, "viewId"), detail, manifest, loading, error, latestFrontendLog, runtimeComponent });
      return automation.handle(kind, payload);
    },
  });
}
watch(() => [props.viewId, props.workspaceRef.checkoutId, props.workspaceRef.expectedGeneration, props.workspaceRef.expectedMaterializationEpoch], () => {
  ++revision; execution?.dispose(); execution = null; compilation.value = null; runtimeComponent.value = null; detail.value = null;
  stateOwner.release(); stateOwner = acquireViewEditorState(stateKey()); state = stateOwner.state;
  registerHost(); void attachWatcher().catch((failure) => { error.value = normalizeAppError(failure).message; }); void reload();
});
watch(active, (visible) => { if (visible && stale) void ensureMounted(); });
onMounted(async () => {
  registerHost();
  try {
    const release = await getLocusRuntime().subscribe<RoutedWorkspaceEvent<{ id?: string }>>(WORKSPACE_EVENT_NAME, (event) => {
      if (event.eventName === "view-package-reloaded" && event.payload.id === props.viewId && matches(props.workspaceRef, event)) scheduleReload();
    }, { owner: "ViewRuntimeHost.reload", signal: lifetime });
    if (lifetime.aborted) release(); else lifetime.addEventListener("abort", release, { once: true });
    await attachWatcher();
  } catch (failure) { error.value = normalizeAppError(failure).message; }
  if (!disposed && props.active) await ensureMounted();
});
onBeforeUnmount(() => {
  disposed = true; ++revision; execution?.dispose(); watcherRelease?.(); unregisterHost?.();
  ++watcherRevision; stateOwner.release();
  if (reloadTimer) clearTimeout(reloadTimer);
  style?.remove(); style = null; flushLogs();
});
</script>

<template>
  <div ref="root" class="view-runtime-host" :data-locus-view-id="viewId" :data-locus-view-instance="instanceId" :data-locus-view-scope="compilation?.scopeId">
    <div v-if="error" class="view-runtime-host-error" role="alert">{{ error }}</div>
    <component :is="runtimeComponent" v-if="runtimeComponent" />
    <div v-else-if="loading" class="view-runtime-host-loading">{{ t('common.loading') }}</div>
  </div>
</template>

<style scoped>
.view-runtime-host { display: flex; flex-direction: column; width: 100%; height: 100%; min-height: 0; min-width: 0; overflow: hidden; background: var(--panel-bg); color: var(--text-color); }
.view-runtime-host :deep(.locus-view-runtime-root) { flex: 1; min-width: 0; min-height: 0; overflow: auto; font-family: var(--font-ui); }
.view-runtime-host-error { flex: none; padding: 8px 12px; border-bottom: 1px solid var(--border-color); color: var(--status-error-fg); font-size: 12px; white-space: pre-wrap; }
.view-runtime-host-loading { padding: 12px; color: var(--text-secondary); font-size: 12px; }
</style>
