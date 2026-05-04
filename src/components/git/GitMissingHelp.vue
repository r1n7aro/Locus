<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { GitInstallHelp, GitProbeResult } from "../../types";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import {
  gitClearOverride,
  gitInstallHelp,
  gitInstallVia,
  gitSetOverride,
} from "../../services/git";

const props = defineProps<{
  probe: GitProbeResult | null;
}>();

const emit = defineEmits<{
  (e: "resolved"): void;
}>();

const help = ref<GitInstallHelp | null>(null);
const helpLoading = ref(false);
const actionError = ref("");
const actionOutput = ref("");
const installBusy = ref("");
const overrideBusy = ref(false);

const hasOverride = computed(() => !!props.probe?.envOverride);

async function ensureHelpLoaded() {
  if (helpLoading.value || help.value || props.probe?.available) return;
  helpLoading.value = true;
  try {
    help.value = await gitInstallHelp();
  } catch (e) {
    actionError.value = normalizeAppError(e).message;
  } finally {
    helpLoading.value = false;
  }
}

function formatCommandOutput(stdout: string, stderr: string) {
  const blocks = [stdout.trim(), stderr.trim()].filter(Boolean);
  return blocks.join("\n\n");
}

async function installWith(managerId: string) {
  installBusy.value = managerId;
  actionError.value = "";
  actionOutput.value = "";
  try {
    const result = await gitInstallVia(managerId);
    actionOutput.value = formatCommandOutput(result.stdout, result.stderr) || t("git.install.commandCompleted");
    if (result.exitCode !== 0) {
      actionError.value = t("git.install.commandFailed", result.exitCode);
      return;
    }
    emit("resolved");
  } catch (e) {
    actionError.value = normalizeAppError(e).message;
  } finally {
    installBusy.value = "";
  }
}

async function setOverride(path: string) {
  overrideBusy.value = true;
  actionError.value = "";
  actionOutput.value = "";
  try {
    const resolved = await gitSetOverride(path);
    actionOutput.value = t("git.install.overrideSuccess", resolved);
    emit("resolved");
  } catch (e) {
    actionError.value = normalizeAppError(e).message;
  } finally {
    overrideBusy.value = false;
  }
}

async function pickGitExecutable() {
  const options: Parameters<typeof open>[0] = {
    multiple: false,
    directory: false,
  };
  if (help.value?.os === "windows") {
    options.filters = [
      {
        name: "Git",
        extensions: ["exe", "cmd", "bat"],
      },
    ];
  }
  const selected = await open(options);
  if (typeof selected === "string" && selected.trim()) {
    await setOverride(selected);
  }
}

async function pickGitDirectory() {
  const selected = await open({ multiple: false, directory: true });
  if (typeof selected === "string" && selected.trim()) {
    await setOverride(selected);
  }
}

async function clearOverride() {
  overrideBusy.value = true;
  actionError.value = "";
  actionOutput.value = "";
  try {
    await gitClearOverride();
    actionOutput.value = t("git.install.overrideCleared");
    emit("resolved");
  } catch (e) {
    actionError.value = normalizeAppError(e).message;
  } finally {
    overrideBusy.value = false;
  }
}

async function openOfficial() {
  if (help.value?.officialUrl) {
    await openUrl(help.value.officialUrl);
  }
}

async function openChinaMirror() {
  if (help.value?.chinaMirrorUrl) {
    await openUrl(help.value.chinaMirrorUrl);
  }
}

watch(() => props.probe?.available, () => {
  if (!props.probe?.available) {
    void ensureHelpLoaded();
  }
}, { immediate: true });

onMounted(() => {
  void ensureHelpLoaded();
});
</script>

<template>
  <div v-if="probe && !probe.available" class="git-missing-help">
    <div class="git-help-title">{{ t("git.install.title") }}</div>
    <div class="git-help-desc">{{ t("git.install.desc") }}</div>

    <div class="git-help-section">
      <div class="git-help-label">{{ t("git.install.findExisting") }}</div>
      <div class="git-help-actions">
        <button class="git-help-btn" :disabled="overrideBusy" @click="pickGitExecutable">
          {{ t("git.install.pickExecutable") }}
        </button>
        <button class="git-help-btn" :disabled="overrideBusy" @click="pickGitDirectory">
          {{ t("git.install.pickDirectory") }}
        </button>
        <button
          class="git-help-btn secondary"
          :disabled="overrideBusy || !!installBusy"
          @click="emit('resolved')"
        >
          {{ t("git.install.refresh") }}
        </button>
        <button
          v-if="hasOverride"
          class="git-help-btn secondary"
          :disabled="overrideBusy"
          @click="clearOverride"
        >
          {{ t("git.install.clearOverride") }}
        </button>
      </div>
    </div>

    <div v-if="helpLoading" class="git-help-note">{{ t("common.loading") }}</div>

    <template v-else-if="help">
      <div v-if="help.packageManagers.length" class="git-help-section">
        <div class="git-help-label">{{ t("git.install.packageManagers") }}</div>
        <div class="git-help-grid">
          <button
            v-for="manager in help.packageManagers"
            :key="manager.id"
            class="git-help-card"
            :class="{ unavailable: !manager.available }"
            :disabled="!manager.available || !!installBusy || overrideBusy"
            @click="installWith(manager.id)"
          >
            <span class="git-help-card-title">
              {{ installBusy === manager.id ? t("git.install.running") : manager.label }}
            </span>
            <span class="git-help-card-command">
              {{ manager.available ? manager.command : t("git.install.managerUnavailable") }}
            </span>
          </button>
        </div>
      </div>

      <div class="git-help-section">
        <div class="git-help-label">{{ t("git.install.download") }}</div>
        <div class="git-help-actions">
          <button class="git-help-btn" @click="openOfficial">{{ t("git.install.openOfficial") }}</button>
          <button
            v-if="help.chinaMirrorUrl"
            class="git-help-btn secondary"
            @click="openChinaMirror"
          >
            {{ t("git.install.openChinaMirror") }}
          </button>
        </div>
        <div class="git-help-note">{{ t("git.install.regionHint") }}</div>
      </div>
    </template>

    <div v-if="actionError" class="git-help-error">{{ actionError }}</div>
    <pre v-if="actionOutput" class="git-help-output">{{ actionOutput }}</pre>
  </div>
</template>

<style scoped>
.git-missing-help {
  width: min(760px, 100%);
  margin-top: 12px;
  padding: 14px;
  border: 1px solid var(--border-color);
  border-radius: 12px;
  background: color-mix(in srgb, var(--hover-bg) 70%, transparent);
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.git-help-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.git-help-desc,
.git-help-note {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.git-help-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.git-help-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.git-help-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.git-help-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 8px;
}

.git-help-btn,
.git-help-card {
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--bg-color);
  color: var(--text-color);
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s, opacity 0.15s;
}

.git-help-btn {
  padding: 8px 12px;
  font-size: 12px;
  font-weight: 500;
}

.git-help-card {
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  text-align: left;
}

.git-help-btn:hover:not(:disabled),
.git-help-card:hover:not(:disabled) {
  border-color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-color) 6%, transparent);
}

.git-help-btn.secondary {
  background: transparent;
}

.git-help-card.unavailable,
.git-help-btn:disabled,
.git-help-card:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.git-help-card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.git-help-card-command {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
  white-space: normal;
  word-break: break-word;
}

.git-help-error {
  padding: 8px 10px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--git-status-deleted) 10%, transparent);
  color: var(--git-status-deleted);
  font-size: 12px;
}

.git-help-output {
  margin: 0;
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 220px;
  overflow: auto;
}
</style>
