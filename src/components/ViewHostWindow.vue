<script setup lang="ts">
import { computed, markRaw, onMounted, onUnmounted, ref, shallowRef, type Component } from "vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import {
  viewAppendFrontendLog,
  viewBindingApply,
  viewBindingRead,
  viewBindingWrite,
  viewCallScript,
  viewHostIdFromLocation,
  viewRead,
  type ViewPackageDetail,
  type ViewPackageSummary,
  type ViewFrontendLogLevel,
  type ViewRuntimeUpdateEvent,
} from "../services/view";
import { createViewRuntimeComponent } from "./view/viewRuntime";

const CONSOLE_LOG_LEVELS: ViewFrontendLogLevel[] = ["debug", "log", "info", "warn", "error"];

const viewId = viewHostIdFromLocation();
const detail = ref<ViewPackageDetail | null>(null);
const runtimeComponent = shallowRef<Component | null>(null);
const loading = ref(false);
const error = ref("");
let unsubscribeReload: RuntimeUnsubscribe | null = null;
let restoreConsoleLogCapture: (() => void) | null = null;

const manifest = computed(() => detail.value?.manifest ?? null);

function installViewConsoleLogCapture(activeViewId: string) {
  if (!activeViewId) return () => {};

  const consoleForCapture = console as unknown as Record<
    ViewFrontendLogLevel,
    (...args: unknown[]) => void
  >;
  const originals = new Map<ViewFrontendLogLevel, (...args: unknown[]) => void>();

  const appendLog = (level: ViewFrontendLogLevel, args: unknown[]) => {
    const message = formatConsoleArgs(args);
    void viewAppendFrontendLog({ viewId: activeViewId, level, message }).catch(() => undefined);
  };

  for (const level of CONSOLE_LOG_LEVELS) {
    const original = consoleForCapture[level]?.bind(console) ?? (() => undefined);
    originals.set(level, original);
    consoleForCapture[level] = (...args: unknown[]) => {
      original(...args);
      appendLog(level, args);
    };
  }

  const handleWindowError = (event: ErrorEvent) => {
    appendLog("error", [formatErrorEvent(event)]);
  };
  const handleUnhandledRejection = (event: PromiseRejectionEvent) => {
    appendLog("error", ["Unhandled promise rejection", event.reason]);
  };

  window.addEventListener("error", handleWindowError);
  window.addEventListener("unhandledrejection", handleUnhandledRejection);

  return () => {
    for (const [level, original] of originals) {
      consoleForCapture[level] = original;
    }
    window.removeEventListener("error", handleWindowError);
    window.removeEventListener("unhandledrejection", handleUnhandledRejection);
  };
}

function formatConsoleArgs(args: unknown[]) {
  return args.map((arg) => formatConsoleValue(arg)).join(" ");
}

function formatConsoleValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.stack || value.message;
  if (value === undefined) return "undefined";
  if (typeof value === "bigint") return value.toString();
  if (typeof value === "symbol") return value.toString();
  if (typeof value === "function") return `[Function ${value.name || "anonymous"}]`;
  if (value === null || typeof value !== "object") return String(value);

  const seen = new WeakSet<object>();
  try {
    return JSON.stringify(value, (_key, nestedValue: unknown) => {
      if (typeof nestedValue === "bigint") return nestedValue.toString();
      if (typeof nestedValue === "function") {
        return `[Function ${nestedValue.name || "anonymous"}]`;
      }
      if (nestedValue && typeof nestedValue === "object") {
        if (seen.has(nestedValue)) return "[Circular]";
        seen.add(nestedValue);
      }
      return nestedValue;
    });
  } catch {
    return String(value);
  }
}

function formatErrorEvent(event: ErrorEvent) {
  const location = [event.filename, event.lineno, event.colno].filter(Boolean).join(":");
  const stack = event.error instanceof Error ? event.error.stack : "";
  return [event.message || "Uncaught error", location, stack].filter(Boolean).join("\n");
}

async function loadView() {
  if (!viewId) {
    error.value = t("view.host.missingId");
    return;
  }
  loading.value = true;
  error.value = "";
  try {
    const next = await viewRead(viewId);
    detail.value = next;
    runtimeComponent.value = markRaw(
      createViewRuntimeComponent({
        detail: next,
        api: {
          callScript: (scriptName, method, args) =>
            viewCallScript({ viewId: next.manifest.id, scriptName, method, args }),
          bindingRead: (request) => viewBindingRead({ viewId: next.manifest.id, ...request }),
          bindingWrite: (request) => viewBindingWrite({ viewId: next.manifest.id, ...request }),
          bindingApply: (request) => viewBindingApply({ viewId: next.manifest.id, ...request }),
          onUpdate: (handler) =>
            getLocusRuntime().subscribe<ViewRuntimeUpdateEvent>("unity-editor-update", handler),
          reload: loadView,
        },
      }),
    );
  } catch (loadError) {
    error.value = normalizeAppError(loadError).message;
    console.error("[view-host]", loadError);
    runtimeComponent.value = null;
  } finally {
    loading.value = false;
  }
}

onMounted(async () => {
  restoreConsoleLogCapture = installViewConsoleLogCapture(viewId);
  await loadView();
  unsubscribeReload = await getLocusRuntime().subscribe<ViewPackageSummary>(
    "view-package-reloaded",
    (payload) => {
      if (payload.id === viewId) void loadView();
    },
  );
});

onUnmounted(() => {
  unsubscribeReload?.();
  unsubscribeReload = null;
  restoreConsoleLogCapture?.();
  restoreConsoleLogCapture = null;
});
</script>

<template>
  <main class="view-host-window">
    <div v-if="error" class="view-host-error">{{ error }}</div>

    <div v-if="loading && !detail" class="view-host-state">{{ t("common.loading") }}</div>
    <component
      :is="runtimeComponent"
      v-else-if="runtimeComponent"
      class="view-runtime-frame"
      :aria-label="manifest?.name || viewId || t('view.host.untitled')"
    />
  </main>
</template>

<style scoped>
.view-host-window {
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
  color: var(--text-color);
}

.view-host-error {
  flex-shrink: 0;
  padding: 7px 12px;
  border-bottom: 1px solid var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
}

.view-host-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.view-runtime-frame {
  flex: 1;
  width: 100%;
  min-height: 0;
  overflow: auto;
  background: var(--bg-color);
}
</style>
