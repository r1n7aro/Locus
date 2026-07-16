<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import BaseButton from "../ui/BaseButton.vue";
import ModelCatalogPicker from "./ModelCatalogPicker.vue";
import ProviderModelForm from "./ProviderModelForm.vue";
import { t } from "../../i18n";
import {
  addCatalogModelRow,
  formatContextLength,
  isListableCatalogModel,
  isRedundantModelId,
  matchesModelSearch,
  modelRowIdFromApiModel,
  newCustomProviderModel,
  providerModelMatchesCatalog,
} from "../../services/modelCatalog";
import type {
  CustomProvider,
  CustomProviderModel,
  ModelCatalogModel,
  ModelCatalogProvider,
  ModelCatalogResponse,
} from "../../types";

const props = defineProps<{
  /** Provider draft, mutated in place. */
  provider: CustomProvider;
  /** Catalog entry backing this provider, when it came from the catalog. */
  catalogProviderId: string | null;
  catalogProvider: ModelCatalogProvider | null;
  /** Full catalog, for the cross-provider preset browser on manual providers. */
  catalog: ModelCatalogResponse | null;
  catalogLoading?: boolean;
  catalogRefreshing?: boolean;
  saving?: boolean;
  testing?: boolean;
  /** Validation failed because no row has an API model id: mark the culprits. */
  highlightMissing?: boolean;
}>();

const emit = defineEmits<{
  test: [model: CustomProviderModel];
  refreshCatalog: [];
}>();

/** The browse panel opens at the bottom of the scroll area; bring it into
 *  view so the toggle visibly does something. */
function toggleBrowse() {
  showBrowse.value = !showBrowse.value;
  if (!showBrowse.value) return;
  void nextTick(() => {
    rootEl.value
      ?.querySelector(".browse-panel")
      ?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  });
}

const filter = ref("");
const expandedKey = ref<string | null>(null);
const showBrowse = ref(false);
const rootEl = ref<HTMLElement | null>(null);

const hasCatalog = computed(() => !!props.catalogProvider && !!props.catalogProviderId);

// A fresh manual draft starts with one blank row: open it so the model fields
// are immediately visible instead of hiding behind a collapsed "新模型" card.
{
  const first = props.provider.models[0];
  if (props.provider.models.length === 1 && first && !first.apiModel.trim()) {
    expandedKey.value = first.id || "row-0";
  }
}

interface CatalogEntry {
  modelId: string;
  model: ModelCatalogModel;
}

const catalogEntries = computed<CatalogEntry[]>(() => {
  if (!props.catalogProvider) return [];
  return Object.entries(props.catalogProvider.models)
    .filter(([, model]) => model.status !== "deprecated")
    .sort(([, a], [, b]) => (b.release_date ?? "").localeCompare(a.release_date ?? ""))
    .map(([modelId, model]) => ({ modelId, model }));
});

/** Catalog models offered for adding: current agent-capable models only
 *  (isListableCatalogModel), not yet configured, narrowed by the filter box.
 *  catalogEntries stays unfiltered so isManualRow keeps recognizing rows
 *  added from older catalog entries. */
const availableEntries = computed<CatalogEntry[]>(() => {
  const q = filter.value.trim();
  return catalogEntries.value.filter(({ modelId, model }) => {
    if (!isListableCatalogModel(model)) return false;
    if (props.provider.models.some((row) => providerModelMatchesCatalog(row, modelId))) {
      return false;
    }
    return !q || matchesModelSearch(q, [model.name, modelId]);
  });
});

function rowKey(model: CustomProviderModel, index: number): string {
  return model.id || `row-${index}`;
}

function isManualRow(model: CustomProviderModel): boolean {
  if (!hasCatalog.value) return false;
  return !catalogEntries.value.some((entry) => providerModelMatchesCatalog(model, entry.modelId));
}

/** Configured rows stay visible while filtering — hiding them reads as data
 *  loss — but non-matches dim so the matches pop. */
function rowMatchesFilter(model: CustomProviderModel): boolean {
  const q = filter.value.trim();
  if (!q) return true;
  return matchesModelSearch(q, [model.name, model.apiModel]);
}

