<script lang="ts">
import { defineComponent, h, onErrorCaptured, ref, watch, type PropType, type VNodeChild } from "vue";
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

function drawFallback(property: InspectorProperty): VNodeChild {
  return h(
    "div",
    {
      class: "unity-property-draw-error",
      "data-property-path": property.propertyPath,
    },
    [
      h("span", { class: "unity-property-draw-error-label" }, property.label),
      h(
        "span",
        { class: "unity-property-draw-error-message" },
        "Custom drawer failed; showing raw value: " + (property.displayValue || ""),
      ),
    ],
  );
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
    // Error boundary: a broken custom drawer (e.g. from a plugin) must never
    // take down the whole inspector tree.
    const drawFailed = ref(false);

    watch(
      () => props.property.propertyPath,
      () => {
        drawFailed.value = false;
      },
    );

    onErrorCaptured((error) => {
      console.warn(
        `[UnityPropertyDraw] custom drawer failed for ${props.property.propertyPath}:`,
        error,
      );
      drawFailed.value = true;
      return false;
    });

    return () => {
      if (drawFailed.value) return drawFallback(props.property);
      try {
        return props.property.draw({
          drawers: props.propertyDrawers,
          disabled: props.disabled,
          readonly: props.readonly,
          compact: props.compact,
          showLabel: props.showLabel,
          onCommit: (commit) => emit("commit", toUnityCommitEvent(commit)),
        });
      } catch (error) {
        console.warn(
          `[UnityPropertyDraw] custom drawer failed for ${props.property.propertyPath}:`,
          error,
        );
        drawFailed.value = true;
        return drawFallback(props.property);
      }
    };
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

:deep(.inspector-property-draw-label) {
  color: var(--text-color);
}

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

:deep(.unity-property-draw-error) {
  min-width: 0;
  display: grid;
  gap: 2px;
  padding: 4px 7px;
  border: 1px solid var(--status-danger-border, var(--border-color));
  border-radius: 6px;
  background: color-mix(in srgb, var(--status-danger-bg, transparent) 30%, transparent);
}

:deep(.unity-property-draw-error-label) {
  color: var(--text-color);
  font-size: 12px;
}

:deep(.unity-property-draw-error-message) {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--status-danger-fg, var(--text-secondary));
  font-size: 11px;
}
</style>
