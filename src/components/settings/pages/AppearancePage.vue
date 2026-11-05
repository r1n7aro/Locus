<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../../i18n";
import { useDisplaySettings, type FontSlot } from "../../../composables/useDisplaySettings";
import { ipcInvoke } from "../../../services/ipc";
import BaseDropdown, { type DropdownOption } from "../../ui/BaseDropdown.vue";
import SettingRow from "../SettingRow.vue";
import SettingsSection from "../SettingsSection.vue";

const { state: display, setFont } = useDisplaySettings();

const languageOptions = computed(() => [
  { value: "system", label: t("settings.language.system") },
  { value: "zh", label: t("language.zh") },
  { value: "en", label: t("language.en") },
]);

const themeOptions = computed(() => [
  { value: "system", label: t("settings.display.themeSystem") },
  { value: "light", label: t("settings.display.themeLight") },
  { value: "dark", label: t("settings.display.themeDark") },
]);

const fontSlots: { slot: FontSlot; labelKey: string }[] = [
  { slot: "ui", labelKey: "settings.display.fontUi" },
  { slot: "prose", labelKey: "settings.display.fontProse" },
  { slot: "monoInline", labelKey: "settings.display.fontMonoInline" },
  { slot: "monoBlock", labelKey: "settings.display.fontMonoBlock" },
  { slot: "monoEditor", labelKey: "settings.display.fontMonoEditor" },
];

const systemFonts = ref<string[]>([]);
const fontOptions = computed<DropdownOption[]>(() => [
  { value: "", label: t("settings.display.fontDefault") },
  ...systemFonts.value.map((name) => ({ value: name, label: name, labelStyle: { fontFamily: name } })),
]);

onMounted(async () => {
  try {
    systemFonts.value = await ipcInvoke<string[]>("get_system_fonts");
  } catch { /* An empty list still allows the default font. */ }
});
</script>

<template>
  <SettingsSection id="language">
    <SettingRow
      setting-key="appearance.language"
      :label="t('settings.general.language')"
      :description="t('settings.general.languageDesc')"
      :options="languageOptions"
    />
  </SettingsSection>

  <SettingsSection id="theme" :title="t('settings.display.themeTitle')" :description="t('settings.display.themeDesc')">
    <SettingRow
      setting-key="appearance.theme.main"
      :label="t('settings.display.themeMainWindow')"
      :description="false"
      :options="themeOptions"
    />
    <SettingRow
      setting-key="appearance.theme.unityEmbed"
      :label="t('settings.display.themeUnityEmbedWindow')"
      :description="false"
      :options="themeOptions"
    />
  </SettingsSection>

  <SettingsSection id="fonts" :title="t('settings.display.fontTitle')" :description="t('settings.display.fontDesc')">
    <div class="font-grid">
      <template v-for="font in fontSlots" :key="font.slot">
        <label class="font-label">{{ t(font.labelKey) }}</label>
        <BaseDropdown
          class="font-select"
          :model-value="display.fonts[font.slot]"
          :options="fontOptions"
          size="md"
          menu-align="start"
          teleport
          :aria-label="t(font.labelKey)"
          @update:model-value="setFont(font.slot, $event)"
        />
      </template>
    </div>
  </SettingsSection>

  <SettingsSection
    id="chrome"
    :title="t('settings.display.mainChromeTitle')"
    :description="t('settings.display.mainChromeDesc')"
  >
    <SettingRow setting-key="appearance.chrome.showWelcomeSubtitle" :description="false" />
    <SettingRow setting-key="appearance.chrome.showPluginsTab" :description="false" />
    <SettingRow setting-key="appearance.chrome.showAgentTab" :description="false" />
    <SettingRow setting-key="appearance.chrome.workspaceWindowTitleBar" />
  </SettingsSection>
</template>

<style scoped>
.font-grid {
  display: grid;
  grid-template-columns: 120px minmax(0, 320px);
  align-items: center;
  gap: 8px 12px;
}

.font-label {
  color: var(--text-secondary);
  font-size: 13px;
}

.font-select {
  width: 100%;
}
</style>
