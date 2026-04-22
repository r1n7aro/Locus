<script setup lang="ts">
import packageJson from "../../../package.json";
import { t } from "../../i18n";

const APP_NAME = "Locus";
const ORGANIZATION = "FarLocus";
const CONTACT_EMAIL = "open@farlocus.com";
const APP_VERSION = packageJson.version;
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
          <dd class="about-value mono">v{{ APP_VERSION }}</dd>
        </div>
        <div class="about-row">
          <dt class="about-label">{{ t("settings.about.organization") }}</dt>
          <dd class="about-value">{{ ORGANIZATION }}</dd>
        </div>
        <div class="about-row">
          <dt class="about-label">{{ t("settings.about.contact") }}</dt>
          <dd class="about-value mono">{{ CONTACT_EMAIL }}</dd>
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

.about-value.mono {
  font-family: var(--font-mono-identifier);
  word-break: break-all;
}
</style>
