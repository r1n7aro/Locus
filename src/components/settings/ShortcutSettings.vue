<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import {
  detectShortcutPlatform,
  formatShortcut,
  formatShortcutParts,
  parseShortcutEvent,
  useKeyboardShortcuts,
  type ShortcutAction,
} from "../../composables/useKeyboardShortcuts";
import {
  getChatSubmitModifierLabel,
  useChatInputSettings,
  type ChatSubmitMode,
  type RunningSendMode,
} from "../../composables/useChatInputSettings";

const { state, setShortcut, resetShortcut } = useKeyboardShortcuts();
const { state: chatInputSettings, setSubmitMode, setRunningSendMode } = useChatInputSettings();

const recordingAction = ref<ShortcutAction | null>(null);
const captureError = ref("");
const platform = detectShortcutPlatform();
const submitModifierLabel = getChatSubmitModifierLabel(platform);

const submitModeOptions = computed(() => [
  {
    value: "enter-send",
    label: t("settings.shortcuts.sendModeEnterSend"),
    hint: t("settings.shortcuts.sendModeEnterSendHint", submitModifierLabel),
  },
  {
    value: "mod-enter-send",
    label: t("settings.shortcuts.sendModeModifierSend", submitModifierLabel),
    hint: t("settings.shortcuts.sendModeModifierSendHint"),
  },
]);

const runningSendModeOptions = computed(() => [
  {
    value: "after-run",
    label: t("settings.shortcuts.runningSendAfterRun"),
    hint: t("settings.shortcuts.runningSendAfterRunHint"),
  },
  {
    value: "insert",
    label: t("settings.shortcuts.runningSendInsert"),
    hint: t("settings.shortcuts.runningSendInsertHint"),
  },
]);

const shortcutRows = computed(() => [
  {
    action: "newChat" as const,
    title: t("settings.shortcuts.newChatTitle"),
    desc: t("settings.shortcuts.newChatDesc"),
    parts: formatShortcutParts(state.newChat),
    titleText: formatShortcut(state.newChat),
  },
]);

function isRecording(action: ShortcutAction): boolean {
  return recordingAction.value === action;
}

function startRecording(action: ShortcutAction) {
  recordingAction.value = action;
  captureError.value = "";
}

function stopRecording() {
  recordingAction.value = null;
  captureError.value = "";
}

function handleRecordKeydown(action: ShortcutAction, event: KeyboardEvent) {
  if (!isRecording(action)) return;

  if (event.key === "Escape") {
    event.preventDefault();
    stopRecording();
    return;
  }

  event.preventDefault();
  event.stopPropagation();

  const shortcut = parseShortcutEvent(event);
  if (!shortcut) {
    captureError.value = t("settings.shortcuts.requireModifier");
    return;
  }

  setShortcut(action, shortcut);
  captureError.value = "";
  recordingAction.value = null;
}

function handleReset(action: ShortcutAction) {
  resetShortcut(action);
  if (isRecording(action)) {
    stopRecording();
  }
}
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.shortcuts.title") }}</div>
    <p class="section-desc">{{ t("settings.shortcuts.desc") }}</p>

    <div class="shortcut-mode-row">
      <div class="shortcut-main">
        <div class="shortcut-title">{{ t("settings.shortcuts.sendModeTitle") }}</div>
        <div class="shortcut-desc">{{ t("settings.shortcuts.sendModeDesc", submitModifierLabel) }}</div>
      </div>

      <div class="shortcut-mode-control">
        <BaseDropdown
          :model-value="chatInputSettings.submitMode"
          :options="submitModeOptions"
          :aria-label="t('settings.shortcuts.sendModeTitle')"
          size="md"
          @update:model-value="setSubmitMode($event as ChatSubmitMode)"
        />
      </div>
    </div>

    <div class="shortcut-mode-row">
      <div class="shortcut-main">
        <div class="shortcut-title">{{ t("settings.shortcuts.runningSendTitle") }}</div>
        <div class="shortcut-desc">{{ t("settings.shortcuts.runningSendDesc") }}</div>
      </div>

      <div class="shortcut-mode-control">
        <BaseDropdown
          :model-value="chatInputSettings.runningSendMode"
          :options="runningSendModeOptions"
          :aria-label="t('settings.shortcuts.runningSendTitle')"
          size="md"
          @update:model-value="setRunningSendMode($event as RunningSendMode)"
        />
      </div>
    </div>

    <div class="shortcut-list">
      <div
        v-for="row in shortcutRows"
        :key="row.action"
        class="shortcut-row"
      >
        <div class="shortcut-main">
          <div class="shortcut-title">{{ row.title }}</div>
          <div class="shortcut-desc">{{ row.desc }}</div>
        </div>

        <div class="shortcut-actions">
          <div class="shortcut-keys" :title="row.titleText">
            <kbd
              v-for="part in row.parts"
              :key="part"
              class="shortcut-key"
            >{{ part }}</kbd>
          </div>

          <BaseButton
            class="shortcut-btn"
            type="button"
            @click="startRecording(row.action)"
            @keydown="handleRecordKeydown(row.action, $event)"
            @blur="isRecording(row.action) ? stopRecording() : undefined"
          >
            {{ isRecording(row.action) ? t("settings.shortcuts.recording") : t("settings.shortcuts.record") }}
          </BaseButton>

          <BaseButton
            class="shortcut-btn"
            type="button"
            @click="handleReset(row.action)"
          >
            {{ t("settings.shortcuts.reset") }}
          </BaseButton>
        </div>
      </div>
    </div>

    <p class="shortcut-help">
      {{ t("settings.shortcuts.captureHint") }}
      <span v-if="captureError" class="shortcut-error">{{ captureError }}</span>
    </p>
  </div>
</template>

<style scoped>
.shortcut-list {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
  overflow: hidden;
}

.shortcut-mode-row,
.shortcut-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 14px 16px;
}

.shortcut-mode-row {
  margin-bottom: 12px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
}

.shortcut-row + .shortcut-row {
  border-top: 1px solid var(--border-color);
}

.shortcut-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.shortcut-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.shortcut-desc {
  font-size: 12px;
  color: var(--text-secondary);
}

.shortcut-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  flex-wrap: wrap;
}

.shortcut-mode-control {
  width: min(320px, 100%);
  flex-shrink: 0;
}

.shortcut-keys {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 132px;
  justify-content: flex-end;
}

.shortcut-key {
  min-width: 28px;
  padding: 0 8px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--bg-color) 76%, var(--panel-bg) 24%);
  color: var(--text-color);
  font-size: 11px;
  font-family: var(--font-mono-identifier);
  box-shadow: none;
}

.shortcut-btn {
  min-width: 88px;
}

.shortcut-help {
  margin: 12px 0 0;
  font-size: 12px;
  color: var(--text-secondary);
}

.shortcut-error {
  margin-left: 8px;
  color: var(--status-warn-fg, var(--text-color));
}

@media (max-width: 860px) {
  .shortcut-mode-row,
  .shortcut-row {
    align-items: flex-start;
    flex-direction: column;
  }

  .shortcut-mode-control {
    width: 100%;
  }

  .shortcut-actions {
    width: 100%;
    justify-content: flex-start;
  }

  .shortcut-keys {
    justify-content: flex-start;
    min-width: 0;
  }
}
</style>
