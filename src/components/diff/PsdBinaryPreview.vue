<script setup lang="ts">
import { ref, computed, onMounted, watch, nextTick } from "vue";
import { refetchDiffByKey } from "../../services/diff";
import type { BinaryPreview } from "../../types";

const props = defineProps<{
  preview: BinaryPreview;
  compact?: boolean;
  diffKey: string;
  mode?: "diff" | "neutral";
}>();

const activeSide = ref<"before" | "after">(props.preview.after ? "after" : "before");
const canvasRef = ref<HTMLCanvasElement | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

const activeRef = computed(() =>
  activeSide.value === "before" ? props.preview.before : props.preview.after,
);
const hasBoth = computed(() => !!props.preview.before && !!props.preview.after);
const statusLabel = computed(() => {
  if (props.mode === "neutral") return null;
  if (hasBoth.value) return null;
  return props.preview.after ? "Added" : "Deleted";
});

// Module-level: only call initializeCanvas once per session
let agPsdModule: typeof import("ag-psd") | null = null;

function toImageData(pixelData: { data: ArrayLike<number>; width: number; height: number }): ImageData {
  return new ImageData(new Uint8ClampedArray(pixelData.data), pixelData.width, pixelData.height);
}

async function ensureAgPsd(): Promise<typeof import("ag-psd")> {
  if (agPsdModule) return agPsdModule;
  const mod = await import("ag-psd");
  mod.initializeCanvas(
    (width: number, height: number) => {
      const c = document.createElement("canvas");
      c.width = width;
      c.height = height;
      return c;
    },
    (width: number, height: number) => {
      const c = document.createElement("canvas");
      c.width = width;
      c.height = height;
      return c.getContext("2d")!.createImageData(width, height);
    },
  );
  agPsdModule = mod;
  return mod;
}

async function renderPsd() {
  const assetRef = activeRef.value;
  if (!assetRef) return;

  loading.value = true;
  error.value = null;

  try {
    const [agPsd, response] = await Promise.all([
      ensureAgPsd(),
      fetch(assetRef.url),
    ]);

    if (!response.ok) {
      await refetchDiffByKey(props.diffKey);
      error.value = "Failed to load PSD data";
      return;
    }

    const buffer = await response.arrayBuffer();

    const psd = agPsd.readPsd(new Uint8Array(buffer), {
      skipThumbnail: true,
      skipLayerImageData: true,
    });

    // Try psd.canvas first (composite or layer-composed)
    let sourceCanvas: HTMLCanvasElement | null = psd.canvas as HTMLCanvasElement | null;

    // If no composite canvas but we have imageData, render it manually
    if (!sourceCanvas && psd.imageData) {
      sourceCanvas = document.createElement("canvas");
      sourceCanvas.width = psd.width;
      sourceCanvas.height = psd.height;
      const ctx = sourceCanvas.getContext("2d")!;
      ctx.putImageData(toImageData(psd.imageData), 0, 0);
    }

    // Last resort: if we have layer children with canvases, use the first visible one
    if (!sourceCanvas && psd.children?.length) {
      for (const layer of psd.children) {
        if ((layer as any).canvas) {
          sourceCanvas = (layer as any).canvas;
          break;
        }
      }
    }

    if (sourceCanvas) {
      // Set loading=false first so the <canvas> element mounts, then draw on nextTick
      loading.value = false;
      await nextTick();
      if (canvasRef.value) {
        const ctx = canvasRef.value.getContext("2d");
        if (ctx) {
          canvasRef.value.width = sourceCanvas.width;
          canvasRef.value.height = sourceCanvas.height;
          ctx.drawImage(sourceCanvas, 0, 0);
        }
      }
    } else {
      error.value = "Unable to render PSD preview";
      loading.value = false;
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    error.value = `PSD parse error: ${msg}`;
    loading.value = false;
  }
}

onMounted(renderPsd);
watch(activeSide, renderPsd);
</script>

<template>
  <div class="psd-preview" :class="{ compact }">
    <div v-if="!compact" class="preview-controls">
      <div v-if="hasBoth" class="side-toggle">
        <button :class="{ active: activeSide === 'before' }" @click="activeSide = 'before'">Before</button>
        <button :class="{ active: activeSide === 'after' }" @click="activeSide = 'after'">After</button>
      </div>
      <span v-if="statusLabel" class="status-badge">{{ statusLabel }}</span>
    </div>

    <div v-if="loading" class="preview-loading">Loading PSD...</div>
    <div v-else-if="error" class="preview-fallback">{{ error }}</div>
    <div v-else class="canvas-container">
      <canvas ref="canvasRef" />
    </div>
  </div>
</template>

<style scoped>
.psd-preview {
  display: flex;
  flex-direction: column;
  width: 100%;
}
.psd-preview.compact {
  max-height: 200px;
}
.preview-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}
.side-toggle {
  display: flex;
}
.side-toggle button {
  padding: 2px 10px;
  border: 1px solid var(--border);
  background: var(--bg-secondary);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
}
.side-toggle button:first-child {
  border-radius: 4px 0 0 4px;
}
.side-toggle button:last-child {
  border-radius: 0 4px 4px 0;
  border-left: none;
}
.side-toggle button.active {
  background: var(--accent);
  color: var(--text-on-accent, #fff);
  border-color: var(--accent);
}
.status-badge {
  padding: 1px 6px;
  border-radius: 3px;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  font-size: 11px;
}
.canvas-container {
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 120px;
  background: var(--bg-primary);
}
.compact .canvas-container {
  max-height: 200px;
}
.canvas-container canvas {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}
.compact .canvas-container canvas {
  max-height: 200px;
}
.preview-loading,
.preview-fallback {
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
}
</style>
