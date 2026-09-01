<script setup lang="ts">
import type { IconNode } from "lucide";
import { X } from "lucide";
import { t } from "../../i18n";
import {
  useInternalDragController,
  type InternalDragFinishResult,
  type InternalDragOperation,
} from "../../composables/useInternalDrag";
import LucideIcon from "../icons/LucideIcon.vue";

export interface BaseTabStripItem {
  id: string;
  title: string;
  icon?: IconNode | null;
  dirty?: boolean;
  preview?: boolean;
  unavailable?: boolean;
  running?: boolean;
  closeable?: boolean;
}

export interface BaseTabDragFinishedPayload {
  tab: BaseTabStripItem;
  result: InternalDragFinishResult;
}

export interface BaseTabNativeDragPayload {
  tab: BaseTabStripItem;
  event: DragEvent;
  shell: HTMLElement;
  anchor: { x: number; y: number };
}

export interface BaseTabContextMenuPayload {
  tab: BaseTabStripItem;
  event: MouseEvent;
}

const props = withDefaults(defineProps<{
  tabs: readonly BaseTabStripItem[];
  activeId: string | null;
  label: string;
  dragType?: string | null;
  dragData?: ((tab: BaseTabStripItem) => unknown) | null;
  dragSourceId?: ((tab: BaseTabStripItem) => string) | null;
  dragExternalize?: ((tab: BaseTabStripItem) => void | Promise<void>) | null;
  canDrag?: ((tab: BaseTabStripItem) => boolean) | null;
  allowedOperations?: readonly InternalDragOperation[];
  activateOnPointerDown?: boolean;
  cancelDragOnWindowBlur?: boolean;
  pinOnDoubleClick?: boolean;
  closeOnMiddleClick?: boolean;
  dropActive?: boolean;
  dropIndex?: number;
  tabIdAttribute?: string;
  nativeDrag?: boolean;
}>(), {
  dragType: null,
  dragData: null,
  dragSourceId: null,
  dragExternalize: null,
  canDrag: null,
  allowedOperations: () => ["move"],
  activateOnPointerDown: false,
  cancelDragOnWindowBlur: true,
  pinOnDoubleClick: false,
  closeOnMiddleClick: true,
  dropActive: false,
  dropIndex: -1,
  tabIdAttribute: "data-locus-tab-id",
  nativeDrag: false,
});

const emit = defineEmits<{
  (event: "activate", tabId: string): void;
  (event: "close", tabId: string): void;
  (event: "pin", tabId: string): void;
  (event: "drag-activated", tab: BaseTabStripItem): void;
  (event: "drag-finished", payload: BaseTabDragFinishedPayload): void;
  (event: "native-drag-start", payload: BaseTabNativeDragPayload): void;
  (event: "native-drag-end", payload: BaseTabNativeDragPayload): void;
  (event: "tab-contextmenu", payload: BaseTabContextMenuPayload): void;
}>();

const internalDrag = useInternalDragController();

function activeDropIndex(): number {
  if (!props.dropActive) return -1;
  return Math.min(props.tabs.length, Math.max(0, props.dropIndex));
}

function tabDomAttributes(tab: BaseTabStripItem): Record<string, string> {
  if (!props.tabIdAttribute || props.tabIdAttribute === "data-locus-tab-id") return {};
  return { [props.tabIdAttribute]: tab.id };
}

function beginTabDrag(event: PointerEvent, tab: BaseTabStripItem): void {
  if (props.nativeDrag || !props.dragType || event.detail > 1 || props.canDrag?.(tab) === false) return;
  if (props.activateOnPointerDown && props.activeId !== tab.id) emit("activate", tab.id);
  internalDrag.start(event, {
    id: props.dragSourceId?.(tab) ?? `${props.dragType}:${tab.id}`,
    payload: {
      type: props.dragType,
      data: props.dragData?.(tab) ?? { tabId: tab.id, title: tab.title },
    },
    preview: {
      label: tab.title,
      kind: "file",
      icon: tab.icon ?? undefined,
    },
    allowedOperations: props.allowedOperations,
    cancelOnWindowBlur: props.cancelDragOnWindowBlur,
    externalize: props.dragExternalize ? () => props.dragExternalize?.(tab) : undefined,
    onActivated: () => emit("drag-activated", tab),
    onFinished: (result) => emit("drag-finished", { tab, result }),
  });
}

