<script setup lang="ts">
import { computed } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "../../i18n";
import { useDisplaySettings } from "../../composables/useDisplaySettings";
import {
  playNotificationSound,
  unlockNotificationSounds,
  type NotificationSoundMode,
  type NotificationSoundSource,
} from "../../services/notificationSounds";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";

const { state: display, set: setDisplay } = useDisplaySettings();

const systemNotificationOptionsDisabled = computed(
  () => !display.systemNotificationsEnabled,
);

const soundAlertOptionsDisabled = computed(
  () => !display.soundAlertsEnabled,
);

const soundSourceOptions = computed(() => [
  {
    value: "builtin",
    label: t("settings.notifications.soundSourceBuiltin"),
    disabled: soundAlertOptionsDisabled.value,
  },
  {
    value: "custom",
    label: t("settings.notifications.soundSourceCustom"),
    disabled: soundAlertOptionsDisabled.value,
  },
]);

const soundModeOptions = computed(() => [
  {
    value: "soft",
    label: t("settings.notifications.soundModeSoft"),
    disabled: soundAlertOptionsDisabled.value,
  },
  {
    value: "bright",
    label: t("settings.notifications.soundModeBright"),
    disabled: soundAlertOptionsDisabled.value,
  },
  {
    value: "urgent",
    label: t("settings.notifications.soundModeUrgent"),
    disabled: soundAlertOptionsDisabled.value,
  },
]);

const selectedSoundFileLabel = computed(() => {
  const filePath = display.soundAlertCustomFilePath.trim();
  if (!filePath) return t("settings.notifications.soundFileEmpty");
  return filePath.split(/[\\/]/).filter(Boolean).pop() || filePath;
});

const hasCustomSoundFile = computed(
  () => display.soundAlertCustomFilePath.trim().length > 0,
);

function clampSoundAlertVolume(value: number): number {
  if (!Number.isFinite(value)) return 50;
  return Math.min(100, Math.max(0, Math.round(value)));
}

const normalizedSoundAlertVolume = computed(() =>
  clampSoundAlertVolume(display.soundAlertVolume),
);

const soundAlertVolumeLabel = computed(() =>
  `${normalizedSoundAlertVolume.value}%`,
);

function setSoundAlertsEnabled(value: boolean) {
  setDisplay("soundAlertsEnabled", value);
  if (value) {
    void unlockNotificationSounds().catch(() => undefined);
  }
}

function setSoundAlertMode(value: string) {
  setDisplay("soundAlertMode", value as NotificationSoundMode);
}

function setSoundAlertSource(value: string) {
  setDisplay("soundAlertSource", value as NotificationSoundSource);
}

function setSoundAlertVolume(event: Event) {
  const target = event.target as HTMLInputElement | null;
  if (!target) return;
  const volume = clampSoundAlertVolume(Number(target.value));
  setDisplay("soundAlertVolume", volume);
}

async function chooseSoundFile() {
  const selected = await open({
    directory: false,
    multiple: false,
    filters: [
      {
        name: t("settings.notifications.soundFileFilter"),
        extensions: ["wav", "mp3", "ogg", "flac", "m4a", "aac"],
      },
    ],
  });

  if (typeof selected !== "string" || !selected.trim()) return;
  setDisplay("soundAlertCustomFilePath", selected);
  setDisplay("soundAlertSource", "custom");
  void unlockNotificationSounds().catch(() => undefined);
}

function clearSoundFile() {
  setDisplay("soundAlertCustomFilePath", "");
  setDisplay("soundAlertSource", "builtin");
}

