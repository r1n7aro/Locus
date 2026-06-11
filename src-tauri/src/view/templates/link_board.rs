pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { view } from "@locus/view-runtime";

interface LinkEndpoint {
  id: string;
  label: string;
}

interface LinkConnection {
  source: string;
  target: string;
}

interface LinkLine {
  id: string;
  d: string;
}

const storageKey = "link-board.connections";

const sources = ref<LinkEndpoint[]>([
  { id: "albedo", label: "Albedo Map" },
  { id: "normal", label: "Normal Map" },
  { id: "mask", label: "Mask Texture" },
]);
const targets = ref<LinkEndpoint[]>([
  { id: "_BaseMap", label: "_BaseMap" },
  { id: "_BumpMap", label: "_BumpMap" },
  { id: "_MaskMap", label: "_MaskMap" },
]);
const connections = ref<LinkConnection[]>([]);
const selectedSourceId = ref("");
const statusText = ref("Ready");
const dirty = ref(false);

const boardRef = ref<HTMLElement | null>(null);
const endpointElements = new Map<string, HTMLElement>();
const lines = ref<LinkLine[]>([]);
let resizeObserver: ResizeObserver | null = null;

const linkedSourceIds = computed(() => new Set(connections.value.map((item) => item.source)));
const linkedTargetIds = computed(() => new Set(connections.value.map((item) => item.target)));
const connectionJson = computed(() => JSON.stringify({ connections: connections.value }, null, 2));

function endpointKey(kind: "source" | "target", id: string): string {
  return `${kind}:${id}`;
}

function registerEndpoint(kind: "source" | "target", id: string, element: unknown) {
  const key = endpointKey(kind, id);
  if (element instanceof HTMLElement) endpointElements.set(key, element);
  else endpointElements.delete(key);
}

function renderLines() {
  const board = boardRef.value;
  if (!board) {
    lines.value = [];
    return;
  }
  const boardRect = board.getBoundingClientRect();
  lines.value = connections.value.flatMap((connection) => {
    const source = endpointElements.get(endpointKey("source", connection.source));
    const target = endpointElements.get(endpointKey("target", connection.target));
    if (!source || !target) return [];
    const sourceRect = source.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    const start = {
      x: sourceRect.right - boardRect.left,
      y: sourceRect.top - boardRect.top + sourceRect.height / 2,
    };
    const end = {
      x: targetRect.left - boardRect.left,
      y: targetRect.top - boardRect.top + targetRect.height / 2,
    };
    const bend = Math.max(48, (end.x - start.x) / 2);
    return [{
      id: `${connection.source}->${connection.target}`,
      d: `M ${start.x} ${start.y} C ${start.x + bend} ${start.y}, ${end.x - bend} ${end.y}, ${end.x} ${end.y}`,
    }];
  });
}

function scheduleLineRender() {
  void nextTick().then(renderLines);
}

function pickSource(id: string) {
  selectedSourceId.value = selectedSourceId.value === id ? "" : id;
}

function pickTarget(id: string) {
  if (!selectedSourceId.value) {
    if (!linkedTargetIds.value.has(id)) return;
    connections.value = connections.value.filter((item) => item.target !== id);
    markDirty("Unlinked");
    scheduleLineRender();
    return;
  }
  const source = selectedSourceId.value;
  connections.value = [
    ...connections.value.filter((item) => item.source !== source && item.target !== id),
    { source, target: id },
  ];
  selectedSourceId.value = "";
  markDirty("Linked");
  scheduleLineRender();
}

function clearLinks() {
  if (!connections.value.length) return;
  connections.value = [];
  selectedSourceId.value = "";
  markDirty("Cleared");
  scheduleLineRender();
}

function markDirty(message: string) {
  dirty.value = true;
  statusText.value = message;
}

async function saveLinks() {
  try {
    await view.storage.set(storageKey, connections.value);
    dirty.value = false;
    statusText.value = "Saved";
  } catch (error) {
    statusText.value = error instanceof Error ? error.message : String(error);
  }
}

async function loadLinks() {
  try {
    const stored = await view.storage.get(storageKey);
    if (Array.isArray(stored)) {
      connections.value = stored.filter((item): item is LinkConnection =>
        !!item && typeof item === "object"
        && typeof (item as LinkConnection).source === "string"
        && typeof (item as LinkConnection).target === "string");
    }
  } catch (error) {
    statusText.value = error instanceof Error ? error.message : String(error);
  }
  scheduleLineRender();
}

onMounted(() => {
  resizeObserver = new ResizeObserver(() => renderLines());
  if (boardRef.value) resizeObserver.observe(boardRef.value);
  void loadLinks();
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  resizeObserver = null;
});
</script>

<template>
  <main class="view-shell link-board-view" data-locus-template="link-board">
    <header class="view-toolbar">
      <div class="toolbar-title">
        <span>Link Board</span>
        <small>{{ statusText }}</small>
      </div>
      <div class="toolbar-actions">
        <button type="button" :disabled="!connections.length" @click="clearLinks">Clear</button>
        <button type="button" :disabled="!dirty" @click="saveLinks">Save Links</button>
      </div>
    </header>

    <section class="link-workspace">
      <section ref="boardRef" class="link-board">
        <div class="link-column">
          <div class="link-column-title">Sources</div>
          <button
            v-for="item in sources"
            :key="item.id"
            :ref="(el) => registerEndpoint('source', item.id, el)"
            type="button"
            class="link-item"
            :class="{ active: selectedSourceId === item.id, linked: linkedSourceIds.has(item.id) }"
            @click="pickSource(item.id)"
          >{{ item.label }}</button>
        </div>

        <svg class="link-lines" aria-hidden="true">
          <path v-for="line in lines" :key="line.id" class="link-line" :d="line.d" />
        </svg>

        <div class="link-column">
          <div class="link-column-title">Targets</div>
          <button
            v-for="item in targets"
            :key="item.id"
            :ref="(el) => registerEndpoint('target', item.id, el)"
            type="button"
            class="link-item"
            :class="{ linked: linkedTargetIds.has(item.id) }"
            @click="pickTarget(item.id)"
          >{{ item.label }}</button>
        </div>
      </section>

      <p class="link-hint">
        Click a source, then a target to connect them. Click a linked target to remove its connection.
      </p>

      <section class="link-data-panel">
        <div class="view-section-title">Link Data</div>
        <pre>{{ connectionJson }}</pre>
      </section>
    </section>
  </main>
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    super::common::style_css(
        r#".link-workspace {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding: 14px;
}

.link-board {
  position: relative;
  min-height: 280px;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 48px;
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.link-column {
  position: relative;
  z-index: 1;
  flex: 0 1 240px;
  min-width: 150px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.link-column-title {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.link-item {
  min-height: 32px;
  justify-content: flex-start;
  text-align: left;
}

.link-item.linked {
  border-color: var(--accent-color);
}

.link-item.active {
  border-color: var(--accent-color);
  background: var(--accent-soft);
}

.link-lines {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}

.link-line {
  fill: none;
  stroke: var(--accent-color);
  stroke-width: 2;
  stroke-linecap: round;
  opacity: 0.9;
}

.link-hint {
  margin: 10px 2px;
  color: var(--text-secondary);
  font-size: 11px;
}

.link-data-panel {
  margin-top: 2px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.view-section-title {
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

pre {
  margin: 0;
  min-height: 88px;
  padding: 10px;
  overflow: auto;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}
"#,
    )
}