function rowSummary(model: CustomProviderModel): string {
  const parts: string[] = [];
  if (model.contextLength) parts.push(`${formatContextLength(model.contextLength)} ctx`);
  if (model.reasoningParamFormat && model.reasoningParamFormat !== "none") parts.push("R");
  if (model.supportsVision) parts.push("V");
  return parts.join(" · ");
}

function scrollExpandedIntoView() {
  void nextTick(() => {
    rootEl.value
      ?.querySelector(".model-row.expanded")
      ?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  });
}

function toggleExpand(model: CustomProviderModel, index: number) {
  const key = rowKey(model, index);
  const opening = expandedKey.value !== key;
  expandedKey.value = opening ? key : null;
  if (opening) scrollExpandedIntoView();
}

function removeRow(index: number) {
  const model = props.provider.models[index];
  if (model && expandedKey.value === rowKey(model, index)) expandedKey.value = null;
  props.provider.models.splice(index, 1);
}

function addFromAvailable(entry: CatalogEntry) {
  if (!props.catalogProviderId) return;
  addCatalogModelRow(props.provider, props.catalogProviderId, entry.modelId, entry.model);
}

function addManualModel() {
  const model = newCustomProviderModel();
  props.provider.models.push(model);
  expandedKey.value = rowKey(model, props.provider.models.length - 1);
  showBrowse.value = false;
  scrollExpandedIntoView();
}

/** Give rows a stable id before a test so the composable can target them;
 *  keeps the expanded state pointing at the same row across the key change. */
function handleTestClick(model: CustomProviderModel, index: number) {
  if (!model.apiModel.trim()) return;
  if (!model.id) {
    const previousKey = rowKey(model, index);
    model.id = modelRowIdFromApiModel(model.apiModel);
    if (expandedKey.value === previousKey) expandedKey.value = rowKey(model, index);
  }
  emit("test", model);
}

/** Esc clears an active filter first; only an empty box lets it bubble up and
 *  close the dialog. */
function handleFilterEscape(e: KeyboardEvent) {
  if (!filter.value) return;
  filter.value = "";
  e.stopPropagation();
}

/** Cross-provider preset pick (manual providers): copies the model row only,
 *  never provider fields — the endpoint the user configured stays theirs. */
function handleBrowseSelect(
  providerId: string,
  _catalogProvider: ModelCatalogProvider,
  modelId: string,
  model: ModelCatalogModel,
) {
  addCatalogModelRow(props.provider, providerId, modelId, model);
}
</script>

