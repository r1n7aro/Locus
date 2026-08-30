<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import type {
  WorkbenchEditorGroup,
  WorkbenchSplitNode,
} from "../../types/workbench";
import type { WorkbenchSplitDropDirection } from "./workbenchDropGeometry";

defineOptions({ name: "WorkbenchSplitHost" });

const props = defineProps<{
  node: WorkbenchSplitNode;
  groups: Record<string, WorkbenchEditorGroup>;
  focusedPaneId: string;
  activeDropKey?: string | null;
  showSingleTabs?: boolean;
}>();

const emit = defineEmits<{
  (event: "focus-pane", paneId: string): void;
  (event: "resize", splitId: string, ratio: number, commit: boolean): void;
}>();

defineSlots<{
  group(props: {
    group: WorkbenchEditorGroup | undefined;
    paneId: string;
    focused: boolean;
  }): unknown;
}>();

const splitElement = ref<HTMLElement | null>(null);
let resizeCleanup: (() => void) | null = null;

const firstStyle = computed(() => {
  if (props.node.kind !== "split") return undefined;
  const percent = `${(props.node.ratio * 100).toFixed(4)}%`;
  return props.node.orientation === "horizontal"
    ? { flexBasis: `calc(${percent} - 2px)` }
    : { flexBasis: `calc(${percent} - 2px)` };
});

const leafShowsTabs = computed(() => {
  if (props.node.kind !== "group") return false;
  const count = props.groups[props.node.paneId]?.tabs.length ?? 0;
  return count >= 2 || (props.showSingleTabs && count === 1);
});

const activeSplitDropDirection = computed<WorkbenchSplitDropDirection | null>(() => {
  if (props.node.kind !== "group" || !props.activeDropKey) return null;
  const prefix = `editor:${props.node.paneId}:`;
  if (!props.activeDropKey.startsWith(prefix)) return null;
  const direction = props.activeDropKey.slice(prefix.length).split(":", 1)[0];
  return direction === "left" || direction === "right" || direction === "top" || direction === "bottom"
    ? direction
    : null;
});

function beginResize(event: PointerEvent): void {
  if (props.node.kind !== "split" || event.button !== 0 || !splitElement.value) return;
  event.preventDefault();
  const splitId = props.node.splitId;
  const orientation = props.node.orientation;
  const element = splitElement.value;
  const pointerId = event.pointerId;
  const separator = event.currentTarget as HTMLElement;
  separator.setPointerCapture?.(pointerId);
  document.body.classList.add("is-dragging-select-lock");
  document.body.style.cursor = orientation === "horizontal" ? "col-resize" : "row-resize";

  const ratioAt = (clientX: number, clientY: number) => {
    const bounds = element.getBoundingClientRect();
    return orientation === "horizontal"
      ? (clientX - bounds.left) / Math.max(1, bounds.width)
      : (clientY - bounds.top) / Math.max(1, bounds.height);
  };
  const move = (moveEvent: PointerEvent) => {
    if (moveEvent.pointerId !== pointerId) return;
    emit("resize", splitId, ratioAt(moveEvent.clientX, moveEvent.clientY), false);
  };
  const finish = (finishEvent: PointerEvent) => {
    if (finishEvent.pointerId !== pointerId) return;
    emit("resize", splitId, ratioAt(finishEvent.clientX, finishEvent.clientY), true);
    resizeCleanup?.();
  };
  resizeCleanup = () => {
    window.removeEventListener("pointermove", move, true);
    window.removeEventListener("pointerup", finish, true);
    window.removeEventListener("pointercancel", finish, true);
    document.body.classList.remove("is-dragging-select-lock");
    document.body.style.cursor = "";
    resizeCleanup = null;
  };
  window.addEventListener("pointermove", move, { capture: true, passive: true });
  window.addEventListener("pointerup", finish, true);
  window.addEventListener("pointercancel", finish, true);
}

function onSeparatorKeydown(event: KeyboardEvent): void {
  if (props.node.kind !== "split") return;
  const horizontal = props.node.orientation === "horizontal";
  let ratio = props.node.ratio;
  if (event.key === "Home") ratio = 0.18;
  else if (event.key === "End") ratio = 0.82;
  else if ((horizontal && event.key === "ArrowLeft") || (!horizontal && event.key === "ArrowUp")) {
    ratio -= 0.02;
  } else if ((horizontal && event.key === "ArrowRight") || (!horizontal && event.key === "ArrowDown")) {
    ratio += 0.02;
  } else {
    return;
  }
  event.preventDefault();
  emit("resize", props.node.splitId, ratio, true);
}

function forwardResize(splitId: string, ratio: number, commit: boolean): void {
  emit("resize", splitId, ratio, commit);
}

function requestPaneFocus(paneId: string): void {
  if (props.focusedPaneId !== paneId) emit("focus-pane", paneId);
}

onUnmounted(() => resizeCleanup?.());
</script>

