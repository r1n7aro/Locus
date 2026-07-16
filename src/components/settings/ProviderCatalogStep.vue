<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import type { ModelCatalogProvider, ModelCatalogResponse } from "../../types";
import {
  FEATURED_CATALOG_PROVIDERS,
  catalogPreferredConnection,
  compactModelSearch,
  isListableCatalogModel,
  isListableCatalogProvider,
  normalizeModelSearch,
} from "../../services/modelCatalog";

const props = defineProps<{
  catalog: ModelCatalogResponse | null;
  loading?: boolean;
  refreshing?: boolean;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  pickCatalog: [providerId: string, provider: ModelCatalogProvider];
  pickManual: [];
  refresh: [];
}>();

const query = ref("");

interface ProviderRow {
  id: string;
  provider: ModelCatalogProvider;
  endpoint: string;
  modelCount: number;
}

/** One pre-normalized haystack per provider so the search can match model
 *  names too (e.g. "kimi" → Moonshot) without re-normalizing 5k+ strings on
 *  every keystroke. */
const haystacks = computed(() => {
  const map = new Map<string, { norm: string; compact: string }>();
  for (const [id, provider] of Object.entries(props.catalog?.providers ?? {})) {
    if (!isListableCatalogProvider(id, provider)) continue;
    const parts: string[] = [provider.name, id];
    for (const [modelId, model] of Object.entries(provider.models)) {
      // Only searchable via models the picker would actually offer.
      if (!isListableCatalogModel(model)) continue;
      parts.push(modelId, model.name);
    }
    const norm = normalizeModelSearch(parts.join(" "));
    map.set(id, { norm, compact: norm.split(" ").join("") });
  }
  return map;
});

function providerOrder(a: string, b: string): number {
  const fa = FEATURED_CATALOG_PROVIDERS.indexOf(a);
  const fb = FEATURED_CATALOG_PROVIDERS.indexOf(b);
  if (fa >= 0 || fb >= 0) {
    if (fa < 0) return 1;
    if (fb < 0) return -1;
    return fa - fb;
  }
  return a.localeCompare(b);
}