<template>
  <div ref="rootEl" class="models-editor">
    <div class="models-header">
      <div class="models-title">
        <span class="models-label">{{ t("settings.custom.models") }}</span>
        <span v-if="provider.models.length > 0" class="models-count">
          {{ t("settings.custom.modelsAddedCount", String(provider.models.length)) }}
        </span>
      </div>
      <input
        v-if="hasCatalog"
        v-model="filter"
        class="models-filter"
        type="text"
        :disabled="saving"
        :placeholder="t('settings.custom.filterModels')"
        @keydown.escape="handleFilterEscape"
      />
      <div class="models-actions">
        <BaseButton
          v-if="!hasCatalog"
          size="sm"
          type="button"
          :disabled="saving"
          @click="toggleBrowse"
        >
          {{ showBrowse ? t("settings.custom.hideCatalog") : t("settings.custom.addFromCatalog") }}
        </BaseButton>
        <BaseButton size="sm" type="button" :disabled="saving" @click="addManualModel">
          {{ t("settings.custom.addManualModel") }}
        </BaseButton>
      </div>
    </div>

    <div class="models-scroll">
      <div v-if="provider.models.length > 0" class="model-rows">
        <div
          v-for="(model, index) in provider.models"
          :key="rowKey(model, index)"
          class="model-row"
          :class="{
            expanded: expandedKey === rowKey(model, index),
            dimmed: !rowMatchesFilter(model),
            missing: highlightMissing && !model.apiModel.trim(),
          }"
        >
          <div
            class="model-row-header"
            role="button"
            tabindex="0"
            @click="toggleExpand(model, index)"
            @keydown.enter.prevent="toggleExpand(model, index)"
          >
            <svg
              class="model-row-chevron"
              :class="{ open: expandedKey === rowKey(model, index) }"
              viewBox="0 0 16 16" fill="currentColor" width="11" height="11"
            >
              <path d="M5.97 4.47a.75.75 0 0 1 1.06 0l3 3a.75.75 0 0 1 0 1.06l-3 3a.75.75 0 1 1-1.06-1.06L8.44 8 5.97 5.53a.75.75 0 0 1 0-1.06z"/>
            </svg>
            <span class="model-row-name">
              {{ model.name || model.apiModel || t("settings.custom.newModel") }}
            </span>
            <!-- The expanded body already shows everything editable right
                 below; keep the open header down to a plain title bar. -->
            <template v-if="expandedKey !== rowKey(model, index)">
              <span v-if="!isRedundantModelId(model.name, model.apiModel)" class="model-row-id mono">
                {{ model.apiModel }}
              </span>
              <span v-if="isManualRow(model)" class="model-badge manual">
                {{ t("settings.custom.manualBadge") }}
              </span>
              <span class="model-row-summary mono">{{ rowSummary(model) }}</span>
            </template>
            <button
              class="model-row-remove"
              type="button"
              :disabled="saving"
              :aria-label="t('settings.custom.removeModel')"
              @click.stop="removeRow(index)"
            >
              <svg viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
                <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
              </svg>
            </button>
          </div>

          <div v-if="expandedKey === rowKey(model, index)" class="model-row-body">
            <ProviderModelForm :model="model" :api-format="provider.apiFormat" :saving="saving" />
            <div class="model-row-footer">
              <BaseButton
                size="sm"
                type="button"
                :disabled="saving || testing || !model.apiModel.trim()"
                @click="handleTestClick(model, index)"
              >
                {{ testing ? '...' : t("settings.custom.testModel") }}
              </BaseButton>
            </div>
          </div>
        </div>
      </div>
      <div v-else-if="hasCatalog" class="models-empty" :class="{ missing: highlightMissing }">
        {{ t("settings.custom.pickModelHint") }}
      </div>
      <div v-else class="models-empty" :class="{ missing: highlightMissing }">
        {{ t("settings.custom.noModels") }}
      </div>

      <template v-if="hasCatalog">
        <div class="available-divider">
          <span class="available-label">{{ t("settings.custom.availableModels") }}</span>
          <span class="available-count">{{ availableEntries.length }}</span>
          <span class="available-rule"></span>
        </div>
        <div class="available-list">
          <button
            v-for="entry in availableEntries"
            :key="entry.modelId"
            type="button"
            class="available-row"
            :disabled="saving"
            @click="addFromAvailable(entry)"
          >
            <svg class="available-add" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
              <path d="M8 2.75a.75.75 0 0 1 .75.75v3.75h3.75a.75.75 0 0 1 0 1.5H8.75v3.75a.75.75 0 0 1-1.5 0V8.75H3.5a.75.75 0 0 1 0-1.5h3.75V3.5A.75.75 0 0 1 8 2.75z"/>
            </svg>
            <span class="available-name">{{ entry.model.name }}</span>
            <span
              v-if="!isRedundantModelId(entry.model.name, entry.modelId)"
              class="available-id mono"
            >{{ entry.modelId }}</span>
            <span class="available-badges">
              <span class="catalog-badge ctx">{{ formatContextLength(entry.model.limit.context) }}</span>
              <span v-if="entry.model.reasoning" class="catalog-badge" :title="t('settings.catalog.capReasoning')">R</span>
              <span
                v-if="entry.model.attachment || entry.model.modalities?.input?.includes('image')"
                class="catalog-badge"
                :title="t('settings.catalog.capVision')"
              >V</span>
            </span>
          </button>
          <div v-if="availableEntries.length === 0" class="available-empty">
            {{ t("settings.catalog.noMatches") }}
          </div>
        </div>
      </template>

      <div v-if="!hasCatalog && showBrowse" class="browse-panel">
        <ModelCatalogPicker
          :catalog="catalog"
          :loading="catalogLoading"
          :refreshing="catalogRefreshing"
          :restrict-provider-id="null"
          @select="handleBrowseSelect"
          @refresh="emit('refreshCatalog')"
          @close="showBrowse = false"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.models-editor {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
  flex: 1;
  min-height: 0;
}

