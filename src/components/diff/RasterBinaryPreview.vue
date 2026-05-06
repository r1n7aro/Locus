<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { t } from "../../i18n";
import { refetchDiffByKey } from "../../services/diff";
import { acquireSelectionLock } from "../../composables/useSelectionLock";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../../composables/resizeObserver";
import type {
  AssetBinaryMeta,
  BinaryAssetRef,
  BinaryPreview,
  UnityTexturePreviewMeta,
} from "../../types";
import {
  defaultRasterAlphaMode,
  drawRasterToCanvas,
  getRasterMetaAlphaState,
  resolveAlphaAsTransparency,
  type RasterAlphaMode,
  type RasterChannelMode,
} from "./rasterPreview";

const props = defineProps<{
  preview: BinaryPreview;
  previewKind: "image" | "psd";
  compact?: boolean;
  diffKey: string;
  mode?: "diff" | "neutral";
  assetMeta?: AssetBinaryMeta;
  unityTextureMeta?: UnityTexturePreviewMeta;
}>();

const activeSide = ref<"before" | "after">(props.preview.after ? "after" : "before");
const canvasRef = ref<HTMLCanvasElement | null>(null);
const stageRef = ref<HTMLDivElement | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);
const rasterSource = ref<ImageData | null>(null);
const alphaMode = ref<RasterAlphaMode>(defaultRasterAlphaMode(props.unityTextureMeta));
const channelMode = ref<RasterChannelMode>("color");
const zoomScale = ref(1);
const panX = ref(0);
const panY = ref(0);
const refetchAttempted = ref(false);
const stageWidth = ref(0);
const stageHeight = ref(0);

const activeRef = computed(() =>
  activeSide.value === "before" ? props.preview.before : props.preview.after,
);
const hasBoth = computed(() => !!props.preview.before && !!props.preview.after);
const statusLabel = computed(() => {
  if (props.mode === "neutral") return null;
  if (hasBoth.value) return null;
  return props.preview.after ? t("diff.change.added") : t("diff.change.removed");
});
const effectiveAlphaAsTransparency = computed(() => resolveAlphaAsTransparency(alphaMode.value));
const metaAlphaState = computed(() => getRasterMetaAlphaState(props.unityTextureMeta));
const metaHint = computed(() => {
  if (metaAlphaState.value === "enabled") return t("asset.preview.raster.metaAlphaOn");
  if (metaAlphaState.value === "disabled") return t("asset.preview.raster.metaAlphaOff");
  return null;
});
const loadingLabel = computed(() =>
  props.previewKind === "psd"
    ? t("asset.preview.raster.loadingPsd")
    : t("asset.preview.raster.loadingImage"),
);
const usesAlphaMode = computed(() => channelMode.value === "color");
const showTransparentBackdrop = computed(() =>
  usesAlphaMode.value && effectiveAlphaAsTransparency.value,
);
const fitScale = computed(() => {
  if (props.compact) return 1;

  const source = rasterSource.value;
  if (!source || !stageWidth.value || !stageHeight.value) return 1;

  const gutter = 48;
  const availableWidth = Math.max(stageWidth.value - gutter * 2, 120);
  const availableHeight = Math.max(stageHeight.value - gutter * 2, 120);
  const sourceWidth = Math.max(source.width, 1);
  const sourceHeight = Math.max(source.height, 1);
  const viewportFit = Math.min(availableWidth / sourceWidth, availableHeight / sourceHeight);
  const targetLongEdge = Math.min(
    Math.max(Math.min(availableWidth, availableHeight) * 0.6, 220),
    420,
  );
  const normalizedScale = targetLongEdge / Math.max(sourceWidth, sourceHeight);

  return Math.max(0.1, Math.min(6, Math.min(viewportFit, normalizedScale)));
});
const effectiveScale = computed(() => (props.compact ? 1 : fitScale.value * zoomScale.value));
const zoomLabel = computed(() => `${Math.round(effectiveScale.value * 100)}%`);
const rasterSizeLabel = computed(() => {
  const source = rasterSource.value;
  if (!source) return null;
  return `${source.width}x${source.height}`;
});
const fileSizeLabel = computed(() => {
  const byteSize = props.assetMeta?.size ?? activeRef.value?.byteSize;
  return typeof byteSize === "number" ? formatSize(byteSize) : null;
});
const formatLabel = computed(() => {
  const ext = props.assetMeta?.ext?.trim().toLowerCase();
  if (ext) {
    if (ext === "jpg" || ext === "jpeg") return "JPEG";
    if (ext === "webp") return "WebP";
    return ext.toUpperCase();
  }

  const mime = activeRef.value?.mimeType;
  if (!mime) return null;
  return mime.split("/").pop()?.toUpperCase() ?? null;
});
const colorSpaceLabel = computed(() => {
  const colorSpace = (rasterSource.value as (ImageData & { colorSpace?: string }) | null)?.colorSpace;
  if (!colorSpace) return null;
  if (colorSpace === "srgb") return "sRGB";
  if (colorSpace === "display-p3") return "Display P3";
  return colorSpace;
});
const infoName = computed(() => props.assetMeta?.name ?? null);
const infoItems = computed(() =>
  [
    rasterSizeLabel.value,
    formatLabel.value,
    colorSpaceLabel.value,
    fileSizeLabel.value,
  ].filter((item): item is string => !!item),
);
const showInfoBar = computed(() =>
  !props.compact
  && !!rasterSource.value
  && (props.mode === "neutral" || !!props.assetMeta)
  && (!!infoName.value || infoItems.value.length > 0),
);

