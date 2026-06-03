<script setup lang="ts">
import { computed } from "vue";
import UnitySerializedPropertyTree from "../unity/UnitySerializedPropertyTree.vue";
import {
  createInspectorPropertyTreeBinding,
  type InspectorPropertyDrawerInput,
  type InspectorPropertySnapshot,
  type InspectorPropertyTreeBinding,
  type InspectorPropertyTreeBindingInput,
} from "../../services/propertyTree";
import type { UnitySerializedPropertyCommitEvent } from "../unity/unitySerializedValue";
import {
  normalizeUnityObjectPreviewModel,
  type UnityObjectPreviewInput,
  type UnityObjectPreviewModel,
} from "./unityObjectPreview";
import UnityObjectIdentity from "./UnityObjectIdentity.vue";

const props = withDefaults(defineProps<{
  model: UnityObjectPreviewInput | UnityObjectPreviewModel;
  disabled?: boolean;
  readonly?: boolean;
  compact?: boolean;
  showHeader?: boolean;
  propertyTree?: InspectorPropertyTreeBindingInput | null;
  propertyDrawers?: InspectorPropertyDrawerInput;
}>(), {
  disabled: false,
  readonly: false,
  compact: false,
  showHeader: true,
  propertyTree: null,
  propertyDrawers: undefined,
});

const emit = defineEmits<{
  commit: [event: UnitySerializedPropertyCommitEvent];
  preview: [event: UnitySerializedPropertyCommitEvent];
  blocked: [model: UnityObjectPreviewModel];
}>();

const objectModel = computed(() => normalizeUnityObjectPreviewModel(props.model));
const propertyTreeBinding = computed<InspectorPropertyTreeBinding>(() =>
  createInspectorPropertyTreeBinding({
    ...(props.propertyTree ?? {}),
    snapshots: props.propertyTree?.snapshots ?? objectModel.value.propertyTree ?? null,
    disabled: props.disabled || props.propertyTree?.disabled === true,
    readonly: props.readonly || props.propertyTree?.readonly === true,
    editable: props.propertyTree?.editable ?? objectModel.value.capabilities.edit,
  }),
);
const canEdit = computed(() =>
  propertyTreeBinding.value.editable &&
  !propertyTreeBinding.value.disabled &&
  !propertyTreeBinding.value.readonly,
);
const propertyTrees = computed(() =>
  Array.isArray(propertyTreeBinding.value.snapshots)
    ? propertyTreeBinding.value.snapshots
    : propertyTreeBinding.value.snapshots
      ? [propertyTreeBinding.value.snapshots]
      : [],
);
const editorStateLabel = computed(() =>
  propertyTreeBinding.value.error
    || (propertyTreeBinding.value.loading ? "Loading properties..." : objectModel.value.readonlyReason || "No editable properties"),
);

function sourceForProperty(property: InspectorPropertySnapshot): InspectorPropertyTreeBindingInput {
  const binding = propertyTreeBinding.value;
  return {
    id: binding.id,
    targetId: binding.targetId,
    snapshots: property,
    loading: binding.loading,
    error: binding.error,
    disabled: binding.disabled,
    readonly: binding.readonly,
    editable: binding.editable,
    commit: binding.commit,
  };
}

function handleCommit(event: UnitySerializedPropertyCommitEvent) {
  emit("commit", event);
}

function handlePreview(event: UnitySerializedPropertyCommitEvent) {
  emit("preview", event);
}
</script>

<template>
  <section class="unity-object-editor-panel" :class="{ compact }">
    <header v-if="showHeader" class="unity-object-editor-header">
      <UnityObjectIdentity
        :model="objectModel"
        mode="row"
        :draggable="false"
      />
    </header>

    <div class="unity-object-editor-body">
      <template v-if="propertyTrees.length">
        <UnitySerializedPropertyTree
          v-for="property in propertyTrees"
          :key="property.propertyPath"
          :source="sourceForProperty(property)"
          :disabled="propertyTreeBinding.disabled"
          :readonly="!canEdit"
          :compact="compact"
          :hide-root-object-header="propertyTrees.length === 1"
          :property-drawers="propertyDrawers"
          @preview="handlePreview"
          @commit="handleCommit"
        />
      </template>
      <div v-else class="unity-object-editor-state">
        {{ editorStateLabel }}
      </div>
    </div>
  </section>
</template>

<style scoped>
.unity-object-editor-panel {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow: hidden;
}

.unity-object-editor-header {
  flex-shrink: 0;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--bg-color) 14%);
}

.unity-object-editor-body {
  min-width: 0;
  min-height: 0;
  padding: 10px;
  overflow: auto;
}

.unity-object-editor-panel.compact .unity-object-editor-body {
  padding: 8px;
}

.unity-object-editor-state {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}
</style>
