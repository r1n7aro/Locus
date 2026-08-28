<script setup lang="ts">
import { computed, onBeforeUnmount, type ComponentPublicInstance } from "vue";
import { File, Folder, GripVertical, Package } from "lucide";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  internalDragFloatingTransform,
  useInternalDragController,
} from "../../composables/useInternalDrag";

const drag = useInternalDragController();
let overlayElement: HTMLElement | null = null;

const preview = computed(() => drag.source.value?.preview ?? null);
const icon = computed(() => {
  if (preview.value?.icon) return preview.value.icon;
  switch (preview.value?.kind) {
    case "folder":
      return Folder;
    case "package":
      return Package;
    case "file":
      return File;
    default:
      return GripVertical;
  }
});
const label = computed(() => {
  const value = preview.value;
  if (!value) return "";
  const count = Math.max(1, value.count ?? 1);
  return count > 1 ? `${value.label} +${count - 1}` : value.label;
});
function applyVisualPoint(point: { x: number; y: number }) {
  if (!overlayElement) return;
  overlayElement.style.transform = internalDragFloatingTransform(
    point,
    drag.previewAnchor.value,
  );
}

function bindOverlayElement(value: Element | ComponentPublicInstance | null) {
  overlayElement = value instanceof HTMLElement ? value : null;
  if (overlayElement) applyVisualPoint(drag.point.value);
}

const unsubscribeVisualPoint = drag.subscribeVisualPoint(applyVisualPoint);
onBeforeUnmount(unsubscribeVisualPoint);
</script>

<template>
  <Teleport to="body">
    <div
      v-if="drag.dragging.value && drag.previewMode.value !== 'inline' && preview"
      :ref="bindOverlayElement"
      class="internal-drag-overlay"
      :class="`operation-${drag.activeTarget.value?.decision.operation ?? 'none'}`"
      data-internal-drag-preview
      aria-hidden="true"
    >
      <LucideIcon
        class="internal-drag-overlay-icon"
        :class="preview.iconClass"
        :icon="icon"
        :size="14"
        :stroke-width="2"
      />
      <span class="internal-drag-overlay-label">{{ label }}</span>
    </div>
  </Teleport>
</template>

<style scoped>
.internal-drag-overlay {
  position: fixed;
  inset: 0 auto auto 0;
  z-index: 320;
  display: flex;
  align-items: center;
  gap: 7px;
  width: 228px;
  min-height: 34px;
  padding: 6px 9px;
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 96%, var(--accent-soft) 4%);
  box-shadow: 0 8px 22px color-mix(in srgb, var(--text-color) 16%, transparent);
  color: var(--text-color);
  pointer-events: none;
  will-change: transform;
}

.internal-drag-overlay.operation-none {
  opacity: 0.72;
}

.internal-drag-overlay-icon {
  flex: 0 0 auto;
  color: var(--text-secondary);
}

.internal-drag-overlay-label {
  min-width: 0;
  overflow: hidden;
  font-family: var(--font-ui);
  font-size: 12px;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
