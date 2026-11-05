<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../../i18n";
import BaseButton from "../../ui/BaseButton.vue";
import BaseSwitch from "../../ui/BaseSwitch.vue";
import SettingRow from "../SettingRow.vue";
import SettingsSection from "../SettingsSection.vue";
import { getCachedDebugMode, getDebugMode, setDebugMode } from "../../../services/permissions";
import { normalizeAppError } from "../../../services/errors";
import { toggleTauriDevtools } from "../../../services/tauriRuntime";
import { useNotificationStore } from "../../../stores/notification";

const notificationStore = useNotificationStore();
const initialDebugMode = getCachedDebugMode();
const debugEnabled = ref(initialDebugMode ?? false);
const debugReady = ref(initialDebugMode !== null);
const debugBusy = ref(false);
const devtoolsBusy = ref(false);

const debugStatusLabel = computed(() => {
  if (!debugReady.value) return t("common.loading");
  return debugEnabled.value
    ? t("settings.general.debugModeOn")
    : t("settings.general.debugModeOff");
});


onMounted(() => {
  void refreshDebugMode();
});

async function refreshDebugMode() {
  try {
    debugEnabled.value = await getDebugMode();
    debugReady.value = true;
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadDebugMode",
    });
    debugReady.value = true;
  }
}

async function toggleDebug() {
  if (!debugReady.value || debugBusy.value) return;
  debugBusy.value = true;
  const next = !debugEnabled.value;
  try {
    await setDebugMode(next);
    debugEnabled.value = next;
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleDebugMode",
    });
  } finally {
    debugBusy.value = false;
  }
}

async function toggleDevtools() {
  if (!debugEnabled.value || devtoolsBusy.value) return;
  devtoolsBusy.value = true;
  try {
    await toggleTauriDevtools();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "toggleDevtools",
    });
  } finally {
    devtoolsBusy.value = false;
  }
}
</script>

<template>
  <SettingsSection
    id="debugMode"
    :title="t('settings.general.debugMode')"
    :description="t('settings.general.debugModeDesc')"
  >
    <div class="debug-mode-controls">
      <label class="debug-toggle" :aria-busy="!debugReady">
        <BaseSwitch
          v-if="debugReady"
          :model-value="debugEnabled"
          :disabled="debugBusy"
          :aria-label="t('settings.general.debugMode')"
          @update:model-value="toggleDebug"
        />
        <span v-else class="debug-toggle-placeholder" aria-hidden="true" />
        <span class="debug-toggle-label">{{ debugStatusLabel }}</span>
      </label>
      <BaseButton
        v-if="debugEnabled"
        :disabled="devtoolsBusy"
        @click="toggleDevtools"
      >
        {{ t("settings.general.devtoolsToggle") }}
      </BaseButton>
    </div>
  </SettingsSection>

  <SettingsSection id="toolFailureLog">
    <SettingRow
      setting-key="system.toolFailureLog"
      :label="t('settings.general.toolFailureLog')"
      :description="t('settings.general.toolFailureLogDesc')"
    />
  </SettingsSection>
</template>

<style scoped>
.debug-mode-controls {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px 14px;
}
.debug-toggle {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  color: var(--text-color);
  user-select: none;
}
.debug-toggle-placeholder {
  flex-shrink: 0;
  width: 34px;
  height: 18px;
  border: 1px solid color-mix(in srgb, var(--border-strong) 82%, var(--text-secondary) 18%);
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--hover-bg) 24%);
  opacity: 0.55;
}
.debug-toggle-label {
  font-size: 13px;
}
</style>