async function previewSoundAlert() {
  await unlockNotificationSounds();
  await playNotificationSound(
    "complete",
    display.soundAlertMode,
    display.soundAlertSource === "custom" ? display.soundAlertCustomFilePath : "",
    display.soundAlertVolume,
  );
}
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.notifications.systemTitle") }}</div>
    <p class="section-desc">{{ t("settings.notifications.systemDesc") }}</p>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.systemNotificationsEnabled"
        :aria-label="t('settings.notifications.systemNotificationsEnabled')"
        @update:model-value="setDisplay('systemNotificationsEnabled', $event)"
      />
      <span>{{ t("settings.notifications.systemNotificationsEnabled") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnChatDone"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.notifications.notifyOnChatDone')"
        @update:model-value="setDisplay('notifyOnChatDone', $event)"
      />
      <span>{{ t("settings.notifications.notifyOnChatDone") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnSubagentDone"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.notifications.notifyOnSubagentDone')"
        @update:model-value="setDisplay('notifyOnSubagentDone', $event)"
      />
      <span>{{ t("settings.notifications.notifyOnSubagentDone") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnAskUser"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.notifications.notifyOnAskUser')"
        @update:model-value="setDisplay('notifyOnAskUser', $event)"
      />
      <span>{{ t("settings.notifications.notifyOnAskUser") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnChatError"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.notifications.notifyOnChatError')"
        @update:model-value="setDisplay('notifyOnChatError', $event)"
      />
      <span>{{ t("settings.notifications.notifyOnChatError") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnToolConfirm"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.notifications.notifyOnToolConfirm')"
        @update:model-value="setDisplay('notifyOnToolConfirm', $event)"
      />
      <span>{{ t("settings.notifications.notifyOnToolConfirm") }}</span>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.notifications.soundTitle") }}</div>
    <p class="section-desc">{{ t("settings.notifications.soundDesc") }}</p>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.soundAlertsEnabled"
        :aria-label="t('settings.notifications.soundAlertsEnabled')"
        @update:model-value="setSoundAlertsEnabled"
      />
      <span>{{ t("settings.notifications.soundAlertsEnabled") }}</span>
    </div>

    <div
      class="choice-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <span class="choice-label">{{ t("settings.notifications.soundSource") }}</span>
      <div class="sound-mode-controls">
        <BaseSegmented
          class="choice-segmented"
          :model-value="display.soundAlertSource"
          :options="soundSourceOptions"
          :aria-label="t('settings.notifications.soundSource')"
          size="sm"
          @update:model-value="setSoundAlertSource"
        />
      </div>
    </div>

    <div
      class="choice-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <span class="choice-label">{{ t("settings.notifications.soundVolume") }}</span>
      <div class="sound-volume-controls">
        <input
          class="sound-volume-slider"
          type="range"
          min="0"
          max="100"
          step="5"
          :value="normalizedSoundAlertVolume"
          :style="{ '--sound-volume-percent': `${normalizedSoundAlertVolume}%` }"
          :disabled="soundAlertOptionsDisabled"
          :aria-label="t('settings.notifications.soundVolume')"
          @input="setSoundAlertVolume"
        />
        <span class="sound-volume-value">{{ soundAlertVolumeLabel }}</span>
      </div>
    </div>

    <div
      v-if="display.soundAlertSource === 'custom'"
      class="choice-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <span class="choice-label">{{ t("settings.notifications.soundFile") }}</span>
      <div class="sound-file-controls">
        <span
          class="sound-file-path"
          :class="{ empty: !hasCustomSoundFile }"
          :title="display.soundAlertCustomFilePath || undefined"
        >
          {{ selectedSoundFileLabel }}
        </span>
        <BaseButton
          size="sm"
          :disabled="soundAlertOptionsDisabled"
          @click="chooseSoundFile"
        >
          {{ t("settings.notifications.soundFileChoose") }}
        </BaseButton>
        <BaseButton
          size="sm"
          :disabled="soundAlertOptionsDisabled || !hasCustomSoundFile"
          @click="clearSoundFile"
        >
          {{ t("settings.notifications.soundFileClear") }}
        </BaseButton>
        <BaseButton
          size="sm"
          :disabled="soundAlertOptionsDisabled || !hasCustomSoundFile"
          @click="previewSoundAlert"
        >
          {{ t("settings.notifications.soundPreview") }}
        </BaseButton>
      </div>
    </div>

    <div
      v-else
      class="choice-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <span class="choice-label">{{ t("settings.notifications.soundMode") }}</span>
      <div class="sound-mode-controls">
        <BaseSegmented
          class="choice-segmented"
          :model-value="display.soundAlertMode"
          :options="soundModeOptions"
          :aria-label="t('settings.notifications.soundMode')"
          size="sm"
          @update:model-value="setSoundAlertMode"
        />
        <BaseButton
          size="sm"
          :disabled="soundAlertOptionsDisabled"
          @click="previewSoundAlert"
        >
          {{ t("settings.notifications.soundPreview") }}
        </BaseButton>
      </div>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.soundOnChatDone"
        :disabled="soundAlertOptionsDisabled"
        :aria-label="t('settings.notifications.soundOnChatDone')"
        @update:model-value="setDisplay('soundOnChatDone', $event)"
      />
      <span>{{ t("settings.notifications.soundOnChatDone") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.soundOnSubagentDone"
        :disabled="soundAlertOptionsDisabled"
        :aria-label="t('settings.notifications.soundOnSubagentDone')"
        @update:model-value="setDisplay('soundOnSubagentDone', $event)"
      />
      <span>{{ t("settings.notifications.soundOnSubagentDone") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.soundOnAskUser"
        :disabled="soundAlertOptionsDisabled"
        :aria-label="t('settings.notifications.soundOnAskUser')"
        @update:model-value="setDisplay('soundOnAskUser', $event)"
      />
      <span>{{ t("settings.notifications.soundOnAskUser") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.soundOnChatError"
        :disabled="soundAlertOptionsDisabled"
        :aria-label="t('settings.notifications.soundOnChatError')"
        @update:model-value="setDisplay('soundOnChatError', $event)"
      />
      <span>{{ t("settings.notifications.soundOnChatError") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: soundAlertOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.soundOnToolConfirm"
        :disabled="soundAlertOptionsDisabled"
        :aria-label="t('settings.notifications.soundOnToolConfirm')"
        @update:model-value="setDisplay('soundOnToolConfirm', $event)"
      />
      <span>{{ t("settings.notifications.soundOnToolConfirm") }}</span>
    </div>
  </div>
</template>

<style scoped>
.toggle-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: fit-content;
  max-width: 100%;
  padding: 7px 0;
  font-size: 13px;
  color: var(--text-color);
}

.toggle-row.disabled,
.choice-row.disabled {
  color: var(--text-secondary);
}

.choice-row {
  display: grid;
  grid-template-columns: 110px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  width: min(620px, 100%);
  padding: 7px 0;
}

.choice-label {
  font-size: 13px;
  color: var(--text-secondary);
}

.sound-mode-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.sound-file-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.sound-volume-controls {
  display: flex;
  align-items: center;
  gap: 10px;
  width: min(300px, 100%);
  min-width: 0;
}

.sound-volume-slider {
  --sound-volume-percent: 100%;
  -webkit-appearance: none;
  appearance: none;
  flex: 1;
  min-width: 120px;
  height: 4px;
  border-radius: 999px;
  background: linear-gradient(
    to right,
    var(--accent-color) 0 var(--sound-volume-percent),
    var(--border-color) var(--sound-volume-percent) 100%
  );
  cursor: pointer;
  outline: none;
}

.sound-volume-slider:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.sound-volume-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 14px;
  height: 14px;
  border: 2px solid var(--panel-bg);
  border-radius: 50%;
  background: var(--accent-color);
}

.sound-volume-slider::-moz-range-thumb {
  width: 14px;
  height: 14px;
  border: 2px solid var(--panel-bg);
  border-radius: 50%;
  background: var(--accent-color);
}

.sound-volume-value {
  width: 38px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: right;
}

.sound-file-path {
  min-width: 0;
  max-width: 260px;
  overflow: hidden;
  color: var(--text-color);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sound-file-path.empty {
  color: var(--text-secondary);
}

.choice-segmented {
  justify-self: start;
  width: fit-content;
  max-width: 100%;
}
</style>
