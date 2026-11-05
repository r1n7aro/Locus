<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import { SESSION_MESSAGE_PAGE_SIZE_OPTIONS } from "../../../composables/useDisplaySettings";
import SettingRow from "../SettingRow.vue";
import SettingsSection from "../SettingsSection.vue";

const pageSizeOptions = computed(() => SESSION_MESSAGE_PAGE_SIZE_OPTIONS.map((value) => ({
  value,
  label: t("settings.display.sessionMessagePageSizeOption", value),
})));
</script>

<template>
  <SettingsSection id="messages" :title="t('settings.section.messages')">
    <SettingRow setting-key="chat.messages.rightAlignUser" :description="false" />
    <SettingRow setting-key="chat.messages.turnNavigationRail" :description="false" />
    <SettingRow setting-key="chat.messages.compactToolCalls" :description="false" />
    <SettingRow setting-key="chat.messages.hideThinkingBlocks" :description="false" />
    <SettingRow setting-key="chat.composer.showAgentSelector" :description="false" />
  </SettingsSection>

  <SettingsSection
    id="history"
    :title="t('settings.display.sessionHistoryTitle')"
    :description="t('settings.display.sessionHistoryDesc')"
  >
    <SettingRow
      setting-key="chat.history.pageSize"
      :label="t('settings.display.sessionMessagePageSize')"
      :description="false"
      :options="pageSizeOptions"
      control="dropdown"
    />
  </SettingsSection>
</template>
