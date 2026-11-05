<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../i18n";

// Full-screen image with pinch zoom, pan and double-tap zoom.
defineProps<{ src: string; title: string }>();
const emit = defineEmits<{ back: [] }>();

const MIN_SCALE = 1;
const MAX_SCALE = 8;
const DOUBLE_TAP_MS = 300;
const DOUBLE_TAP_SCALE = 2.5;

const stageRef = ref<HTMLElement | null>(null);
const scale = ref(1);
const offset = ref({ x: 0, y: 0 });
const failed = ref(false);

type Point = { x: number; y: number };
const pointers = new Map<number, Point>();
let gesture: { scale: number; offset: Point; center: Point; distance: number } | null = null;
let lastTap: { at: number; point: Point } | null = null;
let moved = false;

const transform = computed(() => `translate(${offset.value.x}px, ${offset.value.y}px) scale(${scale.value})`);

function local(event: PointerEvent): Point {
  const rect = stageRef.value?.getBoundingClientRect();
  return { x: event.clientX - (rect?.left ?? 0), y: event.clientY - (rect?.top ?? 0) };
}

function clampScale(value: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

/** Keeps the scaled image covering the stage it started in. */
function clampOffset(next: Point, nextScale: number): Point {
  const width = stageRef.value?.clientWidth ?? 0;
  const height = stageRef.value?.clientHeight ?? 0;
  return {
    x: Math.min(0, Math.max(width - width * nextScale, next.x)),
    y: Math.min(0, Math.max(height - height * nextScale, next.y)),
  };
}

/** Scales to `nextScale` keeping the content under `focus` in place. */
function zoomAt(focus: Point, nextScale: number, from = { scale: scale.value, offset: offset.value }) {
  const target = clampScale(nextScale);
  const content = { x: (focus.x - from.offset.x) / from.scale, y: (focus.y - from.offset.y) / from.scale };
  scale.value = target;
  offset.value = clampOffset({ x: focus.x - content.x * target, y: focus.y - content.y * target }, target);
}

function beginGesture() {
  const points = [...pointers.values()];
  if (points.length === 0) {
    gesture = null;
    return;
  }
  const center = points.length > 1
    ? { x: (points[0]!.x + points[1]!.x) / 2, y: (points[0]!.y + points[1]!.y) / 2 }
    : points[0]!;
  const distance = points.length > 1 ? Math.hypot(points[0]!.x - points[1]!.x, points[0]!.y - points[1]!.y) : 0;
  gesture = { scale: scale.value, offset: offset.value, center, distance };
}

function onPointerDown(event: PointerEvent) {
  stageRef.value?.setPointerCapture(event.pointerId);
  pointers.set(event.pointerId, local(event));
  if (pointers.size === 1) moved = false;
  beginGesture();
}

function onPointerMove(event: PointerEvent) {
  if (!pointers.has(event.pointerId) || !gesture) return;
  pointers.set(event.pointerId, local(event));
  const points = [...pointers.values()];
  if (points.length > 1 && gesture.distance > 0) {
    moved = true;
    const center = { x: (points[0]!.x + points[1]!.x) / 2, y: (points[0]!.y + points[1]!.y) / 2 };
    const distance = Math.hypot(points[0]!.x - points[1]!.x, points[0]!.y - points[1]!.y);
    const content = {
      x: (gesture.center.x - gesture.offset.x) / gesture.scale,
      y: (gesture.center.y - gesture.offset.y) / gesture.scale,
    };
    const next = clampScale(gesture.scale * (distance / gesture.distance));
    scale.value = next;
    offset.value = clampOffset({ x: center.x - content.x * next, y: center.y - content.y * next }, next);
    return;
  }
  const point = points[0]!;
  const dx = point.x - gesture.center.x;
  const dy = point.y - gesture.center.y;
  if (Math.abs(dx) + Math.abs(dy) > 6) moved = true;
  if (scale.value > MIN_SCALE) {
    offset.value = clampOffset({ x: gesture.offset.x + dx, y: gesture.offset.y + dy }, scale.value);
  }
}

function onPointerUp(event: PointerEvent) {
  const point = pointers.get(event.pointerId) ?? local(event);
  pointers.delete(event.pointerId);
  beginGesture();
  if (pointers.size > 0 || moved) return;
  const now = performance.now();
  if (lastTap && now - lastTap.at < DOUBLE_TAP_MS && Math.hypot(point.x - lastTap.point.x, point.y - lastTap.point.y) < 32) {
    lastTap = null;
    if (scale.value > MIN_SCALE) {
      scale.value = MIN_SCALE;
      offset.value = { x: 0, y: 0 };
    } else {
      zoomAt(point, DOUBLE_TAP_SCALE);
    }
    return;
  }
  lastTap = { at: now, point };
}

function onPointerCancel(event: PointerEvent) {
  pointers.delete(event.pointerId);
  beginGesture();
}

function reset() {
  scale.value = MIN_SCALE;
  offset.value = { x: 0, y: 0 };
}
</script>

<template>
  <section class="m-screen m-image-screen">
    <header class="m-topbar">
      <button class="m-icon-button" type="button" :aria-label="t('mobile.common.back')" @click="emit('back')">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 5l-7 7 7 7" /></svg>
      </button>
      <div class="m-topbar-text">
        <span class="m-topbar-title">{{ title || t("mobile.image.title") }}</span>
        <span class="m-topbar-sub">{{ t("mobile.image.hint") }}</span>
      </div>
      <button
        class="m-icon-button"
        type="button"
        :aria-label="t('mobile.image.reset')"
        :disabled="scale === 1"
        @click="reset"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" /></svg>
      </button>
    </header>
    <div
      ref="stageRef"
      class="m-image-stage"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerCancel"
    >
      <div v-if="failed" class="m-placeholder">{{ t("mobile.image.failed") }}</div>
      <img
        v-else
        class="m-image-stage-img"
        :src="src"
        :alt="title"
        :style="{ transform }"
        draggable="false"
        @error="failed = true"
      >
    </div>
  </section>
</template>
