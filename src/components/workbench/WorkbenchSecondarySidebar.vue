<script setup lang="ts">
import { onUnmounted, ref, watch } from "vue";
import { X } from "lucide";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import { cancelWorkbenchSidebarMotion, switchWorkbenchSidebarContent } from "./workbenchSidebarMotion";

const props = defineProps<{ title: string; contentKey?: string; ownerWindow?: Window }>();
const emit = defineEmits<{ close: [] }>();
const ownerWindow = props.ownerWindow ?? window;
const ownerDocument = ownerWindow.document;
const widthKey = "locus:workspaceSecondarySidebarWidth";
const clampWidth = (value: number) => Math.max(240, Math.min(520, value));
const width = ref((() => {
  const saved = Number(ownerWindow.localStorage.getItem(widthKey));
  return saved > 0 && Number.isFinite(saved) ? clampWidth(saved) : 300;
})());
const resizing = ref(false);
const sidebarRef = ref<HTMLElement | null>(null);
const bodyRef = ref<HTMLElement | null>(null);
const toolbarTarget = ref<HTMLElement | null>(null);
let startX = 0;
let startWidth = 0;
let pendingWidth = width.value;
let resizeFrame = 0;

watch(() => props.contentKey ?? props.title, () => {
  if (!resizing.value) switchWorkbenchSidebarContent(bodyRef.value);
}, { flush: "post" });

function stopMotion() {
  cancelWorkbenchSidebarMotion(sidebarRef.value);
  cancelWorkbenchSidebarMotion(bodyRef.value);
}

function paintResize() {
  resizeFrame = 0;
  if (sidebarRef.value) sidebarRef.value.style.width = `${pendingWidth}px`;
}

function startResize(event: MouseEvent) {
  if (event.button !== 0) return;
  event.preventDefault();
  stopMotion();
  startX = event.clientX;
  startWidth = width.value;
  pendingWidth = width.value;
  resizing.value = true;
  ownerDocument.addEventListener("mousemove", resize);
  ownerDocument.addEventListener("mouseup", stopResize);
  ownerWindow.addEventListener("blur", stopResize);
  ownerDocument.body.style.cursor = "col-resize";
  ownerDocument.body.classList.add("is-dragging-select-lock");
}

function resize(event: MouseEvent) {
  pendingWidth = clampWidth(startWidth + event.clientX - startX);
  // Pointer events can outpace paint. Resize the shell once per frame without
  // scheduling Vue updates for the header and its scoped content slot.
  if (!resizeFrame) resizeFrame = ownerWindow.requestAnimationFrame(paintResize);
}

function stopResize() {
  if (!resizing.value) return;
  if (resizeFrame) ownerWindow.cancelAnimationFrame(resizeFrame);
  paintResize();
  width.value = pendingWidth;
  resizing.value = false;
  ownerDocument.removeEventListener("mousemove", resize);
  ownerDocument.removeEventListener("mouseup", stopResize);
  ownerWindow.removeEventListener("blur", stopResize);
  ownerDocument.body.style.cursor = "";
  ownerDocument.body.classList.remove("is-dragging-select-lock");
  ownerWindow.localStorage.setItem(widthKey, String(Math.round(width.value)));
}

function resizeByKeyboard(event: KeyboardEvent) {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  stopMotion();
  width.value = clampWidth(width.value + (event.key === "ArrowRight" ? 20 : -20));
  ownerWindow.localStorage.setItem(widthKey, String(width.value));
}

onUnmounted(() => {
  stopMotion();
  stopResize();
});
</script>

<template>
  <aside ref="sidebarRef" class="workbench-secondary-sidebar" :style="{ width: `${width}px` }" :aria-label="title">
    <div class="secondary-sidebar-surface">
    <div class="secondary-sidebar-header">
      <span class="secondary-sidebar-title">{{ title }}</span>
      <div ref="toolbarTarget" class="secondary-sidebar-actions"></div>
      <BaseButton class="secondary-sidebar-close" :title="t('common.close')" :aria-label="t('common.close')" @click="emit('close')">
        <LucideIcon :icon="X" :size="14" />
      </BaseButton>
    </div>
    <div ref="bodyRef" class="secondary-sidebar-body"><slot :toolbar-target="toolbarTarget" /></div>
    </div>
    <div
      class="secondary-sidebar-resize"
      :class="{ active: resizing }"
      role="separator"
      aria-orientation="vertical"
      :aria-label="title"
      :aria-valuenow="width"
      :aria-valuemin="240"
      :aria-valuemax="520"
      tabindex="0"
      @mousedown="startResize"
      @keydown="resizeByKeyboard"
    />
  </aside>
</template>

<style scoped>
.workbench-secondary-sidebar {
  --explorer-search-height: 28px;
  --explorer-search-padding: 6px 10px;
  --explorer-search-font-size: 12px;
  --explorer-search-radius: 6px;
  position: relative;
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  min-width: 240px;
  min-height: 0;
  border-right: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  contain: layout style;
}
.secondary-sidebar-surface {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}
.secondary-sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: 0 0 34px;
  gap: 8px;
  padding: 0 7px 0 12px;
  border-bottom: 1px solid var(--border-color);
}
.secondary-sidebar-title {
  flex: 1;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.secondary-sidebar-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}
.secondary-sidebar-actions :deep(.secondary-sidebar-count) {
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.secondary-sidebar-actions :deep(.secondary-sidebar-tool),
.secondary-sidebar-close {
  width: 26px;
  min-width: 26px;
  height: 26px;
  padding: 0;
  border-color: transparent;
}
.secondary-sidebar-body {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}
.secondary-sidebar-resize {
  position: absolute;
  z-index: 12;
  top: 0;
  bottom: 0;
  right: -3px;
  width: 6px;
  cursor: col-resize;
}
.secondary-sidebar-resize::after {
  content: "";
  position: absolute;
  inset: 0 2px;
  background: transparent;
}
.secondary-sidebar-resize:hover::after,
.secondary-sidebar-resize.active::after,
.secondary-sidebar-resize:focus-visible::after {
  background: color-mix(in srgb, var(--accent-color) 42%, transparent);
}
</style>
