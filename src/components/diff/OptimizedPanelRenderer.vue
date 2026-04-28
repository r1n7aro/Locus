<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import UnityInspectorFieldTree from "./UnityInspectorFieldTree.vue";
import ParticleSystemPanelRenderer from "./ParticleSystemPanelRenderer.vue";
import { partitionFields } from "./rendererRegistry";
import type { InspectorPanel, InspectorField } from "../../types";
import type { ComponentRendererConfig } from "./rendererRegistry";

const props = defineProps<{
  panel: InspectorPanel;
  config: ComponentRendererConfig;
}>();

const useParticleSystemRenderer = computed(() =>
  props.panel.componentSource === "builtin" && props.panel.componentType === "ParticleSystem",
);

/* ── Empty-value filtering for optimized mode ── */

/** Check if a leaf value is noise (-1, null ref, empty) */
function isNullLikeValue(val: string | undefined): boolean {
  if (val == null || val === "") return true;
  const trimmed = val.trim();
  if (trimmed === "-1") return true;
  if (/^\(?\s*fileID:\s*0\s*\)?$/.test(trimmed)) return true;
  if (trimmed === "[]" || trimmed === "{}") return true;
  return false;
}

/** Check if a field (and all descendants) contain only empty values.
 *  Modified fields are never considered empty — a change to/from -1 is meaningful. */
function isFieldEmpty(field: InspectorField): boolean {
  if (field.changeKind === "modified") return false;
  if (field.children?.length) {
    return field.children.every(isFieldEmpty);
  }
  return isNullLikeValue(field.before) && isNullLikeValue(field.after);
}

/** Remove fields that are entirely empty, recursing into children */
function filterEmptyFields(fields: InspectorField[]): InspectorField[] {
  const result: InspectorField[] = [];
  for (const f of fields) {
    if (!f.children?.length) {
      if (!isFieldEmpty(f)) result.push(f);
      continue;
    }
    const filtered = filterEmptyFields(f.children);
    if (filtered.length > 0) {
      result.push({ ...f, children: filtered });
    }
  }
  return result;
}

/* ── Processed partition: unwrap + filter ── */

interface ProcessedSection {
  titleKey: string;
  fields: InspectorField[];
}

const partition = computed(() => {
  const raw = partitionFields(props.panel.fields, props.config);
  const flatFields = filterEmptyFields(raw.flatFields);

  const sections: ProcessedSection[] = [];
  for (const section of raw.sections) {
    // Unwrap: if the section maps to a single group field, show its children
    // directly under the section header to avoid redundant labels
    let fields = section.fields;
    if (fields.length === 1 && fields[0].children?.length) {
      fields = fields[0].children;
    }
    fields = filterEmptyFields(fields);
    if (fields.length > 0) {
      sections.push({ titleKey: section.titleKey, fields });
    }
  }

  const otherFields = filterEmptyFields(raw.otherFields);

  return { flatFields, sections, otherFields, hiddenCount: raw.hiddenCount };
});

const isEmpty = computed(() =>
  partition.value.flatFields.length === 0
  && partition.value.sections.length === 0
  && partition.value.otherFields.length === 0,
);
</script>

<template>
  <ParticleSystemPanelRenderer
    v-if="useParticleSystemRenderer"
    :panel="panel"
    :config="config"
  />
  <div v-else class="optimized-panel">
    <!-- Empty state: all fields were hidden -->
    <div v-if="isEmpty" class="optimized-empty">
      {{ t("diff.optimized.allFiltered") }}
    </div>

    <template v-else>
      <div v-if="partition.flatFields.length > 0" class="optimized-flat">
        <UnityInspectorFieldTree
          v-for="field in partition.flatFields"
          :key="field.id"
          :field="field"
        />
      </div>

      <template v-else>
        <!-- Named sections -->
        <div
          v-for="section in partition.sections"
          :key="section.titleKey"
          class="optimized-section"
        >
          <div class="section-header">{{ t(section.titleKey) }}</div>
          <div class="section-body">
            <UnityInspectorFieldTree
              v-for="field in section.fields"
              :key="field.id"
              :field="field"
            />
          </div>
        </div>

        <!-- Other Fields catch-all -->
        <div
          v-if="partition.otherFields.length > 0"
          class="optimized-section other-fields"
        >
          <div class="section-header">{{ t("diff.optimized.otherFields") }}</div>
          <div class="section-body">
            <UnityInspectorFieldTree
              v-for="field in partition.otherFields"
              :key="field.id"
              :field="field"
            />
          </div>
        </div>
      </template>
    </template>
  </div>
</template>

<style scoped>
.optimized-panel {
  display: flex;
  flex-direction: column;
}

.optimized-empty {
  padding: 10px 14px;
  color: var(--text-secondary);
  font-size: 12px;
}

.optimized-section {
  border-bottom: 1px solid var(--border-color);
}

.optimized-section:last-child {
  border-bottom: none;
}

.optimized-flat {
  display: flex;
  flex-direction: column;
}

.section-header {
  padding: 5px 14px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  background: var(--unity-field-section-bg, color-mix(in srgb, var(--sidebar-bg) 62%, var(--panel-bg) 38%));
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.section-body {
  /* inherits field tree styles */
}

.other-fields .section-header {
  color: var(--text-secondary);
  opacity: 0.7;
}
</style>
