<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { Maximize, X, ZoomIn, ZoomOut } from "lucide";
import LucideIcon from "../icons/LucideIcon.vue";
import { t } from "../../i18n";
import {
  clampImageView,
  fitImageScale,
  imageClickZoomScale,
  imageScaleBounds,
  zoomImageView,
  type ImageLightboxImage,
  type ImageLightboxSize,
  type ImageLightboxView,
} from "./imageLightboxGeometry";

const props = defineProps<{ image: ImageLightboxImage | null }>();
const emit = defineEmits<{ close: [] }>();

const ZOOM_STEP = 1.5;
const SCALE_EPSILON = 1e-3;
const rootRef = ref<HTMLElement | null>(null);
const stageRef = ref<HTMLElement | null>(null);
const imageRef = ref<HTMLImageElement | null>(null);
const natural = ref<ImageLightboxSize>({ width: 0, height: 0 });
const stage = ref<ImageLightboxSize>({ width: 0, height: 0 });
const view = ref<ImageLightboxView>({ scale: 1, x: 0, y: 0 });
const status = ref<"loading" | "ready" | "error">("loading");
const smooth = ref(false);
const dragging = ref(false);
let drag: { pointerId: number; x: number; y: number; viewX: number; viewY: number; moved: boolean } | null = null;
let suppressClick = false;
let restoreFocus: HTMLElement | null = null;
let listening = false;

const ready = computed(() => status.value === "ready");
const bounds = computed(() => imageScaleBounds(natural.value, stage.value));
const fitScale = computed(() => fitImageScale(natural.value, stage.value));
const atFit = computed(() => view.value.scale <= fitScale.value + SCALE_EPSILON);
const pannable = computed(() => ready.value && (
  natural.value.width * view.value.scale > stage.value.width + 0.5
  || natural.value.height * view.value.scale > stage.value.height + 0.5
));
const percent = computed(() => `${Math.round(view.value.scale * 100)}%`);
const caption = computed(() => props.image?.alt?.trim() ?? "");
const imageStyle = computed(() => {
  if (!ready.value) return undefined;
  const width = natural.value.width * view.value.scale;
  const height = natural.value.height * view.value.scale;
  return {
    width: `${width}px`,
    height: `${height}px`,
    left: `${stage.value.width / 2 - width / 2 + view.value.x}px`,
    top: `${stage.value.height / 2 - height / 2 + view.value.y}px`,
  };
});

function measureStage(): void {
  const element = stageRef.value;
  stage.value = {
    width: element?.clientWidth || window.innerWidth,
    height: element?.clientHeight || window.innerHeight,
  };
}

function setView(next: ImageLightboxView, animate: boolean): void {
  smooth.value = animate;
  view.value = next;
}

function anchorFor(event: { clientX: number; clientY: number }): { x: number; y: number } {
  const rect = stageRef.value?.getBoundingClientRect();
  if (!rect || !rect.width || !rect.height) return { x: 0, y: 0 };
  return { x: event.clientX - rect.left - rect.width / 2, y: event.clientY - rect.top - rect.height / 2 };
}

function zoomTo(scale: number, anchor = { x: 0, y: 0 }, animate = true): void {
  if (!ready.value) return;
  setView(zoomImageView(view.value, scale, anchor, natural.value, stage.value), animate);
}

function fit(animate = true): void {
  if (!ready.value) return;
  setView(clampImageView({ scale: fitScale.value, x: 0, y: 0 }, natural.value, stage.value), animate);
}

