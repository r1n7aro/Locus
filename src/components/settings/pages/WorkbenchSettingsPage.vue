<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import { MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS } from "../../../composables/useDisplaySettings";
import SettingRow from "../SettingRow.vue";
import SettingsSection from "../SettingsSection.vue";

const mountedEditorOptions = computed(() => MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS.map((value) => ({
  value,
  label: value > 0
    ? t("settings.display.mountedSessionEditorsOption", value)
    : t("settings.display.mountedSessionEditorsUnlimited"),
})));

const reviewTargetOptions = computed(() => [
  { value: "inline", label: t("settings.display.diffReviewInline") },
  { value: "window", label: t("settings.display.diffReviewWindow") },
]);
const planApprovalOptions = computed(() => [
  { value: "card", label: t("chat.plan.approvalTarget.card") },
  { value: "window", label: t("chat.plan.approvalTarget.window") },
]);
const memoryFileOptions = computed(() => [
  { value: "window", label: t("settings.display.memoryFileOpenWindow") },
  { value: "knowledge", label: t("settings.display.memoryFileOpenKnowledge") },
]);
const viewOpenOptions = computed(() => [
  { value: "tab", label: t("settings.display.viewOpenTab") },
  { value: "window", label: t("settings.display.viewOpenWindow") },
]);
const assetRefOptions = computed(() => [
  {
    value: "locusInspector",
    label: t("settings.display.assetRefClickInspector"),
    hint: t("settings.display.assetRefClickInspectorDesc"),
  },
  {
    value: "unitySelect",
    label: t("settings.display.assetRefClickUnitySelect"),
    hint: t("settings.display.assetRefClickUnitySelectDesc"),
  },
  {
    value: "fileBrowser",
    label: t("settings.display.assetRefClickFileBrowser"),
    hint: t("settings.display.assetRefClickFileBrowserDesc"),
  },
]);
// Inside the Unity embed window the editor's own Inspector is also available.
const unityEmbedAssetRefOptions = computed(() => [
  {
    value: "unityInspector",
    label: t("settings.display.assetRefClickUnityInspector"),
    hint: t("settings.display.assetRefClickUnityInspectorDesc"),
  },
  ...assetRefOptions.value,
]);
</script>

<template>
  <SettingsSection
    id="tabs"
    :title="t('settings.display.sessionTabsTitle')"
    :description="t('settings.display.mountedSessionEditorsDesc')"
  >
    <SettingRow
      setting-key="workbench.tabs.mountedSessionEditorsPerGroup"
      :label="t('settings.display.mountedSessionEditors')"
      :description="false"
      :options="mountedEditorOptions"
      control="dropdown"
    />
  </SettingsSection>

  <SettingsSection
    id="panels"
    :title="t('settings.display.panelBehaviorTitle')"
    :description="t('settings.display.panelBehaviorDesc')"
  >
    <SettingRow setting-key="workbench.panels.todoAutoOpen" :description="false" />
    <SettingRow setting-key="workbench.panels.changesAutoOpen" :description="false" />
    <SettingRow setting-key="workbench.panels.changesAutoClose" :description="false" />
    <SettingRow setting-key="workbench.panels.fileChangePopover" :description="false" />
  </SettingsSection>

  <SettingsSection id="open" :title="t('settings.section.openTargets')">
    <SettingRow
      setting-key="workbench.open.chatDiffReview"
      :label="t('settings.display.diffReviewChatTarget')"
      :description="false"
      :options="reviewTargetOptions"
    />
    <SettingRow
      setting-key="workbench.open.gitDiffReview"
      :label="t('settings.display.diffReviewGitTarget')"
      :description="false"
      :options="reviewTargetOptions"
    />
    <SettingRow
      setting-key="workbench.open.planApproval"
      :label="t('chat.plan.approvalTargetLabel')"
      :description="false"
      :options="planApprovalOptions"
    />
    <SettingRow
      setting-key="workbench.open.memoryFile"
      :label="t('settings.display.memoryFileOpenTitle')"
      :description="t('settings.display.memoryFileOpenDesc')"
      :options="memoryFileOptions"
    />
    <SettingRow
      setting-key="workbench.open.view"
      :label="t('settings.display.viewOpenTarget')"
      :description="false"
      :options="viewOpenOptions"
    />
    <SettingRow setting-key="workbench.views.showInSessionPanel" :description="false" />
    <SettingRow
      setting-key="workbench.open.assetRefClick"
      :label="t('settings.display.assetRefClickTarget')"
      :description="t('settings.display.assetRefClickDesc')"
      :options="assetRefOptions"
      control="dropdown"
    />
    <SettingRow
      setting-key="workbench.open.unityEmbedAssetRefClick"
      :label="t('settings.display.assetRefClickUnityEmbedTarget')"
      :description="false"
      :options="unityEmbedAssetRefOptions"
      control="dropdown"
    />
  </SettingsSection>

  <SettingsSection id="git" :title="t('settings.display.gitViewTitle')">
    <SettingRow setting-key="workbench.git.mergeTreeStatusIcon" :description="false" />
    <SettingRow setting-key="workbench.git.hideCommandSuggestions" :description="false" />
  </SettingsSection>

  <SettingsSection id="documents" :title="t('settings.section.documentEditing')">
    <SettingRow
      setting-key="documents.punctuationPairing"
      :label="t('settings.general.punctuationPairing')"
      :description="t('settings.general.punctuationPairingDesc')"
    />
    <SettingRow
      setting-key="documents.commentAuthor"
      :label="t('settings.general.commentAuthor')"
      :description="t('settings.general.commentAuthorDesc')"
    />
  </SettingsSection>
</template>