const rows = computed<ProviderRow[]>(() => {
  const providers = props.catalog?.providers ?? {};
  const search = normalizeModelSearch(query.value);
  const compact = compactModelSearch(query.value);
  const ids = Object.keys(providers).sort(providerOrder);
  const result: ProviderRow[] = [];
  for (const id of ids) {
    const provider = providers[id];
    // Relay/reseller gateways and providers Locus cannot reach with just
    // endpoint + key never show up, searched or not.
    if (!isListableCatalogProvider(id, provider)) continue;
    if (!search && !FEATURED_CATALOG_PROVIDERS.includes(id)) continue;
    if (search) {
      const hay = haystacks.value.get(id);
      if (!hay || (!hay.norm.includes(search) && !hay.compact.includes(compact))) continue;
    }
    const modelCount = Object.values(provider.models).filter(isListableCatalogModel).length;
    if (modelCount === 0) continue;
    result.push({
      id,
      provider,
      // Show the endpoint the pick will actually configure (Anthropic base
      // when the vendor has one), not the raw catalog entry.
      endpoint: catalogPreferredConnection(id, provider).endpoint,
      modelCount,
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

/** Esc clears an active search first; only an empty box lets it bubble up and
 *  close the dialog. */
function handleSearchEscape(e: KeyboardEvent) {
  if (!query.value) return;
  query.value = "";
  e.stopPropagation();
}
</script>

<template>
  <div class="pick-step">
    <div class="pick-toolbar">
      <input
        v-model="query"
        class="pick-search"
        type="text"
        autofocus
        :disabled="disabled"
        :placeholder="t('settings.catalog.searchPlaceholder')"
        @keydown.escape="handleSearchEscape"
      />
      <button
        class="pick-refresh"
        type="button"
        :disabled="refreshing || disabled"
        :title="t('settings.catalog.refresh')"
        @click="emit('refresh')"
      >
        <svg viewBox="0 0 16 16" fill="currentColor" width="13" height="13" :class="{ spinning: refreshing }">
          <path d="M8 3a5 5 0 1 0 4.546 2.914.5.5 0 0 1 .908-.417A6 6 0 1 1 8 2v1z"/>
          <path d="M8 4.466V.534a.25.25 0 0 1 .41-.192l2.36 1.966c.12.1.12.284 0 .384L8.41 4.658A.25.25 0 0 1 8 4.466z"/>
        </svg>
      </button>
    </div>

    <button class="manual-card" type="button" :disabled="disabled" @click="emit('pickManual')">
      <span class="manual-card-icon">
        <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
          <path d="M8 2.75a.75.75 0 0 1 .75.75v3.75h3.75a.75.75 0 0 1 0 1.5H8.75v3.75a.75.75 0 0 1-1.5 0V8.75H3.5a.75.75 0 0 1 0-1.5h3.75V3.5A.75.75 0 0 1 8 2.75z"/>
        </svg>
      </span>
      <span class="manual-card-copy">
        <span class="manual-card-name">{{ t("settings.custom.manualProvider") }}</span>
        <span class="manual-card-desc">{{ t("settings.custom.manualProviderDesc") }}</span>
      </span>
      <svg class="pick-chevron" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
        <path d="M5.97 4.47a.75.75 0 0 1 1.06 0l3 3a.75.75 0 0 1 0 1.06l-3 3a.75.75 0 1 1-1.06-1.06L8.44 8 5.97 5.53a.75.75 0 0 1 0-1.06z"/>
      </svg>
    </button>

    <div v-if="loading" class="pick-status">{{ t("settings.catalog.loading") }}</div>
    <div v-else-if="!catalog" class="pick-status">{{ t("settings.catalog.unavailable") }}</div>
    <template v-else>
      <div class="pick-list">
        <button
          v-for="row in rows"
          :key="row.id"
          type="button"
          class="pick-row"
          :disabled="disabled"
          @click="emit('pickCatalog', row.id, row.provider)"
        >
          <span class="pick-row-main">
            <span class="pick-row-name">{{ row.provider.name }}</span>
            <span class="pick-row-endpoint mono">
              {{ row.endpoint || t("settings.custom.endpointMissing") }}
            </span>
          </span>
          <span class="pick-row-side">
            <span class="pick-badge">{{ t("settings.custom.modelCount", String(row.modelCount)) }}</span>
            <svg class="pick-chevron" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
              <path d="M5.97 4.47a.75.75 0 0 1 1.06 0l3 3a.75.75 0 0 1 0 1.06l-3 3a.75.75 0 1 1-1.06-1.06L8.44 8 5.97 5.53a.75.75 0 0 1 0-1.06z"/>
            </svg>
          </span>
        </button>
        <div v-if="rows.length === 0" class="pick-status">{{ t("settings.catalog.noMatches") }}</div>
      </div>
      <div v-if="!query" class="pick-hint">
        {{ t("settings.catalog.searchHint", String(totalProviders)) }}
      </div>
    </template>
  </div>
</template>

<style scoped>
.pick-step {
  display: flex;
  flex-direction: column;
  gap: 10px;
  flex: 1;
  min-height: 0;
}

.pick-toolbar {
  display: flex;
  gap: 6px;
  align-items: center;
  flex-shrink: 0;
}

.pick-search {
  flex: 1;
  min-width: 0;
  padding: 8px 12px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.pick-search:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.pick-refresh {
  width: 34px;
  height: 34px;
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

.pick-refresh:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.pick-refresh:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.spinning {
  animation: pick-spin 0.9s linear infinite;
}

@keyframes pick-spin {
  to { transform: rotate(360deg); }
}

.manual-card {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border: 1px dashed var(--border-color);
  border-radius: 8px;
  background: transparent;
  color: var(--text-color);
  cursor: pointer;
  text-align: left;
  font: inherit;
  flex-shrink: 0;
  transition: background 0.15s ease, border-color 0.15s ease;
}

.manual-card:hover:not(:disabled) {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

.manual-card:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.manual-card-icon {
  width: 26px;
  height: 26px;
  border-radius: 6px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.manual-card-copy {
  display: flex;
  flex-direction: column;
  gap: 1px;
  flex: 1;
  min-width: 0;
}

.manual-card-name {
  font-size: 12.5px;
  font-weight: 600;
}

.manual-card-desc {
  font-size: 11px;
  color: var(--text-secondary);
}

.pick-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 6px;
  background: var(--input-bg);
}

.pick-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 8px 10px;
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

.pick-row:hover:not(:disabled) {
  background: var(--hover-bg);
}

.pick-row:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.pick-row-main {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1;
}

.pick-row-name {
  font-size: 12.5px;
  font-weight: 600;
}

.pick-row-endpoint {
  font-size: 10.5px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pick-row-side {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.pick-badge {
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  white-space: nowrap;
}

.pick-chevron {
  color: var(--text-secondary);
  opacity: 0.6;
  flex-shrink: 0;
}

.pick-status {
  padding: 12px 4px;
  font-size: 12px;
  color: var(--text-secondary);
}

.pick-hint {
  font-size: 11px;
  color: var(--text-secondary);
  padding: 0 2px;
  flex-shrink: 0;
}

.mono {
  font-family: var(--font-mono-identifier);
}
</style>