.models-header {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  flex-shrink: 0;
}

.models-title {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-shrink: 0;
}

.models-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.models-count {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
}

.models-filter {
  flex: 1;
  min-width: 120px;
  max-width: 240px;
  margin-left: auto;
  padding: 5px 9px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 12px;
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.models-filter:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.models-actions {
  display: flex;
  gap: 6px;
  margin-left: auto;
  flex-shrink: 0;
}

.models-filter + .models-actions {
  margin-left: 0;
}

/* One scroll area for configured cards and the catalog below them: expanding
 * a card grows the page naturally instead of fighting sibling sections. */
.models-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-right: 2px;
  padding-bottom: 8px;
}

.model-rows {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex-shrink: 0;
}

.model-row {
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow: hidden;
  background: var(--input-bg);
  flex-shrink: 0;
  transition: opacity 0.15s ease;
}

.model-row.expanded {
  border-color: var(--border-strong);
}

.model-row.dimmed {
  opacity: 0.45;
}

.model-row.missing {
  border-color: var(--status-danger-border);
}

.model-row-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  cursor: pointer;
  min-width: 0;
}

.model-row-header:hover {
  background: var(--hover-bg);
}

.model-row-chevron {
  color: var(--text-secondary);
  opacity: 0.7;
  flex-shrink: 0;
  transition: transform 0.15s ease;
}

.model-row-chevron.open {
  transform: rotate(90deg);
}

.model-row-name {
  font-size: 12px;
  font-weight: 600;
  flex-shrink: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 45%;
}

.model-row-id {
  font-size: 11px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.model-badge {
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  padding: 3px 5px;
  border-radius: 4px;
  flex-shrink: 0;
}

.model-badge.manual {
  background: var(--status-warn-bg, var(--hover-bg));
  color: var(--status-warn-fg, var(--text-secondary));
}

.model-row-summary {
  font-size: 10px;
  color: var(--text-secondary);
  flex-shrink: 0;
  margin-left: auto;
}

.model-row-remove {
  width: 22px;
  height: 22px;
  margin-left: auto;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  padding: 0;
}

.model-row-remove:hover:not(:disabled) {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.model-row-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 12px 10px 10px;
  border-top: 1px solid var(--border-color);
}

.model-row-footer {
  display: flex;
  justify-content: flex-end;
}

.models-empty {
  font-size: 12px;
  color: var(--text-secondary);
  padding: 10px 12px;
  border: 1px dashed var(--border-color);
  border-radius: 8px;
  flex-shrink: 0;
}

.models-empty.missing {
  border-color: var(--status-danger-border);
  color: var(--status-danger-fg);
}

.available-divider {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
  flex-shrink: 0;
  user-select: none;
}

.available-label {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  flex-shrink: 0;
}

.available-count {
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 8px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  flex-shrink: 0;
}

.available-rule {
  flex: 1;
  height: 1px;
  background: var(--border-color);
}

.available-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  flex-shrink: 0;
}

.available-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  cursor: pointer;
  text-align: left;
  font: inherit;
  min-width: 0;
  flex-shrink: 0;
  transition: background 0.12s ease;
}

.available-row:hover:not(:disabled) {
  background: var(--hover-bg);
}

.available-row:hover:not(:disabled) .available-add {
  color: var(--accent-color);
  opacity: 1;
}

.available-row:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.available-add {
  color: var(--text-secondary);
  opacity: 0.6;
  flex-shrink: 0;
}

.available-name {
  font-size: 12px;
  flex-shrink: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 45%;
}

.available-id {
  font-size: 11px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.available-badges {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  margin-left: auto;
}

.catalog-badge {
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  padding: 3px 5px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
}

.catalog-badge.ctx {
  min-width: 30px;
  text-align: center;
}

.catalog-badge.warn {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.available-empty {
  padding: 8px;
  font-size: 12px;
  color: var(--text-secondary);
}

.browse-panel {
  margin-top: 2px;
  flex-shrink: 0;
}

.mono {
  font-family: var(--font-mono-identifier);
}

@media (max-width: 860px) {
  .models-scroll {
    overflow: visible;
    flex: none;
    min-height: auto;
  }
}
</style>
