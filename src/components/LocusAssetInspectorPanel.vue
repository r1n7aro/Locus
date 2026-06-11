<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { PanelTopOpen, X } from "lucide";
import {
  closeLocusAssetInspectorPanel,
  useLocusAssetInspectorPanel,
} from "../composables/useLocusAssetInspectorPanel";
import {
  LOCUS_ASSET_INSPECTOR_WINDOW_TITLE,
  openLocusAssetInspectorWindow,
  type LocusAssetInspectorWindowPayload,
} from "../services/locusAssetInspectorWindow";
import { t } from "../i18n";
import LucideIcon from "./icons/LucideIcon.vue";
import UnityObjectPreview from "./unity-preview/UnityObjectPreview.vue";
import type {
  UnityObjectPreviewInput,
  UnityObjectPreviewSourceState,
} from "./unity-preview/unityObjectPreview";

type LocusInspectorTarget =
  | {
      kind: "asset";
      path: string;
    }
  | {
      kind: "sceneObject";
      path: string;
      scenePath: string;
      objectPath: string;
    };

interface PanelRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const PANEL_RECT_STORAGE_KEY = "locus:assetInspectorPanelRect";
const PANEL_MIN_WIDTH = 360;
const PANEL_MIN_HEIGHT = 280;
const PANEL_VIEWPORT_MARGIN = 8;
const PANEL_DEFAULT_WIDTH = 520;
const PANEL_DEFAULT_HEIGHT = 640;

const { state } = useLocusAssetInspectorPanel();

const panelRef = ref<HTMLElement | null>(null);
const inspectorTarget = ref<LocusInspectorTarget | null>(null);
const inspectorSourceState = ref<UnityObjectPreviewSourceState>("disk");
const panelRect = ref<PanelRect>(loadInitialRect());

const targetPath = computed(() => inspectorTarget.value?.path ?? "");
const previewModel = computed<UnityObjectPreviewInput>(() => ({
  kind: inspectorTarget.value?.kind ?? "asset",
  path: targetPath.value,
  title: panelTitleName.value,
  writable: true,
  capabilities: {
    inspect: true,
    edit: true,
    preview: true,
    select: true,
    drag: true,
  },
}));
const panelTitleName = computed(() => {
  const path = inspectorTarget.value?.kind === "sceneObject"
    ? inspectorTarget.value.objectPath
    : targetPath.value;
  const segments = path.split("/").filter(Boolean);
  return segments[segments.length - 1] || LOCUS_ASSET_INSPECTOR_WINDOW_TITLE;
});
const inspectorSourceBadgeState = computed(() => {
  if (!targetPath.value) return "disk";
  return inspectorSourceState.value;
});
const inspectorSourceLabel = computed(() => {
  if (inspectorSourceBadgeState.value === "live") return t("asset.inspector.source.live");
  if (inspectorSourceBadgeState.value === "loading") return t("asset.inspector.source.loading");
  return t("asset.inspector.source.disk");
});
const inspectorSourceTitle = computed(() => {
  if (inspectorSourceBadgeState.value === "live") return t("asset.inspector.source.title.live");
  if (inspectorSourceBadgeState.value === "loading") return t("asset.inspector.source.title.loading");
  return t("asset.inspector.source.title.disk");
});
const panelStyle = computed(() => ({
  left: `${panelRect.value.x}px`,
  top: `${panelRect.value.y}px`,
  width: `${panelRect.value.width}px`,
  height: `${panelRect.value.height}px`,
}));

