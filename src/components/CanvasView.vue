
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from "vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import {
  canvasGetSpec,
  canvasUpdateField,
  canvasRefresh,
  canvasSave,
  canvasLoad,
  canvasList,
  type CanvasRefreshQuery,
} from "../services/canvas";
import { getWorkingDir } from "../services/project";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import type {
  CanvasGraphSpec,
  CanvasNode,
  CanvasField,
  CanvasEdge,
  UndoEntry,
} from "../types";

const NODE_WIDTH = 280;
const NODE_HEADER_HEIGHT = 36;
const FIELD_ROW_HEIGHT = 32;
const UNDO_MAX = 50;
const MERGE_THRESHOLD_MS = 500;

const spec = ref<CanvasGraphSpec | null>(null);
const specId = ref("");
const projectPath = ref("");
const loading = ref(true);
const loadError = ref("");

const panX = ref(0);
const panY = ref(0);
const zoom = ref(1);

const nodePositions = ref<Record<string, { x: number; y: number }>>({});

const fieldValues = ref<Record<string, any>>({});

const nodeStatus = ref<Record<string, "ok" | "unresolved">>({});

const undoStack = ref<UndoEntry[]>([]);

const dragging = ref<{
  nodeId: string;
  startX: number;
  startY: number;
  origX: number;
  origY: number;
} | null>(null);
const panning = ref<{
  startX: number;
  startY: number;
  origPanX: number;
  origPanY: number;
} | null>(null);
let releaseSelectionLock: (() => void) | null = null;

const toastMsg = ref("");
let toastTimer: number | null = null;

function showToast(msg: string) {
  toastMsg.value = msg;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toastMsg.value = "";
  }, 3000);
}

const fieldReadonly = computed<Record<string, boolean>>(() => {
  const map: Record<string, boolean> = {};
  if (!spec.value) return map;
  for (const node of spec.value.nodes) {
    for (const field of node.fields) {
      map[field.id] = !field.update || field.readonly === true;
    }
  }
  return map;
});

// ── Transform style ──
const transformStyle = computed(() => ({
  transform: `translate(${panX.value}px, ${panY.value}px) scale(${zoom.value})`,
  transformOrigin: "0 0",
}));

onMounted(async () => {
  try {
    const params = new URLSearchParams(window.location.search);
    specId.value = params.get("specId") || "";

    if (!specId.value) {
      loadError.value = "Missing specId parameter";
      loading.value = false;
      return;
    }

    projectPath.value = await getWorkingDir();

    const specJson = await canvasGetSpec(specId.value);
    const parsed = JSON.parse(specJson) as CanvasGraphSpec;
    spec.value = parsed;

    for (const node of parsed.nodes) {
      nodePositions.value[node.id] = node.position
        ? { ...node.position }
        : { x: 0, y: 0 };
    }

    for (const node of parsed.nodes) {
      for (const field of node.fields) {
        fieldValues.value[field.id] = normalizeFieldValue(field.value, field.type);
      }
      nodeStatus.value[node.id] = "ok";
    }

    nextTick(() => fitView());
  } catch (e) {
    loadError.value = normalizeAppError(e).message;
  } finally {
    loading.value = false;
  }
});

function onKeyDown(e: KeyboardEvent) {
  if (e.ctrlKey && e.key === "z") {
    e.preventDefault();
    undo();
  } else if (e.ctrlKey && e.key === "s") {
    e.preventDefault();
    saveCanvas();
  } else if (e.ctrlKey && e.key === "0") {
    e.preventDefault();
    resetZoom();
  } else if (e.key === "F5") {
    e.preventDefault();
    refreshFromUnity();
  }
}

onMounted(() => {
  window.addEventListener("keydown", onKeyDown);
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
  releaseSelectionLock?.();
  releaseSelectionLock = null;
});

function nodeStyle(node: CanvasNode) {
  const pos = nodePositions.value[node.id] || { x: 0, y: 0 };
  return {
    left: pos.x + "px",
    top: pos.y + "px",
    width: NODE_WIDTH + "px",
  };
}

