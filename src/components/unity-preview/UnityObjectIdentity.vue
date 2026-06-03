<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  normalizeUnityObjectPreviewModel,
  unityObjectPreviewAssetRef,
  type UnityObjectPreviewInput,
  type UnityObjectPreviewModel,
} from "./unityObjectPreview";
import {
  unityAssetIconClassForKind,
  unityAssetIconKindForPath,
  unityAssetIconNodeForKind,
} from "../icons/unityAssetIcons";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  armUnityReferencePointerDrag,
  startUnityReferenceHtmlDrag,
} from "../../composables/useUnityReferenceDragSource";
import { resolveRefGraphGuid, resolveRefGraphPath } from "../../services/refGraph";

const props = withDefaults(defineProps<{
  model: UnityObjectPreviewInput | UnityObjectPreviewModel;
  mode?: "inline" | "row";
  selected?: boolean;
  interactive?: boolean;
  highlightable?: boolean;
  draggable?: boolean;
  disabled?: boolean;
  showPath?: boolean;
  showEditState?: boolean;
}>(), {
  mode: "inline",
  selected: false,
  interactive: false,
  highlightable: false,
  draggable: true,
  disabled: false,
  showPath: false,
  showEditState: false,
});

const emit = defineEmits<{
  select: [model: UnityObjectPreviewModel];
}>();

const objectModel = computed(() => normalizeUnityObjectPreviewModel(props.model));
const resolvedGuid = ref("");
const resolvedPath = ref("");
let resolveToken = 0;

const displayModel = computed<UnityObjectPreviewModel>(() => {
  const model = objectModel.value;
  const nextGuid = model.ref.guid || resolvedGuid.value;
  const nextPath = model.ref.path || resolvedPath.value;
  if (!nextGuid && !nextPath) return model;

  const nextRef = {
    ...model.ref,
    guid: nextGuid || model.ref.guid,
    path: nextPath || model.ref.path,
  };
  const nextTitle = resolvedTitle(model, nextPath);
  const nextIconKind = model.ref.path
    ? model.iconKind
    : nextPath
      ? unityAssetIconKindForPath(nextPath, {
        isSceneObject: model.ref.kind === "sceneObject" || model.ref.kind === "subObject",
        isFolder: false,
        fallbackKind: "asset",
      })
      : model.iconKind;

  return {
    ...model,
    ref: nextRef,
    title: nextTitle,
    iconKind: nextIconKind,
  };
});
const iconNode = computed(() => unityAssetIconNodeForKind(displayModel.value.iconKind));
const iconClass = computed(() => unityAssetIconClassForKind(displayModel.value.iconKind));
const dragRef = computed(() => props.draggable ? unityObjectPreviewAssetRef(displayModel.value) : null);
const rootTag = computed(() => props.interactive ? "button" : "div");
const titleText = computed(() => {
  const parts = [
    displayModel.value.ref.guid ? `guid: ${displayModel.value.ref.guid}` : "",
    displayModel.value.ref.path,
    displayModel.value.readonlyReason,
  ].filter(Boolean);
  return parts.join("\n");
});

watch(
  () => [objectModel.value.ref.guid || "", objectModel.value.ref.path] as const,
  ([guid, path]) => {
    const token = ++resolveToken;
    resolvedGuid.value = guid;
    resolvedPath.value = path;

    if (!guid && /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i.test(path)) {
      void resolveRefGraphGuid(path)
        .then((nextGuid) => {
          if (token !== resolveToken) return;
          resolvedGuid.value = nextGuid || "";
        })
        .catch((error: unknown) => {
          if (token !== resolveToken) return;
          resolvedGuid.value = "";
          console.warn("[UnityObjectIdentity] failed to resolve asset guid:", error);
        });
      return;
    }

    if (guid && !path) {
      void resolveRefGraphPath(guid)
        .then((nextPath) => {
          if (token !== resolveToken) return;
          resolvedPath.value = nextPath || "";
        })
        .catch((error: unknown) => {
          if (token !== resolveToken) return;
          resolvedPath.value = "";
          console.warn("[UnityObjectIdentity] failed to resolve asset path:", error);
        });
    }
  },
  { immediate: true },
);

