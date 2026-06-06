pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { computed, ref } from "vue";
import { CanvasView } from "@locus/components";

interface CanvasBlock {
  id: string;
  title: string;
  subtitle: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

const canvasRef = ref(null);
const selectedBlockIds = ref<string[]>(["overview"]);
const dirty = ref(false);
const blocks = ref<CanvasBlock[]>([
  {
    id: "overview",
    title: "Overview",
    subtitle: "Free canvas block",
    x: 80,
    y: 80,
    width: 260,
    height: 118,
  },
  {
    id: "details",
    title: "Details",
    subtitle: "Custom node rendering",
    x: 390,
    y: 160,
    width: 280,
    height: 132,
  },
]);
const canvasBehavior = {
  readonly: false,
  allowCreate: true,
  allowDelete: true,
  allowCopy: true,
  allowPaste: true,
  allowMove: true,
  allowSelect: true,
  allowBoxSelect: true,
  allowContextMenu: true,
};
const clipboard = ref<CanvasBlock[]>([]);
let blockSequence = blocks.value.length;

const selectedBlock = computed(() => {
  return blocks.value.find((block) => block.id === selectedBlockIds.value[0]) ?? null;
});

function blockClass(_block: CanvasBlock, selected: boolean) {
  return ["canvas-block", selected ? "selected" : ""];
}

function nextBlockId() {
  let id = "";
  do {
    blockSequence += 1;
    id = `block-${blockSequence}`;
  } while (blocks.value.some((block) => block.id === id));
  return id;
}

function addBlockAt(point?: { x: number; y: number }) {
  if (canvasBehavior.readonly || !canvasBehavior.allowCreate) return;
  const id = nextBlockId();
  const index = blocks.value.length + 1;
  blocks.value.push({
    id,
    title: `Block ${index}`,
    subtitle: "Custom content",
    x: point ? point.x : 120 + index * 34,
    y: point ? point.y : 120 + index * 28,
    width: 260,
    height: 118,
  });
  selectedBlockIds.value = [id];
  dirty.value = true;
}

function addBlock() {
  addBlockAt();
}

function removeSelectedBlocks(event?: { itemIds?: string[] }) {
  if (canvasBehavior.readonly || !canvasBehavior.allowDelete) return;
  const selectedIds = event?.itemIds?.length ? event.itemIds : selectedBlockIds.value;
  if (!selectedIds.length) return;
  const selectedSet = new Set(selectedIds);
  const before = blocks.value.length;
  blocks.value = blocks.value.filter((block) => !selectedSet.has(block.id));
  if (blocks.value.length === before) return;
  selectedBlockIds.value = blocks.value[0] ? [blocks.value[0].id] : [];
  dirty.value = true;
}

function markDirty() {
  dirty.value = true;
}

function copySelection(event: { itemIds: string[] }) {
  if (!canvasBehavior.allowCopy || !event.itemIds.length) return;
  const selectedSet = new Set(event.itemIds);
  clipboard.value = blocks.value
    .filter((block) => selectedSet.has(block.id))
    .map((block) => ({ ...block }));
}

function pasteSelection() {
  if (canvasBehavior.readonly || !canvasBehavior.allowPaste || !clipboard.value.length) return;
  const pasted = clipboard.value.map((block, index) => ({
    ...block,
    id: nextBlockId(),
    title: `${block.title} Copy`,
    x: block.x + 32 + index * 12,
    y: block.y + 32 + index * 12,
  }));
  blocks.value.push(...pasted);
  selectedBlockIds.value = pasted.map((block) => block.id);
  dirty.value = true;
}

function onCanvasContextMenu(event: { itemId?: string; x: number; y: number }) {
  if (event.itemId) return;
  addBlockAt({ x: event.x, y: event.y });
}

function fitCanvas() {
  canvasRef.value?.fitContent?.();
}
</script>

<template>
  <main class="view-shell canvas-board-view" data-locus-template="canvas-board">
    <header class="view-toolbar">
      <div class="toolbar-title">
        <span>Canvas</span>
        <small>{{ dirty ? "Modified" : "Ready" }}</small>
      </div>
      <div class="toolbar-actions">
        <button type="button" @click="fitCanvas">Fit</button>
        <button type="button" :disabled="canvasBehavior.readonly || !canvasBehavior.allowCreate" @click="addBlock">Add</button>
        <button type="button" :disabled="!selectedBlock || canvasBehavior.readonly || !canvasBehavior.allowDelete" @click="removeSelectedBlocks()">Delete</button>
      </div>
    </header>

    <CanvasView
      ref="canvasRef"
      v-model:selected-item-ids="selectedBlockIds"
      :items="blocks"
      :item-class="blockClass"
      :edit-behavior="canvasBehavior"
      @copy-selection="copySelection"
      @paste-selection="pasteSelection"
      @item-move-end="markDirty"
      @delete-selection="removeSelectedBlocks"
      @context-menu="onCanvasContextMenu"
    >
      <template #default="{ item, selected }">
        <div class="canvas-block-header">
          <span>{{ item.title }}</span>
          <small>{{ item.id }}</small>
        </div>
        <div class="canvas-block-body">
          <div class="canvas-block-title">{{ item.subtitle }}</div>
          <input v-model="item.subtitle" :readonly="!selected || canvasBehavior.readonly" @change="markDirty" />
        </div>
      </template>
    </CanvasView>
  </main>
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
}

.view-shell {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
}

.view-toolbar {
  min-height: 42px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.toolbar-title {
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.toolbar-title span {
  font-size: 13px;
  font-weight: 650;
}

.toolbar-title small {
  color: var(--text-secondary);
  font-size: 11px;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

button {
  min-height: 28px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
}

button:disabled {
  opacity: 0.58;
}

.canvas-board-view > .locus-canvas-view {
  flex: 1;
  min-height: 0;
}

.canvas-block {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  background: var(--surface-elevated);
  color: var(--text-color);
  box-shadow: 0 1px 0 color-mix(in srgb, var(--border-color) 70%, transparent);
  overflow: hidden;
}

.canvas-block.selected {
  border-color: var(--accent-color);
}

.canvas-block-header {
  min-height: 34px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 0 10px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--surface-elevated) 82%, var(--sidebar-bg) 18%);
}

.canvas-block-header span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 650;
}

.canvas-block-header small {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-size: 11px;
}

.canvas-block-body {
  display: grid;
  gap: 8px;
  padding: 10px;
}

.canvas-block-title {
  color: var(--text-secondary);
  font-size: 12px;
}

input {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  box-sizing: border-box;
}
"#
    .to_string()
}
