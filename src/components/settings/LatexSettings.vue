<script setup lang="ts">
// Settings > LaTeX: the detected TeX toolchain and the preview options. The
// preview itself has no switch: it starts when a `.tex` preview is shown.
import { computed, onMounted, ref } from "vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { getLatexToolchain, type LatexToolchain } from "../../services/latexPreview";
import BaseButton from "../ui/BaseButton.vue";
import SettingRow from "./SettingRow.vue";
import SettingsSection from "./SettingsSection.vue";

const toolchain = ref<LatexToolchain | null>(null);
const detecting = ref(false);
const detectError = ref("");

async function detect(refresh: boolean) {
  if (detecting.value) return;
  detecting.value = true;
  detectError.value = "";
  try {
    toolchain.value = await getLatexToolchain(refresh);
  } catch (cause) {
    detectError.value = normalizeAppError(cause).message;
  } finally {
    detecting.value = false;
  }
}

onMounted(() => { void detect(false); });

const programs = computed(() => {
  const current = toolchain.value;
  if (!current) return [];
  return [
    { name: "XeLaTeX", path: current.xelatex },
    { name: "pdfLaTeX", path: current.pdflatex },
    { name: "LuaLaTeX", path: current.lualatex },
    { name: "dvisvgm", path: current.dvisvgm },
  ];
});

const distributionLabel = computed(() => {
  const current = toolchain.value;
  if (!current?.distribution) return t("settings.latex.noDistribution");
  const name = current.distribution === "miktex"
    ? "MiKTeX"
    : current.distribution === "texlive"
      ? "TeX Live"
      : t("settings.latex.unknownDistribution");
  return current.version ? `${name} · ${current.version}` : name;
});

const engineOptions = computed(() => [
  { value: "auto", label: t("settings.latex.engine.auto"), hint: t("settings.latex.engine.autoHint") },
  { value: "xelatex", label: "XeLaTeX" },
  { value: "pdflatex", label: "pdfLaTeX" },
  { value: "lualatex", label: "LuaLaTeX", hint: t("settings.latex.engine.lualatexHint") },
]);
</script>

<template>
  <div class="settings-section">
    <SettingsSection id="toolchain" :title="t('settings.latex.toolchain')" :description="t('settings.latex.toolchainDesc')">
      <div class="latex-toolchain" :aria-busy="detecting">
        <div class="latex-toolchain-head">
          <span class="latex-toolchain-distribution">{{ distributionLabel }}</span>
          <BaseButton size="sm" :disabled="detecting" @click="detect(true)">
            {{ detecting ? t("settings.latex.detecting") : t("settings.latex.redetect") }}
          </BaseButton>
        </div>
        <div v-for="program in programs" :key="program.name" class="latex-toolchain-row">
          <span class="latex-toolchain-name">{{ program.name }}</span>
          <span v-if="program.path" class="latex-toolchain-path" :title="program.path">{{ program.path }}</span>
          <span v-else class="latex-toolchain-missing">{{ t("settings.latex.notFound") }}</span>
        </div>
        <div v-if="toolchain && !toolchain.dvisvgm" class="latex-toolchain-note">
          {{ t("settings.latex.dvisvgmMissing") }}
        </div>
        <div v-if="detectError" class="latex-toolchain-error">{{ detectError }}</div>
      </div>
    </SettingsSection>

    <SettingsSection id="preview" :title="t('settings.latex.previewSection')">
      <SettingRow setting-key="latex.engine" :options="engineOptions" control="dropdown" />
      <SettingRow setting-key="latex.preview.svg" />
      <SettingRow setting-key="latex.toolchain.allowPackageInstall" />
    </SettingsSection>
  </div>
</template>

<style scoped>
.latex-toolchain {
  max-width: 760px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 12px;
}
.latex-toolchain-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 4px;
}
.latex-toolchain-distribution {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 500;
}
.latex-toolchain-row {
  display: flex;
  align-items: baseline;
  gap: 12px;
  min-width: 0;
}
.latex-toolchain-name {
  flex: 0 0 72px;
  color: var(--text-color);
}
.latex-toolchain-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
  color: var(--text-secondary);
}
.latex-toolchain-missing,
.latex-toolchain-note {
  color: var(--warning-color, #d29922);
}
.latex-toolchain-error {
  color: var(--danger-color, #e5484d);
}
</style>