/** Grows the enlarged image out of the inline image so the dialog reads as a floating zoom. */
function growFromOrigin(): void {
  const image = imageRef.value;
  const origin = props.image?.originRect;
  if (!image || !origin || origin.width <= 0 || origin.height <= 0 || typeof image.animate !== "function") return;
  if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
  const target = image.getBoundingClientRect();
  if (target.width <= 0 || target.height <= 0) return;
  image.animate([
    {
      transform: `translate(${origin.left - target.left}px, ${origin.top - target.top}px) `
        + `scale(${origin.width / target.width}, ${origin.height / target.height})`,
    },
    { transform: "none" },
  ], { duration: 240, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" });
}

function handleLoad(): void {
  const image = imageRef.value;
  if (!image || !props.image) return;
  const origin = props.image.originRect;
  natural.value = {
    width: image.naturalWidth || origin?.width || 300,
    height: image.naturalHeight || origin?.height || 150,
  };
  measureStage();
  status.value = "ready";
  fit(false);
  void nextTick(growFromOrigin);
}

function handleError(): void {
  if (props.image) status.value = "error";
}

function handleWheel(event: WheelEvent): void {
  if (!ready.value) return;
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? stage.value.height : 1;
  // Pinch gestures arrive as ctrl+wheel with small deltas.
  const factor = Math.exp(-event.deltaY * unit * (event.ctrlKey ? 0.01 : 0.002));
  zoomTo(view.value.scale * factor, anchorFor(event), false);
}

function handlePointerDown(event: PointerEvent): void {
  suppressClick = false;
  if (event.button !== 0 || !ready.value) return;
  drag = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, viewX: view.value.x, viewY: view.value.y, moved: false };
  imageRef.value?.setPointerCapture?.(event.pointerId);
}

function handlePointerMove(event: PointerEvent): void {
  if (!drag || event.pointerId !== drag.pointerId) return;
  const dx = event.clientX - drag.x;
  const dy = event.clientY - drag.y;
  if (!drag.moved && Math.hypot(dx, dy) < 4) return;
  drag.moved = true;
  if (!pannable.value) return;
  dragging.value = true;
  setView(clampImageView({ scale: view.value.scale, x: drag.viewX + dx, y: drag.viewY + dy }, natural.value, stage.value), false);
}

function handlePointerEnd(event: PointerEvent): void {
  if (!drag || event.pointerId !== drag.pointerId) return;
  suppressClick = drag.moved;
  drag = null;
  dragging.value = false;
}

function handleImageClick(event: MouseEvent): void {
  event.stopPropagation();
  if (suppressClick) { suppressClick = false; return; }
  if (!ready.value) return;
  if (atFit.value) zoomTo(imageClickZoomScale(natural.value, stage.value), anchorFor(event));
  else fit();
}

function handleStageClick(event: MouseEvent): void {
  if (event.target !== event.currentTarget) return;
  if (suppressClick) { suppressClick = false; return; }
  emit("close");
}

function trapFocus(event: KeyboardEvent): void {
  const focusable = Array.from(rootRef.value?.querySelectorAll<HTMLElement>("button:not(:disabled)") ?? []);
  event.preventDefault();
  if (!focusable.length) return;
  const index = focusable.indexOf(document.activeElement as HTMLElement);
  const next = event.shiftKey
    ? (index <= 0 ? focusable.length - 1 : index - 1)
    : (index + 1) % focusable.length;
  focusable[next]?.focus();
}

function handleKeydown(event: KeyboardEvent): void {
  if (!props.image) return;
  const plain = !event.ctrlKey && !event.metaKey && !event.altKey;
  if (event.key === "Escape") emit("close");
  else if (event.key === "Tab") trapFocus(event);
  else if (plain && (event.key === "+" || event.key === "=")) zoomTo(view.value.scale * ZOOM_STEP);
  else if (plain && (event.key === "-" || event.key === "_")) zoomTo(view.value.scale / ZOOM_STEP);
  else if (plain && event.key === "0") fit();
  else return;
  event.preventDefault();
  event.stopPropagation();
}

function handleResize(): void {
  if (!ready.value) return;
  const wasFit = atFit.value;
  measureStage();
  if (wasFit) fit(false);
  else setView(clampImageView(view.value, natural.value, stage.value), false);
}

function listen(active: boolean): void {
  if (active === listening) return;
  listening = active;
  // Capture so Escape closes the image before panels underneath react to it.
  if (active) {
    window.addEventListener("keydown", handleKeydown, true);
    window.addEventListener("resize", handleResize);
  } else {
    window.removeEventListener("keydown", handleKeydown, true);
    window.removeEventListener("resize", handleResize);
  }
}

function reset(): void {
  status.value = "loading";
  natural.value = { width: 0, height: 0 };
  setView({ scale: 1, x: 0, y: 0 }, false);
  drag = null;
  dragging.value = false;
  suppressClick = false;
  // Reopening the same source keeps the decoded element, which fires no new load event.
  void nextTick(() => {
    const image = imageRef.value;
    if (status.value === "loading" && image?.complete && image.naturalWidth > 0) handleLoad();
  });
}

watch(() => props.image, (image, previous) => {
  if (image) {
    if (!previous) {
      restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      listen(true);
      void nextTick(() => rootRef.value?.focus({ preventScroll: true }));
    }
    reset();
    return;
  }
  listen(false);
  const focusTarget = restoreFocus;
  restoreFocus = null;
  if (focusTarget?.isConnected) focusTarget.focus({ preventScroll: true });
}, { immediate: true });

onBeforeUnmount(() => listen(false));
</script>

<template>
  <Transition name="image-lightbox">
    <div
      v-if="image"
      ref="rootRef"
      class="image-lightbox"
      role="dialog"
      aria-modal="true"
      :aria-label="caption || t('imageLightbox.title')"
      tabindex="-1"
      @wheel.prevent="handleWheel"
    >
      <div ref="stageRef" class="image-lightbox-stage" @click="handleStageClick">
        <img
          :key="image.src"
          ref="imageRef"
          class="image-lightbox-image"
          :class="{
            'is-ready': ready,
            'is-smooth': smooth,
            'is-zoom-in': ready && atFit,
            'is-zoomed': ready && !atFit,
            'is-pannable': pannable,
            'is-dragging': dragging,
          }"
          :src="image.src"
          :alt="image.alt ?? ''"
          :style="imageStyle"
          draggable="false"
          @load="handleLoad"
          @error="handleError"
          @click="handleImageClick"
          @pointerdown="handlePointerDown"
          @pointermove="handlePointerMove"
          @pointerup="handlePointerEnd"
          @pointercancel="handlePointerEnd"
          @dragstart.prevent
        >
        <span v-if="status === 'error'" class="image-lightbox-status" role="alert">{{ t("imageLightbox.loadFailed") }}</span>
      </div>
      <div class="image-lightbox-toolbar" role="toolbar" :aria-label="t('imageLightbox.title')">
        <button
          type="button"
          :title="t('imageLightbox.zoomOut')"
          :aria-label="t('imageLightbox.zoomOut')"
          :disabled="!ready || view.scale <= bounds.min + SCALE_EPSILON"
          @click="zoomTo(view.scale / ZOOM_STEP)"
        >
          <LucideIcon :icon="ZoomOut" :size="16" />
        </button>
        <button
          type="button"
          class="image-lightbox-percent"
          :title="t('imageLightbox.actualSize')"
          :aria-label="t('imageLightbox.actualSize')"
          :disabled="!ready"
          @click="zoomTo(1)"
        >
          {{ percent }}
        </button>
        <button
          type="button"
          :title="t('imageLightbox.zoomIn')"
          :aria-label="t('imageLightbox.zoomIn')"
          :disabled="!ready || view.scale >= bounds.max - SCALE_EPSILON"
          @click="zoomTo(view.scale * ZOOM_STEP)"
        >
          <LucideIcon :icon="ZoomIn" :size="16" />
        </button>
        <button
          type="button"
          :title="t('imageLightbox.fit')"
          :aria-label="t('imageLightbox.fit')"
          :disabled="!ready || atFit"
          @click="fit()"
        >
          <LucideIcon :icon="Maximize" :size="16" />
        </button>
        <span class="image-lightbox-divider" aria-hidden="true" />
        <button
          type="button"
          class="image-lightbox-close"
          :title="t('imageLightbox.close')"
          :aria-label="t('imageLightbox.close')"
          @click="emit('close')"
        >
          <LucideIcon :icon="X" :size="16" />
        </button>
      </div>
      <div v-if="caption" class="image-lightbox-caption">{{ caption }}</div>
    </div>
  </Transition>
