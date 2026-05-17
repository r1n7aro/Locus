<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../i18n";
import { getProxyStatus, saveProxyConfig } from "../../services/system";
import type {
  ManualProxyConfig,
  ProxyConfig,
  ProxyMode,
  ProxyRoute,
  ProxyRouteSource,
  ProxyStatus,
} from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented, { type SegmentedOption } from "../ui/BaseSegmented.vue";

const status = ref<ProxyStatus | null>(null);
const draftConfig = ref<ProxyConfig>(defaultProxyConfig());
const loading = ref(false);
const saving = ref(false);
const error = ref<string | null>(null);
const saveError = ref<string | null>(null);

function emptyManualConfig(): ManualProxyConfig {
  return {
    httpProxy: "",
    httpsProxy: "",
    allProxy: "",
    noProxy: "",
  };
}

function defaultProxyConfig(): ProxyConfig {
  return {
    mode: "auto",
    manual: emptyManualConfig(),
  };
}

function cloneManualConfig(config?: ManualProxyConfig | null): ManualProxyConfig {
  return {
    httpProxy: config?.httpProxy ?? "",
    httpsProxy: config?.httpsProxy ?? "",
    allProxy: config?.allProxy ?? "",
    noProxy: config?.noProxy ?? "",
  };
}

function cloneProxyConfig(config?: ProxyConfig | null): ProxyConfig {
  return {
    mode: config?.mode ?? "auto",
    manual: cloneManualConfig(config?.manual),
  };
}

function applyStatus(nextStatus: ProxyStatus) {
  status.value = nextStatus;
  draftConfig.value = cloneProxyConfig(nextStatus.config ?? {
    mode: nextStatus.mode,
    manual: emptyManualConfig(),
  });
}

