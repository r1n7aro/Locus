<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { InspectorPanel } from "../../types";
import type { ComponentRendererConfig } from "./rendererRegistry";
import UnityInspectorFieldTree from "./UnityInspectorFieldTree.vue";
import {
  buildParticleSystemSemanticView,
  type ParticleGradientStop,
  type ParticleSemanticValue,
} from "./particleSystemSemantic";

const props = defineProps<{
  panel: InspectorPanel;
  config: ComponentRendererConfig;
}>();

const view = computed(() =>
  buildParticleSystemSemanticView(props.panel, props.config),
);

const isEmpty = computed(() =>
  view.value.sections.every(
    (section) =>
      section.summaryRows.length === 0 && section.otherFields.length === 0,
  ) && view.value.otherFields.length === 0,
);

const COLOR_RE =
  /^\{?\s*r:\s*([\d.+-]+),\s*g:\s*([\d.+-]+),\s*b:\s*([\d.+-]+),\s*a:\s*([\d.+-]+)\s*\}?$/;

function previewStyle(value: ParticleSemanticValue) {
  if (value.gradientStops?.length) {
    return { background: gradientCss(value.gradientStops) };
  }
  if (value.color) {
    return { background: colorCss(value.color) };
  }
  return undefined;
}

function hasPreview(value?: ParticleSemanticValue | null) {
  return !!(value?.color || value?.gradientStops?.length);
}

function colorCss(value: string): string {
  const match = COLOR_RE.exec(value.trim());
  if (!match) return "transparent";
  const [r, g, b, a] = match.slice(1).map((part) => Number.parseFloat(part ?? "0"));
  return `rgba(${Math.round(clamp01(r) * 255)}, ${Math.round(clamp01(g) * 255)}, ${Math.round(clamp01(b) * 255)}, ${clamp01(a)})`;
}

function gradientCss(stops: ParticleGradientStop[]): string {
  const normalized = stops
    .map((stop) => `${colorCss(stop.color)} ${Math.round(clamp01(stop.offset) * 100)}%`)
    .join(", ");
  return `linear-gradient(90deg, ${normalized})`;
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  if (value < 0) return 0;
  if (value > 1) return 1;
  return value;
}
</script>

<template>
  <div class="optimized-panel particle-system-panel">
    <div v-if="isEmpty" class="optimized-empty">
      {{ t("diff.optimized.allFiltered") }}
    </div>

    <template v-else>
      <div
        v-for="section in view.sections"
        :key="section.titleKey"
        class="optimized-section"
      >
        <div class="section-header">{{ t(section.titleKey) }}</div>

        <div v-if="section.summaryRows.length > 0" class="ps-summary-list">
          <div
            v-for="row in section.summaryRows"
            :key="row.id"
            class="ps-summary-row"
            :class="row.changeKind"
          >
            <span
              v-if="row.changeKind !== 'unchanged'"
              class="ps-change-bar"
              :class="row.changeKind"
            />
            <span class="ps-label">{{ row.label }}</span>

            <div class="ps-value-wrap">
              <template v-if="row.changeKind === 'removed' && row.before">
                <span class="ps-value before">
                  <span
                    v-if="hasPreview(row.before)"
                    class="ps-preview"
                    :style="previewStyle(row.before)"
                  />
                  <span class="ps-text">{{ row.before.text }}</span>
                </span>
              </template>

              <template v-else-if="row.changeKind === 'added' && row.after">
                <span class="ps-value after">
                  <span
                    v-if="hasPreview(row.after)"
                    class="ps-preview"
                    :style="previewStyle(row.after)"
                  />
                  <span class="ps-text">{{ row.after.text }}</span>
                </span>
              </template>

              <template
                v-else-if="
                  row.changeKind === 'modified' && row.before && row.after
                "
              >
                <span class="ps-value before">
                  <span
                    v-if="hasPreview(row.before)"
                    class="ps-preview"
                    :style="previewStyle(row.before)"
                  />
                  <span class="ps-text">{{ row.before.text }}</span>
                </span>
                <span class="ps-arrow">&rarr;</span>
                <span class="ps-value after">
                  <span
                    v-if="hasPreview(row.after)"
                    class="ps-preview"
                    :style="previewStyle(row.after)"
                  />
                  <span class="ps-text">{{ row.after.text }}</span>
                </span>
              </template>

              <template v-else-if="row.after || row.before">
                <span class="ps-value">
                  <span
                    v-if="hasPreview(row.after ?? row.before)"
                    class="ps-preview"
                    :style="previewStyle((row.after ?? row.before)!)"
                  />
                  <span class="ps-text">{{ (row.after ?? row.before)?.text }}</span>
                </span>
              </template>
            </div>
          </div>
        </div>

        <template v-if="section.otherFields.length > 0">
          <div
            v-if="section.summaryRows.length > 0"
            class="ps-subheader"
          >
            {{ t("diff.optimized.otherFields") }}
          </div>
          <UnityInspectorFieldTree
            v-for="field in section.otherFields"
            :key="field.id"
            :field="field"
          />
        </template>
      </div>

      <div
        v-if="view.otherFields.length > 0"
        class="optimized-section other-fields"
      >
        <div class="section-header">{{ t("diff.optimized.otherFields") }}</div>
        <UnityInspectorFieldTree
          v-for="field in view.otherFields"
          :key="field.id"
          :field="field"
        />
      </div>
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

.section-header {
  padding: 5px 14px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  background: var(--unity-field-section-bg, color-mix(in srgb, var(--sidebar-bg) 62%, var(--panel-bg) 38%));
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.ps-summary-list {
  display: flex;
  flex-direction: column;
}

.ps-summary-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 28px;
  padding: 4px 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 56%, transparent);
  background: var(--unity-field-row-bg, color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%));
}

.ps-summary-row:last-child {
  border-bottom: none;
}

.ps-change-bar {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
}

.ps-change-bar.added {
  background: var(--git-status-added);
}

.ps-change-bar.removed {
  background: var(--git-status-deleted);
}

.ps-change-bar.modified {
  background: var(--git-status-modified);
}

.ps-label {
  flex-shrink: 0;
  min-width: 112px;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.ps-value-wrap {
  min-width: 0;
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 8px;
  justify-content: flex-end;
  flex-wrap: wrap;
}

.ps-value {
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--text-color);
  font-size: 12px;
}

.ps-value.before {
  color: var(--text-secondary);
}

.ps-value.after {
  color: var(--text-color);
}

.ps-text {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ps-preview {
  width: 42px;
  height: 12px;
  border-radius: 4px;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.03);
  flex-shrink: 0;
}

.ps-arrow {
  color: var(--text-secondary);
  font-size: 11px;
  flex-shrink: 0;
}

.ps-subheader {
  padding: 5px 14px;
  font-size: 10px;
  font-weight: 600;
  color: var(--text-secondary);
  opacity: 0.72;
  background: color-mix(in srgb, var(--unity-field-section-bg, var(--sidebar-bg)) 65%, transparent);
  border-top: 1px solid color-mix(in srgb, var(--border-color) 42%, transparent);
}

.other-fields .section-header {
  opacity: 0.7;
}
</style>
