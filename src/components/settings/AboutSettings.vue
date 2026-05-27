<script setup lang="ts">
import { computed, onMounted } from "vue";
import { locale, t } from "../../i18n";
import { useAppUpdateStore } from "../../stores/appUpdate";
import type { AppUpdateChannel } from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented, { type SegmentedOption } from "../ui/BaseSegmented.vue";

const APP_NAME = "Locus";
const ORGANIZATION = "FarLocus";
const CONTACT_EMAIL = "open@farlocus.com";
const appUpdateStore = useAppUpdateStore();

const updateChannelOptions = computed<SegmentedOption[]>(() => [
  {
    value: "stable",
    label: t("app.update.channelStable"),
  },
  {
    value: "experimental",
    label: t("app.update.channelExperimental"),
  },
]);

const currentVersionChannelLabel = computed(() =>
  appUpdateStore.currentIsExperimental
    ? t("app.update.channelExperimental")
    : t("app.update.channelStable"),
);

const lastCheckedLabel = computed(() => {
  if (!appUpdateStore.lastCheckedAt) {
    return t("settings.about.neverChecked");
  }

  return new Date(appUpdateStore.lastCheckedAt).toLocaleString(
    locale.value === "zh" ? "zh-CN" : "en-US",
    {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    },
  );
});

onMounted(async () => {
  await appUpdateStore.ensureCurrentVersion();
});

async function checkForUpdates() {
  await appUpdateStore.checkForUpdates();
}

async function selectUpdateChannel(value: string) {
  appUpdateStore.setUpdateChannel(value as AppUpdateChannel);
  await appUpdateStore.checkForUpdates();
}
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.about.title") }}</div>
    <p class="section-desc">{{ t("settings.about.desc") }}</p>

    <div class="about-panel">
      <div class="about-header">
        <div class="about-name">{{ APP_NAME }}</div>
        <div class="about-subtitle">Unity Dev Agent</div>
      </div>

      <dl class="about-grid">
        <div class="about-row">
          <dt class="about-label">{{ t("settings.about.app") }}</dt>
          <dd class="about-value">{{ APP_NAME }}</dd>
        </div>
        <div class="about-row">
          <dt class="about-label">{{ t("settings.about.version") }}</dt>
          <dd class="about-value about-version-value">
            <span class="mono">v{{ appUpdateStore.currentVersion || "-" }}</span>
            <span class="about-release-channel">{{ currentVersionChannelLabel }}</span>
          </dd>
        </div>
        <div class="about-row">
          <dt class="about-label">{{ t("settings.about.organization") }}</dt>
          <dd class="about-value">{{ ORGANIZATION }}</dd>
        </div>
        <div class="about-row">
          <dt class="about-label">{{ t("settings.about.contact") }}</dt>
          <dd class="about-value mono">{{ CONTACT_EMAIL }}</dd>
        </div>
        <div class="about-row">
          <dt class="about-label">{{ t("settings.about.versionSource") }}</dt>
          <dd class="about-value">{{ appUpdateStore.sourceLabel }}</dd>
        </div>
        <div class="about-row about-row-actions">
          <dt class="about-label">{{ t("settings.about.updateChannel") }}</dt>
          <dd class="about-update-controls">
            <BaseSegmented
              :model-value="appUpdateStore.updateChannel"
              :options="updateChannelOptions"
              size="sm"
              @update:model-value="selectUpdateChannel"
            />
          </dd>
        </div>
        <div class="about-row about-row-actions">
          <dt class="about-label">{{ t("settings.about.lastChecked") }}</dt>
          <dd class="about-update-controls">
            <span class="about-value">{{ lastCheckedLabel }}</span>
            <BaseButton
              size="md"
              :disabled="appUpdateStore.checking"
              @click="checkForUpdates"
            >
              {{
                appUpdateStore.checking
                  ? t("settings.about.checkingUpdates")
                  : t("settings.about.checkUpdates")
              }}
            </BaseButton>
          </dd>
        </div>
      </dl>
    </div>
  </div>
</template>

<style scoped>
.about-panel {
  display: flex;
  flex-direction: column;
  gap: 16px;
  max-width: 720px;
  padding: 16px 18px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}

.about-header {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-color);
}

.about-name {
  font-size: 18px;
  font-weight: 700;
  color: var(--text-color);
}

.about-subtitle {
  font-size: 12px;
  color: var(--text-secondary);
}

.about-grid {
  display: grid;
  gap: 10px;
  margin: 0;
}

.about-row {
  display: grid;
  grid-template-columns: 84px minmax(0, 1fr);
  gap: 16px;
  align-items: baseline;
}

.about-row-actions {
  align-items: center;
}

.about-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.about-value {
  margin: 0;
  min-width: 0;
  font-size: 13px;
  color: var(--text-color);
}

.about-value .mono {
  font-family: var(--font-mono-identifier);
  word-break: break-all;
}

.about-version-value {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-wrap: wrap;
}

.about-release-channel {
  font-size: 12px;
  color: var(--text-secondary);
}

.about-update-controls {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  margin: 0;
  min-width: 0;
}
</style>