function resolvedTitle(model: UnityObjectPreviewModel, path: string): string {
  if (!path || model.ref.path) return model.title;
  const guidTitle = shortGuid(model.ref.guid);
  if (model.title && model.title !== guidTitle && model.title !== "Unity Object") return model.title;
  return basenameForTitle(path) || model.title;
}

function basenameForTitle(path: string): string {
  return path.trim().replace(/\\/g, "/").split("/").filter(Boolean).pop() ?? "";
}

function shortGuid(guid: string | undefined): string {
  const normalized = (guid || "").trim();
  return normalized.length > 10 ? normalized.slice(0, 8) : normalized;
}

function handleSelect() {
  if (!props.interactive || props.disabled) return;
  emit("select", displayModel.value);
}

function handleDragStart(event: DragEvent) {
  if (!dragRef.value) return;
  startUnityReferenceHtmlDrag(event, [dragRef.value]);
}

function handlePointerDown(event: PointerEvent) {
  if (!dragRef.value) return;
  armUnityReferencePointerDrag(event, [dragRef.value]);
}
</script>

<template>
  <component
    :is="rootTag"
    class="unity-object-identity"
    :class="[
      `mode-${mode}`,
      {
        selected,
        interactive,
        highlightable,
        disabled,
        readonly: showEditState && !displayModel.capabilities.edit,
      },
    ]"
    :type="interactive ? 'button' : undefined"
    :disabled="interactive ? disabled : undefined"
    :aria-disabled="disabled || undefined"
    :title="titleText || undefined"
    :draggable="!!dragRef"
    :data-unity-ref-kind="displayModel.ref.kind"
    :data-unity-ref-path="displayModel.ref.path"
    :data-unity-guid="displayModel.ref.guid || undefined"
    @click="handleSelect"
    @pointerdown="handlePointerDown"
    @dragstart="handleDragStart"
  >
    <LucideIcon
      class="unity-object-identity-icon"
      :class="iconClass"
      :icon="iconNode"
      :size="mode === 'inline' ? 14 : 16"
    />
    <span class="unity-object-identity-main">
      <span class="unity-object-identity-title">{{ displayModel.title }}</span>
      <span
        v-if="mode === 'row' && (showPath ? displayModel.ref.path : displayModel.subtitle)"
        class="unity-object-identity-subtitle"
      >
        {{ showPath ? displayModel.ref.path : displayModel.subtitle }}
      </span>
    </span>
    <span
      v-if="showEditState && !displayModel.capabilities.edit"
      class="unity-object-identity-state"
    >
      {{ displayModel.readonlyReason || "Read only" }}
    </span>
  </component>
</template>

<style scoped>
.unity-object-identity {
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 0;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  box-shadow: none;
}

.unity-object-identity.mode-inline {
  max-width: 300px;
  vertical-align: baseline;
}

.unity-object-identity.mode-row {
  width: 100%;
  min-height: 30px;
  padding: 4px 8px;
  gap: 8px;
}

.unity-object-identity.interactive {
  cursor: pointer;
}

.unity-object-identity.highlightable {
  cursor: grab;
}

.unity-object-identity.interactive:hover,
.unity-object-identity.highlightable:hover {
  background: var(--hover-bg);
}

.unity-object-identity.selected {
  background: var(--active-bg);
}

.unity-object-identity.disabled {
  opacity: 0.58;
  cursor: default;
}

.unity-object-identity:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.unity-object-identity-icon {
  flex-shrink: 0;
}

.unity-object-identity-main {
  min-width: 0;
  display: inline-flex;
  flex-direction: column;
  gap: 1px;
}

.mode-inline .unity-object-identity-main {
  display: inline-block;
}

.unity-object-identity-title,
.unity-object-identity-subtitle {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-object-identity-title {
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  font-weight: 500;
}

.mode-inline .unity-object-identity-title {
  font-size: 13px;
}

.unity-object-identity-subtitle {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.unity-object-identity-state {
  flex-shrink: 0;
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 1px 6px;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--hover-bg) 22%);
  color: var(--text-secondary);
  font-size: 11px;
}
</style>
