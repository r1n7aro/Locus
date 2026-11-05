
<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import { Search, X } from "lucide";
import type { ModelOption, AgentInfo } from "../types";
import { t, locale } from "../i18n";
import { useSettingsSchema } from "../composables/useSettingsStore";
import { useSearchScope } from "../composables/useSearchScope";
import {
  DEFAULT_SETTINGS_PAGE,
  SETTINGS_GROUPS,
  buildSettingsSearchIndex,
  searchSettings,
  settingsPage,
  settingsPagesInGroup,
  type SettingsPageId,
  type SettingsSearchResult,
} from "./settings/nav";
import "./settings/settingsView.css";
import NotificationsSettings from "./settings/NotificationsSettings.vue";
import ShortcutSettings from "./settings/ShortcutSettings.vue";
import ConsoleSettings from "./settings/ConsoleSettings.vue";
import AboutSettings from "./settings/AboutSettings.vue";
import ProxySettings from "./settings/ProxySettings.vue";
import CustomProviderModal from "./settings/CustomProviderModal.vue";
import ModelUsageStats from "./settings/ModelUsageStats.vue";
import McpSettings from "./settings/McpSettings.vue";
import McpServerSettings from "./settings/McpServerSettings.vue";
import RemoteAccessSettings from "./settings/RemoteAccessSettings.vue";
import UnitySettings from "./settings/UnitySettings.vue";
import BlenderSettings from "./settings/BlenderSettings.vue";
import AgentBehaviorPage from "./settings/pages/AgentBehaviorPage.vue";
import AppearancePage from "./settings/pages/AppearancePage.vue";
import ChatSettingsPage from "./settings/pages/ChatSettingsPage.vue";
import DebugSettingsPage from "./settings/pages/DebugSettingsPage.vue";
import ModelDefaultsPage from "./settings/pages/ModelDefaultsPage.vue";
import PermissionsPage from "./settings/pages/PermissionsPage.vue";
import ProvidersPage from "./settings/pages/ProvidersPage.vue";
import SystemSettingsPage from "./settings/pages/SystemSettingsPage.vue";
import WorkbenchSettingsPage from "./settings/pages/WorkbenchSettingsPage.vue";
import WorkspacePage from "./settings/pages/WorkspacePage.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { resetToOnboarding } from "./settings/resetToOnboarding";
import { useUiStore } from "../stores/ui";
import { useCustomProviderSettingsStore } from "../stores/customProviderSettings";
import { useSettingsFeedbackStore } from "../stores/settingsFeedback";

// Loaded when the LaTeX page is opened.
const LatexSettings = defineAsyncComponent(() => import("./settings/LatexSettings.vue"));

const props = withDefaults(defineProps<{
  active?: boolean;
  allModels: ModelOption[];
  agents: AgentInfo[];
  subagents: AgentInfo[];
}>(), {
  active: true,
});

const emit = defineEmits<{
  authChanged: [];
  resetOnboarding: [];
}>();

const uiStore = useUiStore();
const feedback = useSettingsFeedbackStore();
const customProviderSettings = useCustomProviderSettingsStore();
const {
  editingCustomProvider,
  isAddingCustomProvider,
  customProviderSaving,
  testStatus,
  testResult,
  modelCatalog,
  modelCatalogLoading,
  modelCatalogRefreshing,
} = storeToRefs(customProviderSettings);
const activeCategory = ref<SettingsPageId>(DEFAULT_SETTINGS_PAGE);
const panelRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);

// Provider sign-in changes reach the host, which reloads auth and models.
watch(() => feedback.authRevision, () => emit("authChanged"));

// ── reset ───────────────────────────────────────────────────────────────

const resetConfirm = ref(false);

async function handleResetOnboarding() {
  await resetToOnboarding();
  resetConfirm.value = false;
  activeCategory.value = DEFAULT_SETTINGS_PAGE;
  emit("authChanged");
  emit("resetOnboarding");
}

// ── navigation ──────────────────────────────────────────────────────────

const TARGET_HIGHLIGHT_MS = 1600;

/** Scrolls to a section once the page has rendered it (pages may load async). */
async function revealSection(sectionId: string) {
  await nextTick();
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const target = contentRef.value?.querySelector<HTMLElement>(`[data-settings-section="${sectionId}"]`);
    if (target) {
      target.scrollIntoView({ block: "start", behavior: "smooth" });
      target.classList.add("settings-section-target");
      window.setTimeout(() => target.classList.remove("settings-section-target"), TARGET_HIGHLIGHT_MS);
      return;
    }
    await new Promise((resolve) => requestAnimationFrame(resolve));
  }
  // Pages that are a single block have no section anchors.
  contentRef.value?.scrollTo({ top: 0 });
}

