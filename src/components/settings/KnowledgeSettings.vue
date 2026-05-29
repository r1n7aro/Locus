<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import {
  getDefaultSkillPackageNamespace,
  setDefaultSkillPackageNamespace,
} from "../../services/knowledge";
import { normalizeAppError } from "../../services/errors";
import { useNotificationStore } from "../../stores/notification";
import BaseButton from "../ui/BaseButton.vue";

const notificationStore = useNotificationStore();

const defaultSkillPackageNamespace = ref("");
const defaultSkillPackageNamespaceDraft = ref("");
const defaultSkillPackageNamespaceLoading = ref(false);
const defaultSkillPackageNamespaceSaving = ref(false);
const defaultSkillPackageNamespaceSaved = ref("");
const defaultSkillPackageNamespaceError = ref("");
let defaultSkillPackageNamespaceSaveTimer: ReturnType<typeof setTimeout> | null = null;

const skillPackageNamespaceChanged = computed(
  () => defaultSkillPackageNamespaceDraft.value.trim() !== defaultSkillPackageNamespace.value,
);

async function loadDefaultSkillPackageNamespace() {
  defaultSkillPackageNamespaceLoading.value = true;
  defaultSkillPackageNamespaceError.value = "";
  try {
    const namespace = await getDefaultSkillPackageNamespace();
    defaultSkillPackageNamespace.value = namespace;
    defaultSkillPackageNamespaceDraft.value = namespace;
  } catch (cause) {
    const err = normalizeAppError(cause);
    defaultSkillPackageNamespaceError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "getDefaultSkillPackageNamespace",
    });
  } finally {
    defaultSkillPackageNamespaceLoading.value = false;
  }
}

async function saveDefaultSkillPackageNamespace() {
  if (defaultSkillPackageNamespaceSaving.value) return;
  defaultSkillPackageNamespaceSaving.value = true;
  defaultSkillPackageNamespaceError.value = "";
  try {
    const namespace = await setDefaultSkillPackageNamespace(
      defaultSkillPackageNamespaceDraft.value,
    );
    defaultSkillPackageNamespace.value = namespace;
    defaultSkillPackageNamespaceDraft.value = namespace;
    defaultSkillPackageNamespaceSaved.value = t("settings.knowledge.defaultSkillPackageNamespaceSaved");
    if (defaultSkillPackageNamespaceSaveTimer) {
      clearTimeout(defaultSkillPackageNamespaceSaveTimer);
    }
    defaultSkillPackageNamespaceSaveTimer = setTimeout(() => {
      defaultSkillPackageNamespaceSaved.value = "";
      defaultSkillPackageNamespaceSaveTimer = null;
    }, 2000);
  } catch (cause) {
    const err = normalizeAppError(cause);
    defaultSkillPackageNamespaceError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "setDefaultSkillPackageNamespace",
    });
  } finally {
    defaultSkillPackageNamespaceSaving.value = false;
  }
}

onMounted(() => {
  void loadDefaultSkillPackageNamespace();
});

onUnmounted(() => {
  if (defaultSkillPackageNamespaceSaveTimer) {
    clearTimeout(defaultSkillPackageNamespaceSaveTimer);
  }
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.knowledge.defaultSkillPackageNamespace") }}</div>
    <p class="section-desc">{{ t("settings.knowledge.defaultSkillPackageNamespaceHint") }}</p>

    <div class="package-namespace-row">
      <input
        v-model="defaultSkillPackageNamespaceDraft"
        class="package-namespace-input"
        :placeholder="t('settings.knowledge.defaultSkillPackageNamespacePlaceholder')"
        :disabled="defaultSkillPackageNamespaceLoading || defaultSkillPackageNamespaceSaving"
        spellcheck="false"
        @keydown.enter.prevent="saveDefaultSkillPackageNamespace"
      />
      <BaseButton
        type="button"
        :disabled="
          defaultSkillPackageNamespaceLoading ||
          defaultSkillPackageNamespaceSaving ||
          !skillPackageNamespaceChanged
        "
        @click="saveDefaultSkillPackageNamespace"
      >
        {{ t("common.save") }}
      </BaseButton>
    </div>

    <div v-if="defaultSkillPackageNamespaceSaved" class="settings-success">
      {{ defaultSkillPackageNamespaceSaved }}
    </div>
    <div v-if="defaultSkillPackageNamespaceError" class="settings-error">
      {{ defaultSkillPackageNamespaceError }}
    </div>
  </div>
</template>

<style scoped>
.package-namespace-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: min(620px, 100%);
}

.package-namespace-input {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: var(--font-mono-editor);
  outline: none;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.package-namespace-input::placeholder {
  color: var(--text-tertiary);
}

.package-namespace-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.package-namespace-input:disabled {
  opacity: 0.6;
}

.settings-success,
.settings-error {
  margin-top: 8px;
  font-size: 12px;
}

.settings-success {
  color: var(--status-good-fg);
}

.settings-error {
  color: var(--status-danger-fg);
}

@media (max-width: 640px) {
  .package-namespace-row {
    flex-direction: column;
    align-items: stretch;
  }
}
</style>
