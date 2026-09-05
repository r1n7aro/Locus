<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import {
  getAsyncTasksEnabled,
  getUnityMultiAgentEditorEnabled,
  setAsyncTasksEnabled,
  setUnityMultiAgentEditorEnabled,
} from "../../services/system";
import { useNotificationStore } from "../../stores/notification";
import BaseSwitch from "../ui/BaseSwitch.vue";

const notificationStore = useNotificationStore();
const asyncTasksEnabled = ref(true);
const asyncTasksReady = ref(false);
const asyncTasksBusy = ref(false);
const asyncTasksStatusLabel = computed(() =>
  t(asyncTasksEnabled.value ? "common.enabled" : "common.disabled"),
);
const unityMultiAgentEditorEnabled = ref(false);
const unityMultiAgentEditorReady = ref(false);
const unityMultiAgentEditorBusy = ref(false);
const unityMultiAgentEditorStatusLabel = computed(() =>
  t(unityMultiAgentEditorEnabled.value ? "common.enabled" : "common.disabled"),
);

async function loadAsyncTasksEnabled() {
  try {
    asyncTasksEnabled.value = await getAsyncTasksEnabled();
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "loadAsyncTasksEnabled",
    });
  } finally {
    asyncTasksReady.value = true;
  }
}

async function updateAsyncTasksEnabled(value: boolean) {
  if (!asyncTasksReady.value || asyncTasksBusy.value) return;
  const previous = asyncTasksEnabled.value;
  asyncTasksEnabled.value = value;
  asyncTasksBusy.value = true;
  try {
    await setAsyncTasksEnabled(value);
  } catch (error) {
    asyncTasksEnabled.value = previous;
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "setAsyncTasksEnabled",
      replaceOperation: true,
    });
  } finally {
    asyncTasksBusy.value = false;
  }
}

async function loadUnityMultiAgentEditorEnabled() {
  try {
    unityMultiAgentEditorEnabled.value = await getUnityMultiAgentEditorEnabled();
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "loadUnityMultiAgentEditorEnabled",
    });
  } finally {
    unityMultiAgentEditorReady.value = true;
  }
}

async function updateUnityMultiAgentEditorEnabled(value: boolean) {
  if (!unityMultiAgentEditorReady.value || unityMultiAgentEditorBusy.value) return;
  const previous = unityMultiAgentEditorEnabled.value;
  unityMultiAgentEditorEnabled.value = value;
  unityMultiAgentEditorBusy.value = true;
  try {
    await setUnityMultiAgentEditorEnabled(value);
  } catch (error) {
    unityMultiAgentEditorEnabled.value = previous;
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "setUnityMultiAgentEditorEnabled",
      replaceOperation: true,
    });
  } finally {
    unityMultiAgentEditorBusy.value = false;
  }
}

onMounted(() => {
  void loadAsyncTasksEnabled();
  void loadUnityMultiAgentEditorEnabled();
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.experimental.toolExecution") }}</div>
    <div
      class="experimental-list"
      :aria-busy="!asyncTasksReady || !unityMultiAgentEditorReady"
    >
      <div class="experimental-row">
        <div class="experimental-info">
          <span class="experimental-name">{{ t("settings.experimental.asyncTasks") }}</span>
          <span class="experimental-desc">{{ t("settings.experimental.asyncTasksDesc") }}</span>
        </div>
        <div class="experimental-control">
          <span class="experimental-status">{{ asyncTasksStatusLabel }}</span>
          <BaseSwitch
            v-if="asyncTasksReady"
            :model-value="asyncTasksEnabled"
            :disabled="asyncTasksBusy"
            :aria-label="t('settings.experimental.asyncTasks')"
            @update:model-value="updateAsyncTasksEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>
      <div class="experimental-row">
        <div class="experimental-info">
          <span class="experimental-name">{{ t("settings.experimental.unityMultiAgentEditor") }}</span>
          <span class="experimental-desc">{{ t("settings.experimental.unityMultiAgentEditorDesc") }}</span>
        </div>
        <div class="experimental-control">
          <span class="experimental-status">{{ unityMultiAgentEditorStatusLabel }}</span>
          <BaseSwitch
            v-if="unityMultiAgentEditorReady"
            :model-value="unityMultiAgentEditorEnabled"
            :disabled="unityMultiAgentEditorBusy"
            :aria-label="t('settings.experimental.unityMultiAgentEditor')"
            @update:model-value="updateUnityMultiAgentEditorEnabled"
          />
          <span v-else class="switch-placeholder" aria-hidden="true" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.experimental-list {
  max-width: 760px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  overflow: hidden;
}

.experimental-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 12px 16px;
}

.experimental-row + .experimental-row {
  border-top: 1px solid var(--border-color);
}

.experimental-info {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.experimental-name {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 500;
}

.experimental-desc,
.experimental-status {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.experimental-control {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}

.switch-placeholder {
  width: 32px;
  height: 18px;
}
</style>
