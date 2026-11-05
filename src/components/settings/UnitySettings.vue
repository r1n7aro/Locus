<script setup lang="ts">
// Unity page: global switches first, then the focused workspace's live
// state and self-tests in "Diagnostics", which never mixes with the switches.
import { computed } from "vue";
import { t } from "../../i18n";
import { settingValue } from "../../composables/useSettingsStore";
import { useServiceSwitches } from "../../composables/useServiceSwitches";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import ServiceSwitchCard from "./ServiceSwitchCard.vue";
import SettingRow from "./SettingRow.vue";
import SettingsSection from "./SettingsSection.vue";
import UnityDiagnostics from "./UnityDiagnostics.vue";
import UnityEditorResources from "./UnityEditorResources.vue";

const { switches, busy, setEnabled } = useServiceSwitches();
const enabled = computed(() => switches.value?.unity ?? false);
const workspaceContext = useWorkspaceContextStore();
const workspaceFocused = computed(() => workspaceContext.focusedWorkspaceRef !== null);

const connectionKeys = [
  "unity.externalEditor.autoSelect",
  "unity.embed.enabled",
  "unity.connection.nativeBridge",
  "unity.connection.stateProbe",
  "unity.connection.backgroundHook",
] as const;

const lspToolKeys = [
  "codeSymbolSearch",
  "codeGotoDefinition",
  "codeFindReferences",
  "editWriteDiagnostics",
  "codeDiagnostics",
  "codeHover",
  "unityAnalyzers",
] as const;

const lspEnabled = computed(() => settingValue("unity.codeAnalysis.csharpLsp") === true);
const sidecarEnabled = computed(() => settingValue("unity.compile.sidecar") !== false);
</script>

<template>
  <div class="settings-section" data-settings-section="enable">
    <ServiceSwitchCard
      :label="t('settings.services.unityEnabled')"
      :description="t('settings.services.unityEnabledDesc')"
      :model-value="enabled"
      :ready="switches !== null"
      :busy="busy"
      @update:model-value="setEnabled('unity', $event)"
    />
  </div>

  <template v-if="enabled">
    <SettingsSection id="connection" :title="t('settings.section.unityConnection')">
      <SettingRow v-for="key in connectionKeys" :key="key" :setting-key="key" />
    </SettingsSection>

    <SettingsSection
      id="codeAnalysis"
      :title="t('settings.codeAnalysis.master')"
      :description="t('settings.codeAnalysis.lspToolsDesc')"
    >
      <SettingRow setting-key="unity.codeAnalysis.csharpLsp" focused-workspace />
      <SettingRow
        v-for="key in lspToolKeys"
        :key="key"
        :setting-key="`unity.codeAnalysis.tools.${key}`"
        :disabled="!lspEnabled"
        :focused-workspace="key === 'unityAnalyzers'"
      />
      <p v-if="!lspEnabled" class="section-desc unity-note">{{ t("settings.codeAnalysis.lspToolsOffNote") }}</p>
      <div class="unity-subtitle">{{ t("settings.codeAnalysis.assetTools") }}</div>
      <p class="section-desc">{{ t("settings.codeAnalysis.assetToolsDesc") }}</p>
      <SettingRow setting-key="unity.codeAnalysis.tools.unityCodeUsages" />
    </SettingsSection>

    <SettingsSection id="compile" :title="t('settings.tab.hotReload')">
      <SettingRow setting-key="unity.compile.sidecar" />
      <SettingRow setting-key="unity.compile.nonPublicAccess" :disabled="!sidecarEnabled" />
      <SettingRow setting-key="unity.hotReload.enabled" :disabled="!sidecarEnabled" />
      <p v-if="!sidecarEnabled" class="section-desc unity-note">{{ t("settings.codeAnalysis.hotReloadDepSidecar") }}</p>
    </SettingsSection>

    <div class="settings-section" data-settings-section="editors">
      <UnityEditorResources />
    </div>

    <SettingsSection id="testing" :title="t('settings.tab.testing')">
      <SettingRow setting-key="project.unityTestToolsEnabled" project :disabled="!workspaceFocused" />
    </SettingsSection>

    <SettingsSection id="multiAgent" :title="t('settings.experimental.unityMultiAgentEditor')" experimental>
      <SettingRow setting-key="unity.multiAgentEditor" :label="t('settings.experimental.unityMultiAgentEditor')" />
    </SettingsSection>

    <SettingsSection
      id="diagnostics"
      class="unity-diagnostics"
      :title="t('settings.section.unityDiagnostics')"
      :description="workspaceFocused ? t('settings.unity.diagnosticsDesc') : t('settings.unity.diagnosticsNone')"
    >
      <UnityDiagnostics v-if="workspaceFocused" />
    </SettingsSection>
  </template>
</template>

<style scoped>
.unity-diagnostics {
  border-top: 1px solid var(--border-color);
}
.unity-subtitle {
  margin-top: 14px;
  color: var(--text-color);
  font-size: 12.5px;
  font-weight: 600;
}
.unity-note {
  margin-top: 6px;
}
</style>
