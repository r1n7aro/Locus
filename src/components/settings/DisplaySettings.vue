<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import { t } from "../../i18n";
import { useTheme, type ThemePreference } from "../../composables/useTheme";
import {
  useDisplaySettings,
  SESSION_MESSAGE_PAGE_SIZE_OPTIONS,
  type AssetRefClickAction,
  type DiffReviewTarget,
  type FontSlot,
  type KnowledgeFolderKind,
  type MemoryFileOpenTarget,
  type PlanApprovalTarget,
  type WorkspaceDisplayMode,
  type WorkspaceSectionVisibilityKind,
} from "../../composables/useDisplaySettings";
import { ipcInvoke } from "../../services/ipc";
import BaseDropdown, { type DropdownOption } from "../ui/BaseDropdown.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";

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

const workspaceDisplayModeOptions = computed(() => [
  { value: "single", label: t("settings.display.workspaceModeSingle") },
  { value: "multi", label: t("settings.display.workspaceModeMulti") },
]);

const knowledgeFolderToggles: { kind: KnowledgeFolderKind; labelKey: string }[] = [
  { kind: "plan", labelKey: "knowledge.type.plan" },
  { kind: "memory", labelKey: "knowledge.type.memory" },
  { kind: "design", labelKey: "knowledge.type.design" },
  { kind: "skill", labelKey: "knowledge.type.skill" },
  { kind: "reference", labelKey: "knowledge.type.reference" },
];

const workspaceSectionToggles: {
  kind: WorkspaceSectionVisibilityKind;
  labelKey: string;
}[] = [
  { kind: "knowledge", labelKey: "app.tab.knowledge" },
  { kind: "collab", labelKey: "app.tab.collab" },
  { kind: "assets", labelKey: "app.tab.asset" },
  { kind: "views", labelKey: "app.tab.views" },
];

function setWorkspaceSectionVisibility(
  kind: WorkspaceSectionVisibilityKind,
  visible: boolean,
): void {
  setDisplay("workspaceSectionVisibility", {
    ...display.workspaceSectionVisibility,
    [kind]: visible,
  });
}

function setKnowledgeFolderVisibility(kind: KnowledgeFolderKind, visible: boolean): void {
  setDisplay("knowledgeFolderVisibility", {
    ...display.knowledgeFolderVisibility,
    [kind]: visible,
  });
}

function hiddenDirectoriesText(directories: readonly string[]): string {
  return directories.join(", ");
}

function updateHiddenDirectories(
  key: "fileExplorerHiddenDirectories" | "unityFileExplorerHiddenDirectories",
  event: Event,
): void {
  const value = (event.target as HTMLInputElement).value;
  setDisplay(key, value.split(/[,，;；\n]+/));
}

const sessionMessagePageSizeOptions = computed<DropdownOption[]>(() =>
  SESSION_MESSAGE_PAGE_SIZE_OPTIONS.map((value) => ({
    value: String(value),
    label: t("settings.display.sessionMessagePageSizeOption", value),
  })),
);

const diffReviewTargetOptions = computed(() => [
  { value: "inline", label: t("settings.display.diffReviewInline") },
  { value: "window", label: t("settings.display.diffReviewWindow") },
]);

const planApprovalTargetOptions = computed(() => [
  { value: "card", label: t("chat.plan.approvalTarget.card") },
  { value: "window", label: t("chat.plan.approvalTarget.window") },
]);

const memoryFileOpenTargetOptions = computed(() => [
  { value: "window", label: t("settings.display.memoryFileOpenWindow") },
  { value: "knowledge", label: t("settings.display.memoryFileOpenKnowledge") },
]);