function startDrag(nodeId: string, e: MouseEvent) {
  const pos = nodePositions.value[nodeId];
  if (!pos) return;
  dragging.value = {
    nodeId,
    startX: e.clientX,
    startY: e.clientY,
    origX: pos.x,
    origY: pos.y,
  };
  releaseSelectionLock?.();
  releaseSelectionLock = acquireSelectionLock();
  e.preventDefault();
}

function onMouseMove(e: MouseEvent) {
  if (dragging.value) {
    const d = dragging.value;
    const dx = (e.clientX - d.startX) / zoom.value;
    const dy = (e.clientY - d.startY) / zoom.value;
    nodePositions.value[d.nodeId] = {
      x: d.origX + dx,
      y: d.origY + dy,
    };
  } else if (panning.value) {
    const p = panning.value;
    panX.value = p.origPanX + (e.clientX - p.startX);
    panY.value = p.origPanY + (e.clientY - p.startY);
  }
}

function onMouseUp() {
  if (dragging.value) {
    const d = dragging.value;
    const newPos = nodePositions.value[d.nodeId];
    if (newPos && (newPos.x !== d.origX || newPos.y !== d.origY)) {
      pushUndo({
        type: "node_move",
        nodeId: d.nodeId,
        oldPosition: { x: d.origX, y: d.origY },
        newPosition: { ...newPos },
        timestamp: Date.now(),
      });
    }
    dragging.value = null;
  }
  panning.value = null;
  releaseSelectionLock?.();
  releaseSelectionLock = null;
}

function onBackgroundDrag(e: MouseEvent) {
  if (e.target === e.currentTarget || (e.target as HTMLElement).classList.contains("canvas-edges")) {
    panning.value = {
      startX: e.clientX,
      startY: e.clientY,
      origPanX: panX.value,
      origPanY: panY.value,
    };
    releaseSelectionLock?.();
    releaseSelectionLock = acquireSelectionLock();
    e.preventDefault();
  }
}

function onWheel(e: WheelEvent) {
  e.preventDefault();
  const factor = e.deltaY > 0 ? 0.9 : 1.1;
  const newZoom = Math.min(3, Math.max(0.1, zoom.value * factor));

  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const my = e.clientY - rect.top;
  panX.value = mx - ((mx - panX.value) / zoom.value) * newZoom;
  panY.value = my - ((my - panY.value) / zoom.value) * newZoom;
  zoom.value = newZoom;
}

function computeEdgeAnchors(edge: CanvasEdge): { sx: number; sy: number; tx: number; ty: number } | null {
  const sourcePos = nodePositions.value[edge.source];
  const targetPos = nodePositions.value[edge.target];
  if (!sourcePos || !targetPos) return null;

  const sx = sourcePos.x + NODE_WIDTH;
  let sy = sourcePos.y + NODE_HEADER_HEIGHT / 2;
  if (edge.sourceField && spec.value) {
    const sourceNode = spec.value.nodes.find((n) => n.id === edge.source);
    if (sourceNode) {
      const fieldIdx = sourceNode.fields.findIndex(
        (f) => f.id === edge.sourceField
      );
      if (fieldIdx >= 0) {
        sy =
          sourcePos.y +
          NODE_HEADER_HEIGHT +
          fieldIdx * FIELD_ROW_HEIGHT +
          FIELD_ROW_HEIGHT / 2;
      }
    }
  }

  const tx = targetPos.x;
  const ty = targetPos.y + NODE_HEADER_HEIGHT / 2;

  return { sx, sy, tx, ty };
}

const edgeRenderData = computed(() => {
  if (!spec.value) return [];
  return (spec.value.edges || []).map((edge) => {
    const a = computeEdgeAnchors(edge);
    if (!a) return { id: edge.id, path: "", sx: 0, sy: 0, tx: 0, ty: 0 };
    const dx = Math.abs(a.tx - a.sx) * 0.5;
    return {
      id: edge.id,
      path: `M ${a.sx} ${a.sy} C ${a.sx + dx} ${a.sy}, ${a.tx - dx} ${a.ty}, ${a.tx} ${a.ty}`,
      ...a,
    };
  }).filter((e) => e.path !== "");
});

