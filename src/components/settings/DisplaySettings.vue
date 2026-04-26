<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import { t } from "../../i18n";
import { useTheme, type ThemePreference } from "../../composables/useTheme";
import { useDisplaySettings, type FontSlot } from "../../composables/useDisplaySettings";
import { ipcInvoke } from "../../services/ipc";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";

const { mainPreference, unityEmbedPreference, setThemePreference } = useTheme();
const { state: display, set: setDisplay, setFont } = useDisplaySettings();

const options: { value: ThemePreference; labelKey: string }[] = [
  { value: "system", labelKey: "settings.display.themeSystem" },
  { value: "light",  labelKey: "settings.display.themeLight" },
  { value: "dark",   labelKey: "settings.display.themeDark" },
];

const themeOptions = computed(() =>
  options.map((opt) => ({
    value: opt.value,
    label: t(opt.labelKey),
  })),
);

const systemNotificationOptionsDisabled = computed(
  () => !display.systemNotificationsEnabled,
);

const fontSlots: { slot: FontSlot; labelKey: string; mono: boolean }[] = [
  { slot: "ui",        labelKey: "settings.display.fontUi",        mono: false },
  { slot: "prose",     labelKey: "settings.display.fontProse",     mono: false },
  { slot: "monoInline", labelKey: "settings.display.fontMonoInline", mono: true },
  { slot: "monoBlock", labelKey: "settings.display.fontMonoBlock", mono: true },
  { slot: "monoEditor", labelKey: "settings.display.fontMonoEditor", mono: true },
];

const systemFonts = ref<string[]>([]);

onMounted(async () => {
  try {
    systemFonts.value = await ipcInvoke<string[]>("get_system_fonts");
  } catch { /* fallback: empty list, user can still type */ }
});
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.themeTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.themeDesc") }}</p>
    <div class="theme-rows">
      <div class="theme-row">
        <span class="theme-label">{{ t("settings.display.themeMainWindow") }}</span>
        <BaseSegmented
          class="theme-segmented"
          :model-value="mainPreference"
          :options="themeOptions"
          :aria-label="t('settings.display.themeMainWindow')"
          size="sm"
          @update:model-value="setThemePreference('main', $event as ThemePreference)"
        />
      </div>
      <div class="theme-row">
        <span class="theme-label">{{ t("settings.display.themeUnityEmbedWindow") }}</span>
        <BaseSegmented
          class="theme-segmented"
          :model-value="unityEmbedPreference"
          :options="themeOptions"
          :aria-label="t('settings.display.themeUnityEmbedWindow')"
          size="sm"
          @update:model-value="setThemePreference('unityEmbed', $event as ThemePreference)"
        />
      </div>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.panelBehaviorTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.panelBehaviorDesc") }}</p>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.todoAutoOpen"
        :aria-label="t('settings.display.todoAutoOpen')"
        @update:model-value="setDisplay('todoAutoOpen', $event)"
      />
      <span>{{ t("settings.display.todoAutoOpen") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.changesAutoOpen"
        :aria-label="t('settings.display.changesAutoOpen')"
        @update:model-value="setDisplay('changesAutoOpen', $event)"
      />
      <span>{{ t("settings.display.changesAutoOpen") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.changesAutoClose"
        :aria-label="t('settings.display.changesAutoClose')"
        @update:model-value="setDisplay('changesAutoClose', $event)"
      />
      <span>{{ t("settings.display.changesAutoClose") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.rightAlignUserMessages"
        :aria-label="t('settings.display.rightAlignUserMessages')"
        @update:model-value="setDisplay('rightAlignUserMessages', $event)"
      />
      <span>{{ t("settings.display.rightAlignUserMessages") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.compactToolCalls"
        :aria-label="t('settings.display.compactToolCalls')"
        @update:model-value="setDisplay('compactToolCalls', $event)"
      />
      <span>{{ t("settings.display.compactToolCalls") }}</span>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.notificationsTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.notificationsDesc") }}</p>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.systemNotificationsEnabled"
        :aria-label="t('settings.display.systemNotificationsEnabled')"
        @update:model-value="setDisplay('systemNotificationsEnabled', $event)"
      />
      <span>{{ t("settings.display.systemNotificationsEnabled") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnChatDone"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.display.notifyOnChatDone')"
        @update:model-value="setDisplay('notifyOnChatDone', $event)"
      />
      <span>{{ t("settings.display.notifyOnChatDone") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnAskUser"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.display.notifyOnAskUser')"
        @update:model-value="setDisplay('notifyOnAskUser', $event)"
      />
      <span>{{ t("settings.display.notifyOnAskUser") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnChatError"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.display.notifyOnChatError')"
        @update:model-value="setDisplay('notifyOnChatError', $event)"
      />
      <span>{{ t("settings.display.notifyOnChatError") }}</span>
    </div>

    <div
      class="toggle-row"
      :class="{ disabled: systemNotificationOptionsDisabled }"
    >
      <BaseSwitch
        :model-value="display.notifyOnToolConfirm"
        :disabled="systemNotificationOptionsDisabled"
        :aria-label="t('settings.display.notifyOnToolConfirm')"
        @update:model-value="setDisplay('notifyOnToolConfirm', $event)"
      />
      <span>{{ t("settings.display.notifyOnToolConfirm") }}</span>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.fontTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.fontDesc") }}</p>

    <div class="font-grid">
      <template v-for="f in fontSlots" :key="f.slot">
        <label class="font-label">{{ t(f.labelKey) }}</label>
        <select
          class="font-select"
          :value="display.fonts[f.slot]"
          @change="setFont(f.slot, ($event.target as HTMLSelectElement).value)"
        >
          <option value="">{{ t("settings.display.fontDefault") }}</option>
          <option
            v-for="name in systemFonts"
            :key="name"
            :value="name"
            :style="{ fontFamily: name }"
          >{{ name }}</option>
        </select>
      </template>
    </div>
  </div>
</template>

<style scoped>
.theme-rows {
  display: grid;
  gap: 8px;
  max-width: 560px;
}

.theme-row {
  display: grid;
  grid-template-columns: 110px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
}

.theme-label {
  font-size: 13px;
  color: var(--text-secondary);
}

.theme-segmented {
  justify-self: start;
  width: fit-content;
  max-width: 100%;
}

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

.toggle-row.disabled {
  color: var(--text-secondary);
}

.font-grid {
  display: grid;
  grid-template-columns: 100px minmax(0, 360px);
  gap: 6px 10px;
  align-items: center;
  margin-top: 8px;
  width: min(470px, 100%);
}

.font-label {
  font-size: 13px;
  color: var(--text-secondary);
  text-align: right;
  white-space: nowrap;
}

.font-select {
  width: 100%;
  min-width: 0;
  padding: 5px 8px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  outline: none;
  cursor: pointer;
  transition: border-color 0.15s;
}

.font-select:focus {
  border-color: var(--accent-color);
}
</style>