function nativeDragPayload(event: DragEvent, tab: BaseTabStripItem): BaseTabNativeDragPayload | null {
  const currentTarget = event.currentTarget as Node | null;
  const shell = currentTarget?.nodeType === 1 ? currentTarget as HTMLElement : null;
  if (!shell) return null;
  const bounds = shell.getBoundingClientRect();
  return {
    tab,
    event,
    shell,
    anchor: {
      x: Math.max(0, Math.min(bounds.width, event.clientX - bounds.left)),
      y: Math.max(0, Math.min(bounds.height, event.clientY - bounds.top)),
    },
  };
}

function beginNativeTabDrag(event: DragEvent, tab: BaseTabStripItem): void {
  if (!props.nativeDrag || props.canDrag?.(tab) === false || !event.dataTransfer) {
    event.preventDefault();
    return;
  }
  const payload = nativeDragPayload(event, tab);
  if (!payload) {
    event.preventDefault();
    return;
  }
  if (props.activateOnPointerDown && props.activeId !== tab.id) emit("activate", tab.id);
  event.dataTransfer.effectAllowed = "copyMove";
  event.dataTransfer.setData("text/plain", tab.title);
  event.dataTransfer.setDragImage(payload.shell, payload.anchor.x, payload.anchor.y);
  emit("native-drag-start", payload);
}

function finishNativeTabDrag(event: DragEvent, tab: BaseTabStripItem): void {
  const payload = nativeDragPayload(event, tab);
  if (payload) emit("native-drag-end", payload);
}

function handleDoubleClick(event: MouseEvent, tab: BaseTabStripItem): void {
  if (!props.pinOnDoubleClick) return;
  event.stopPropagation();
  emit("pin", tab.id);
}

function handleAuxClick(event: MouseEvent, tab: BaseTabStripItem): void {
  if (!props.closeOnMiddleClick || event.button !== 1 || tab.closeable === false) return;
  event.preventDefault();
  emit("close", tab.id);
}

function openTabContextMenu(event: MouseEvent, tab: BaseTabStripItem): void {
  event.preventDefault();
  event.stopPropagation();
  emit("tab-contextmenu", { tab, event });
}

function onTabKeydown(event: KeyboardEvent, index: number): void {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  const delta = event.key === "ArrowRight" ? 1 : -1;
  const nextIndex = (index + delta + props.tabs.length) % props.tabs.length;
  const next = props.tabs[nextIndex];
  if (!next) return;
  emit("activate", next.id);
  requestAnimationFrame(() => {
    const currentTarget = event.currentTarget as Node | null;
    const ownerDocument = currentTarget?.nodeType === 1
      ? (currentTarget as HTMLElement).ownerDocument
      : document;
    ownerDocument.querySelector<HTMLElement>(
      `[data-locus-tab-id="${CSS.escape(next.id)}"]`,
    )?.focus();
  });
}
</script>

<template>
  <div
    class="base-tab-strip"
    :class="{ 'drop-active': dropActive }"
    data-locus-tab-strip
    role="tablist"
    :aria-label="label"
  >
    <div
      v-for="(tab, index) in tabs"
      :key="tab.id"
      class="base-tab-shell"
      :class="{
        active: activeId === tab.id,
        dirty: tab.dirty,
        preview: tab.preview,
        unavailable: tab.unavailable,
        running: tab.running,
        'drop-before': activeDropIndex() === index,
      }"
      data-locus-tab-shell
      :draggable="nativeDrag && canDrag?.(tab) !== false"
      @dragstart="beginNativeTabDrag($event, tab)"
      @dragend="finishNativeTabDrag($event, tab)"
      @contextmenu="openTabContextMenu($event, tab)"
    >
      <button
        v-bind="tabDomAttributes(tab)"
        type="button"
        class="base-tab"
        role="tab"
        :data-locus-tab-id="tab.id"
        :aria-selected="activeId === tab.id"
        :tabindex="activeId === tab.id ? 0 : -1"
        :title="tab.title"
        @click="emit('activate', tab.id)"
        @dblclick="handleDoubleClick($event, tab)"
        @auxclick="handleAuxClick($event, tab)"
        @keydown="onTabKeydown($event, index)"
        @pointerdown="beginTabDrag($event, tab)"
      >
        <span v-if="tab.icon" class="base-tab-icon" aria-hidden="true">
          <LucideIcon :icon="tab.icon" :size="12" :stroke-width="2" />
        </span>
        <span class="base-tab-title">{{ tab.title }}</span>
        <span v-if="tab.dirty" class="base-tab-dirty" aria-hidden="true" />
      </button>
      <button
        v-if="tab.closeable !== false"
        type="button"
        class="base-tab-close"
        :title="t('common.close')"
        :aria-label="t('common.close')"
        @pointerdown.stop
        draggable="false"
        @dragstart.stop.prevent
        @click.stop="emit('close', tab.id)"
      >
        <LucideIcon :icon="X" :size="12" :stroke-width="2" />
      </button>
    </div>
    <div
      v-if="dropActive && activeDropIndex() === tabs.length"
      class="base-tab-drop-end"
      aria-hidden="true"
    />
  </div>