const assetRefClickActionOptions = computed(() => [
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
const unityEmbedAssetRefClickActionOptions = computed(() => [
  {
    value: "unityInspector",
    label: t("settings.display.assetRefClickUnityInspector"),
    hint: t("settings.display.assetRefClickUnityInspectorDesc"),
  },
  ...assetRefClickActionOptions.value,
]);

const topNavigationToggles = [
  { key: "showPluginsTab", labelKey: "settings.display.showPluginsTab" },
  { key: "showAgentTab", labelKey: "settings.display.showAgentTab" },
] as const;

const fontSlots: { slot: FontSlot; labelKey: string; mono: boolean }[] = [
  { slot: "ui",        labelKey: "settings.display.fontUi",        mono: false },
  { slot: "prose",     labelKey: "settings.display.fontProse",     mono: false },
  { slot: "monoInline", labelKey: "settings.display.fontMonoInline", mono: true },
  { slot: "monoBlock", labelKey: "settings.display.fontMonoBlock", mono: true },
  { slot: "monoEditor", labelKey: "settings.display.fontMonoEditor", mono: true },
];

const systemFonts = ref<string[]>([]);

const fontOptions = computed<DropdownOption[]>(() => [
  { value: "", label: t("settings.display.fontDefault") },
  ...systemFonts.value.map((name) => ({
    value: name,
    label: name,
    labelStyle: { fontFamily: name },
  })),
]);

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
    <div class="section-label">{{ t("settings.display.sessionHistoryTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.sessionHistoryDesc") }}</p>

    <div class="choice-row">
      <span class="choice-label">{{ t("settings.display.sessionMessagePageSize") }}</span>
      <BaseDropdown
        class="history-page-size-dropdown"
        :model-value="String(display.sessionMessagePageSize)"
        :options="sessionMessagePageSizeOptions"
        :aria-label="t('settings.display.sessionMessagePageSize')"
        size="sm"
        menu-align="start"
        @update:model-value="setDisplay('sessionMessagePageSize', Number($event))"
      />
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.mainChromeTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.mainChromeDesc") }}</p>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.showWelcomeSubtitle"
        :aria-label="t('settings.display.showWelcomeSubtitle')"
        @update:model-value="setDisplay('showWelcomeSubtitle', $event)"
      />
      <span>{{ t("settings.display.showWelcomeSubtitle") }}</span>
    </div>

    <div class="choice-row">
      <span class="choice-label">{{ t("settings.display.workspaceMode") }}</span>
      <BaseSegmented
        class="choice-segmented"
        :model-value="display.workspaceDisplayMode"
        :options="workspaceDisplayModeOptions"
        :aria-label="t('settings.display.workspaceMode')"
        size="sm"
        @update:model-value="setDisplay('workspaceDisplayMode', $event as WorkspaceDisplayMode)"
      />
    </div>

    <div v-for="item in knowledgeFolderToggles" :key="item.kind" class="toggle-row">
      <BaseSwitch
        :model-value="display.knowledgeFolderVisibility[item.kind]"
        :aria-label="t('settings.display.showKnowledgeFolder', t(item.labelKey))"
        @update:model-value="setKnowledgeFolderVisibility(item.kind, $event)"
      />
      <span>{{ t("settings.display.showKnowledgeFolder", t(item.labelKey)) }}</span>
    </div>

    <div v-for="item in workspaceSectionToggles" :key="`section:${item.kind}`" class="toggle-row">
      <BaseCheckbox
        :model-value="display.workspaceSectionVisibility[item.kind]"
        :aria-label="t('settings.display.showWorkspaceSection', t(item.labelKey))"
        @update:model-value="setWorkspaceSectionVisibility(item.kind, $event)"
      />
      <span>{{ t("settings.display.showWorkspaceSection", t(item.labelKey)) }}</span>
    </div>

    <div class="toggle-row">
      <BaseCheckbox
        :model-value="display.autoPlaceNewPlanDesignKnowledgeDocuments"
        :aria-label="t('settings.display.autoPlaceNewPlanDesignKnowledgeDocuments')"
        @update:model-value="setDisplay('autoPlaceNewPlanDesignKnowledgeDocuments', $event)"
      />
      <span>{{ t("settings.display.autoPlaceNewPlanDesignKnowledgeDocuments") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.showAgentSelector"
        :aria-label="t('settings.display.showAgentSelector')"
        @update:model-value="setDisplay('showAgentSelector', $event)"
      />
      <span>{{ t("settings.display.showAgentSelector") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.showCollabSidebar"
        :aria-label="t('settings.display.showCollabSidebar')"
        @update:model-value="setDisplay('showCollabSidebar', $event)"
      />
      <span>{{ t("settings.display.showCollabSidebar") }}</span>
    </div>

    <div v-for="item in topNavigationToggles" :key="item.key" class="toggle-row">
      <BaseSwitch
        :model-value="display[item.key]"
        :aria-label="t(item.labelKey)"
        @update:model-value="setDisplay(item.key, $event)"
      />
      <span>{{ t(item.labelKey) }}</span>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.fileExplorerTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.fileExplorerDesc") }}</p>

    <div class="directory-filter-row">
      <label class="directory-filter-label" for="file-explorer-hidden-directories">
        {{ t("settings.display.fileExplorerHiddenDirectories") }}
      </label>
      <input
        id="file-explorer-hidden-directories"
        class="directory-filter-input"
        type="text"
        spellcheck="false"
        :value="hiddenDirectoriesText(display.fileExplorerHiddenDirectories)"
        :placeholder="t('settings.display.fileExplorerHiddenDirectoriesPlaceholder')"
        @change="updateHiddenDirectories('fileExplorerHiddenDirectories', $event)"
      />
    </div>

    <div class="directory-filter-row">
      <label class="directory-filter-label" for="unity-file-explorer-hidden-directories">
        {{ t("settings.display.fileExplorerUnityHiddenDirectories") }}
      </label>
      <input
        id="unity-file-explorer-hidden-directories"
        class="directory-filter-input"
        type="text"
        spellcheck="false"
        :value="hiddenDirectoriesText(display.unityFileExplorerHiddenDirectories)"
        :placeholder="t('settings.display.fileExplorerUnityHiddenDirectoriesPlaceholder')"
        @change="updateHiddenDirectories('unityFileExplorerHiddenDirectories', $event)"
      />
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.panelBehaviorTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.panelBehaviorDesc") }}</p>

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
        :model-value="display.fileChangePopoverEnabled"
        :aria-label="t('settings.display.fileChangePopoverEnabled')"
        @update:model-value="setDisplay('fileChangePopoverEnabled', $event)"
      />
      <span>{{ t("settings.display.fileChangePopoverEnabled") }}</span>
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
        :model-value="display.showTurnNavigationRail"
        :aria-label="t('settings.display.showTurnNavigationRail')"
        @update:model-value="setDisplay('showTurnNavigationRail', $event)"
      />
      <span>{{ t("settings.display.showTurnNavigationRail") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.compactToolCalls"
        :aria-label="t('settings.display.compactToolCalls')"
        @update:model-value="setDisplay('compactToolCalls', $event)"
      />
      <span>{{ t("settings.display.compactToolCalls") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.hideThinkingBlocks"
        :aria-label="t('settings.display.hideThinkingBlocks')"
        @update:model-value="setDisplay('hideThinkingBlocks', $event)"
      />
      <span>{{ t("settings.display.hideThinkingBlocks") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.showViewsInSessionPanel"
        :aria-label="t('settings.display.showViewsInSessionPanel')"
        @update:model-value="setDisplay('showViewsInSessionPanel', $event)"
      />
      <span>{{ t("settings.display.showViewsInSessionPanel") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.showViewLogBar"
        :aria-label="t('settings.display.showViewLogBar')"
        @update:model-value="setDisplay('showViewLogBar', $event)"
      />
      <span>{{ t("settings.display.showViewLogBar") }}</span>
    </div>

  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.diffReviewTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.diffReviewDesc") }}</p>

    <div class="choice-row">
      <span class="choice-label">{{ t("settings.display.diffReviewChatTarget") }}</span>
      <BaseSegmented
        class="choice-segmented"
        :model-value="display.chatDiffReviewTarget"
        :options="diffReviewTargetOptions"
        :aria-label="t('settings.display.diffReviewChatTarget')"
        size="sm"
        @update:model-value="setDisplay('chatDiffReviewTarget', $event as DiffReviewTarget)"
      />
    </div>

    <div class="choice-row">
      <span class="choice-label">{{ t("settings.display.diffReviewGitTarget") }}</span>
      <BaseSegmented
        class="choice-segmented"
        :model-value="display.gitDiffReviewTarget"
        :options="diffReviewTargetOptions"
        :aria-label="t('settings.display.diffReviewGitTarget')"
        size="sm"
        @update:model-value="setDisplay('gitDiffReviewTarget', $event as DiffReviewTarget)"
      />
    </div>

    <div class="choice-row">
      <span class="choice-label">{{ t("chat.plan.approvalTargetLabel") }}</span>
      <BaseSegmented
        class="choice-segmented"
        :model-value="display.planApprovalTarget"
        :options="planApprovalTargetOptions"
        :aria-label="t('chat.plan.approvalTargetLabel')"
        size="sm"
        @update:model-value="setDisplay('planApprovalTarget', $event as PlanApprovalTarget)"
      />
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.memoryFileOpenTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.memoryFileOpenDesc") }}</p>

    <div class="choice-row">
      <span class="choice-label">{{ t("settings.display.memoryFileOpenTarget") }}</span>
      <BaseSegmented
        class="choice-segmented"
        :model-value="display.memoryFileOpenTarget"
        :options="memoryFileOpenTargetOptions"
        :aria-label="t('settings.display.memoryFileOpenTarget')"
        size="sm"
        @update:model-value="setDisplay('memoryFileOpenTarget', $event as MemoryFileOpenTarget)"
      />
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.assetRefClickTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.assetRefClickDesc") }}</p>

    <div class="choice-row">
      <span class="choice-label">{{ t("settings.display.assetRefClickTarget") }}</span>
      <BaseDropdown
        class="choice-dropdown"
        :model-value="display.assetRefClickAction"
        :options="assetRefClickActionOptions"
        :aria-label="t('settings.display.assetRefClickTarget')"
        size="sm"
        menu-align="start"
        @update:model-value="setDisplay('assetRefClickAction', $event as AssetRefClickAction)"
      />
    </div>

    <div class="choice-row">
      <span class="choice-label">{{ t("settings.display.assetRefClickUnityEmbedTarget") }}</span>
      <BaseDropdown
        class="choice-dropdown"
        :model-value="display.unityEmbedAssetRefClickAction"
        :options="unityEmbedAssetRefClickActionOptions"
        :aria-label="t('settings.display.assetRefClickUnityEmbedTarget')"
        size="sm"
        menu-align="start"
        @update:model-value="setDisplay('unityEmbedAssetRefClickAction', $event as AssetRefClickAction)"
      />
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.gitViewTitle") }}</div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.mergeGitTreeStatusIcon"
        :aria-label="t('settings.display.mergeGitTreeStatusIcon')"
        @update:model-value="setDisplay('mergeGitTreeStatusIcon', $event)"
      />
      <span>{{ t("settings.display.mergeGitTreeStatusIcon") }}</span>
    </div>

    <div class="toggle-row">
      <BaseSwitch
        :model-value="display.hideGitCommandSuggestions"
        :aria-label="t('settings.display.hideGitCommandSuggestions')"
        @update:model-value="setDisplay('hideGitCommandSuggestions', $event)"
      />
      <span>{{ t("settings.display.hideGitCommandSuggestions") }}</span>
    </div>
  </div>

  <div class="settings-section">
    <div class="section-label">{{ t("settings.display.fontTitle") }}</div>
    <p class="section-desc">{{ t("settings.display.fontDesc") }}</p>

    <div class="font-grid">
      <template v-for="f in fontSlots" :key="f.slot">
        <label class="font-label">{{ t(f.labelKey) }}</label>
        <BaseDropdown
          class="font-select"
          :model-value="display.fonts[f.slot]"
          :options="fontOptions"
          size="md"
          menu-align="start"
          teleport
          :aria-label="t(f.labelKey)"
          @update:model-value="setFont(f.slot, $event)"
        />
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

.choice-row {
  display: grid;
  grid-template-columns: 110px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  width: min(560px, 100%);
  padding: 7px 0;
}

.choice-label {
  font-size: 13px;
  color: var(--text-secondary);
}

.choice-segmented {
  justify-self: start;
  width: fit-content;
  max-width: 100%;
}

.choice-dropdown {
  justify-self: start;
  width: fit-content;
  min-width: 220px;
  max-width: 100%;
}

.history-page-size-dropdown {
  justify-self: start;
  width: 96px;
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

.directory-filter-row {
  display: grid;
  grid-template-columns: 110px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  width: min(560px, 100%);
  padding: 7px 0;
}

.directory-filter-label {
  color: var(--text-secondary);
  font-size: 13px;
}

.directory-filter-input {
  width: 100%;
  min-width: 0;
  height: 30px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: 12px var(--font-mono-identifier);
  outline: none;
}

.directory-filter-input:hover {
  border-color: var(--border-strong);
}

.directory-filter-input:focus {
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 12%, transparent);
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
}
</style>
