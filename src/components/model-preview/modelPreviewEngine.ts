import type * as ThreeNamespace from "three";
import type { ModelFormat } from "./modelFormat";

export type ModelMaterialMode = "original" | "clay";

export interface ModelStats {
  meshes: number;
  vertices: number;
  triangles: number;
  materials: number;
  textures: number;
  bones: number;
  /** Axis-aligned bounding box size in model units. */
  size: [number, number, number];
}

export interface ModelClipInfo {
  name: string;
  duration: number;
}

export interface ModelPreviewOptions {
  /** Thumbnail mode: slow auto-rotate and no user interaction. */
  compact?: boolean;
}

export type ModelPreviewErrorCode = "unsupportedCompression" | "parse";

export class ModelPreviewError extends Error {
  constructor(readonly code: ModelPreviewErrorCode, message: string) {
    super(message);
    this.name = "ModelPreviewError";
  }
}

export interface ModelPreviewEngine {
  /** Resolves to `null` when a newer `load` (or `dispose`) replaced this one. */
  load(buffer: ArrayBuffer, format: ModelFormat): Promise<{ stats: ModelStats; clips: ModelClipInfo[] } | null>;
  setMaterialMode(mode: ModelMaterialMode): void;
  setWireframe(enabled: boolean): void;
  setGridVisible(visible: boolean): void;
  setPlaying(playing: boolean): void;
  selectClip(index: number): void;
  resetView(): void;
  /** Pauses rendering while the preview is not visible. */
  setActive(active: boolean): void;
  resize(): void;
  dispose(): void;
}

/** Texture slots an FBX material can reference besides the base color map. */
const FBX_AUXILIARY_TEXTURE_SLOTS = [
  "normalMap", "bumpMap", "displacementMap", "aoMap", "lightMap", "specularMap",
  "alphaMap", "envMap", "metalnessMap", "roughnessMap", "emissiveMap",
] as const;

const VIEW_DIRECTION: [number, number, number] = [1, 0.7, 1];
/** Poses sampled along the first clip to frame the model's whole range of motion. */
const CLIP_FRAMING_SAMPLES = 8;
const CLIP_FRAMING_MAX_VERTICES = 400_000;

function themeColor(token: string, fallback: string): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
  return value || fallback;
}

function whitePixelDataUrl(): string {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext("2d");
  if (!context) return "";
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, 1, 1);
  return canvas.toDataURL("image/png");
}

function materialsOf(mesh: ThreeNamespace.Mesh): ThreeNamespace.Material[] {
  return Array.isArray(mesh.material) ? mesh.material : [mesh.material];
}