function normalizeAssetPath(value: string | null | undefined): string {
  return (value ?? "").trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function inspectorTargetKey(target: LocusInspectorTarget | null): string {
  return target ? `${target.kind}:${target.path}` : "";
}

function applyPanelPayload(next: LocusAssetInspectorWindowPayload | null) {
  const previousKey = inspectorTargetKey(inspectorTarget.value);
  const assetPath = normalizeAssetPath(next?.assetPath);
  const scenePath = normalizeAssetPath(next?.scenePath);
  const objectPath = normalizeAssetPath(next?.objectPath);
  if (next?.kind === "sceneObject" && scenePath && objectPath) {
    inspectorTarget.value = {
      kind: "sceneObject",
      path: `${scenePath}/${objectPath}`,
      scenePath,
      objectPath,
    };
  } else {
    inspectorTarget.value = assetPath
      ? { kind: "asset", path: assetPath }
      : null;
  }
  // Same target => the preview keeps its state and will not re-emit
  // source-change, so resetting the badge here would leave it stuck.
  if (inspectorTargetKey(inspectorTarget.value) !== previousKey) {
    inspectorSourceState.value = targetPath.value ? "loading" : "disk";
  }
}

function handlePreviewSourceChange(state: UnityObjectPreviewSourceState) {
  inspectorSourceState.value = state;
}

// ── Geometry ───────────────────────────────────────────────────────────

function defaultRect(): PanelRect {
  const vw = typeof window !== "undefined" ? window.innerWidth : 1280;
  const vh = typeof window !== "undefined" ? window.innerHeight : 800;
  const width = Math.min(PANEL_DEFAULT_WIDTH, Math.max(PANEL_MIN_WIDTH, vw - PANEL_VIEWPORT_MARGIN * 2));
  const height = Math.min(PANEL_DEFAULT_HEIGHT, Math.max(PANEL_MIN_HEIGHT, vh - PANEL_VIEWPORT_MARGIN * 2));
  return clampRect({
    x: vw - width - 24,
    y: 64,
    width,
    height,
  });
}

function clampRect(rect: PanelRect): PanelRect {
  const vw = typeof window !== "undefined" ? window.innerWidth : 1280;
  const vh = typeof window !== "undefined" ? window.innerHeight : 800;
  const maxWidth = Math.max(PANEL_MIN_WIDTH, vw - PANEL_VIEWPORT_MARGIN * 2);
  const maxHeight = Math.max(PANEL_MIN_HEIGHT, vh - PANEL_VIEWPORT_MARGIN * 2);
  const width = Math.min(Math.max(rect.width, PANEL_MIN_WIDTH), maxWidth);
  const height = Math.min(Math.max(rect.height, PANEL_MIN_HEIGHT), maxHeight);
  const x = Math.min(
    Math.max(rect.x, PANEL_VIEWPORT_MARGIN),
    Math.max(PANEL_VIEWPORT_MARGIN, vw - width - PANEL_VIEWPORT_MARGIN),
  );
  const y = Math.min(
    Math.max(rect.y, PANEL_VIEWPORT_MARGIN),
    Math.max(PANEL_VIEWPORT_MARGIN, vh - height - PANEL_VIEWPORT_MARGIN),
  );
  return { x, y, width, height };
}

function clampSpan(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}

/**
 * Resize keeping the edges opposite to the dragged handle anchored. clampRect
 * is unsuitable here: it clamps position and size independently, which would
 * let the anchored edge drift once the panel hits a min size or the viewport
 * margin.
 */
function resizeRect(
  origin: PanelRect,
  direction: PanelResizeDirection,
  dx: number,
  dy: number,
): PanelRect {
  const vw = typeof window !== "undefined" ? window.innerWidth : 1280;
  const vh = typeof window !== "undefined" ? window.innerHeight : 800;
  const rect = { ...origin };
  if (direction.includes("e")) {
    rect.width = clampSpan(origin.width + dx, PANEL_MIN_WIDTH, vw - PANEL_VIEWPORT_MARGIN - origin.x);
  } else if (direction.includes("w")) {
    const right = origin.x + origin.width;
    rect.width = clampSpan(origin.width - dx, PANEL_MIN_WIDTH, right - PANEL_VIEWPORT_MARGIN);
    rect.x = right - rect.width;
  }
  if (direction.includes("s")) {
    rect.height = clampSpan(origin.height + dy, PANEL_MIN_HEIGHT, vh - PANEL_VIEWPORT_MARGIN - origin.y);
  } else if (direction.includes("n")) {
    const bottom = origin.y + origin.height;
    rect.height = clampSpan(origin.height - dy, PANEL_MIN_HEIGHT, bottom - PANEL_VIEWPORT_MARGIN);
    rect.y = bottom - rect.height;
  }
  return rect;
}

function loadInitialRect(): PanelRect {
  try {
    const raw = localStorage.getItem(PANEL_RECT_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<PanelRect>;
      if (
        typeof parsed.x === "number" && Number.isFinite(parsed.x)
        && typeof parsed.y === "number" && Number.isFinite(parsed.y)
        && typeof parsed.width === "number" && Number.isFinite(parsed.width)
        && typeof parsed.height === "number" && Number.isFinite(parsed.height)
      ) {
        return clampRect(parsed as PanelRect);
      }
    }
  } catch { /* ignore */ }
  return defaultRect();
}

function persistRect() {
  try {
    localStorage.setItem(PANEL_RECT_STORAGE_KEY, JSON.stringify(panelRect.value));
  } catch { /* ignore */ }
}

function handleWindowResize() {
  panelRect.value = clampRect(panelRect.value);
}

// ── Drag / resize ──────────────────────────────────────────────────────

interface PanelPointerSession {
  pointerId: number;
  startClientX: number;
  startClientY: number;
  originRect: PanelRect;
}

type PanelResizeDirection = "n" | "s" | "e" | "w" | "nw" | "ne" | "sw" | "se";

interface PanelResizeSession extends PanelPointerSession {
  direction: PanelResizeDirection;
}

/** Transparent hit areas; the south-east corner keeps the visible glyph handle. */
const PANEL_EDGE_RESIZE_DIRECTIONS: readonly PanelResizeDirection[] = ["n", "s", "e", "w", "nw", "ne", "sw"];

const dragSession = ref<PanelPointerSession | null>(null);
const resizeSession = ref<PanelResizeSession | null>(null);
const panelInteracting = computed(() => !!dragSession.value || !!resizeSession.value);

function shouldIgnorePanelDrag(target: EventTarget | null): boolean {
  return target instanceof Element
    && !!target.closest("button, a, input, textarea, select, [data-panel-no-drag]");
}

function handleTitlebarPointerDown(event: PointerEvent) {
  if (event.button !== 0 || dragSession.value || resizeSession.value) return;
  if (shouldIgnorePanelDrag(event.target)) return;
  const titlebar = event.currentTarget as HTMLElement | null;
  if (!titlebar) return;
  event.preventDefault();
  try {
    titlebar.setPointerCapture(event.pointerId);
  } catch { /* ignore */ }
  dragSession.value = {
    pointerId: event.pointerId,
    startClientX: event.clientX,
    startClientY: event.clientY,
    originRect: { ...panelRect.value },
  };
}

function handleTitlebarPointerMove(event: PointerEvent) {
  const session = dragSession.value;
  if (!session || event.pointerId !== session.pointerId) return;
  panelRect.value = clampRect({
    ...session.originRect,
    x: session.originRect.x + (event.clientX - session.startClientX),
    y: session.originRect.y + (event.clientY - session.startClientY),
  });
}

function handleTitlebarPointerEnd(event: PointerEvent) {
  const session = dragSession.value;
  if (!session || event.pointerId !== session.pointerId) return;
  dragSession.value = null;
  persistRect();
}

function handleResizePointerDown(event: PointerEvent, direction: PanelResizeDirection) {
  if (event.button !== 0 || dragSession.value || resizeSession.value) return;
  const handle = event.currentTarget as HTMLElement | null;
  if (!handle) return;
  event.preventDefault();
  try {
    handle.setPointerCapture(event.pointerId);
  } catch { /* ignore */ }
  resizeSession.value = {
    pointerId: event.pointerId,
    startClientX: event.clientX,
    startClientY: event.clientY,
    originRect: { ...panelRect.value },
    direction,
  };
}

function handleResizePointerMove(event: PointerEvent) {
  const session = resizeSession.value;
  if (!session || event.pointerId !== session.pointerId) return;
  panelRect.value = resizeRect(
    session.originRect,
    session.direction,
    event.clientX - session.startClientX,
    event.clientY - session.startClientY,
  );
}

function handleResizePointerEnd(event: PointerEvent) {
  const session = resizeSession.value;
  if (!session || event.pointerId !== session.pointerId) return;
  resizeSession.value = null;
  persistRect();
}

// ── Actions ────────────────────────────────────────────────────────────

function closePanel() {
  closeLocusAssetInspectorPanel();
}

async function popOutToWindow() {
  const target = inspectorTarget.value;
  if (!target) return;
  const payload: LocusAssetInspectorWindowPayload = target.kind === "sceneObject"
    ? { kind: "sceneObject", scenePath: target.scenePath, objectPath: target.objectPath }
    : { assetPath: target.path };
  try {
    const opened = await openLocusAssetInspectorWindow(payload);
    if (opened) closePanel();
  } catch (error) {
    console.warn("[LocusAssetInspectorPanel] pop out failed:", error);
  }
}

function handlePanelKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  const target = event.target as HTMLElement | null;
  // ESC inside form controls means "cancel the edit", not "close the panel".
  if (target?.closest("input, textarea, select, [contenteditable='true']")) return;
  event.preventDefault();
  closePanel();
}