function normalizeFieldValue(value: any, type: string): any {
  if (Array.isArray(value)) {
    if (type === "vector2" && value.length >= 2)
      return { x: value[0], y: value[1] };
    if (type === "vector3" && value.length >= 3)
      return { x: value[0], y: value[1], z: value[2] };
    if (type === "vector4" && value.length >= 4)
      return { x: value[0], y: value[1], z: value[2], w: value[3] };
    if (type === "color" && value.length >= 4)
      return { r: value[0], g: value[1], b: value[2], a: value[3] };
    if (type === "color" && value.length >= 3)
      return { r: value[0], g: value[1], b: value[2], a: 1 };
  }
  return value;
}

function formatDisplayValue(field: CanvasField): string {
  const val = fieldValues.value[field.id];
  if (val === null || val === undefined) return "(none)";
  if (field.type === "enum" && field.enumNames) {
    return field.enumNames[val] || String(val);
  }
  if (field.type === "color" && typeof val === "object") {
    const r = ((val.r ?? 0) * 255).toFixed(0);
    const g = ((val.g ?? 0) * 255).toFixed(0);
    const b = ((val.b ?? 0) * 255).toFixed(0);
    const a = (val.a ?? 1).toFixed(2);
    return `rgba(${r}, ${g}, ${b}, ${a})`;
  }
  if (field.type === "bool") return val ? "true" : "false";
  if (typeof val === "object") return JSON.stringify(val);
  return String(val);
}

function vectorComponents(type: string): string[] {
  switch (type) {
    case "vector2": return ["x", "y"];
    case "vector3": return ["x", "y", "z"];
    case "vector4": return ["x", "y", "z", "w"];
    default: return [];
  }
}

