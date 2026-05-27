<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, useAttrs, watch } from "vue";
import type { StyleValue } from "vue";
import { clampFloatingPosition } from "./floatingPosition";

defineOptions({ inheritAttrs: false });

const props = withDefaults(defineProps<{
  x: number;
  y: number;
  minWidth?: number | string;
  maxWidth?: number | string;
  maxHeight?: number | string;
  viewportMargin?: number;
  zIndex?: number;
  role?: string;
  ariaLabel?: string;
}>(), {
  minWidth: 156,
  maxWidth: "calc(100vw - 16px)",
  maxHeight: "calc(100vh - 16px)",
  viewportMargin: 8,
  zIndex: 9999,
  role: "menu",
  ariaLabel: "",
});

const emit = defineEmits<{
  close: [];
}>();

const attrs = useAttrs();
const menuRef = ref<HTMLElement | null>(null);
const position = ref({ x: props.x, y: props.y });

const menuClass = computed(() => ["base-context-menu", attrs.class]);
const menuAttrs = computed(() => {
  const { class: _class, style: _style, ...rest } = attrs;
  return rest;
});
const menuStyle = computed<StyleValue>(() => {
  const baseStyle = {
    left: `${position.value.x}px`,
    top: `${position.value.y}px`,
    minWidth: toCssSize(props.minWidth),
    maxWidth: toCssSize(props.maxWidth),
    maxHeight: toCssSize(props.maxHeight),
    zIndex: props.zIndex + 1,
  };
  const attrStyle = attrs.style as StyleValue | undefined;
  return attrStyle ? [baseStyle, attrStyle] : baseStyle;
});
const backdropStyle = computed(() => ({
  zIndex: props.zIndex,
}));

function toCssSize(value: number | string): string {
  return typeof value === "number" ? `${value}px` : value;
}

function viewportSize() {
  return {
    width: window.innerWidth || document.documentElement.clientWidth,
    height: window.innerHeight || document.documentElement.clientHeight,
  };
}

async function updatePosition() {
  position.value = { x: props.x, y: props.y };
  await nextTick();
  const menu = menuRef.value;
  if (!menu) return;
  const rect = menu.getBoundingClientRect();
  position.value = clampFloatingPosition(
    { x: props.x, y: props.y },
    { width: rect.width, height: rect.height },
    viewportSize(),
    props.viewportMargin,
  );
}

function close() {
  emit("close");
}

function onKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  event.preventDefault();
  close();
}

watch(
  () => [props.x, props.y, props.minWidth, props.maxWidth, props.maxHeight, props.viewportMargin] as const,
  () => {
    void updatePosition();
  },
  { immediate: true },
);

onMounted(() => {
  document.addEventListener("keydown", onKeydown, true);
  void updatePosition();
});

onUnmounted(() => {
  document.removeEventListener("keydown", onKeydown, true);
});
</script>

<template>
  <Teleport to="body">
    <Transition name="base-context-menu-fade" appear>
      <div
        class="base-context-menu-backdrop"
        :style="backdropStyle"
        @click="close"
        @contextmenu.prevent="close"
      >
        <div
          ref="menuRef"
          v-bind="menuAttrs"
          :class="menuClass"
          :style="menuStyle"
          :role="role"
          :aria-label="ariaLabel || undefined"
          tabindex="-1"
          @click.stop
          @contextmenu.prevent.stop
        >
          <slot />
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.base-context-menu-backdrop {
  position: fixed;
  inset: 0;
}

.base-context-menu {
  position: fixed;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow: auto;
  padding: 5px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--elevated-bg, var(--panel-bg));
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.22);
  color: var(--text-color);
  outline: none;
}

:global(:root[data-theme="dark"]) .base-context-menu {
  box-shadow: 0 14px 32px rgba(0, 0, 0, 0.36);
}

.base-context-menu :deep(button) {
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 28px;
  min-width: 0;
  padding: 0 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  line-height: 1.2;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}

.base-context-menu :deep(button:hover:not(:disabled)),
.base-context-menu :deep(button:focus-visible:not(:disabled)) {
  background: var(--hover-bg);
}

.base-context-menu :deep(button:focus-visible) {
  outline: none;
}

.base-context-menu :deep(button:disabled),
.base-context-menu :deep(button.disabled) {
  color: var(--text-secondary);
  cursor: default;
  opacity: 0.55;
}

.base-context-menu :deep(button:disabled:hover),
.base-context-menu :deep(button.disabled:hover) {
  background: transparent;
}

.base-context-menu :deep(button.danger),
.base-context-menu :deep(button.ctx-danger),
.base-context-menu :deep(button.agent-rule-ctx-item-danger),
.base-context-menu :deep(button.kx-ctx-item-danger) {
  color: var(--status-danger-fg);
}

.base-context-menu :deep(button.danger:hover:not(:disabled)),
.base-context-menu :deep(button.ctx-danger:hover:not(:disabled)),
.base-context-menu :deep(button.agent-rule-ctx-item-danger:hover:not(:disabled)),
.base-context-menu :deep(button.kx-ctx-item-danger:hover:not(:disabled)),
.base-context-menu :deep(button.danger:focus-visible:not(:disabled)),
.base-context-menu :deep(button.ctx-danger:focus-visible:not(:disabled)),
.base-context-menu :deep(button.agent-rule-ctx-item-danger:focus-visible:not(:disabled)),
.base-context-menu :deep(button.kx-ctx-item-danger:focus-visible:not(:disabled)) {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.base-context-menu :deep(.ctx-hint) {
  padding: 6px 10px;
  border-radius: 6px;
  color: var(--status-warn-fg);
  font-size: 12px;
  line-height: 1.4;
  cursor: default;
}

.base-context-menu :deep(.base-context-menu-separator),
.base-context-menu :deep(.recent-dir-ctx-sep),
.base-context-menu :deep(.agent-rule-ctx-sep),
.base-context-menu :deep(.view-ctx-sep),
.base-context-menu :deep(.kx-ctx-sep),
.base-context-menu :deep(.asset-ref-ctx-sep),
.base-context-menu :deep(.ctx-sep),
.base-context-menu :deep(.sp-ctx-sep) {
  height: 1px;
  margin: 4px 0;
  background: var(--border-color);
}

.base-context-menu-fade-enter-active,
.base-context-menu-fade-leave-active {
  transition: opacity 0.12s ease;
}

.base-context-menu-fade-enter-active .base-context-menu,
.base-context-menu-fade-leave-active .base-context-menu {
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.base-context-menu-fade-enter-from,
.base-context-menu-fade-leave-to {
  opacity: 0;
}

.base-context-menu-fade-enter-from .base-context-menu,
.base-context-menu-fade-leave-to .base-context-menu {
  opacity: 0;
  transform: translateY(-4px) scale(0.985);
}
</style>