function focusPanel() {
  void nextTick(() => {
    panelRef.value?.focus({ preventScroll: true });
  });
}

watch(
  () => state.revision,
  () => {
    applyPanelPayload(state.payload);
    panelRect.value = clampRect(panelRect.value);
    focusPanel();
  },
  { immediate: true },
);

onMounted(() => {
  window.addEventListener("resize", handleWindowResize);
  focusPanel();
});

onUnmounted(() => {
  window.removeEventListener("resize", handleWindowResize);
});
</script>

<template>
  <div
    ref="panelRef"
    class="locus-asset-inspector-panel"
    :class="{ 'is-interacting': panelInteracting }"
    :style="panelStyle"
    role="dialog"
    :aria-label="t('asset.inspector.windowTitle')"
    tabindex="-1"
    @keydown="handlePanelKeydown"
  >
    <div
      class="locus-asset-inspector-panel-titlebar"
      @pointerdown="handleTitlebarPointerDown"
      @pointermove="handleTitlebarPointerMove"
      @pointerup="handleTitlebarPointerEnd"
      @pointercancel="handleTitlebarPointerEnd"
    >
      <div class="locus-asset-inspector-panel-title">
        <span class="locus-asset-inspector-panel-title-main">{{ t("asset.inspector.windowTitle") }}</span>
        <span class="locus-asset-inspector-panel-title-path" :title="targetPath">{{ targetPath }}</span>
      </div>
      <span
        class="locus-asset-inspector-panel-source"
        :class="`source-${inspectorSourceBadgeState}`"
        :title="inspectorSourceTitle"
      >
        {{ inspectorSourceLabel }}
      </span>
      <button
        type="button"
        class="locus-asset-inspector-panel-btn"
        data-panel-no-drag
        :title="t('asset.inspector.panel.popOut')"
        @click="popOutToWindow"
      >
        <LucideIcon :icon="PanelTopOpen" :size="14" />
      </button>
      <button
        type="button"
        class="locus-asset-inspector-panel-btn"
        data-panel-no-drag
        :title="t('asset.inspector.panel.close')"
        @click="closePanel"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <div class="locus-asset-inspector-panel-preview">
      <div v-if="!targetPath" class="locus-asset-inspector-panel-state">
        {{ t("asset.inspector.missingAsset") }}
      </div>
      <UnityObjectPreview
        v-else
        :key="`${previewModel.kind}:${targetPath}`"
        :model="previewModel"
        level="inspector"
        :auto-load-preview="true"
        @source-change="handlePreviewSourceChange"
      />
    </div>

    <div
      class="locus-asset-inspector-panel-resize"
      :title="t('asset.inspector.panel.resize')"
      @pointerdown="handleResizePointerDown($event, 'se')"
      @pointermove="handleResizePointerMove"
      @pointerup="handleResizePointerEnd"
      @pointercancel="handleResizePointerEnd"
    >
      <svg viewBox="0 0 8 8" width="8" height="8" aria-hidden="true">
        <path d="M7 1v6H1" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      </svg>
    </div>

    <div
      v-for="direction in PANEL_EDGE_RESIZE_DIRECTIONS"
      :key="direction"
      class="locus-asset-inspector-panel-resize-handle"
      :class="`resize-${direction}`"
      aria-hidden="true"
      @pointerdown="handleResizePointerDown($event, direction)"
      @pointermove="handleResizePointerMove"
      @pointerup="handleResizePointerEnd"
      @pointercancel="handleResizePointerEnd"
    ></div>
  </div>
