<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import { t } from "../../i18n";
import { useTheme, type ThemePreference } from "../../composables/useTheme";
import { useDisplaySettings, type DiffReviewTarget, type FontSlot } from "../../composables/useDisplaySettings";
import { normalizeAppError } from "../../services/errors";
import { ipcInvoke } from "../../services/ipc";
import {
  getViewOpenInExistingWindow,
  getViewWindowsAboveMain,
  setViewOpenInExistingWindow,
  setViewWindowsAboveMain,
} from "../../services/system";
import { useNotificationStore } from "../../stores/notification";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";

const { mainPreference, unityEmbedPreference, setThemePreference } = useTheme();
const { state: display, set: setDisplay, setFont } = useDisplaySettings();
const notificationStore = useNotificationStore();
const viewOpenInExistingWindow = ref(true);
const viewOpenInExistingWindowReady = ref(false);
const viewOpenInExistingWindowBusy = ref(false);
const viewWindowsAboveMain = ref(false);
const viewWindowsAboveMainReady = ref(false);
const viewWindowsAboveMainBusy = ref(false);

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

const diffReviewTargetOptions = computed(() => [
  { value: "inline", label: t("settings.display.diffReviewInline") },
  { value: "window", label: t("settings.display.diffReviewWindow") },
]);

const fontSlots: { slot: FontSlot; labelKey: string; mono: boolean }[] = [
  { slot: "ui",        labelKey: "settings.display.fontUi",        mono: false },
  { slot: "prose",     labelKey: "settings.display.fontProse",     mono: false },
  { slot: "monoInline", labelKey: "settings.display.fontMonoInline", mono: true },
  { slot: "monoBlock", labelKey: "settings.display.fontMonoBlock", mono: true },
  { slot: "monoEditor", labelKey: "settings.display.fontMonoEditor", mono: true },
];

const systemFonts = ref<string[]>([]);

onMounted(async () => {
  void refreshViewOpenInExistingWindow();
  void refreshViewWindowsAboveMain();
  try {
    systemFonts.value = await ipcInvoke<string[]>("get_system_fonts");
  } catch { /* fallback: empty list, user can still type */ }
});

async function refreshViewOpenInExistingWindow() {
  try {
    viewOpenInExistingWindow.value = await getViewOpenInExistingWindow();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadViewOpenInExistingWindow",
    });
  } finally {
    viewOpenInExistingWindowReady.value = true;
  }
}

async function refreshViewWindowsAboveMain() {
  try {
    viewWindowsAboveMain.value = await getViewWindowsAboveMain();
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "loadViewWindowsAboveMain",
    });
  } finally {
    viewWindowsAboveMainReady.value = true;
  }
}

async function updateViewOpenInExistingWindow(value: boolean) {
  if (!viewOpenInExistingWindowReady.value || viewOpenInExistingWindowBusy.value) return;
  const previous = viewOpenInExistingWindow.value;
  viewOpenInExistingWindow.value = value;
  viewOpenInExistingWindowBusy.value = true;
  try {
    await setViewOpenInExistingWindow(value);
  } catch (e) {
    viewOpenInExistingWindow.value = previous;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "saveViewOpenInExistingWindow",
    });
  } finally {
    viewOpenInExistingWindowBusy.value = false;
  }
}

async function updateViewWindowsAboveMain(value: boolean) {
  if (!viewWindowsAboveMainReady.value || viewWindowsAboveMainBusy.value) return;
  const previous = viewWindowsAboveMain.value;
  viewWindowsAboveMain.value = value;
  viewWindowsAboveMainBusy.value = true;
  try {
    await setViewWindowsAboveMain(value);
  } catch (e) {
    viewWindowsAboveMain.value = previous;
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "saveViewWindowsAboveMain",
    });
  } finally {
    viewWindowsAboveMainBusy.value = false;
  }
}
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

    <div class="toggle-row" :class="{ disabled: !viewOpenInExistingWindowReady || viewOpenInExistingWindowBusy }">
      <BaseSwitch
        :model-value="viewOpenInExistingWindow"
        :disabled="!viewOpenInExistingWindowReady || viewOpenInExistingWindowBusy"
        :aria-label="t('settings.display.viewOpenInExistingWindow')"
        @update:model-value="updateViewOpenInExistingWindow"
      />
      <span>{{ t("settings.display.viewOpenInExistingWindow") }}</span>
    </div>

    <div class="toggle-row" :class="{ disabled: !viewWindowsAboveMainReady || viewWindowsAboveMainBusy }">
      <BaseSwitch
        :model-value="viewWindowsAboveMain"
        :disabled="!viewWindowsAboveMainReady || viewWindowsAboveMainBusy"
        :aria-label="t('settings.display.viewWindowsAboveMain')"
        @update:model-value="updateViewWindowsAboveMain"
      />
      <span>{{ t("settings.display.viewWindowsAboveMain") }}</span>
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
