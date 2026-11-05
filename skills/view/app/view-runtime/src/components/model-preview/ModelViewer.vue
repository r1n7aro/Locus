<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Pause, Play, RotateCcw } from "lucide";
import { t } from "../../i18n";
import { createAnimationFrameResizeObserver, type ResizeObserverHandle } from "../../composables/resizeObserver";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseDropdown, { type DropdownOption } from "../ui/BaseDropdown.vue";
import { dataUrlToArrayBuffer, detectModelFormat, type ModelFormat } from "./modelFormat";
import {
  createModelPreviewEngine,
  ModelPreviewError,
  type ModelClipInfo,
  type ModelMaterialMode,
  type ModelPreviewEngine,
  type ModelStats,
} from "./modelPreviewEngine";

const props = withDefaults(defineProps<{
  /** Location of the model bytes: a `data:` URL or an HTTP(S)/blob URL. */
  url: string;
  /** Format implied by the file name; content sniffing takes precedence. */
  format?: ModelFormat | null;
  /** Thumbnail mode: slow auto-rotate, no toolbar and no interaction. */
  compact?: boolean;
  /** Rendering pauses while the viewer is not visible (inactive tab). */
  active?: boolean;
}>(), {
  format: null,
  compact: false,
  active: true,
});

const emit = defineEmits<{
  /** The bytes could not be fetched; the owner may refresh a stale source. */
  (event: "fetchError"): void;
  /** The current source was fetched and parsed. */
  (event: "loaded"): void;
}>();

const viewportRef = ref<HTMLDivElement | null>(null);
const loading = ref(true);
const errorMessage = ref("");
const stats = ref<ModelStats | null>(null);
const clips = ref<ModelClipInfo[]>([]);
const clipIndex = ref(0);
const playing = ref(!props.compact);
const showGrid = ref(!props.compact);
const wireframe = ref(false);
const materialMode = ref<ModelMaterialMode>("original");

let engine: ModelPreviewEngine | null = null;
let enginePromise: Promise<ModelPreviewEngine> | null = null;
let resizeObserver: ResizeObserverHandle | null = null;
let disposed = false;
let loadToken = 0;

/** One engine (one WebGL context) per viewer, shared by concurrent loads. */
function ensureEngine(host: HTMLElement): Promise<ModelPreviewEngine> {
  if (enginePromise) return enginePromise;
  const pending = createModelPreviewEngine(host, { compact: props.compact }).then((created) => {
    if (disposed) {
      created.dispose();
      throw new Error("Model viewer was disposed");
    }
    engine = created;
    created.setActive(props.active);
    resizeObserver = createAnimationFrameResizeObserver(() => created.resize());
    resizeObserver?.observe(host);
    return created;
  });
  enginePromise = pending;
  pending.catch(() => {
    if (enginePromise === pending) enginePromise = null;
  });
  return pending;
}

const sizeLabel = computed(() => {
  const size = stats.value?.size;
  if (!size) return "";
  const format = (value: number) => Number(value.toPrecision(3)).toString();
  return t("model.preview.statsSize", format(size[0]), format(size[1]), format(size[2]));
});
const summaryLabel = computed(() => {
  const current = stats.value;
  if (!current) return "";
  return current.meshes
    ? t("model.preview.stats", current.meshes, current.triangles.toLocaleString(), current.materials)
    : t("model.preview.statsRig", current.bones);
});

function clipLabel(clip: ModelClipInfo, index: number): string {
  return clip.name || t("model.preview.clipUnnamed", index + 1);
}

const clipOptions = computed<DropdownOption[]>(() =>
  clips.value.map((clip, index) => ({ value: String(index), label: clipLabel(clip, index) })),
);

async function fetchBuffer(url: string): Promise<ArrayBuffer> {
  if (url.startsWith("data:")) return dataUrlToArrayBuffer(url);
  const response = await fetch(url);
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.arrayBuffer();
}

async function loadModel(): Promise<void> {
  const token = ++loadToken;
  const stale = () => disposed || token !== loadToken;
  loading.value = true;
  errorMessage.value = "";
  let buffer: ArrayBuffer;
  try {
    buffer = await fetchBuffer(props.url);
  } catch {
    if (stale()) return;
    loading.value = false;
    errorMessage.value = t("model.preview.error.load");
    emit("fetchError");
    return;
  }
  if (stale()) return;
  const format = detectModelFormat(buffer, props.format);
  if (!format) {
    loading.value = false;
    errorMessage.value = t("model.preview.error.format");
    return;
  }
  const host = viewportRef.value;
  if (!host) return;
  let target: ModelPreviewEngine;
  try {
    target = await ensureEngine(host);
  } catch {
    if (stale()) return;
    loading.value = false;
    errorMessage.value = t("model.preview.error.webgl");
    return;
  }
  if (stale()) return;
  try {
    target.setGridVisible(showGrid.value);
    target.setWireframe(wireframe.value);
    target.setMaterialMode(materialMode.value);
    target.setPlaying(playing.value);
    const loaded = await target.load(buffer, format);
    if (!loaded || stale()) return;
    stats.value = loaded.stats;
    clips.value = loaded.clips;
    clipIndex.value = 0;
    loading.value = false;
    emit("loaded");
  } catch (cause) {
    if (stale()) return;
    loading.value = false;
    stats.value = null;
    clips.value = [];
    if (cause instanceof ModelPreviewError && cause.code === "unsupportedCompression") {
      errorMessage.value = t("model.preview.error.unsupportedCompression");
    } else {
      errorMessage.value = t("model.preview.error.parse", cause instanceof Error ? cause.message : String(cause));
    }
  }
}