<template>
  <div
    v-if="node.kind === 'split'"
    ref="splitElement"
    class="workbench-split"
    :class="`is-${node.orientation}`"
  >
    <div class="workbench-split-child is-first" :style="firstStyle">
      <WorkbenchSplitHost
        :node="node.first"
        :groups="groups"
        :focused-pane-id="focusedPaneId"
        :active-drop-key="activeDropKey"
        :show-single-tabs="showSingleTabs"
        @focus-pane="emit('focus-pane', $event)"
        @resize="forwardResize"
      >
        <template #group="{ group, paneId, focused }">
          <slot name="group" :group="group" :pane-id="paneId" :focused="focused" />
        </template>
      </WorkbenchSplitHost>
    </div>
    <div
      class="workbench-split-separator"
      role="separator"
      :aria-orientation="node.orientation === 'horizontal' ? 'vertical' : 'horizontal'"
      :aria-valuemin="18"
      :aria-valuemax="82"
      :aria-valuenow="Math.round(node.ratio * 100)"
      tabindex="0"
      @pointerdown="beginResize"
      @keydown="onSeparatorKeydown"
    />
    <div class="workbench-split-child is-second">
      <WorkbenchSplitHost
        :node="node.second"
        :groups="groups"
        :focused-pane-id="focusedPaneId"
        :active-drop-key="activeDropKey"
        :show-single-tabs="showSingleTabs"
        @focus-pane="emit('focus-pane', $event)"
        @resize="forwardResize"
      >
        <template #group="{ group, paneId, focused }">
          <slot name="group" :group="group" :pane-id="paneId" :focused="focused" />
        </template>
      </WorkbenchSplitHost>
    </div>
  </div>
  <section
    v-else
    class="workbench-editor-group"
    :class="{ focused: focusedPaneId === node.paneId }"
    :data-workbench-pane-id="node.paneId"
    @pointerdown.capture="requestPaneFocus(node.paneId)"
  >
    <slot
      name="group"
      :group="groups[node.paneId]"
      :pane-id="node.paneId"
      :focused="focusedPaneId === node.paneId"
    />
    <div
      v-if="activeSplitDropDirection"
      class="workbench-editor-split-preview-layer"
      :class="{ 'has-tabs': leafShowsTabs }"
      aria-hidden="true"
    >
      <div
        class="workbench-editor-split-preview"
        :class="`is-${activeSplitDropDirection}`"
      />
    </div>
  </section>
</template>

<style scoped>
.workbench-split,
.workbench-split-child,
.workbench-editor-group {
  min-width: 0;
  min-height: 0;
}

.workbench-split {
  display: flex;
  width: 100%;
  height: 100%;
  overflow: hidden;
}

.workbench-split.is-horizontal {
  flex-direction: row;
}

.workbench-split.is-vertical {
  flex-direction: column;
}

.workbench-split-child {
  position: relative;
  display: flex;
  flex: 1 1 0;
  overflow: hidden;
}

.workbench-split.is-horizontal > .workbench-split-child {
  min-width: 180px;
}

.workbench-split.is-vertical > .workbench-split-child {
  min-height: 140px;
}

.workbench-split-child.is-first {
  flex-grow: 0;
  flex-shrink: 0;
}

.workbench-split-separator {
  position: relative;
  z-index: 8;
  flex: 0 0 4px;
  background: var(--border-color);
  outline: none;
}

.workbench-split.is-horizontal > .workbench-split-separator {
  cursor: col-resize;
}

.workbench-split.is-vertical > .workbench-split-separator {
  cursor: row-resize;
}

.workbench-split-separator::after {
  content: "";
  position: absolute;
  inset: -3px;
}

.workbench-split-separator:hover,
.workbench-split-separator:focus-visible {
  background: color-mix(in srgb, var(--accent-color) 58%, var(--border-color));
}

.workbench-editor-group {
  position: relative;
  display: flex;
  flex: 1 1 0;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.workbench-editor-group.focused {
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.workbench-editor-split-preview-layer {
  position: absolute;
  z-index: 40;
  inset: 0;
  pointer-events: none;
}

.workbench-editor-split-preview-layer.has-tabs {
  top: 31px;
}

.workbench-editor-split-preview {
  position: absolute;
  border: 1px solid color-mix(in srgb, var(--accent-color) 58%, var(--border-strong));
  border-radius: 4px;
  background: color-mix(in srgb, var(--accent-color) 13%, var(--panel-bg));
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 12%, transparent);
  transition: inset 70ms ease, opacity 70ms ease;
}

.workbench-editor-split-preview.is-left { inset: 6px 50% 6px 6px; }
.workbench-editor-split-preview.is-right { inset: 6px 6px 6px 50%; }
.workbench-editor-split-preview.is-top { inset: 6px 6px 50% 6px; }
.workbench-editor-split-preview.is-bottom { inset: 50% 6px 6px 6px; }
</style>