</template>

<style scoped>
.locus-asset-inspector-panel {
  position: fixed;
  z-index: 180;
  /* WebView2 resolves -webkit-app-region natively and ignores z-order, so the
     panel must carve itself out of the tab-bar drag region it may float over;
     otherwise pressing the panel drags the whole main window. */
  -webkit-app-region: no-drag;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  background: var(--panel-bg);
  color: var(--text-color);
  box-shadow: 0 12px 36px rgba(0, 0, 0, 0.28), 0 2px 10px rgba(0, 0, 0, 0.16);
  outline: none;
}

.locus-asset-inspector-panel.is-interacting {
  user-select: none;
  -webkit-user-select: none;
}

.locus-asset-inspector-panel.is-interacting :deep(*) {
  user-select: none !important;
  -webkit-user-select: none !important;
}

.locus-asset-inspector-panel-titlebar {
  min-height: 36px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 0 8px 0 12px;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  cursor: grab;
  touch-action: none;
}

.locus-asset-inspector-panel.is-interacting .locus-asset-inspector-panel-titlebar {
  cursor: grabbing;
}

.locus-asset-inspector-panel-title {
  min-width: 0;
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  gap: 8px;
}

.locus-asset-inspector-panel-title-main {
  flex-shrink: 0;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.locus-asset-inspector-panel-title-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  /* Truncate from the left so the file name (path tail) stays visible. */
  direction: rtl;
  text-align: left;
}

