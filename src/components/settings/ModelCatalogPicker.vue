<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import type { ModelCatalogModel, ModelCatalogProvider, ModelCatalogResponse } from "../../types";
import {
  FEATURED_CATALOG_PROVIDERS,
  catalogProviderEndpoint,
  isCatalogProviderConnectable,
  isListableCatalogModel,
  isListableCatalogProvider,
  isRedundantModelId,
  matchesModelSearch,
} from "../../services/modelCatalog";

const props = defineProps<{
  catalog: ModelCatalogResponse | null;
  loading?: boolean;
  refreshing?: boolean;
  /** Restrict rows to one catalog provider (editing an existing provider). */
  restrictProviderId?: string | null;
}>();

const emit = defineEmits<{
  select: [providerId: string, provider: ModelCatalogProvider, modelId: string, model: ModelCatalogModel];
  refresh: [];
  close: [];
}>();

const query = ref("");
const MAX_ROWS = 200;

interface PickerRow {
  providerId: string;
  provider: ModelCatalogProvider;
  modelId: string;
  model: ModelCatalogModel;
}

interface PickerGroup {
  providerId: string;
  provider: ModelCatalogProvider;
  connectable: boolean;
  rows: PickerRow[];
}

function sortedModelRows(providerId: string, provider: ModelCatalogProvider): PickerRow[] {
  return Object.entries(provider.models)
    .filter(([, model]) => isListableCatalogModel(model))
    .sort(([, a], [, b]) => (b.release_date ?? "").localeCompare(a.release_date ?? ""))
    .map(([modelId, model]) => ({ providerId, provider, modelId, model }));
}

const groups = computed<PickerGroup[]>(() => {
  const providers = props.catalog?.providers ?? {};
  const search = query.value.trim();

  // An explicit restriction (editing an existing provider) bypasses the
  // allowlist so already-configured providers keep their model list; open
  // browsing never surfaces relay/reseller gateways or providers Locus
  // cannot reach with just endpoint + key.
  const providerIds = props.restrictProviderId
    ? [props.restrictProviderId].filter((id) => providers[id])
    : Object.keys(providers).filter((id) => isListableCatalogProvider(id, providers[id]));

  const orderedIds = [...providerIds].sort((a, b) => {
    const fa = FEATURED_CATALOG_PROVIDERS.indexOf(a);
    const fb = FEATURED_CATALOG_PROVIDERS.indexOf(b);
    if (fa >= 0 || fb >= 0) {
      if (fa < 0) return 1;
      if (fb < 0) return -1;
      return fa - fb;
    }
    return a.localeCompare(b);
  });

  const result: PickerGroup[] = [];
  let rowBudget = MAX_ROWS;
  for (const providerId of orderedIds) {
    if (rowBudget <= 0) break;
    const provider = providers[providerId];
    // Without a query, keep the picker small: featured providers only
    // (unless restricted to a single provider).
    if (
      !search &&
      !props.restrictProviderId &&
      !FEATURED_CATALOG_PROVIDERS.includes(providerId)
    ) {
      continue;
    }

    let rows = sortedModelRows(providerId, provider);
    if (search) {
      const providerMatches = matchesModelSearch(search, [provider.name, providerId]);
      if (!providerMatches) {
        rows = rows.filter((row) =>
          matchesModelSearch(search, [row.model.name, row.modelId]),
        );
      }
    }
    if (rows.length === 0) continue;
    rows = rows.slice(0, rowBudget);
    rowBudget -= rows.length;
    result.push({
      providerId,
      provider,
      connectable: isCatalogProviderConnectable(providerId, provider),
      rows,
    });
  }
  return result;
});

const totalProviders = computed(
  () =>
    Object.entries(props.catalog?.providers ?? {}).filter(([id, provider]) =>
      isListableCatalogProvider(id, provider),
    ).length,
);