let dragging = false;
let lastX = 0;
let lastY = 0;
let loadToken = 0;
let agPsdModule: typeof import("ag-psd") | null = null;
let stageResizeObserver: ResizeObserverHandle | null = null;
let releaseSelectionLock: (() => void) | null = null;

async function ensureAgPsd(): Promise<typeof import("ag-psd")> {
  if (agPsdModule) return agPsdModule;
  const mod = await import("ag-psd");
  mod.initializeCanvas(
    (width: number, height: number) => {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      return canvas;
    },
    (width: number, height: number) => {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      return canvas.getContext("2d")!.createImageData(width, height);
    },
  );
  agPsdModule = mod;
  return mod;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function fitToViewport() {
  zoomScale.value = 1;
  panX.value = 0;
  panY.value = 0;
}

function showActualSize() {
  const currentFitScale = fitScale.value;
  zoomScale.value = currentFitScale > 0 ? 1 / currentFitScale : 1;
  panX.value = 0;
  panY.value = 0;
}

function updateStageSize() {
  if (!stageRef.value) return;
  stageWidth.value = stageRef.value.clientWidth;
  stageHeight.value = stageRef.value.clientHeight;
}

function renderRaster() {
  if (!canvasRef.value || !rasterSource.value) return;
  drawRasterToCanvas(
    canvasRef.value,
    rasterSource.value,
    channelMode.value,
    effectiveAlphaAsTransparency.value,
  );
}

async function maybeRefetchDiff() {
  if (props.mode === "neutral" || refetchAttempted.value) return;
  refetchAttempted.value = true;
  try {
    await refetchDiffByKey(props.diffKey);
  } catch {
    // Ignore preview refresh failures and keep the local error visible.
  }
}

async function loadImageSource(assetRef: BinaryAssetRef): Promise<ImageData> {
  const response = await fetch(assetRef.url);
  if (!response.ok) {
    throw new Error("Failed to load image data");
  }

  const blob = await response.blob();
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) {
    throw new Error("2D canvas is unavailable");
  }

  if (typeof createImageBitmap === "function") {
    const bitmap = await createImageBitmap(blob);
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    ctx.drawImage(bitmap, 0, 0);
    const imageData = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
    bitmap.close();
    return imageData;
  }

  const objectUrl = URL.createObjectURL(blob);
  try {
    const image = await new Promise<HTMLImageElement>((resolve, reject) => {
      const img = new Image();
      img.onload = () => resolve(img);
      img.onerror = () => reject(new Error("Failed to decode image data"));
      img.src = objectUrl;
    });
    const width = image.naturalWidth || image.width;
    const height = image.naturalHeight || image.height;
    canvas.width = width;
    canvas.height = height;
    ctx.drawImage(image, 0, 0);
    return ctx.getImageData(0, 0, width, height);
  } finally {
    URL.revokeObjectURL(objectUrl);
  }
}

function canvasToImageData(sourceCanvas: HTMLCanvasElement): ImageData {
  const ctx = sourceCanvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) {
    throw new Error("2D canvas is unavailable");
  }
  return ctx.getImageData(0, 0, sourceCanvas.width, sourceCanvas.height);
}

function pixelDataToImageData(pixelData: { data: ArrayLike<number>; width: number; height: number }): ImageData {
  return new ImageData(new Uint8ClampedArray(pixelData.data), pixelData.width, pixelData.height);
}

async function loadPsdSource(assetRef: BinaryAssetRef): Promise<ImageData> {
  const [agPsd, response] = await Promise.all([
    ensureAgPsd(),
    fetch(assetRef.url),
  ]);

  if (!response.ok) {
    throw new Error("Failed to load PSD data");
  }

  const buffer = await response.arrayBuffer();
  const psd = agPsd.readPsd(new Uint8Array(buffer), {
    skipThumbnail: true,
    skipLayerImageData: true,
  });

  if (psd.imageData) {
    return pixelDataToImageData(psd.imageData);
  }

  if (psd.canvas) {
    return canvasToImageData(psd.canvas as HTMLCanvasElement);
  }

  if (psd.children?.length) {
    for (const layer of psd.children) {
      const layerCanvas = (layer as { canvas?: HTMLCanvasElement }).canvas;
      if (layerCanvas) {
        return canvasToImageData(layerCanvas);
      }
    }
  }

  throw new Error("Unable to render PSD preview");
}