.locus-asset-inspector-panel-source {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  min-height: 20px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1;
}

.locus-asset-inspector-panel-source.source-live {
  border-color: color-mix(in srgb, var(--status-good-fg) 34%, var(--border-color));
  background: color-mix(in srgb, var(--status-good-bg) 22%, var(--panel-bg));
  color: var(--status-good-fg);
}

.locus-asset-inspector-panel-source.source-loading {
  color: var(--text-secondary);
}

.locus-asset-inspector-panel-btn {
  width: 26px;
  height: 26px;
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

.locus-asset-inspector-panel-btn:hover,
.locus-asset-inspector-panel-btn:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.locus-asset-inspector-panel-preview {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.locus-asset-inspector-panel-preview :deep(.unity-object-preview.level-inspector) {
  flex: 1;
  border: 0;
  border-radius: 0;
}

.locus-asset-inspector-panel-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  color: var(--text-secondary);
  font-size: 12px;
}

.locus-asset-inspector-panel-resize {
  position: absolute;
  right: 0;
  bottom: 0;
  z-index: 7;
  width: 16px;
  height: 16px;
  display: flex;
  align-items: flex-end;
  justify-content: flex-end;
  padding: 0 3px 3px 0;
  color: var(--text-tertiary);
  cursor: nwse-resize;
  touch-action: none;
}

.locus-asset-inspector-panel-resize:hover {
  color: var(--text-secondary);
}

/* Transparent edge / corner resize hit areas. The panel clips its children
   (overflow: hidden), so the handles sit inside the border instead of
   straddling it. Edges stop short of the corners so the diagonal handles win
   there; corners also sit one z-level above the edges. */
.locus-asset-inspector-panel-resize-handle {
  position: absolute;
  z-index: 6;
  touch-action: none;
}

.locus-asset-inspector-panel-resize-handle.resize-n,
.locus-asset-inspector-panel-resize-handle.resize-s {
  left: 14px;
  right: 14px;
  height: 5px;
  cursor: ns-resize;
}

.locus-asset-inspector-panel-resize-handle.resize-n {
  top: 0;
}

.locus-asset-inspector-panel-resize-handle.resize-s {
  bottom: 0;
}

.locus-asset-inspector-panel-resize-handle.resize-e,
.locus-asset-inspector-panel-resize-handle.resize-w {
  top: 14px;
  bottom: 14px;
  width: 5px;
  cursor: ew-resize;
}

.locus-asset-inspector-panel-resize-handle.resize-e {
  right: 0;
}

.locus-asset-inspector-panel-resize-handle.resize-w {
  left: 0;
}

.locus-asset-inspector-panel-resize-handle.resize-nw,
.locus-asset-inspector-panel-resize-handle.resize-ne,
.locus-asset-inspector-panel-resize-handle.resize-sw {
  z-index: 7;
  width: 12px;
  height: 12px;
}

.locus-asset-inspector-panel-resize-handle.resize-nw {
  top: 0;
  left: 0;
  cursor: nwse-resize;
}

.locus-asset-inspector-panel-resize-handle.resize-ne {
  top: 0;
  right: 0;
  cursor: nesw-resize;
}

.locus-asset-inspector-panel-resize-handle.resize-sw {
  bottom: 0;
  left: 0;
  cursor: nesw-resize;
}
</style>