function formatContext(context: number): string {
  if (context >= 1_000_000) {
    const m = context / 1_000_000;
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`;
  }
  if (context >= 1_000) return `${Math.round(context / 1_000)}k`;
  return `${context}`;
}
</script>

<template>
  <div class="catalog-picker">
    <div class="catalog-toolbar">
      <input
        v-model="query"
        class="catalog-search"
        type="text"
        :placeholder="t('settings.catalog.searchPlaceholder')"
        @keydown.escape.stop="emit('close')"
      />
      <button
        class="catalog-refresh"
        type="button"
        :disabled="refreshing"
        :title="t('settings.catalog.refresh')"
        @click="emit('refresh')"
      >
        <svg viewBox="0 0 16 16" fill="currentColor" width="13" height="13" :class="{ spinning: refreshing }">
          <path d="M8 3a5 5 0 1 0 4.546 2.914.5.5 0 0 1 .908-.417A6 6 0 1 1 8 2v1z"/>
          <path d="M8 4.466V.534a.25.25 0 0 1 .41-.192l2.36 1.966c.12.1.12.284 0 .384L8.41 4.658A.25.25 0 0 1 8 4.466z"/>
        </svg>
      </button>
    </div>

    <div v-if="loading" class="catalog-status">{{ t("settings.catalog.loading") }}</div>
    <div v-else-if="!catalog" class="catalog-status">{{ t("settings.catalog.unavailable") }}</div>
    <template v-else>
      <div v-if="!query && !restrictProviderId" class="catalog-hint">
        {{ t("settings.catalog.searchHint", String(totalProviders)) }}
      </div>
      <div class="catalog-list">
        <div v-for="group in groups" :key="group.providerId" class="catalog-group">
          <div class="catalog-group-header">
            <span class="catalog-group-name">{{ group.provider.name }}</span>
            <span v-if="catalogProviderEndpoint(group.providerId, group.provider)" class="catalog-group-endpoint mono">
              {{ catalogProviderEndpoint(group.providerId, group.provider) }}
            </span>
            <span v-if="!group.connectable" class="catalog-badge warn">
              {{ t("settings.catalog.gatewayRequired") }}
            </span>
          </div>
          <button
            v-for="row in group.rows"
            :key="`${row.providerId}/${row.modelId}`"
            type="button"
            class="catalog-row"
            @click="emit('select', row.providerId, row.provider, row.modelId, row.model)"
          >
            <span class="catalog-row-name">{{ row.model.name }}</span>
            <span
              v-if="!isRedundantModelId(row.model.name, row.modelId)"
              class="catalog-row-id mono"
            >{{ row.modelId }}</span>
            <span class="catalog-row-badges">
              <span class="catalog-badge ctx">{{ formatContext(row.model.limit.context) }}</span>
              <span v-if="row.model.reasoning" class="catalog-badge" :title="t('settings.catalog.capReasoning')">R</span>
              <span
                v-if="row.model.attachment || row.model.modalities?.input?.includes('image')"
                class="catalog-badge"
                :title="t('settings.catalog.capVision')"
              >V</span>
            </span>
          </button>
        </div>
        <div v-if="groups.length === 0" class="catalog-status">
          {{ t("settings.catalog.noMatches") }}
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.catalog-picker {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
}

.catalog-toolbar {
  display: flex;
  gap: 6px;
  align-items: center;
}

.catalog-search {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.catalog-search:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.catalog-refresh {
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.15s ease, color 0.15s ease;
  padding: 0;
}

.catalog-refresh:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.catalog-refresh:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.spinning {
  animation: catalog-spin 0.9s linear infinite;
}

@keyframes catalog-spin {
  to { transform: rotate(360deg); }
}

.catalog-status {
  padding: 12px 4px;
  font-size: 12px;
  color: var(--text-secondary);
}

.catalog-hint {
  font-size: 11px;
  color: var(--text-secondary);
  padding: 0 2px;
}

.catalog-list {
  overflow-y: auto;
  max-height: 320px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 8px;
  background: var(--input-bg);
}

.catalog-group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.catalog-group-header {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 2px 6px;
  flex-wrap: wrap;
}

.catalog-group-name {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-color);
}

.catalog-group-endpoint {
  font-size: 10px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 60%;
}

.catalog-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 6px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  cursor: pointer;
  text-align: left;
  font: inherit;
  min-width: 0;
  transition: background 0.12s ease;
}

.catalog-row:hover {
  background: var(--hover-bg);
}

.catalog-row-name {
  font-size: 12px;
  flex-shrink: 0;
}

.catalog-row-id {
  font-size: 11px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.catalog-row-badges {
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

.mono {
  font-family: var(--font-mono-identifier);
}
</style>