async function loadRaster() {
  const assetRef = activeRef.value;
  if (!assetRef) return;

  const token = ++loadToken;
  loading.value = true;
  error.value = null;
  rasterSource.value = null;

  try {
    const source = props.previewKind === "psd"
      ? await loadPsdSource(assetRef)
      : await loadImageSource(assetRef);
    if (token !== loadToken) return;
    rasterSource.value = source;
    fitToViewport();
    loading.value = false;
    await nextTick();
    if (token !== loadToken) return;
    updateStageSize();
    renderRaster();
  } catch (e) {
    if (token !== loadToken) return;
    const message = e instanceof Error ? e.message : String(e);
    error.value = message;
    await maybeRefetchDiff();
  } finally {
    if (token === loadToken && loading.value) {
      loading.value = false;
    }
  }
}

function onWheel(e: WheelEvent) {
  if (props.compact) return;
  e.preventDefault();
  const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
  zoomScale.value = Math.max(0.25, Math.min(12, zoomScale.value * factor));
}

function onMouseDown(e: MouseEvent) {
  if (props.compact) return;
  dragging = true;
  lastX = e.clientX;
  lastY = e.clientY;
  releaseSelectionLock?.();
  releaseSelectionLock = acquireSelectionLock();
}

function onMouseMove(e: MouseEvent) {
  if (!dragging) return;
  panX.value += e.clientX - lastX;
  panY.value += e.clientY - lastY;
  lastX = e.clientX;
  lastY = e.clientY;
}

function onMouseUp() {
  dragging = false;
  releaseSelectionLock?.();
  releaseSelectionLock = null;
}

watch(activeRef, () => {
  refetchAttempted.value = false;
  fitToViewport();
  loadRaster();
});

watch(
  () => props.unityTextureMeta?.alphaIsTransparency,
  () => {
    alphaMode.value = defaultRasterAlphaMode(props.unityTextureMeta);
  },
);

watch([channelMode, effectiveAlphaAsTransparency], () => {
  renderRaster();
});

watch(stageRef, (el) => {
  stageResizeObserver?.disconnect();
  stageResizeObserver = null;

  if (!el || typeof ResizeObserver === "undefined") {
    updateStageSize();
    return;
  }

  stageResizeObserver = createAnimationFrameResizeObserver(() => {
    updateStageSize();
  });
  stageResizeObserver?.observe(el);
  updateStageSize();
});

onMounted(loadRaster);

onBeforeUnmount(() => {
  stageResizeObserver?.disconnect();
  releaseSelectionLock?.();
  releaseSelectionLock = null;
});
</script>