function openPage(page: SettingsPageId, section?: string | null) {
  closeSearch(false);
  activeCategory.value = page;
  if (section) void revealSection(section);
  else contentRef.value?.scrollTo({ top: 0 });
}

watch(
  () => uiStore.settingsCategoryHint,
  (hint) => {
    if (!hint) return;
    openPage(hint.page, hint.section);
    uiStore.clearSettingsCategoryHint();
  },
  { immediate: true },
);

// ── search ──────────────────────────────────────────────────────────────
// Hidden until the find shortcut (Ctrl+F) is pressed inside the settings
// panel; it then opens above the content and lists matches in its place.

const searchOpen = ref(false);
const searchInputRef = ref<HTMLInputElement | null>(null);
const selectedResult = ref(0);
const searchQuery = ref("");
const schema = useSettingsSchema();
const searchIndex = computed(() => {
  void locale.value;
  const entries = schema.value;
  const keys = entries ? [...entries.keys()] : [];
  const experimental = new Set(keys.filter((key) => entries?.get(key)?.experimental));
  return buildSettingsSearchIndex(t, keys, experimental);
});
const searchActive = computed(() => searchQuery.value.trim().length > 0);
const searchResults = computed(() => searchSettings(searchIndex.value, searchQuery.value, t));

function resultTitle(result: SettingsSearchResult): string {
  if (result.settingLabel) return result.settingLabel;
  const page = settingsPage(result.pageId);
  const section = page.sections.find((item) => item.id === result.sectionId);
  return t(section?.titleKey ?? page.labelKey);
}

function resultPath(result: SettingsSearchResult): string {
  const page = settingsPage(result.pageId);
  const section = page.sections.find((item) => item.id === result.sectionId);
  const pageLabel = t(page.labelKey);
  if (!section || !result.settingLabel) return pageLabel;
  const sectionLabel = t(section.titleKey);
  return sectionLabel === pageLabel ? pageLabel : `${pageLabel} › ${sectionLabel}`;
}

function openResult(result: SettingsSearchResult) {
  openPage(result.pageId, result.sectionId);
}

async function openSearch() {
  searchOpen.value = true;
  await nextTick();
  searchInputRef.value?.focus();
  searchInputRef.value?.select();
}

/** Hides the search bar; `refocus` keeps the find shortcut in this panel. */
function closeSearch(refocus = true) {
  if (!searchOpen.value) return;
  searchOpen.value = false;
  searchQuery.value = "";
  if (refocus) panelRef.value?.focus({ preventScroll: true });
}

watch(searchQuery, () => {
  selectedResult.value = 0;
});

function moveSelection(step: number) {
  const count = searchResults.value.length;
  if (count === 0) return;
  selectedResult.value = (selectedResult.value + step + count) % count;
  void nextTick(() => {
    contentRef.value
      ?.querySelector(".settings-search-result.selected")
      ?.scrollIntoView({ block: "nearest" });
  });
}

function openSelectedResult() {
  const result = searchResults.value[selectedResult.value];
  if (result) openResult(result);
}

useSearchScope(panelRef, () => void openSearch(), { enabled: () => props.active });

// Entering the page from the top navigation leaves focus outside the panel;
// move it in so the find shortcut opens this search instead of the global one.
watch(
  () => props.active,
  (active) => {
    if (!active) return;
    void nextTick(() => {
      const panel = panelRef.value;
      if (panel && !panel.contains(panel.ownerDocument.activeElement)) panel.focus({ preventScroll: true });
    });
  },
  { immediate: true },
);
</script>

