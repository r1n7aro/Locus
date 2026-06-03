<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from "vue";
import { refetchDiffByKey } from "../../services/diff";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../../composables/resizeObserver";
import type { BinaryPreview } from "../../types";

const props = defineProps<{
  preview: BinaryPreview;
  diffKey: string;
  mode?: "diff" | "neutral";
  compact?: boolean;
}>();

const containerRef = ref<HTMLDivElement | null>(null);
const activeSide = ref<"before" | "after">(props.preview.after ? "after" : "before");
const loading = ref(false);
const error = ref<string | null>(null);
const showGrid = ref(!props.compact);
const wireframe = ref(false);

const activeRef = computed(() =>
  activeSide.value === "before" ? props.preview.before : props.preview.after,
);
const hasBoth = computed(() => !!props.preview.before && !!props.preview.after);
const statusLabel = computed(() => {
  if (props.mode === "neutral") return null;
  if (hasBoth.value) return null;
  return props.preview.after ? "Added" : "Deleted";
});

// Singleton renderer (module-level to survive component re-mounts)
let sharedRenderer: any = null;

let scene: any = null;
let camera: any = null;
let controls: any = null;
let gridHelper: any = null;
let currentModel: any = null;
let animationId: number | null = null;
let containerResizeObserver: ResizeObserverHandle | null = null;

// Module-level: cache Three.js imports so subsequent loads skip the import phase
let threeCache: { THREE: any; FBXLoader: any; OrbitControls: any } | null = null;

async function ensureThree() {
  if (threeCache) return threeCache;
  const [THREE, { FBXLoader }, { OrbitControls }] = await Promise.all([
    import("three"),
    import("three/addons/loaders/FBXLoader.js"),
    import("three/addons/controls/OrbitControls.js"),
  ]);
  threeCache = { THREE, FBXLoader, OrbitControls };
  return threeCache;
}

async function loadModel() {
  const assetRef = activeRef.value;
  if (!assetRef || !containerRef.value) return;

  loading.value = true;
  error.value = null;

  try {
    const { THREE, FBXLoader, OrbitControls } = await ensureThree();

    const response = await fetch(assetRef.url);
    if (!response.ok) {
      await refetchDiffByKey(props.diffKey);
      error.value = "Failed to load FBX data";
      return;
    }

    const buffer = await response.arrayBuffer();

    // Setup scene
    if (!scene) {
      scene = new THREE.Scene();
      scene.background = new THREE.Color(themeColor("--panel-bg", "#101116"));
    }

    // Remove previous model
    if (currentModel) {
      scene.remove(currentModel);
      disposeObject(currentModel);
      currentModel = null;
    }

    // Grid
    disposeGridHelper();
    gridHelper = new THREE.GridHelper(10, 10, 0xcccccc, 0xe0e0e0);
    gridHelper.visible = showGrid.value;
    scene.add(gridHelper);

    // Lights
    if (scene.children.filter((c: any) => c.isLight).length === 0) {
      const ambient = new THREE.AmbientLight(0xffffff, 0.6);
      const directional = new THREE.DirectionalLight(0xffffff, 0.8);
      directional.position.set(5, 10, 7);
      scene.add(ambient, directional);
    }

    // Parse FBX
    const loader = new FBXLoader();
    const object = loader.parse(buffer, "");

    // Override materials to white model
    const overrideMat = new THREE.MeshStandardMaterial({
      color: 0xcccccc,
      wireframe: wireframe.value,
    });
    object.traverse((child: any) => {
      if (child.isMesh) {
        child.material = overrideMat;
      }
    });

    // Auto-center and fit
    const box = new THREE.Box3().setFromObject(object);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    const maxDim = Math.max(size.x, size.y, size.z);
    object.position.sub(center);

    // Scale grid to fit
    if (gridHelper) {
      const gridSize = maxDim * 2;
      disposeGridHelper();
      gridHelper = new THREE.GridHelper(gridSize, 10, 0xcccccc, 0xe0e0e0);
      gridHelper.visible = showGrid.value;
      scene.add(gridHelper);
    }

    scene.add(object);
    currentModel = object;

    // Camera
    const el = containerRef.value;
    const width = Math.max(el.clientWidth, 1);
    const height = Math.max(el.clientHeight, 1);
    const aspect = width / height;
    if (!camera) {
      camera = new THREE.PerspectiveCamera(50, aspect, 0.01, maxDim * 100);
    } else {
      camera.aspect = aspect;
      camera.updateProjectionMatrix();
    }
    camera.position.set(maxDim * 1.2, maxDim * 0.8, maxDim * 1.2);
    camera.lookAt(0, 0, 0);

    // Renderer (singleton)
    if (!sharedRenderer) {
      sharedRenderer = new THREE.WebGLRenderer({ antialias: true });
    }
    sharedRenderer.setPixelRatio(window.devicePixelRatio);
    if (!el.contains(sharedRenderer.domElement)) {
      el.appendChild(sharedRenderer.domElement);
    }
    resizeRendererToContainer();

    // Controls
    if (controls) controls.dispose();
    controls = new OrbitControls(camera, sharedRenderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.1;
    controls.enablePan = !props.compact;
    controls.enableRotate = !props.compact;
    controls.enableZoom = !props.compact;
    controls.autoRotate = !!props.compact;
    controls.autoRotateSpeed = 1.2;

    // Animate
    function animate() {
      animationId = requestAnimationFrame(animate);
      controls?.update();
      sharedRenderer?.render(scene, camera);
    }
    if (animationId !== null) cancelAnimationFrame(animationId);
    animate();
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    error.value = `FBX error: ${msg}`;
  } finally {
    loading.value = false;
  }
}

function resizeRendererToContainer() {
  const el = containerRef.value;
  if (!el || !sharedRenderer || !camera) return;

  const width = Math.max(el.clientWidth, 1);
  const height = Math.max(el.clientHeight, 1);

  sharedRenderer.setPixelRatio(window.devicePixelRatio);
  sharedRenderer.setSize(width, height, false);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}

function observeContainerSize() {
  containerResizeObserver?.disconnect();
  containerResizeObserver = null;

  if (!containerRef.value || typeof ResizeObserver === "undefined") return;

  containerResizeObserver = createAnimationFrameResizeObserver(() => {
    resizeRendererToContainer();
  });
  containerResizeObserver?.observe(containerRef.value);
}

function themeColor(token: string, fallback: string): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
  return value || fallback;
}

