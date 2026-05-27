<script setup lang="ts">
import { computed, markRaw, onBeforeUnmount, onMounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Check, X } from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import {
  agentGraphToolCancel,
  agentGraphToolRequest,
  agentGraphToolRequestIdFromLocation,
  agentGraphToolSubmit,
  type AgentGraphToolOption,
  type AgentGraphToolPayload,
} from "../services/agentGraphTool";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";
import {
  GraphView,
  GraphViewController,
  cloneGraphData,
  normalizeGraphLayoutMode,
  normalizeGraphData,
  type GraphController,
  type GraphData,
} from "./graph";
import { renderGraphPngAttachment } from "./graph/graphImage";
import BaseButton from "./ui/BaseButton.vue";
import LucideIcon from "./icons/LucideIcon.vue";

let appWindow: ReturnType<typeof getCurrentWindow> | null = null;
if (hasTauriWindowRuntime()) {
  try {
    appWindow = getCurrentWindow();
  } catch {
    appWindow = null;
  }
}
const requestId = agentGraphToolRequestIdFromLocation();
const payload = ref<AgentGraphToolPayload | null>(null);
const currentGraph = ref<GraphData>(normalizeGraphData({ nodes: [], links: [] }));
const loading = ref(false);
const submitting = ref(false);
const error = ref("");
const settled = ref(false);

const fallbackOption: AgentGraphToolOption = {
  label: t("common.confirm"),
  description: "",
  value: "confirm",
};

const confirmationOptions = computed(() => {
  const options = payload.value?.options ?? [];
  return options.length > 0 ? options : [fallbackOption];
});

const graphController = markRaw<GraphController>({
  ...new GraphViewController(),
  loadGraph() {
    return cloneGraphData(currentGraph.value);
  },
  saveGraph(graph) {
    currentGraph.value = cloneGraphData(graph);
  },
  applyGraph(graph) {
    currentGraph.value = cloneGraphData(graph);
    if (payload.value?.editable) {
      void submitGraph(confirmationOptions.value[0] ?? fallbackOption);
    }
  },
  onGraphChange(graph) {
    currentGraph.value = cloneGraphData(graph);
  },
});

async function loadPayload() {
  if (!requestId) {
    error.value = "Graph request id is missing.";
    return;
  }
  loading.value = true;
  error.value = "";
  try {
    const next = await agentGraphToolRequest(requestId);
    payload.value = next;
    currentGraph.value = normalizeGraphData(next.graph);
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  } finally {
    loading.value = false;
  }
}

async function closeWindow() {
  if (appWindow) {
    try {
      await appWindow.close();
      return;
    } catch {
      // fall through
    }
    await appWindow.destroy().catch(() => {});
    return;
  }
  window.close();
}

async function cancelAndClose() {
  if (payload.value?.editable && !settled.value) {
    settled.value = true;
    await agentGraphToolCancel(requestId).catch(() => undefined);
  }
  await closeWindow();
}

async function submitGraph(option: AgentGraphToolOption) {
  if (!payload.value?.editable || submitting.value) return;
  submitting.value = true;
  error.value = "";
  try {
    const selectedOption = {
      label: option.label,
      description: option.description,
      value: option.value ?? null,
    };
    const graph = cloneGraphData(currentGraph.value);
    const shouldReturnLayoutImage = !!payload.value.returnImage
      && normalizeGraphLayoutMode(graph.layout?.mode) === "manual";
    const image = shouldReturnLayoutImage
      ? await renderGraphPngAttachment(graph)
      : null;
    await agentGraphToolSubmit({
      requestId,
      option: selectedOption,
      graph,
      images: image ? [image] : undefined,
    });
    settled.value = true;
    await closeWindow();
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
    submitting.value = false;
  }
}

onMounted(() => {
  void loadPayload();
});

onBeforeUnmount(() => {
  if (payload.value?.editable && !settled.value) {
    void agentGraphToolCancel(requestId).catch(() => undefined);
  }
});
</script>

<template>
  <div class="agent-graph-window">
    <div class="agent-graph-titlebar">
      <div class="agent-graph-title">
        <span class="agent-graph-title-main">{{ payload?.title || "Graph" }}</span>
        <span v-if="payload?.editable" class="agent-graph-title-meta">{{ t("common.edit") }}</span>
      </div>
      <button
        type="button"
        class="agent-graph-close"
        :title="t('app.win.close')"
        @click="cancelAndClose"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <div v-if="error" class="agent-graph-header">
      <div v-if="error" class="agent-graph-error">{{ error }}</div>
    </div>

    <div class="agent-graph-body">
      <div v-if="loading" class="agent-graph-state">{{ t("common.loading") }}</div>
      <GraphView
        v-else-if="payload"
        :controller="graphController"
        title=""
        :readonly="!payload.editable"
        :auto-layout="payload.graph.layout?.auto ?? 'missing'"
        :layout-options="payload.graph.layout ?? {}"
        :show-persistence-actions="false"
      />
      <div v-else class="agent-graph-state">{{ error }}</div>
    </div>

    <div v-if="payload?.editable" class="agent-graph-footer">
      <BaseButton size="sm" variant="neutral" :disabled="submitting" @click="cancelAndClose">
        {{ t("common.cancel") }}
      </BaseButton>
      <BaseButton
        v-for="(option, index) in confirmationOptions"
        :key="option.value || option.label"
        size="sm"
        :variant="index === 0 ? 'primary' : 'neutral'"
        :disabled="submitting"
        @click="submitGraph(option)"
      >
        <LucideIcon v-if="index === 0" :icon="Check" :size="13" />
        <span>{{ option.label }}</span>
      </BaseButton>
    </div>
  </div>
</template>

<style scoped>
.agent-graph-window {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
}

.agent-graph-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.agent-graph-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.agent-graph-title-main {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.agent-graph-title-meta {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-size: 12px;
}

.agent-graph-close {
  -webkit-app-region: no-drag;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.agent-graph-close:hover,
.agent-graph-close:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.agent-graph-header {
  flex-shrink: 0;
  display: grid;
  gap: 6px;
  padding: 9px 14px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 92%, var(--sidebar-bg) 8%);
}

.agent-graph-error {
  color: var(--status-danger-fg);
  font-size: 12px;
}

.agent-graph-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.agent-graph-body :deep(.locus-graph-view) {
  height: 100%;
}

.agent-graph-state {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.agent-graph-footer {
  min-height: 44px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 0 12px;
  border-top: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}
</style>