<template>
  <div ref="panelRef" class="settings-panel" tabindex="-1">
    <div class="settings-sidebar">
      <nav class="sidebar-nav" :class="{ 'is-cjk': locale === 'zh' }">
        <template v-for="group in SETTINGS_GROUPS" :key="group.id">
          <div class="sidebar-group-label">{{ t(group.labelKey) }}</div>
          <button
            v-for="page in settingsPagesInGroup(group.id)"
            :key="page.id"
            type="button"
            class="sidebar-item"
            :class="{ active: activeCategory === page.id }"
            :data-settings-page="page.id"
            @click="openPage(page.id as SettingsPageId)"
          >
            <LucideIcon :icon="page.icon" :size="14" />
            <span>{{ t(page.labelKey) }}</span>
          </button>
        </template>
      </nav>
    </div>

    <div ref="contentRef" class="settings-content">
      <div v-if="searchOpen" class="settings-search-bar">
        <div class="settings-search">
          <LucideIcon :icon="Search" :size="14" />
          <input
            ref="searchInputRef"
            v-model="searchQuery"
            type="search"
            spellcheck="false"
            autocomplete="off"
            :placeholder="t('settings.search.placeholder')"
            :aria-label="t('settings.search.placeholder')"
            @keydown.escape.stop.prevent="closeSearch()"
            @keydown.enter.prevent="openSelectedResult"
            @keydown.down.prevent="moveSelection(1)"
            @keydown.up.prevent="moveSelection(-1)"
          />
          <button
            type="button"
            class="settings-search-clear"
            :title="t('settings.search.close')"
            :aria-label="t('settings.search.close')"
            @click="closeSearch()"
          >
            <LucideIcon :icon="X" :size="13" />
          </button>
        </div>
      </div>

      <div v-if="searchActive" class="settings-search-results" :class="{ 'is-cjk': locale === 'zh' }">
        <button
          v-for="(result, index) in searchResults"
          :key="`${result.pageId}/${result.sectionId ?? ''}/${result.settingLabel ?? ''}`"
          type="button"
          class="settings-search-result"
          :class="{ selected: index === selectedResult }"
          @click="openResult(result)"
          @mouseenter="selectedResult = index"
        >
          <span class="settings-search-result-title">{{ resultTitle(result) }}</span>
          <span class="settings-search-result-path">{{ resultPath(result) }}</span>
        </button>
        <div v-if="searchResults.length === 0" class="settings-search-empty">
          {{ t("settings.search.noResults") }}
        </div>
      </div>

      <div v-show="!searchActive" class="settings-page-host">
      <Transition name="fade">
        <div v-if="feedback.success" class="success-msg">{{ feedback.success }}</div>
      </Transition>

      <ProvidersPage v-if="activeCategory === 'providers'" :all-models="allModels" />

      <ModelDefaultsPage
        v-else-if="activeCategory === 'modelDefaults'"
        :all-models="allModels"
        :agents="agents"
        :subagents="subagents"
      />

      <ModelUsageStats v-else-if="activeCategory === 'modelUsage'" />

      <AgentBehaviorPage v-else-if="activeCategory === 'agentBehavior'" :agents="agents" />

      <PermissionsPage v-else-if="activeCategory === 'permissions'" />

      <McpSettings v-else-if="activeCategory === 'mcp'" />

      <WorkspacePage v-else-if="activeCategory === 'workspace'" />

      <UnitySettings v-else-if="activeCategory === 'unity'" />

      <BlenderSettings v-else-if="activeCategory === 'blender'" />

      <LatexSettings v-else-if="activeCategory === 'latex'" />

      <AppearancePage v-else-if="activeCategory === 'appearance'" />

      <ChatSettingsPage v-else-if="activeCategory === 'chat'" />

      <WorkbenchSettingsPage v-else-if="activeCategory === 'workbench'" />

      <NotificationsSettings v-else-if="activeCategory === 'notifications'" />

      <ShortcutSettings v-else-if="activeCategory === 'shortcuts'" />

      <ProxySettings v-else-if="activeCategory === 'proxy'" />

      <McpServerSettings v-else-if="activeCategory === 'mcpServer'" />

      <RemoteAccessSettings v-else-if="activeCategory === 'remoteAccess'" />

      <SystemSettingsPage
        v-else-if="activeCategory === 'system'"
        :reset-confirm="resetConfirm"
        @start-reset="resetConfirm = true"
        @confirm-reset="handleResetOnboarding"
        @cancel-reset="resetConfirm = false"
      />

      <DebugSettingsPage v-else-if="activeCategory === 'debug'" />

      <ConsoleSettings v-else-if="props.active && activeCategory === 'logs'" />

      <AboutSettings v-else-if="activeCategory === 'about'" />

      <div v-if="feedback.error" class="error-msg">{{ feedback.error }}</div>
      </div>
    </div><!-- end settings-content -->

    <CustomProviderModal
      v-model:provider="editingCustomProvider"
      :is-adding="isAddingCustomProvider"
      :saving="customProviderSaving"
      :test-status="testStatus"
      :test-result="testResult"
      :catalog="modelCatalog"
      :catalog-loading="modelCatalogLoading"
      :catalog-refreshing="modelCatalogRefreshing"
      @close="customProviderSettings.cancelEditCustomProvider"
      @save="customProviderSettings.saveCustomProvider"
      @test="customProviderSettings.testCustomProvider"
      @open-catalog="customProviderSettings.loadModelCatalog()"
      @refresh-catalog="customProviderSettings.refreshCatalog"
    />

  </div>
</template>

<style scoped>
/* Shared settings styles: ./settings/settingsView.css */
.settings-panel {
  flex: 1;
  display: flex;
  flex-direction: row;
  background: var(--bg-color);
  height: 100%;
  overflow: hidden;
  outline: none;
}
</style>