function resetView(): void {
  engine?.resetView();
}

function toggleGrid(): void {
  showGrid.value = !showGrid.value;
  engine?.setGridVisible(showGrid.value);
}

function toggleWireframe(): void {
  wireframe.value = !wireframe.value;
  engine?.setWireframe(wireframe.value);
}

function toggleClay(): void {
  materialMode.value = materialMode.value === "clay" ? "original" : "clay";
  engine?.setMaterialMode(materialMode.value);
}

function togglePlaying(): void {
  playing.value = !playing.value;
  engine?.setPlaying(playing.value);
}

function onClipChange(value: string): void {
  clipIndex.value = Number(value) || 0;
  engine?.selectClip(clipIndex.value);
  engine?.setPlaying(playing.value);
}

onMounted(() => { void loadModel(); });
watch(() => props.url, () => { void loadModel(); });
watch(() => props.active, (active) => engine?.setActive(active));

onBeforeUnmount(() => {
  disposed = true;
  resizeObserver?.disconnect();
  resizeObserver = null;
  engine?.dispose();
  engine = null;
});
</script>

<template>
  <div class="model-viewer" :class="{ compact }" data-model-viewer>
    <div v-if="!compact && !errorMessage" class="model-viewer-toolbar">
      <button type="button" class="model-viewer-tool" :aria-pressed="showGrid" @click="toggleGrid">
        {{ t("model.preview.grid") }}
      </button>
      <button type="button" class="model-viewer-tool" :aria-pressed="wireframe" @click="toggleWireframe">
        {{ t("model.preview.wireframe") }}
      </button>
      <button type="button" class="model-viewer-tool" :aria-pressed="materialMode === 'clay'" @click="toggleClay">
        {{ t("model.preview.clay") }}
      </button>
      <template v-if="clips.length">
        <span class="model-viewer-separator" aria-hidden="true" />
        <button
          type="button"
          class="model-viewer-tool icon"
          :title="playing ? t('model.preview.pause') : t('model.preview.play')"
          :aria-label="playing ? t('model.preview.pause') : t('model.preview.play')"
          @click="togglePlaying"
        >
          <LucideIcon :icon="playing ? Pause : Play" :size="13" :stroke-width="1.8" />
        </button>
        <BaseDropdown
          v-if="clips.length > 1"
          class="model-viewer-clip"
          size="sm"
          menu-align="start"
          teleport
          :model-value="String(clipIndex)"
          :options="clipOptions"
          :aria-label="t('model.preview.clip')"
          @update:model-value="onClipChange"
        />
      </template>
      <button
        type="button"
        class="model-viewer-tool icon trailing"
        :title="t('model.preview.resetView')"
        :aria-label="t('model.preview.resetView')"
        @click="resetView"
      >
        <LucideIcon :icon="RotateCcw" :size="13" :stroke-width="1.8" />
      </button>
    </div>

    <div class="model-viewer-stage">
      <div ref="viewportRef" class="model-viewport" />
      <div v-if="loading" class="model-viewer-overlay">{{ t("model.preview.loading") }}</div>
      <div v-else-if="errorMessage" class="model-viewer-overlay error" role="alert">{{ errorMessage }}</div>
      <div v-if="stats && !compact && !loading" class="model-viewer-stats">
        <span>{{ summaryLabel }}</span>
        <span>{{ sizeLabel }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.model-viewer {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.model-viewer.compact {
  min-height: 96px;
}

.model-viewer-toolbar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 5px 10px;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  font-size: 11px;
}

.model-viewer-tool {
  height: 24px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  font-family: var(--font-ui);
  font-size: 11px;
  cursor: pointer;
}

.model-viewer-tool.icon {
  width: 26px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.model-viewer-tool.trailing {
  margin-left: auto;
}

.model-viewer-tool:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.model-viewer-tool[aria-pressed="true"] {
  border-color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-color) 18%, transparent);
  color: var(--text-color);
}

.model-viewer-clip {
  max-width: 180px;
}

.model-viewer-separator {
  width: 1px;
  height: 16px;
  margin: 0 4px;
  background: var(--border-color);
}

.model-viewer-stage {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.model-viewport {
  position: absolute;
  inset: 0;
}

.model-viewport :deep(canvas) {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  outline: none;
}

.model-viewer-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 12px;
  pointer-events: none;
}

.model-viewer-overlay.error {
  background: var(--panel-bg);
  color: var(--status-error-fg, var(--text-color));
}

.model-viewer-stats {
  position: absolute;
  left: 10px;
  bottom: 8px;
  display: flex;
  gap: 12px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  pointer-events: none;
  user-select: none;
}
</style>
