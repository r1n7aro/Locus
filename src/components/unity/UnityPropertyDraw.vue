<script lang="ts">
import { defineComponent, type PropType } from "vue";
import type {
  InspectorProperty,
  InspectorPropertyCommit,
  InspectorPropertyDrawerInput,
} from "../../services/propertyTree";
import type { UnitySerializedPropertyCommitEvent } from "./unitySerializedValue";

function toUnityCommitEvent(commit: InspectorPropertyCommit): UnitySerializedPropertyCommitEvent {
  return {
    propertyPath: commit.propertyPath,
    value: commit.value,
    property: commit.snapshot as UnitySerializedPropertyCommitEvent["property"],
  };
}

export default defineComponent({
  name: "UnityPropertyDraw",
  props: {
    property: {
      type: Object as PropType<InspectorProperty>,
      required: true,
    },
    propertyDrawers: {
      type: null as unknown as PropType<InspectorPropertyDrawerInput>,
      default: undefined,
    },
    disabled: {
      type: Boolean,
      default: false,
    },
    readonly: {
      type: Boolean,
      default: false,
    },
    compact: {
      type: Boolean,
      default: false,
    },
    showLabel: {
      type: Boolean,
      default: true,
    },
  },
  emits: {
    commit: (_event: UnitySerializedPropertyCommitEvent) => true,
  },
  setup(props, { emit }) {
    return () =>
      props.property.draw({
        drawers: props.propertyDrawers,
        disabled: props.disabled,
        readonly: props.readonly,
        compact: props.compact,
        showLabel: props.showLabel,
        onCommit: (commit) => emit("commit", toUnityCommitEvent(commit)),
      });
  },
});
</script>

<style scoped>
:deep(.inspector-property-draw-group) {
  min-width: 0;
  display: grid;
  gap: 6px;
}

:deep(.inspector-property-draw-header),
:deep(.inspector-property-draw-row) {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(84px, 0.42fr) minmax(0, 1fr);
  align-items: center;
  gap: 8px;
}

:deep(.inspector-property-draw-group.compact > .inspector-property-draw-header),
:deep(.inspector-property-draw-row.compact) {
  grid-template-columns: minmax(72px, 0.34fr) minmax(0, 1fr);
}

:deep(.inspector-property-draw-label),
:deep(.inspector-property-draw-type),
:deep(.inspector-property-draw-value) {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

:deep(.inspector-property-draw-label),
:deep(.inspector-property-draw-type) {
  color: var(--text-secondary);
}

:deep(.inspector-property-draw-type),
:deep(.inspector-property-draw-value) {
  font-family: var(--font-mono-identifier);
}

:deep(.inspector-property-draw-type) {
  font-size: 11px;
}

:deep(.inspector-property-draw-value) {
  color: var(--text-color);
  font-size: 12px;
}

:deep(.inspector-property-draw-children) {
  min-width: 0;
  display: grid;
  gap: 6px;
  padding-left: 10px;
  border-left: 1px solid var(--border-color);
}
</style>