async function loadProxyStatus() {
  loading.value = true;
  error.value = null;
  saveError.value = null;
  try {
    applyStatus(await getProxyStatus());
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function persistProxyConfig(config: ProxyConfig) {
  saving.value = true;
  saveError.value = null;
  try {
    applyStatus(await saveProxyConfig(config));
  } catch (err) {
    saveError.value = err instanceof Error ? err.message : String(err);
  } finally {
    saving.value = false;
  }
}

async function updateMode(value: string) {
  const mode = value as ProxyMode;
  if (mode === draftConfig.value.mode || saving.value) return;
  const nextConfig = {
    mode,
    manual: cloneManualConfig(draftConfig.value.manual),
  };
  draftConfig.value = nextConfig;
  await persistProxyConfig(nextConfig);
}

function proxyUrlFromManualConfig(config: ManualProxyConfig): string {
  if (config.allProxy.trim()) return config.allProxy;
  if (config.httpProxy.trim() && config.httpProxy === config.httpsProxy) return config.httpProxy;
  return config.httpsProxy.trim() || config.httpProxy.trim();
}

const manualProxyUrl = computed(() => proxyUrlFromManualConfig(draftConfig.value.manual));

function updateManualProxyUrl(event: Event) {
  const input = event.target as HTMLInputElement | null;
  draftConfig.value = {
    ...draftConfig.value,
    manual: {
      httpProxy: "",
      httpsProxy: "",
      allProxy: input?.value ?? "",
      noProxy: "",
    },
  };
}

async function saveManualConfig() {
  await persistProxyConfig(cloneProxyConfig(draftConfig.value));
}

onMounted(() => {
  void loadProxyStatus();
});

const modeOptions = computed<SegmentedOption[]>(() => [
  {
    value: "auto",
    label: t("settings.proxy.modeAuto"),
    hint: t("settings.proxy.modeAutoHint"),
  },
  {
    value: "manual",
    label: t("settings.proxy.modeManual"),
    hint: t("settings.proxy.modeManualHint"),
  },
  {
    value: "disabled",
    label: t("settings.proxy.modeDisabled"),
    hint: t("settings.proxy.modeDisabledHint"),
  },
]);

const modeHint = computed(() => {
  switch (draftConfig.value.mode) {
    case "manual":
      return t("settings.proxy.modeManualHint");
    case "disabled":
      return t("settings.proxy.modeDisabledHint");
    case "auto":
    default:
      return t("settings.proxy.modeAutoHint");
  }
});

const environmentEntries = computed(() => status.value?.environment ?? status.value?.manual ?? []);
const systemEntries = computed(() => {
  const system = status.value?.system;
  if (!system?.available) return [];

  const entries: Array<{ key: string; value: string }> = [];
  const pushText = (key: string, value?: string | null) => {
    const text = value?.trim();
    if (text) entries.push({ key, value: text });
  };
  const pushBool = (key: string, value?: boolean | null) => {
    if (value === null || value === undefined) return;
    entries.push({ key, value: value ? t("settings.proxy.yes") : t("settings.proxy.no") });
  };

  pushBool(t("settings.proxy.enabled"), system.enabled);
  pushBool(t("settings.proxy.autoDetect"), system.autoDetect);
  pushText(t("settings.proxy.autoConfigUrl"), system.autoConfigUrl);
  pushText(t("settings.proxy.proxyServer"), system.proxyServer);
  pushText(t("settings.proxy.proxyOverride"), system.proxyOverride);
  pushText(t("settings.proxy.httpProxy"), system.httpProxy);
  pushText(t("settings.proxy.httpsProxy"), system.httpsProxy);
  pushText(t("settings.proxy.socksProxy"), system.socksProxy);

  return entries;
});

const manualChanged = computed(() => {
  const saved = status.value?.config?.manual ?? emptyManualConfig();
  return proxyUrlFromManualConfig(saved) !== manualProxyUrl.value;
});

function routeSourceLabel(source: ProxyRouteSource): string {
  switch (source) {
    case "environment":
      return t("settings.proxy.routeEnvironment");
    case "manual":
      return t("settings.proxy.routeManual");
    case "system":
      return t("settings.proxy.routeSystem");
    case "direct":
      return t("settings.proxy.routeDirect");
  }
}

function routeIsUnconfigured(route: ProxyRoute): boolean {
  return !route.proxyUrl && draftConfig.value.mode !== "disabled";
}

function routeSourceText(route: ProxyRoute): string {
  if (routeIsUnconfigured(route)) return t("settings.proxy.routeUnconfigured");
  return routeSourceLabel(route.source);
}

function routeDetailText(route: ProxyRoute): string {
  if (route.proxyUrl) return route.proxyUrl;
  if (routeIsUnconfigured(route)) return t("settings.proxy.routeUnconfigured");
  return t("settings.proxy.routeDirect");
}
</script>

<template>
  <div class="settings-section proxy-settings">
    <div class="proxy-header">
      <div>
        <div class="section-label">{{ t("settings.proxy.title") }}</div>
        <p class="section-desc">{{ t("settings.proxy.desc") }}</p>
      </div>
      <BaseButton size="sm" :disabled="loading || saving" @click="loadProxyStatus">
        {{ loading ? t("common.loading") : t("settings.proxy.refresh") }}
      </BaseButton>
    </div>

    <div v-if="error" class="proxy-error">
      {{ t("settings.proxy.loadFailed", error) }}
    </div>

    <div v-else-if="status" class="proxy-body">
      <section class="proxy-block">
        <div class="proxy-block-title">{{ t("settings.proxy.mode") }}</div>
        <div class="proxy-mode-row">
          <BaseSegmented
            :model-value="draftConfig.mode"
            :options="modeOptions"
            size="sm"
            @update:model-value="updateMode"
          />
          <span class="proxy-mode-hint">{{ modeHint }}</span>
        </div>
      </section>

      <section v-if="draftConfig.mode === 'auto'" class="proxy-block">
        <div class="proxy-block-title">{{ t("settings.proxy.environmentConfig") }}</div>
        <dl v-if="environmentEntries.length" class="proxy-grid">
          <template v-for="entry in environmentEntries" :key="entry.key">
            <dt>{{ entry.key }}</dt>
            <dd>{{ entry.value }}</dd>
          </template>
        </dl>
        <div v-else class="proxy-empty">{{ t("settings.proxy.manualEmpty") }}</div>
      </section>

      <section v-if="draftConfig.mode === 'auto' && systemEntries.length" class="proxy-block">
        <div class="proxy-block-title">{{ t("settings.proxy.systemConfig") }}</div>
        <dl class="proxy-grid">
          <template v-for="entry in systemEntries" :key="entry.key">
            <dt>{{ entry.key }}</dt>
            <dd>{{ entry.value }}</dd>
          </template>
        </dl>
      </section>

      <section v-if="draftConfig.mode === 'manual'" class="proxy-block">
        <div class="proxy-block-title">{{ t("settings.proxy.manualConfig") }}</div>
        <div class="proxy-form">
          <label class="proxy-field">
            <span class="proxy-field-key">{{ t("settings.proxy.manualProxyUrl") }}</span>
            <input
              class="proxy-input"
              :value="manualProxyUrl"
              :placeholder="t('settings.proxy.manualProxyUrlPlaceholder')"
              :disabled="saving"
              spellcheck="false"
              @input="updateManualProxyUrl"
            />
          </label>
        </div>
        <div class="proxy-actions">
          <BaseButton size="sm" :disabled="saving || !manualChanged" @click="saveManualConfig">
            {{ saving ? t("common.loading") : t("common.save") }}
          </BaseButton>
        </div>
      </section>

      <div v-if="saveError" class="proxy-error">
        {{ t("settings.proxy.saveFailed", saveError) }}
      </div>

      <section class="proxy-block">
        <div class="proxy-block-title">{{ t("settings.proxy.routes") }}</div>
        <div class="proxy-routes">
          <div v-for="route in status.routes" :key="route.targetUrl" class="proxy-route">
            <div class="proxy-route-main">
              <span class="proxy-route-target">{{ route.targetLabel }}</span>
              <span class="proxy-route-source">{{ routeSourceText(route) }}</span>
            </div>
            <div class="proxy-route-url">{{ routeDetailText(route) }}</div>
          </div>
        </div>
      </section>
    </div>

    <div v-else class="proxy-empty">{{ t("common.loading") }}</div>
  </div>
</template>

<style scoped>
.proxy-settings {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.proxy-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.proxy-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.proxy-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.proxy-block-title {
  font-size: 12px;
  font-weight: 650;
  color: var(--text-secondary);
}

.proxy-mode-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 34px;
}

.proxy-mode-hint {
  min-width: 0;
  font-size: 12px;
  color: var(--text-secondary);
}

.proxy-grid {
  display: grid;
  grid-template-columns: minmax(120px, 180px) minmax(0, 1fr);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
  background: var(--panel-bg);
}

.proxy-grid dt,
.proxy-grid dd {
  min-width: 0;
  margin: 0;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
  font-size: 12px;
  line-height: 1.4;
}

.proxy-grid dt {
  color: var(--text-secondary);
  background: var(--sidebar-bg);
}

.proxy-grid dd {
  color: var(--text-color);
  overflow-wrap: anywhere;
}

.proxy-grid dt:nth-last-child(2),
.proxy-grid dd:last-child {
  border-bottom: none;
}

.proxy-form {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 8px;
}

.proxy-field {
  display: grid;
  grid-template-columns: minmax(120px, 180px) minmax(0, 1fr);
  align-items: center;
  min-height: 34px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
  background: var(--panel-bg);
}

.proxy-field-key {
  align-self: stretch;
  display: flex;
  align-items: center;
  padding: 0 10px;
  color: var(--text-secondary);
  background: var(--sidebar-bg);
  border-right: 1px solid var(--border-color);
  font-size: 12px;
}

.proxy-input {
  width: 100%;
  min-width: 0;
  height: 32px;
  padding: 0 10px;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
}

.proxy-input::placeholder {
  color: var(--text-tertiary);
}

.proxy-input:focus {
  box-shadow: inset 0 0 0 1px var(--accent-color);
}

.proxy-actions {
  display: flex;
  justify-content: flex-end;
}

.proxy-routes {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
  background: var(--panel-bg);
}

.proxy-route {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
}

.proxy-route:last-child {
  border-bottom: none;
}

.proxy-route-main {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  font-size: 12px;
}

.proxy-route-target {
  font-weight: 650;
  color: var(--text-color);
}

.proxy-route-source,
.proxy-route-url,
.proxy-empty,
.proxy-error {
  font-size: 12px;
  color: var(--text-secondary);
}

.proxy-route-url {
  overflow-wrap: anywhere;
}

.proxy-error {
  color: var(--status-danger-fg);
}
</style>
