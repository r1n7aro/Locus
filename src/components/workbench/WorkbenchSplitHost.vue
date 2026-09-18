<script setup lang="ts">
import { computed, nextTick, shallowReactive, watch } from "vue";
import type { WorkbenchEditorGroup, WorkbenchSplitNode } from "../../types/workbench";
import WorkbenchSplitLayout from "./WorkbenchSplitLayout.vue";

defineOptions({ inheritAttrs: false });

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

const paneIds = computed(() => {
  const ids: string[] = [];
  function visit(node: WorkbenchSplitNode): void {
    if (node.kind === "group") ids.push(node.paneId);
    else {
      visit(node.first);
      visit(node.second);
    }
  }
  visit(props.node);
  return ids;
});
const paneTargets = shallowReactive(new Map<string, HTMLElement>());
const paneScrollPositions = new Map<string, Map<HTMLElement, { left: number; top: number }>>();

function recordPaneScroll(paneId: string, event: Event): void {
  const target = event.target as HTMLElement | null;
  if (target?.nodeType !== 1 || !target.isConnected) return;
  let positions = paneScrollPositions.get(paneId);
  if (!positions) {
    positions = new Map();
    paneScrollPositions.set(paneId, positions);
  }
  // Track only elements that actually scroll; never walk the editor DOM at drop.
  for (const element of positions.keys()) {
    if (!paneTargets.get(paneId)?.contains(element)) positions.delete(element);
  }
  const position = { left: target.scrollLeft, top: target.scrollTop };
  if (position.left || position.top) positions.set(target, position);
  else positions.delete(target);
}

function registerPaneTarget(paneId: string, element: HTMLElement): void {
  // Keep the old target while the layout replaces its leaf. The keyed Teleport
  // then moves the existing editor subtree to the new target without remounting.
  if (paneTargets.get(paneId) === element) return;
  paneTargets.set(paneId, element);
  // Chromium resets nested scroll offsets when Teleport reparents its content.
  // Restore after that move, using the offsets captured by passive scroll events.
  void nextTick(() => {
    if (paneTargets.get(paneId) !== element) return;
    const positions = paneScrollPositions.get(paneId);
    if (!positions) return;
    for (const [scroller, position] of positions) {
      if (!element.contains(scroller)) positions.delete(scroller);
      else {
        scroller.scrollLeft = position.left;
        scroller.scrollTop = position.top;
      }
    }
  });
}

watch(paneIds, (ids) => {
  const visible = new Set(ids);
  for (const paneId of paneTargets.keys()) {
    if (!visible.has(paneId)) {
      paneTargets.delete(paneId);
      paneScrollPositions.delete(paneId);
    }
  }
});

function forwardResize(splitId: string, ratio: number, commit: boolean): void {
  emit("resize", splitId, ratio, commit);
}
</script>

<template>
  <WorkbenchSplitLayout
    v-bind="$attrs"
    :node="node"
    :groups="groups"
    :focused-pane-id="focusedPaneId"
    :active-drop-key="activeDropKey"
    :show-single-tabs="showSingleTabs"
    :register-pane-target="registerPaneTarget"
    :record-pane-scroll="recordPaneScroll"
    @focus-pane="emit('focus-pane', $event)"
    @resize="forwardResize"
  />
  <template v-for="paneId in paneIds" :key="paneId">
    <Teleport v-if="paneTargets.get(paneId)" :to="paneTargets.get(paneId)">
      <slot
        name="group"
        :group="groups[paneId]"
        :pane-id="paneId"
        :focused="focusedPaneId === paneId"
      />
    </Teleport>
  </template>
</template>
