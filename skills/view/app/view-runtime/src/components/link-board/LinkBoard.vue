<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type ComponentPublicInstance } from "vue";
import BaseButton from "../ui/BaseButton.vue";
import { t } from "../../i18n";
import type { LinkBoardConnection, LinkBoardEndpoint } from "./linkBoardTypes";

const props = withDefaults(defineProps<{
  sources: readonly LinkBoardEndpoint[];
  targets: readonly LinkBoardEndpoint[];
  modelValue: readonly LinkBoardConnection[];
  sourceTitle?: string;
  targetTitle?: string;
  readonly?: boolean;
  /** Allow several connections per endpoint; defaults to one-to-one. */
  multiple?: boolean;
}>(), { readonly: false, multiple: false });
const emit = defineEmits<{ "update:modelValue": [connections: LinkBoardConnection[]] }>();
const selectedSource = ref("");
const board = ref<HTMLElement | null>(null);
const endpoints = new Map<string, Element>();
const lines = ref<Array<{ key: string; path: string }>>([]);
const sourceIds = computed(() => new Set(props.sources.map((item) => item.id)));
const targetIds = computed(() => new Set(props.targets.map((item) => item.id)));
const connections = computed(() => {
  const seen = new Set<string>();
  return props.modelValue.filter((connection) => {
    const key = JSON.stringify([connection.source, connection.target]);
    if (!sourceIds.value.has(connection.source) || !targetIds.value.has(connection.target) || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
});
const linkedSources = computed(() => new Set(connections.value.map((item) => item.source)));
const linkedTargets = computed(() => new Set(connections.value.map((item) => item.target)));
let observer: ResizeObserver | null = null;
let pending = false;
let disposed = false;

function scheduleLines() {
  if (pending || disposed) return;
  pending = true;
  void nextTick().then(() => {
    pending = false;
    if (disposed || !board.value) return;
    const rect = board.value.getBoundingClientRect();
    lines.value = connections.value.flatMap((connection) => {
      const source = endpoints.get(`source:${connection.source}`);
      const target = endpoints.get(`target:${connection.target}`);
      if (!source || !target) return [];
      const from = source.getBoundingClientRect();
      const to = target.getBoundingClientRect();
      const x1 = from.right - rect.left;
      const y1 = from.top + from.height / 2 - rect.top;
      const x2 = to.left - rect.left;
      const y2 = to.top + to.height / 2 - rect.top;
      const bend = Math.max(24, (x2 - x1) / 2);
      return [{ key: JSON.stringify([connection.source, connection.target]), path: `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}` }];
    });
  });
}

function registerEndpoint(kind: string, id: string, value: Element | ComponentPublicInstance | null) {
  const key = `${kind}:${id}`;
  const element = value && "$el" in value ? value.$el as Element : value;
  const previous = endpoints.get(key);
  if (element === previous) return;
  if (previous) observer?.unobserve(previous);
  if (element) { endpoints.set(key, element); observer?.observe(element); }
  else endpoints.delete(key);
  scheduleLines();
}

function selectSource(item: LinkBoardEndpoint) {
  if (props.readonly || item.disabled) return;
  selectedSource.value = selectedSource.value === item.id ? "" : item.id;
}

function selectTarget(item: LinkBoardEndpoint) {
  if (props.readonly || item.disabled) return;
  const source = selectedSource.value;
  if (!source) {
    if (linkedTargets.value.has(item.id)) emit("update:modelValue", props.modelValue.filter((link) => link.target !== item.id));
    return;
  }
  if (!props.sources.some((item) => item.id === source && !item.disabled)) return;
  const exists = props.modelValue.some((link) => link.source === source && link.target === item.id);
  const next = props.modelValue.filter((link) => props.multiple
    ? !(link.source === source && link.target === item.id)
    : link.source !== source && link.target !== item.id);
  if (!exists) next.push({ source, target: item.id });
  selectedSource.value = "";
  emit("update:modelValue", next);
}

watch(() => [props.sources, props.targets, props.modelValue, props.readonly], () => {
  if (props.readonly || !props.sources.some((item) => item.id === selectedSource.value && !item.disabled)) selectedSource.value = "";
  scheduleLines();
}, { deep: true, flush: "post" });
onMounted(() => {
  const Observer = board.value?.ownerDocument.defaultView?.ResizeObserver;
  if (Observer) {
    observer = new Observer(scheduleLines);
    observer.observe(board.value!);
    for (const element of endpoints.values()) observer.observe(element);
  }
  scheduleLines();
});
onBeforeUnmount(() => { disposed = true; observer?.disconnect(); endpoints.clear(); });
</script>

<template>
  <div class="locus-link-board" @keydown.esc="selectedSource = ''">
    <div ref="board" class="link-board-content">
      <div class="link-board-column">
        <div class="link-board-title">{{ sourceTitle ?? t("view.linkBoard.sources") }}</div>
        <BaseButton
          v-for="item in sources" :key="item.id" :ref="(value) => registerEndpoint('source', item.id, value)"
          class="link-board-endpoint" :class="{ 'is-selected': selectedSource === item.id, 'is-linked': linkedSources.has(item.id) }"
          :aria-pressed="selectedSource === item.id" :disabled="readonly || item.disabled" @click="selectSource(item)"
        ><slot name="source" :item="item" :linked="linkedSources.has(item.id)" :selected="selectedSource === item.id">{{ item.label }}</slot></BaseButton>
      </div>
      <svg class="link-board-lines" aria-hidden="true"><path v-for="line in lines" :key="line.key" :d="line.path" /></svg>
      <div class="link-board-column">
        <div class="link-board-title">{{ targetTitle ?? t("view.linkBoard.targets") }}</div>
        <BaseButton
          v-for="item in targets" :key="item.id" :ref="(value) => registerEndpoint('target', item.id, value)"
          class="link-board-endpoint" :class="{ 'is-linked': linkedTargets.has(item.id) }"
          :disabled="readonly || item.disabled" @click="selectTarget(item)"
        ><slot name="target" :item="item" :linked="linkedTargets.has(item.id)">{{ item.label }}</slot></BaseButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.locus-link-board { min-width: 0; min-height: 0; overflow: auto; color: var(--text-color); }
.link-board-content { position: relative; display: grid; grid-template-columns: minmax(120px, 1fr) minmax(120px, 1fr); gap: 64px; min-width: 304px; padding: 12px; }
.link-board-column { position: relative; z-index: 1; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.link-board-title { padding-bottom: 2px; color: var(--text-secondary); font-size: 12px; }
.link-board-endpoint { justify-content: flex-start; min-width: 0; text-align: left; white-space: normal; background: var(--panel-bg); }
.link-board-endpoint.is-linked { border-color: var(--border-strong); color: var(--text-color); }
.link-board-endpoint.is-selected { border-color: var(--accent-color); background: var(--accent-soft); }
.link-board-lines { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; overflow: visible; }
.link-board-lines path { fill: none; stroke: var(--text-secondary); stroke-width: 1.5; }
</style>