</template>

<style scoped>
.image-lightbox {
  position: fixed;
  inset: 0;
  z-index: 10000;
  background: rgba(8, 8, 10, 0.82);
  backdrop-filter: blur(6px);
  outline: none;
  overscroll-behavior: contain;
  touch-action: none;
  user-select: none;
}

.image-lightbox-stage {
  position: absolute;
  inset: 0;
  overflow: hidden;
  cursor: zoom-out;
}

.image-lightbox-image {
  position: absolute;
  display: block;
  max-width: none;
  max-height: none;
  margin: 0;
  border: none;
  border-radius: 4px;
  background: var(--bg-color, #1e1e1e);
  box-shadow: 0 12px 48px rgba(0, 0, 0, 0.55);
  opacity: 0;
  transform-origin: 0 0;
  cursor: default;
  -webkit-user-drag: none;
}

.image-lightbox-image.is-ready {
  opacity: 1;
}

.image-lightbox-image.is-smooth {
  transition: width 0.2s ease, height 0.2s ease, left 0.2s ease, top 0.2s ease;
}

.image-lightbox-image.is-zoom-in {
  cursor: zoom-in;
}

.image-lightbox-image.is-zoomed {
  cursor: zoom-out;
}

.image-lightbox-image.is-pannable {
  cursor: grab;
}

.image-lightbox-image.is-dragging {
  cursor: grabbing;
}

.image-lightbox-status {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  color: #e8e8ea;
  font-family: var(--font-ui);
  font-size: 13px;
  pointer-events: none;
}

.image-lightbox-toolbar {
  position: absolute;
  top: 12px;
  right: 12px;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 8px;
  background: rgba(24, 24, 28, 0.78);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.35);
  color: #e8e8ea;
  font-family: var(--font-ui);
  font-size: 12px;
}

.image-lightbox-toolbar button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 28px;
  height: 28px;
  padding: 0 6px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}

.image-lightbox-toolbar button:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.12);
}

.image-lightbox-toolbar button:focus-visible {
  outline: 2px solid var(--accent-color, #5b8def);
  outline-offset: -2px;
}

.image-lightbox-toolbar button:disabled {
  opacity: 0.4;
  cursor: default;
}

.image-lightbox-percent {
  min-width: 52px;
  font-variant-numeric: tabular-nums;
}

.image-lightbox-divider {
  width: 1px;
  height: 16px;
  margin: 0 4px;
  background: rgba(255, 255, 255, 0.16);
}

.image-lightbox-caption {
  position: absolute;
  left: 50%;
  bottom: 16px;
  max-width: min(720px, 80vw);
  padding: 6px 12px;
  overflow: hidden;
  transform: translateX(-50%);
  border-radius: 8px;
  background: rgba(24, 24, 28, 0.72);
  color: #e8e8ea;
  font-family: var(--font-ui);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
  pointer-events: none;
}

.image-lightbox-enter-active {
  transition: opacity 0.18s ease;
}

.image-lightbox-leave-active {
  transition: opacity 0.14s ease;
}

.image-lightbox-enter-from,
.image-lightbox-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .image-lightbox-image.is-smooth,
  .image-lightbox-enter-active,
  .image-lightbox-leave-active {
    transition: none;
  }
}
</style>