function rgbaToHex(val: any): string {
  if (!val || typeof val !== "object") return "#000000";
  const r = Math.round((val.r || 0) * 255);
  const g = Math.round((val.g || 0) * 255);
  const b = Math.round((val.b || 0) * 255);
  return `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
}

function hexToRgba(hex: string, alpha: number = 1): any {
  const r = parseInt(hex.slice(1, 3), 16) / 255;
  const g = parseInt(hex.slice(3, 5), 16) / 255;
  const b = parseInt(hex.slice(5, 7), 16) / 255;
  return { r, g, b, a: alpha };
}

async function commitFieldEdit(field: CanvasField, event: Event) {
  if (fieldReadonly.value[field.id]) return;
  if (!field.update) return;

  const target = event.target as HTMLInputElement | HTMLSelectElement;
  let newValue: any;

  if (field.type === "bool") {
    newValue = (target as HTMLInputElement).checked;
  } else if (field.type === "float") {
    newValue = parseFloat(target.value);
  } else if (field.type === "int" || field.type === "enum") {
    newValue = parseInt(target.value, 10);
  } else if (field.type === "color") {
    const oldVal = fieldValues.value[field.id];
    newValue = hexToRgba(target.value, oldVal?.a ?? 1);
  } else {
    newValue = target.value;
  }

  const oldValue = fieldValues.value[field.id];

  fieldValues.value[field.id] = newValue;

  pushUndo({
    type: "field_edit",
    fieldId: field.id,
    oldValue,
    newValue,
    update: field.update,
    valueType: field.type,
    timestamp: Date.now(),
  });

  try {
    await canvasUpdateField(
      projectPath.value,
      spec.value?.context?.scenePath || null,
      {
        mode: field.update.mode,
        gameObjectPath: field.update.gameObjectPath,
        componentType: field.update.componentType,
        propertyPath: field.update.propertyPath,
        code: field.update.code,
      },
      newValue,
      field.type,
    );
  } catch (e) {
    showToast(t("canvas.updateFailed", normalizeAppError(e).message));
  }
}

async function commitVectorEdit(
  field: CanvasField,
  comp: string,
  event: Event
) {
  if (fieldReadonly.value[field.id] || !field.update) return;

  const target = event.target as HTMLInputElement;
  const oldValue = { ...fieldValues.value[field.id] };
  const newValue = { ...oldValue, [comp]: parseFloat(target.value) };

  fieldValues.value[field.id] = newValue;

  pushUndo({
    type: "field_edit",
    fieldId: field.id,
    oldValue,
    newValue,
    update: field.update,
    valueType: field.type,
    timestamp: Date.now(),
  });

  try {
    await canvasUpdateField(
      projectPath.value,
      spec.value?.context?.scenePath || null,
      {
        mode: field.update.mode,
        gameObjectPath: field.update.gameObjectPath,
        componentType: field.update.componentType,
        propertyPath: field.update.propertyPath,
        code: field.update.code,
      },
      newValue,
      field.type,
    );
  } catch (e) {
    showToast(t("canvas.updateFailed", normalizeAppError(e).message));
  }
}

// ── Undo ──
function pushUndo(entry: UndoEntry) {
  const stack = undoStack.value;
  const last = stack[stack.length - 1];
  if (
    last &&
    last.type === "field_edit" &&
    entry.type === "field_edit" &&
    last.fieldId === entry.fieldId &&
    entry.timestamp - last.timestamp < MERGE_THRESHOLD_MS
  ) {
    last.newValue = entry.newValue;
    last.timestamp = entry.timestamp;
  } else {
    stack.push(entry);
    if (stack.length > UNDO_MAX) stack.shift();
  }
}

async function undo() {
  const entry = undoStack.value.pop();
  if (!entry) return;

  switch (entry.type) {
    case "field_edit":
      fieldValues.value[entry.fieldId] = entry.oldValue;
      try {
        const result = (await canvasUpdateField(
          projectPath.value,
          spec.value?.context?.scenePath ?? null,
          {
            mode: entry.update.mode,
            gameObjectPath: entry.update.gameObjectPath,
            componentType: entry.update.componentType,
            propertyPath: entry.update.propertyPath,
            code: entry.update.code,
          },
          entry.oldValue,
          entry.valueType,
        )) as { ok?: boolean; error?: string } | null;
        if (result && !result.ok) {
          showToast(t("canvas.undoFailed", result.error || "unknown"));
        }
      } catch (e) {
        showToast(t("canvas.undoFailed", normalizeAppError(e).message));
      }
      break;

    case "node_move":
      nodePositions.value[entry.nodeId] = { ...entry.oldPosition };
      break;
  }
}

async function refreshFromUnity() {
  if (!spec.value) return;

  const queries: CanvasRefreshQuery[] = [];
  for (const node of spec.value.nodes) {
    for (const field of node.fields) {
      if (field.update && field.update.mode === "serialized") {
        const { gameObjectPath, componentType, propertyPath } = field.update;
        if (!gameObjectPath || !componentType || !propertyPath) continue;
        queries.push({
          id: field.id,
          gameObjectPath,
          componentType,
          propertyPath,
        });
      }
    }
  }

  if (queries.length === 0) {
    showToast(t("canvas.noRefreshFields"));
    return;
  }

  try {
    const result = await canvasRefresh(
      projectPath.value,
      spec.value.context?.scenePath ?? null,
      queries,
    );

    if (result && result.results) {
      for (const r of result.results) {
        if (r.exists) {
          fieldValues.value[r.id] = r.value;
          const node = spec.value.nodes.find((n) =>
            n.fields.some((f) => f.id === r.id)
          );
          if (node) nodeStatus.value[node.id] = "ok";
        } else {
          const node = spec.value.nodes.find((n) =>
            n.fields.some((f) => f.id === r.id)
          );
          if (node) nodeStatus.value[node.id] = "unresolved";
        }
      }
      showToast(t("canvas.refreshDone"));
    }
  } catch (e) {
    showToast(t("canvas.refreshFailed", normalizeAppError(e).message));
  }
}

async function saveCanvas() {
  if (!spec.value) return;
  const name = spec.value.title || "untitled";
  const data = JSON.stringify(
    {
      version: 1,
      savedAt: new Date().toISOString(),
      spec: spec.value,
      viewState: {
        panX: panX.value,
        panY: panY.value,
        zoom: zoom.value,
        nodePositions: nodePositions.value,
      },
    },
    null,
    2
  );

  try {
    await canvasSave(projectPath.value, name, data);
    showToast(t("canvas.saved", name));
  } catch (e) {
    showToast(t("canvas.saveFailed", normalizeAppError(e).message));
  }
}

async function loadCanvas() {
  try {
    const names = await canvasList(projectPath.value);
    if (names.length === 0) {
      showToast(t("canvas.noSavedCanvas"));
      return;
    }
    const data = await canvasLoad(projectPath.value, names[0]);
    const parsed = JSON.parse(data);
    if (parsed.spec) {
      spec.value = parsed.spec;
      if (parsed.viewState) {
        panX.value = parsed.viewState.panX || 0;
        panY.value = parsed.viewState.panY || 0;
        zoom.value = parsed.viewState.zoom || 1;
        if (parsed.viewState.nodePositions) {
          nodePositions.value = parsed.viewState.nodePositions;
        }
      }
      for (const node of parsed.spec.nodes) {
        for (const field of node.fields) {
          fieldValues.value[field.id] = normalizeFieldValue(field.value, field.type);
        }
        nodeStatus.value[node.id] = "ok";
      }
      undoStack.value = [];
      showToast(t("canvas.loaded", names[0]));
    }
  } catch (e) {
    showToast(t("canvas.loadFailed", normalizeAppError(e).message));
  }
}

function fitView() {
  if (!spec.value || spec.value.nodes.length === 0) return;
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const node of spec.value.nodes) {
    const pos = nodePositions.value[node.id];
    if (!pos) continue;
    minX = Math.min(minX, pos.x);
    minY = Math.min(minY, pos.y);
    maxX = Math.max(maxX, pos.x + NODE_WIDTH);
    maxY = Math.max(maxY, pos.y + NODE_HEADER_HEIGHT + node.fields.length * FIELD_ROW_HEIGHT);
  }
  const w = maxX - minX + 100;
  const h = maxY - minY + 100;
  const vw = window.innerWidth;
  const vh = window.innerHeight - 48; // toolbar height
  const z = Math.min(1.5, vw / w, vh / h);
  zoom.value = z;
  panX.value = (vw - w * z) / 2 - minX * z;
  panY.value = (vh - h * z) / 2 - minY * z + 48;
}

function resetZoom() {
  zoom.value = 1;
}
</script>

<template>
  <div
    class="canvas-root"
    tabindex="0"
    @mousemove="onMouseMove"
    @mouseup="onMouseUp"
    @mouseleave="onMouseUp"
  >
    <div class="canvas-toolbar">
      <span class="toolbar-title">{{ spec?.title || 'Canvas' }}</span>
      <span class="toolbar-separator" />
      <button @click="refreshFromUnity" :title="t('canvas.toolbar.refresh') + ' (F5)'">↻ {{ t("canvas.toolbar.refresh") }}</button>
      <button @click="undo" :title="t('canvas.toolbar.undo') + ' (Ctrl+Z)'">↩ {{ t("canvas.toolbar.undo") }}</button>
      <span class="toolbar-separator" />
      <button @click="saveCanvas" :title="t('canvas.toolbar.save') + ' (Ctrl+S)'">{{ t("canvas.toolbar.save") }}</button>
      <button @click="loadCanvas" :title="t('canvas.toolbar.load')">{{ t("canvas.toolbar.load") }}</button>
      <span class="toolbar-separator" />
      <button @click="fitView" :title="t('canvas.toolbar.fit')">{{ t("canvas.toolbar.fit") }}</button>
      <button @click="resetZoom" :title="t('canvas.toolbar.resetZoom')">1:1</button>
    </div>

    <div v-if="loading" class="canvas-loading">{{ t("common.loading") }}</div>
    <div v-else-if="loadError" class="canvas-error">{{ loadError }}</div>

    <div
      v-else-if="spec"
      class="canvas-viewport"
      @wheel.prevent="onWheel"
      @mousedown="onBackgroundDrag"
    >
      <div class="canvas-transform" :style="transformStyle">
        <svg class="canvas-edges">
          <path v-for="er in edgeRenderData" :key="er.id" :d="er.path" class="edge-path" />
        </svg>

        <div
          v-for="node in spec.nodes"
          :key="node.id"
          class="canvas-node"
          :class="{ unresolved: nodeStatus[node.id] === 'unresolved' }"
          :style="nodeStyle(node)"
        >
          <div
            class="node-header"
            @mousedown.stop="startDrag(node.id, $event)"
          >
            <span class="node-label">{{ node.label }}</span>
            <span v-if="node.subtitle" class="node-subtitle">{{ node.subtitle }}</span>
          </div>

          <div class="node-fields">
            <div
              v-for="field in node.fields"
              :key="field.id"
              class="node-field"
              :class="{ readonly: fieldReadonly[field.id] }"
            >
              <label>{{ field.label || field.name }}</label>

              <span v-if="fieldReadonly[field.id]" class="field-readonly-value">
                {{ formatDisplayValue(field) }}
              </span>

              <template v-else>
                <!-- float / int -->
                <input
                  v-if="field.type === 'float' || field.type === 'int'"
                  type="number"
                  :step="field.type === 'int' ? 1 : 0.1"
                  :min="field.range?.[0]"
                  :max="field.range?.[1]"
                  :value="fieldValues[field.id]"
                  @change="commitFieldEdit(field, $event)"
                />
                <!-- bool -->
                <input
                  v-else-if="field.type === 'bool'"
                  type="checkbox"
                  :checked="fieldValues[field.id]"
                  @change="commitFieldEdit(field, $event)"
                />
                <!-- string -->
                <input
                  v-else-if="field.type === 'string'"
                  type="text"
                  :value="fieldValues[field.id]"
                  @blur="commitFieldEdit(field, $event)"
                />
                <!-- enum -->
                <select
                  v-else-if="field.type === 'enum'"
                  :value="fieldValues[field.id]"
                  @change="commitFieldEdit(field, $event)"
                >
                  <option
                    v-for="(name, idx) in field.enumNames"
                    :key="idx"
                    :value="idx"
                  >
                    {{ name }}
                  </option>
                </select>
                <!-- color -->
                <input
                  v-else-if="field.type === 'color'"
                  type="color"
                  :value="rgbaToHex(fieldValues[field.id])"
                  @change="commitFieldEdit(field, $event)"
                />
                <!-- vector2/3/4 -->
                <div
                  v-else-if="field.type.startsWith('vector')"
                  class="vector-inputs"
                >
                  <input
                    v-for="comp in vectorComponents(field.type)"
                    :key="comp"
                    type="number"
                    step="0.01"
                    :value="fieldValues[field.id]?.[comp]"
                    @blur="commitVectorEdit(field, comp, $event)"
                  />
                </div>
                <span v-else class="field-ref-value">
                  {{ fieldValues[field.id] || '(none)' }}
                </span>
              </template>
            </div>
          </div>
        </div>

        <svg class="canvas-edges canvas-edge-dots">
          <template v-for="er in edgeRenderData" :key="er.id">
            <circle :cx="er.sx" :cy="er.sy" r="4" class="edge-dot" />
            <circle :cx="er.tx" :cy="er.ty" r="4" class="edge-dot" />
          </template>
        </svg>
      </div>
    </div>

    <!-- Toast -->
    <div v-if="toastMsg" class="canvas-toast">{{ toastMsg }}</div>
  </div>
</template>

<style scoped>
.canvas-root {
  width: 100vw;
  height: 100vh;
  background: #1a1a1a;
  color: #e5e5e5;
  font-family: var(--font-ui);
  font-size: 13px;
  overflow: hidden;
  outline: none;
  position: relative;
}

.canvas-toolbar {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: 40px;
  background: #222;
  border-bottom: 1px solid #333;
  display: flex;
  align-items: center;
  padding: 0 12px;
  gap: 6px;
  z-index: 100;
  -webkit-app-region: drag;
}

.canvas-toolbar button {
  -webkit-app-region: no-drag;
  background: #333;
  border: 1px solid #444;
  color: #ccc;
  padding: 4px 10px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
}

.canvas-toolbar button:hover {
  background: #444;
  color: #fff;
}

.toolbar-title {
  font-weight: 600;
  color: #fff;
  margin-right: 8px;
  -webkit-app-region: no-drag;
}

.toolbar-separator {
  width: 1px;
  height: 20px;
  background: #444;
  margin: 0 4px;
}

.canvas-loading,
.canvas-error {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  font-size: 16px;
}

.canvas-error {
  color: #e55;
}

.canvas-viewport {
  position: absolute;
  top: 40px;
  left: 0;
  right: 0;
  bottom: 0;
  overflow: hidden;
  background:
    radial-gradient(circle, #2a2a2a 1px, transparent 1px);
  background-size: 20px 20px;
  cursor: grab;
}

.canvas-viewport:active {
  cursor: grabbing;
}

.canvas-transform {
  position: absolute;
  top: 0;
  left: 0;
  width: 0;
  height: 0;
}

.canvas-edges {
  position: absolute;
  top: 0;
  left: 0;
  width: 10000px;
  height: 10000px;
  pointer-events: none;
  overflow: visible;
}

.edge-path {
  fill: none;
  stroke: rgba(100, 180, 255, 0.5);
  stroke-width: 2;
}

.canvas-edge-dots {
  z-index: 10;
}

.edge-dot {
  fill: rgba(100, 180, 255, 0.7);
  stroke: rgba(100, 180, 255, 0.9);
  stroke-width: 1.5;
}

.canvas-node {
  position: absolute;
  background: #252525;
  border: 1px solid #3a3a3a;
  border-radius: 8px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  overflow: hidden;
}

.canvas-node.unresolved {
  border: 2px dashed #e55;
  opacity: 0.7;
}

.node-header {
  padding: 8px 12px;
  background: #2d2d2d;
  border-bottom: 1px solid #3a3a3a;
  cursor: move;
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 36px;
}

.node-label {
  font-weight: 600;
  color: #fff;
  font-size: 13px;
}

.node-subtitle {
  color: #888;
  font-size: 11px;
}

.node-fields {
  padding: 4px 0;
}

.node-field {
  display: flex;
  align-items: center;
  padding: 4px 12px;
  min-height: 28px;
  gap: 8px;
}

.node-field label {
  flex-shrink: 0;
  width: 90px;
  font-size: 11px;
  color: #aaa;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.node-field.readonly label {
  color: #666;
}

.field-readonly-value {
  color: #777;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.field-ref-value {
  color: #6ab;
  font-size: 12px;
}

.node-field input[type="number"],
.node-field input[type="text"],
.node-field select {
  flex: 1;
  min-width: 0;
  background: #1a1a1a;
  border: 1px solid #3a3a3a;
  color: #e5e5e5;
  padding: 3px 6px;
  border-radius: 3px;
  font-size: 12px;
  outline: none;
}

.node-field input[type="number"]:focus,
.node-field input[type="text"]:focus,
.node-field select:focus {
  border-color: #5a8;
}

.node-field input[type="checkbox"] {
  accent-color: #5a8;
}

.node-field input[type="color"] {
  width: 40px;
  height: 24px;
  padding: 0;
  border: 1px solid #3a3a3a;
  border-radius: 3px;
  cursor: pointer;
}

.vector-inputs {
  display: flex;
  gap: 4px;
  flex: 1;
}

.vector-inputs input {
  flex: 1;
  min-width: 0;
  background: #1a1a1a;
  border: 1px solid #3a3a3a;
  color: #e5e5e5;
  padding: 3px 4px;
  border-radius: 3px;
  font-size: 11px;
  outline: none;
}

.vector-inputs input:focus {
  border-color: #5a8;
}

/* ── Toast ── */
.canvas-toast {
  position: fixed;
  bottom: 20px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(0, 0, 0, 0.8);
  color: #fff;
  padding: 8px 20px;
  border-radius: 6px;
  font-size: 13px;
  z-index: 200;
  pointer-events: none;
}
</style>
