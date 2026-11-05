<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import type { AgentInfo } from "../../../types";
import { useNewSessionSettings } from "../../../composables/useNewSessionSettings";
import { readString, useSetting } from "../../../composables/useSettingsStore";
import BaseDropdown from "../../ui/BaseDropdown.vue";
import SettingRow from "../SettingRow.vue";
import SettingsSection from "../SettingsSection.vue";

const props = withDefaults(defineProps<{ agents?: AgentInfo[] }>(), { agents: () => [] });

const { state: newSessionSettings, setAgentMode, setDefaultAgentId } = useNewSessionSettings();
// Agent ids never start with "@", so the two modes share the dropdown with pinned Agents.
const REMEMBER_AGENT_OPTION = "@remember";
const WORKSPACE_AGENT_OPTION = "@workspace";
const defaultAgentOptions = computed(() => [
  { value: REMEMBER_AGENT_OPTION, label: t("settings.general.defaultAgentRemember") },
  { value: WORKSPACE_AGENT_OPTION, label: t("settings.general.defaultAgentAuto") },
  ...props.agents.map((agent) => ({
    value: agent.id,
    label: t("settings.general.defaultAgentFixed", agent.name),
  })),
]);
const defaultAgentValue = computed(() => {
  if (newSessionSettings.agentMode === "fixed") return newSessionSettings.defaultAgentId;
  return newSessionSettings.agentMode === "workspace" ? WORKSPACE_AGENT_OPTION : REMEMBER_AGENT_OPTION;
});
const defaultAgentLabel = computed(() => (
  defaultAgentOptions.value.find((option) => option.value === defaultAgentValue.value)?.label
    ?? t("settings.general.defaultAgentFixed", newSessionSettings.defaultAgentId)
));
function updateDefaultAgent(value: string) {
  if (value === REMEMBER_AGENT_OPTION) setAgentMode("remember");
  else if (value === WORKSPACE_AGENT_OPTION) setAgentMode("workspace");
  else setDefaultAgentId(value);
}

const toolLoadingOptions = computed(() => [
  { value: "native", label: t("settings.dynamicToolLoading.native"), hint: t("settings.dynamicToolLoading.nativeDesc") },
  { value: "metaTool", label: t("settings.dynamicToolLoading.metaTool"), hint: t("settings.dynamicToolLoading.metaToolDesc") },
  { value: "direct", label: t("settings.dynamicToolLoading.direct"), hint: t("settings.dynamicToolLoading.directDesc") },
]);
const toolLoadingMode = useSetting("agent.tools.loadingMode", readString("native"));
const toolLoadingDescription = computed(() =>
  toolLoadingOptions.value.find((option) => option.value === toolLoadingMode.value.value)?.hint ?? "");
</script>

<template>
  <SettingsSection
    id="newSession"
    :title="t('settings.general.defaultAgent')"
    :description="t('settings.general.defaultAgentDesc')"
  >
    <BaseDropdown
      class="default-agent-dropdown"
      :model-value="defaultAgentValue"
      :options="defaultAgentOptions"
      :selected-label="defaultAgentLabel"
      :aria-label="t('settings.general.defaultAgent')"
      menu-align="start"
      teleport
      @update:model-value="updateDefaultAgent"
    />
  </SettingsSection>

  <SettingsSection
    id="subagents"
    :title="t('settings.general.subagentLimits')"
    :description="t('settings.general.subagentLimitsDesc')"
  >
    <SettingRow setting-key="agent.subagent.maxDepth" :label="t('settings.general.subagentMaxDepth')" :description="false" />
    <SettingRow setting-key="agent.subagent.maxConcurrent" :label="t('settings.general.subagentMaxConcurrent')" :description="false" />
  </SettingsSection>

  <SettingsSection id="requests" :title="t('settings.section.requests')">
    <SettingRow
      setting-key="agent.llm.retryMaxAttempts"
      :label="t('settings.general.llmRetry')"
      :description="t('settings.general.llmRetryDesc')"
    />
    <SettingRow setting-key="agent.llm.stripInlineThinkTags" />
  </SettingsSection>

  <SettingsSection id="toolLoading" :title="t('settings.dynamicToolLoading.title')">
    <SettingRow
      setting-key="agent.tools.loadingMode"
      :label="t('settings.dynamicToolLoading.mode')"
      :description="toolLoadingDescription"
      :options="toolLoadingOptions"
    />
  </SettingsSection>

  <SettingsSection id="sessionUndo">
    <SettingRow
      setting-key="agent.sessionUndo"
      :label="t('settings.general.sessionUndo')"
      :description="t('settings.general.sessionUndoDesc')"
    />
  </SettingsSection>
</template>

<style scoped>
.default-agent-dropdown {
  width: min(420px, 100%);
}
</style>