function disposeObject(obj: any) {
  obj.traverse((child: any) => {
    if (child.geometry) child.geometry.dispose();
    if (child.material) {
      if (Array.isArray(child.material)) {
        child.material.forEach((m: any) => m.dispose());
      } else {
        child.material.dispose();
      }
    }
  });
}

function disposeGridHelper() {
  if (!gridHelper) return;

  if (scene) scene.remove(gridHelper);
  if (gridHelper.geometry) gridHelper.geometry.dispose();
  if (gridHelper.material) {
    if (Array.isArray(gridHelper.material)) {
      gridHelper.material.forEach((material: any) => material?.dispose?.());
    } else {
      gridHelper.material.dispose();
    }
  }
  gridHelper = null;
}

function toggleGrid() {
  showGrid.value = !showGrid.value;
  if (gridHelper) gridHelper.visible = showGrid.value;
}

function toggleWireframe() {
  wireframe.value = !wireframe.value;
  if (currentModel) {
    currentModel.traverse((child: any) => {
      if (child.isMesh && child.material) {
        child.material.wireframe = wireframe.value;
      }
    });
  }
}

onMounted(() => {
  observeContainerSize();
  loadModel();
});
watch(activeSide, loadModel);

onBeforeUnmount(() => {
  containerResizeObserver?.disconnect();
  containerResizeObserver = null;
  if (animationId !== null) cancelAnimationFrame(animationId);
  if (controls) controls.dispose();
  disposeGridHelper();
  if (currentModel && scene) {
    scene.remove(currentModel);
    // Dynamic import is cached, so we can do a light cleanup
    try {
      const THREE = (globalThis as any).__THREE_CACHED__;
      if (THREE) disposeObject(currentModel);
    } catch { /* ignore */ }
    currentModel = null;
  }
  // Don't dispose sharedRenderer — it's reused across instances
  if (sharedRenderer?.domElement?.parentNode) {
    sharedRenderer.domElement.parentNode.removeChild(sharedRenderer.domElement);
  }
});
</script>

<template>
  <div class="fbx-preview" :class="{ compact }">
    <div v-if="!compact" class="preview-controls">
      <div v-if="hasBoth" class="side-toggle">
        <button :class="{ active: activeSide === 'before' }" @click="activeSide = 'before'">Before</button>
        <button :class="{ active: activeSide === 'after' }" @click="activeSide = 'after'">After</button>
      </div>
      <span v-if="statusLabel" class="status-badge">{{ statusLabel }}</span>
      <div class="toolbar">
        <button @click="toggleGrid" :class="{ active: showGrid }" title="Toggle grid">Grid</button>
        <button @click="toggleWireframe" :class="{ active: wireframe }" title="Toggle wireframe">Wire</button>
      </div>
    </div>

    <div v-if="loading" class="preview-loading">Loading FBX...</div>
    <div v-else-if="error" class="preview-fallback">{{ error }}</div>
    <div ref="containerRef" class="three-container" />
  </div>
</template>

<style scoped>
.fbx-preview {
  flex: 1;
  display: flex;
  flex-direction: column;
  width: 100%;
  min-height: 0;
  overflow: hidden;
}

.fbx-preview.compact {
  min-height: 96px;
}
.preview-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border-color, var(--border));
  background: var(--sidebar-bg, var(--bg-secondary));
  font-size: 12px;
  flex-shrink: 0;
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
.toolbar {
  display: flex;
  gap: 4px;
  margin-left: auto;
}
.toolbar button {
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
}
.toolbar button.active {
  background: var(--accent);
  color: var(--text-on-accent, #fff);
  border-color: var(--accent);
}
.three-container {
  flex: 1;
  width: 100%;
  min-height: 0;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
  overflow: hidden;
}

.fbx-preview.compact .three-container {
  min-height: 96px;
}
.three-container canvas {
  display: block;
}
.preview-loading,
.preview-fallback {
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
}
</style>