export async function createModelPreviewEngine(
  container: HTMLElement,
  options: ModelPreviewOptions = {},
): Promise<ModelPreviewEngine> {
  const compact = !!options.compact;
  const [THREE, { OrbitControls }, { RoomEnvironment }] = await Promise.all([
    import("three"),
    import("three/addons/controls/OrbitControls.js"),
    import("three/addons/environments/RoomEnvironment.js"),
  ]);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.toneMapping = THREE.NeutralToneMapping;
  renderer.domElement.style.display = "block";
  container.appendChild(renderer.domElement);

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(themeColor("--panel-bg", "#101116"));
  const pmrem = new THREE.PMREMGenerator(renderer);
  const environmentTarget = pmrem.fromScene(new RoomEnvironment(), 0.04);
  scene.environment = environmentTarget.texture;
  scene.environmentIntensity = 0.75;
  pmrem.dispose();

  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 1000);
  // The key light travels with the camera so every orbit angle stays lit.
  const keyLight = new THREE.DirectionalLight(0xffffff, 1.4);
  keyLight.position.set(0.5, 1, 1);
  camera.add(keyLight);
  scene.add(camera);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x3a3d46, 0.5));

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.1;
  controls.enablePan = !compact;
  controls.enableRotate = !compact;
  controls.enableZoom = !compact;
  controls.autoRotate = compact;
  controls.autoRotateSpeed = 1.2;

  const clayMaterial = new THREE.MeshStandardMaterial({ color: 0xc9ccd1, roughness: 0.85, metalness: 0 });
  const blockedTextureUrl = whitePixelDataUrl();
  const manager = new THREE.LoadingManager();
  // Textures referenced by file name cannot be resolved from a bare buffer;
  // only embedded (blob/data) images are loaded.
  manager.setURLModifier((url) => (/^(?:blob:|data:)/i.test(url) ? url : blockedTextureUrl));

  let model: ThreeNamespace.Object3D | null = null;
  let skeletonHelper: ThreeNamespace.SkeletonHelper | null = null;
  let grid: ThreeNamespace.GridHelper | null = null;
  let mixer: ThreeNamespace.AnimationMixer | null = null;
  let clips: ThreeNamespace.AnimationClip[] = [];
  let currentAction: ThreeNamespace.AnimationAction | null = null;
  let materialMode: ModelMaterialMode = "original";
  let wireframe = false;
  let gridVisible = !compact;
  let playing = !compact;
  let active = true;
  let disposed = false;
  let loadToken = 0;
  let fitDistance = 1;
  let fitTarget = new THREE.Vector3();
  let raf = 0;
  let lastFrame = 0;
  const originalMaterials = new WeakMap<ThreeNamespace.Mesh, ThreeNamespace.Material | ThreeNamespace.Material[]>();

  function schedule(): void {
    if (raf || disposed || !active) return;
    raf = requestAnimationFrame(frame);
  }

  function frame(now: number): void {
    raf = 0;
    if (disposed) return;
    // A hidden tab collapses the container; the resize observer restarts the
    // loop once it is visible again.
    if (container.clientWidth < 1 || container.clientHeight < 1) {
      lastFrame = 0;
      return;
    }
    const delta = lastFrame ? Math.min((now - lastFrame) / 1000, 0.1) : 0;
    lastFrame = now;
    let again = false;
    if (mixer && playing && currentAction) {
      mixer.update(delta);
      again = true;
    }
    if (controls.update(delta || null)) again = true;
    renderer.render(scene, camera);
    if (again) schedule();
    else lastFrame = 0;
  }
  controls.addEventListener("change", schedule);
  renderer.domElement.addEventListener("webglcontextlost", (event) => event.preventDefault());
  renderer.domElement.addEventListener("webglcontextrestored", schedule);

  function disposeMaterial(material: ThreeNamespace.Material): void {
    for (const value of Object.values(material)) {
      if (value && (value as ThreeNamespace.Texture).isTexture) (value as ThreeNamespace.Texture).dispose();
    }
    material.dispose();
  }

  function disposeObject(root: ThreeNamespace.Object3D): void {
    root.traverse((child) => {
      const mesh = child as ThreeNamespace.Mesh;
      if (!mesh.isMesh) return;
      mesh.geometry.dispose();
      // The clay swap hides the loaded materials, which still need releasing.
      const loaded = originalMaterials.get(mesh) ?? mesh.material;
      for (const material of Array.isArray(loaded) ? loaded : [loaded]) {
        if (material !== clayMaterial) disposeMaterial(material);
      }
    });
  }

  function clearModel(): void {
    mixer?.stopAllAction();
    mixer = null;
    currentAction = null;
    clips = [];
    if (skeletonHelper) {
      scene.remove(skeletonHelper);
      skeletonHelper.dispose();
      skeletonHelper = null;
    }
    if (grid) {
      scene.remove(grid);
      grid.geometry.dispose();
      (grid.material as ThreeNamespace.Material).dispose();
      grid = null;
    }
    if (model) {
      scene.remove(model);
      disposeObject(model);
      model = null;
    }
  }

  function applyMaterials(): void {
    model?.traverse((child) => {
      const mesh = child as ThreeNamespace.Mesh;
      if (!mesh.isMesh) return;
      const original = originalMaterials.get(mesh);
      if (!original) return;
      mesh.material = materialMode === "clay" ? clayMaterial : original;
      for (const material of materialsOf(mesh)) {
        (material as ThreeNamespace.MeshStandardMaterial).wireframe = wireframe;
        material.needsUpdate = true;
      }
    });
  }

  function sanitizeFbxMaterials(root: ThreeNamespace.Object3D): void {
    root.traverse((child) => {
      const mesh = child as ThreeNamespace.Mesh;
      if (!mesh.isMesh) return;
      for (const material of materialsOf(mesh)) {
        const slots = material as unknown as Record<string, ThreeNamespace.Texture | null | undefined>;
        // Unresolved external textures were replaced by a white pixel, which is
        // only meaningful as a base color; auxiliary maps would distort shading.
        for (const slot of FBX_AUXILIARY_TEXTURE_SLOTS) {
          if (slots[slot]) slots[slot] = null;
        }
        material.needsUpdate = true;
      }
    });
  }

  async function parse(buffer: ArrayBuffer, format: ModelFormat): Promise<{ root: ThreeNamespace.Object3D; clips: ThreeNamespace.AnimationClip[] }> {
    if (format === "fbx") {
      const { FBXLoader } = await import("three/addons/loaders/FBXLoader.js");
      const root = new FBXLoader(manager).parse(buffer, "");
      sanitizeFbxMaterials(root);
      return { root, clips: root.animations };
    }
    const [{ GLTFLoader }, { MeshoptDecoder }] = await Promise.all([
      import("three/addons/loaders/GLTFLoader.js"),
      import("three/addons/libs/meshopt_decoder.module.js"),
    ]);
    const loader = new GLTFLoader(manager);
    loader.setMeshoptDecoder(MeshoptDecoder);
    const gltf = await loader.parseAsync(buffer, "");
    return { root: gltf.scene, clips: gltf.animations };
  }

  function collectStats(root: ThreeNamespace.Object3D, box: ThreeNamespace.Box3): ModelStats {
    const materials = new Set<ThreeNamespace.Material>();
    const textures = new Set<ThreeNamespace.Texture>();
    let meshes = 0;
    let vertices = 0;
    let triangles = 0;
    let bones = 0;
    root.traverse((child) => {
      if ((child as ThreeNamespace.Bone).isBone) bones += 1;
      const mesh = child as ThreeNamespace.Mesh;
      if (!mesh.isMesh) return;
      meshes += 1;
      const position = mesh.geometry.getAttribute("position");
      const index = mesh.geometry.getIndex();
      vertices += position?.count ?? 0;
      triangles += Math.floor((index?.count ?? position?.count ?? 0) / 3);
      for (const material of materialsOf(mesh)) {
        materials.add(material);
        for (const value of Object.values(material)) {
          if (value && (value as ThreeNamespace.Texture).isTexture) textures.add(value as ThreeNamespace.Texture);
        }
      }
    });
    const size = box.getSize(new THREE.Vector3());
    return { meshes, vertices, triangles, materials: materials.size, textures: textures.size, bones, size: [size.x, size.y, size.z] };
  }

  function frameModel(box: ThreeNamespace.Box3): void {
    const sphere = box.getBoundingSphere(new THREE.Sphere());
    const radius = Math.max(sphere.radius, 1e-3);
    const verticalFov = THREE.MathUtils.degToRad(camera.fov);
    const horizontalFov = 2 * Math.atan(Math.tan(verticalFov / 2) * camera.aspect);
    fitDistance = (radius / Math.sin(Math.min(verticalFov, horizontalFov) / 2)) * 1.1;
    fitTarget = sphere.center.clone();
    camera.near = Math.max(radius / 1000, 1e-4);
    camera.far = fitDistance + radius * 200;
    camera.updateProjectionMatrix();
    controls.minDistance = radius * 0.05;
    controls.maxDistance = fitDistance * 20;
    resetView();

    const maxDim = Math.max(...box.getSize(new THREE.Vector3()).toArray(), 1e-3);
    // Grid cells are a power of ten so one cell reads as a round unit (1 m, 10 cm).
    const cell = 10 ** Math.floor(Math.log10(maxDim / 2));
    const divisions = Math.min(60, Math.max(2, Math.round((maxDim * 3) / cell / 2) * 2));
    grid = new THREE.GridHelper(divisions * cell, divisions, 0x8a8f98, 0x8a8f98);
    const gridMaterial = grid.material as ThreeNamespace.LineBasicMaterial;
    gridMaterial.transparent = true;
    gridMaterial.opacity = 0.3;
    grid.position.set(sphere.center.x, box.min.y, sphere.center.z);
    grid.visible = gridVisible;
    scene.add(grid);
  }

  function selectClipAt(index: number): void {
    if (!mixer || !clips.length) return;
    const clip = clips[Math.min(Math.max(index, 0), clips.length - 1)];
    currentAction?.stop();
    currentAction = mixer.clipAction(clip);
    currentAction.reset().play();
    mixer.update(0);
    schedule();
  }

  function resetView(): void {
    const direction = new THREE.Vector3(...VIEW_DIRECTION).normalize();
    camera.position.copy(fitTarget).addScaledVector(direction, fitDistance);
    controls.target.copy(fitTarget);
    camera.lookAt(fitTarget);
    controls.update();
    schedule();
  }

  function resize(): void {
    const width = container.clientWidth;
    const height = container.clientHeight;
    if (width < 1 || height < 1) return;
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    schedule();
  }

  async function load(buffer: ArrayBuffer, format: ModelFormat) {
    const token = ++loadToken;
    const superseded = (): boolean => disposed || token !== loadToken;
    let parsed: Awaited<ReturnType<typeof parse>>;
    try {
      parsed = await parse(buffer, format);
    } catch (cause) {
      if (superseded()) return null;
      const message = cause instanceof Error ? cause.message : String(cause);
      if (/DRACO|KTX2|BASISU|compressed/i.test(message)) {
        throw new ModelPreviewError("unsupportedCompression", message);
      }
      throw new ModelPreviewError("parse", message);
    }
    if (superseded()) {
      disposeObject(parsed.root);
      return null;
    }

    clearModel();
    // Fit the camera against the real container size.
    resize();
    model = parsed.root;
    model.traverse((child) => {
      const mesh = child as ThreeNamespace.Mesh;
      if (mesh.isMesh) originalMaterials.set(mesh, mesh.material);
    });
    scene.add(model);

    let hasMesh = false;
    model.traverse((child) => { if ((child as ThreeNamespace.Mesh).isMesh) hasMesh = true; });
    if (!hasMesh) {
      // Animation-only assets carry no geometry; show the rig instead.
      skeletonHelper = new THREE.SkeletonHelper(model);
      scene.add(skeletonHelper);
    }
    scene.updateMatrixWorld(true);

    clips = parsed.clips;
    if (clips.length) {
      mixer = new THREE.AnimationMixer(model);
      selectClipAt(0);
      if (currentAction) currentAction.paused = !playing;
    }
    const { pose, framing } = measureBounds(skeletonHelper ?? model);
    const stats = collectStats(model, pose);
    frameModel(framing);
    applyMaterials();
    resize();
    schedule();
    return { stats, clips: clips.map((clip) => ({ name: clip.name, duration: clip.duration })) };
  }

  /**
   * `pose` is the model at the start of the active clip. `framing` also covers
   * the rest of the clip, so root motion does not walk the model out of view.
   */
  function measureBounds(target: ThreeNamespace.Object3D): { pose: ThreeNamespace.Box3; framing: ThreeNamespace.Box3 } {
    let vertices = 0;
    target.traverse((child) => {
      const mesh = child as ThreeNamespace.Mesh;
      if (mesh.isMesh) vertices += mesh.geometry.getAttribute("position")?.count ?? 0;
    });
    // Skinned bounds cost one pass over every vertex per sample.
    const sampleCount = mixer && currentAction && currentAction.getClip().duration > 0 && vertices <= CLIP_FRAMING_MAX_VERTICES
      ? CLIP_FRAMING_SAMPLES
      : 1;
    const framing = new THREE.Box3();
    let pose = new THREE.Box3();
    for (let index = 0; index < sampleCount; index += 1) {
      if (sampleCount > 1) mixer!.setTime((currentAction!.getClip().duration * index) / (sampleCount - 1));
      scene.updateMatrixWorld(true);
      const sampled = new THREE.Box3().setFromObject(target, true);
      if (index === 0) pose = sampled.clone();
      framing.union(sampled);
    }
    if (sampleCount > 1) {
      mixer!.setTime(0);
      scene.updateMatrixWorld(true);
    }
    for (const box of [pose, framing]) {
      if (box.isEmpty() || !Number.isFinite(box.min.x + box.max.x + box.min.y + box.max.y + box.min.z + box.max.z)) {
        box.set(new THREE.Vector3(-0.5, 0, -0.5), new THREE.Vector3(0.5, 1, 0.5));
      }
    }
    return { pose, framing };
  }

  resize();

  return {
    load,
    setMaterialMode(mode) {
      materialMode = mode;
      applyMaterials();
      schedule();
    },
    setWireframe(enabled) {
      wireframe = enabled;
      applyMaterials();
      schedule();
    },
    setGridVisible(visible) {
      gridVisible = visible;
      if (grid) grid.visible = visible;
      schedule();
    },
    setPlaying(next) {
      playing = next;
      if (currentAction) currentAction.paused = !next;
      schedule();
    },
    selectClip: selectClipAt,
    resetView,
    setActive(next) {
      active = next;
      if (!next) {
        if (raf) cancelAnimationFrame(raf);
        raf = 0;
        lastFrame = 0;
        return;
      }
      resize();
      schedule();
    },
    resize,
    dispose() {
      if (disposed) return;
      disposed = true;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      controls.removeEventListener("change", schedule);
      controls.dispose();
      clearModel();
      clayMaterial.dispose();
      environmentTarget.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      renderer.domElement.remove();
    },
  };
}