<template>
  <div class="raster-preview" :class="{ compact }">
    <div v-if="!compact" class="preview-controls">
      <div v-if="hasBoth" class="segmented">
        <button :class="{ active: activeSide === 'before' }" @click="activeSide = 'before'">
          {{ t("asset.preview.raster.before") }}
        </button>
        <button :class="{ active: activeSide === 'after' }" @click="activeSide = 'after'">
          {{ t("asset.preview.raster.after") }}
        </button>
      </div>

      <span v-if="statusLabel" class="status-badge">{{ statusLabel }}</span>

      <div class="control-group">
        <span class="group-label">{{ t("asset.preview.raster.alphaMode") }}</span>
        <div class="segmented">
          <button
            :class="{ active: alphaMode === 'transparent' }"
            :disabled="!usesAlphaMode"
            @click="alphaMode = 'transparent'"
          >
            {{ t("asset.preview.raster.alpha.transparent") }}
          </button>
          <button
            :class="{ active: alphaMode === 'opaque' }"
            :disabled="!usesAlphaMode"
            @click="alphaMode = 'opaque'"
          >
            {{ t("asset.preview.raster.alpha.opaque") }}
          </button>
        </div>
      </div>

      <div class="control-group">
        <span class="group-label">{{ t("asset.preview.raster.channel") }}</span>
        <div class="segmented">
          <button :class="{ active: channelMode === 'color' }" @click="channelMode = 'color'">
            {{ t("asset.preview.raster.channel.color") }}
          </button>
          <button :class="{ active: channelMode === 'r' }" @click="channelMode = 'r'">R</button>
          <button :class="{ active: channelMode === 'g' }" @click="channelMode = 'g'">G</button>
          <button :class="{ active: channelMode === 'b' }" @click="channelMode = 'b'">B</button>
          <button :class="{ active: channelMode === 'a' }" @click="channelMode = 'a'">
            {{ t("asset.preview.raster.channel.alpha") }}
          </button>
        </div>
      </div>

      <span v-if="metaHint" class="meta-chip">{{ metaHint }}</span>
      <span v-if="unityTextureMeta?.importer" class="meta-chip">{{ unityTextureMeta.importer }}</span>
      <span class="toolbar-spacer" />
      <span v-if="rasterSource" class="size-label">{{ zoomLabel }}</span>
      <button class="reset-btn" :title="t('asset.preview.raster.fit')" @click="fitToViewport">
        {{ t("asset.preview.raster.fit") }}
      </button>
      <button class="reset-btn" :title="t('asset.preview.raster.actualSize')" @click="showActualSize">
        1:1
      </button>
    </div>

    <div v-if="loading" class="preview-loading">{{ loadingLabel }}</div>
    <div v-else-if="error" class="preview-fallback">{{ error }}</div>
    <div
      v-else-if="rasterSource"
      ref="stageRef"
      class="canvas-stage"
      :class="{ 'is-transparent': showTransparentBackdrop }"
      @wheel="onWheel"
      @mousedown="onMouseDown"
      @mousemove="onMouseMove"
      @mouseup="onMouseUp"
      @mouseleave="onMouseUp"
      @dblclick="fitToViewport"
    >
      <canvas
        ref="canvasRef"
        :style="{
          transform: compact ? 'none' : `translate(${panX}px, ${panY}px) scale(${effectiveScale})`,
        }"
        draggable="false"
      />
    </div>
    <div v-else class="preview-fallback">{{ t("asset.preview.raster.unavailable") }}</div>

    <div v-if="showInfoBar" class="preview-info">
      <div v-if="infoName" class="preview-info-name">{{ infoName }}</div>
      <div class="preview-info-meta">
        <span
          v-for="item in infoItems"
          :key="item"
          class="preview-info-item"
        >
          {{ item }}
        </span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.raster-preview {
  display: flex;
  flex-direction: column;
  width: 100%;
  min-height: 0;
}

.raster-preview.compact {
  max-height: 200px;
}

.preview-controls {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border-color, var(--border));
  background: var(--sidebar-bg, var(--bg-secondary));
  font-size: 12px;
}

.control-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.group-label {
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
}

.segmented {
  display: inline-flex;
}

.segmented button,
.reset-btn {
  padding: 2px 9px;
  border: 1px solid var(--border-color, var(--border));
  background: var(--bg-color, var(--bg-secondary));
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
  line-height: 1.5;
  transition: background-color 0.15s, color 0.15s, border-color 0.15s;
}

.segmented button:first-child {
  border-radius: 4px 0 0 4px;
}

.segmented button + button {
  border-left: none;
}

.segmented button:last-child {
  border-radius: 0 4px 4px 0;
}

.segmented button:hover:not(:disabled),
.reset-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.segmented button.active,
.reset-btn.active {
  background: var(--active-bg, var(--hover-bg));
  color: var(--text-color);
}

.segmented button:disabled {
  opacity: 0.45;
  cursor: default;
}

.status-badge,
.meta-chip {
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
}

.toolbar-spacer {
  flex: 1;
}

.size-label {
  color: var(--text-secondary);
  font-size: 11px;
}

.reset-btn {
  border-radius: 4px;
}

.canvas-stage {
  flex: 1;
  min-height: 120px;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-color);
  cursor: grab;
  padding: 24px;
}

.canvas-stage:active {
  cursor: grabbing;
}

.canvas-stage.is-transparent {
  background-color: var(--bg-color);
  background-image:
    linear-gradient(45deg, rgba(255, 255, 255, 0.04) 25%, transparent 25%, transparent 75%, rgba(255, 255, 255, 0.04) 75%, rgba(255, 255, 255, 0.04)),
    linear-gradient(45deg, rgba(255, 255, 255, 0.04) 25%, transparent 25%, transparent 75%, rgba(255, 255, 255, 0.04) 75%, rgba(255, 255, 255, 0.04));
  background-position: 0 0, 8px 8px;
  background-size: 16px 16px;
}

.compact .canvas-stage {
  cursor: default;
  max-height: 200px;
}

.canvas-stage canvas {
  max-width: 100%;
  max-height: 100%;
  transform-origin: center center;
  user-select: none;
}

.compact .canvas-stage canvas {
  max-height: 200px;
}

.preview-loading,
.preview-fallback {
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 12px;
}

.preview-info {
  flex-shrink: 0;
  padding: 10px 16px 14px;
  border-top: 1px solid var(--border-color, var(--border));
  background: var(--bg-color);
  text-align: center;
}

.preview-info-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.4;
  word-break: break-word;
}

.preview-info-meta {
  margin-top: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  flex-wrap: wrap;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.4;
}

.preview-info-item {
  white-space: nowrap;
}
</style>
