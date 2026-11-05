<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import { formatShortcut, useKeyboardShortcuts } from "../../../composables/useKeyboardShortcuts";
import { useWorkspaceContextStore } from "../../../stores/workspaceContext";
import SettingRow from "../SettingRow.vue";
import SettingsSection from "../SettingsSection.vue";
import MarkdownImageSettings from "../MarkdownImageSettings.vue";
import WorkspaceResourceSettings from "../WorkspaceResourceSettings.vue";
import WorktreePoolSettings from "../WorktreePoolSettings.vue";

const displayModeOptions = computed(() => [
  { value: "single", label: t("settings.display.workspaceModeSingle") },
  { value: "multi", label: t("settings.display.workspaceModeMulti") },
]);

const sectionToggles = [
  { kind: "knowledge", labelKey: "app.tab.knowledge" },
  { kind: "collab", labelKey: "app.tab.collab" },
  { kind: "assets", labelKey: "app.tab.asset" },
  { kind: "views", labelKey: "app.tab.views" },
] as const;

const knowledgeFolderToggles = [
  { kind: "plan", labelKey: "knowledge.type.plan" },
  { kind: "memory", labelKey: "knowledge.type.memory" },
  { kind: "design", labelKey: "knowledge.type.design" },
  { kind: "skill", labelKey: "knowledge.type.skill" },
  { kind: "reference", labelKey: "knowledge.type.reference" },
] as const;

const searchSources = [
  "knowledgeTitle",
  "knowledgeContent",
  "sessionTitle",
  "sessionContent",
  "fileName",
  "viewName",
] as const;

const { state: shortcuts } = useKeyboardShortcuts();
const searchShortcut = computed(() => formatShortcut(shortcuts.globalSearch));

// Project settings apply to the project focused in this window.
const workspaceContext = useWorkspaceContextStore();
const projectRoot = computed(() => (workspaceContext.focusedWorkspaceRef ? workspaceContext.focusedRoot : ""));
const projectName = computed(() =>
  projectRoot.value.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || projectRoot.value);
</script>

<template>
  <SettingsSection
    id="project"
    :title="projectRoot ? t('settings.project.title', projectName) : t('settings.project.titleNone')"
    :description="projectRoot ? t('settings.project.desc') : t('settings.project.none')"
  >
    <SettingRow
      setting-key="project.knowledgeEnabled"
      project
      :disabled="!projectRoot"
      :description="t('settings.project.knowledgeEnabledDesc')"
    />
    <MarkdownImageSettings :disabled="!projectRoot" />
  </SettingsSection>

  <SettingsSection id="tree" :title="t('settings.section.workspaceTree')">
    <SettingRow
      setting-key="workspace.tree.displayMode"
      :label="t('settings.display.workspaceMode')"
      :description="false"
      :options="displayModeOptions"
    />
    <SettingRow
      v-for="item in sectionToggles"
      :key="item.kind"
      :setting-key="`workspace.tree.sections.${item.kind}`"
      :label="t('settings.display.showWorkspaceSection', t(item.labelKey))"
      :description="false"
      control="checkbox"
    />
    <SettingRow
      v-for="item in knowledgeFolderToggles"
      :key="item.kind"
      :setting-key="`workspace.tree.knowledgeFolders.${item.kind}`"
      :label="t('settings.display.showKnowledgeFolder', t(item.labelKey))"
      :description="false"
      control="checkbox"
    />
    <SettingRow setting-key="workspace.tree.autoPlaceNewPlanDesignDocuments" :description="false" />
    <SettingRow setting-key="workspace.tree.showUnreadIndicators" :description="false" />
    <SettingRow
      setting-key="workspace.tree.autoPromoteCompletedSessions"
      :description="t('settings.display.autoPromoteCompletedSessionsDesc')"
    />
  </SettingsSection>

  <SettingsSection
    id="files"
    :title="t('settings.display.fileExplorerTitle')"
    :description="t('settings.display.fileExplorerDesc')"
  >
    <SettingRow
      setting-key="workspace.explorer.hiddenDirectories"
      :label="t('settings.display.fileExplorerHiddenDirectories')"
      :description="false"
      :placeholder="t('settings.display.fileExplorerHiddenDirectoriesPlaceholder')"
      control="list"
    />
    <SettingRow
      setting-key="workspace.explorer.unityHiddenDirectories"
      :label="t('settings.display.fileExplorerUnityHiddenDirectories')"
      :description="false"
      :placeholder="t('settings.display.fileExplorerUnityHiddenDirectoriesPlaceholder')"
      control="list"
    />
  </SettingsSection>

  <SettingsSection
    id="search"
    :title="t('settings.tab.globalSearch')"
    :description="t('globalSearch.shortcutHint', searchShortcut)"
  >
    <SettingRow setting-key="workspace.search.enabled" :description="false" />
    <SettingRow setting-key="workspace.search.includeArchived" :description="false" />
    <SettingRow
      v-for="source in searchSources"
      :key="source"
      :setting-key="`workspace.search.sources.${source}`"
      :description="false"
      control="checkbox"
    />
  </SettingsSection>

  <SettingsSection id="worktree" :title="t('settings.section.worktree')" experimental>
    <SettingRow
      setting-key="workspace.worktree.enabled"
      :description="t('settings.experimental.worktreeDesc')"
    />
    <WorktreePoolSettings />
  </SettingsSection>

  <SettingsSection id="resources">
    <WorkspaceResourceSettings />
  </SettingsSection>
</template>