</template>

<style scoped>
.base-tab-strip {
  position: relative;
  display: flex;
  flex: 0 0 31px;
  min-width: 0;
  height: 31px;
  overflow-x: auto;
  overflow-y: hidden;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
  scrollbar-width: none;
}

.base-tab-strip.drop-active {
  box-shadow: inset 0 -1px 0 color-mix(in srgb, var(--accent-color) 72%, var(--border-color));
}

.base-tab-strip::-webkit-scrollbar {
  display: none;
}

.base-tab-shell {
  position: relative;
  display: flex;
  flex: 0 1 190px;
  min-width: 88px;
  max-width: 240px;
  border-right: 1px solid var(--border-color);
  color: var(--text-secondary);
}

.base-tab-shell.drop-before::before,
.base-tab-drop-end {
  content: "";
  z-index: 3;
  flex: 0 0 2px;
  align-self: center;
  width: 2px;
  height: 23px;
  border-radius: 1px;
  background: var(--accent-color);
  pointer-events: none;
}

.base-tab-shell.drop-before::before {
  position: absolute;
  top: 4px;
  bottom: 4px;
  left: -1px;
  height: auto;
}

.base-tab-shell.active {
  color: var(--text-color);
  background: var(--panel-bg);
}

.base-tab-shell.active::after {
  content: "";
  position: absolute;
  right: 0;
  bottom: -1px;
  left: 0;
  height: 1px;
  background: var(--panel-bg);
}

.base-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  width: 100%;
  padding: 0 27px 0 9px;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}

.base-tab:hover,
.base-tab:focus-visible {
  background: var(--hover-bg);
  outline: none;
}

.base-tab-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.base-tab-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 12px;
  height: 12px;
  flex: 0 0 auto;
  color: inherit;
}

.base-tab-shell.running .base-tab-icon {
  color: var(--accent-color);
  animation: base-tab-icon-breathe 1.8s ease-in-out infinite;
}

@keyframes base-tab-icon-breathe {
  0%,
  100% {
    opacity: 0.58;
    filter: drop-shadow(0 0 0 transparent);
  }
  50% {
    opacity: 1;
    filter: drop-shadow(0 0 2px color-mix(in srgb, var(--accent-color) 72%, transparent));
  }
}

@media (prefers-reduced-motion: reduce) {
  .base-tab-shell.running .base-tab-icon {
    animation: none;
    opacity: 1;
    filter: none;
  }
}

.base-tab-dirty {
  flex: 0 0 auto;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-secondary);
}

.base-tab-close {
  position: absolute;
  z-index: 1;
  top: 6px;
  right: 6px;
  display: grid;
  place-items: center;
  width: 19px;
  height: 19px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  opacity: 0;
}

.base-tab-shell:hover .base-tab-close,
.base-tab-shell.active .base-tab-close,
.base-tab-close:focus-visible {
  opacity: 1;
}

.base-tab-close:hover,
.base-tab-close:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.base-tab-shell.unavailable .base-tab-title {
  color: var(--text-secondary);
  font-style: italic;
}

.base-tab-shell.preview .base-tab-title {
  font-style: italic;
}
</style>
