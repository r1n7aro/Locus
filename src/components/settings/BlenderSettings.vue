<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "../../i18n";
import { useServiceSwitches } from "../../composables/useServiceSwitches";
import {
  addBlenderInstallation,
  closeBlenderBackground,
  installBlenderAddon,
  listBlenderBackgroundInstances,
  listBlenderInstallations,
  refreshBlenderInstallations,
  removeBlenderInstallation,
  setDefaultBlenderInstallation,
  subscribeBlenderInstallationsChanged,
  subscribeBlenderInstancesChanged,
  unhideBlenderInstallation,
  type BlenderBackgroundInstance,
  type BlenderInstallation,
} from "../../services/blender";
import { blenderErrorText } from "../../services/blenderErrors";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import {
  getWorkspaceResourceLimits,
  setWorkspaceResourceLimits,
  type WorkspaceResourceLimits,
} from "../../services/worktrees";
import BaseButton from "../ui/BaseButton.vue";
import ServiceSwitchCard from "./ServiceSwitchCard.vue";

const { switches, busy, setEnabled } = useServiceSwitches();
const enabled = computed(() => switches.value?.blender ?? false);

const installations = ref<BlenderInstallation[]>([]);
const background = ref<BlenderBackgroundInstance[]>([]);
const limits = ref<WorkspaceResourceLimits | null>(null);
const loading = ref(false);
const actionBusy = ref("");
const error = ref("");
// The block the last error belongs to, so it shows next to its action.
const errorSection = ref<"installations" | "background">("installations");
const showHidden = ref(false);

function showError(cause: unknown, section: "installations" | "background") {
  error.value = blenderErrorText(cause);
  errorSection.value = section;
}
let unsubscribers: Promise<RuntimeUnsubscribe>[] = [];

const visibleInstallations = computed(() => installations.value.filter((item) => showHidden.value || !item.hidden));
const hiddenCount = computed(() => installations.value.filter((item) => item.hidden).length);

async function load() {
  loading.value = true;
  try {
    const [list, running, policy] = await Promise.all([
      listBlenderInstallations(),
      listBlenderBackgroundInstances(),
      getWorkspaceResourceLimits(),
    ]);
    installations.value = list;
    background.value = running;
    limits.value = policy.limits;
  } catch (cause) {
    showError(cause, "installations");
  } finally {
    loading.value = false;
  }
}

async function reloadInstallations() {
  try {
    installations.value = await listBlenderInstallations();
  } catch (cause) {
    showError(cause, "installations");
  }
}

async function reloadBackground() {
  try {
    background.value = await listBlenderBackgroundInstances();
  } catch {
    // The list refreshes with the next event.
  }
}

function stopListening() {
  for (const pending of unsubscribers) void pending.then((unsubscribe) => unsubscribe(), () => undefined);
  unsubscribers = [];
}

// Probing runs Blender, so nothing loads while the integration is off.
watch(enabled, (value) => {
  stopListening();
  if (!value) return;
  unsubscribers = [
    subscribeBlenderInstallationsChanged(() => void reloadInstallations()),
    subscribeBlenderInstancesChanged(() => void reloadBackground()),
  ];
  void load();
}, { immediate: true });

onUnmounted(stopListening);

async function run(key: string, action: () => Promise<unknown>) {
  if (actionBusy.value) return;
  actionBusy.value = key;
  error.value = "";
  try {
    await action();
    await reloadInstallations();
  } catch (cause) {
    showError(cause, key.startsWith("close:") ? "background" : "installations");
  } finally {
    actionBusy.value = "";
  }
}

async function addInstallation() {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Blender", extensions: ["exe"] }],
  });
  if (typeof selected !== "string" || !selected) return;
  await run("add", () => addBlenderInstallation(selected));
}

/** The command reports per-installation results instead of failing. */
async function installAddon(id: string) {
  const reports = await installBlenderAddon(id);
  const failed = reports.find((report) => !report.ok);
  if (failed) {
    throw { code: "blender.install_failed", message: "", detail: failed.error ?? "", retryable: false, severity: "error" };
  }
}

function refresh() {
  void run("refresh", async () => {
    installations.value = await refreshBlenderInstallations();
  });
}

function versionLabel(item: BlenderInstallation): string {
  if (item.probeState === "pending") return t("settings.blender.probing");
  if (item.probeState === "failed") return t("settings.blender.probeFailed");
  const probe = item.probe;
  if (!probe) return "?";
  // Official builds come from main or a release branch; anything else is a
  // custom build worth showing (e.g. "4.2.1 farlocus"), unless the version
  // string already names the build ("4.2.1 farlocus-0.1").
  const display = item.displayVersion ?? probe.versionString;
  const custom = probe.buildBranch
    && !/\s/.test(display)
    && !["main", "master"].includes(probe.buildBranch)
    && !/^blender-v\d+\.\d+-release$/.test(probe.buildBranch);
  return `${display}${custom ? ` ${probe.buildBranch}` : ""}`;
}

function sourceLabel(item: BlenderInstallation): string {
  return item.sources.map((source) => t(`settings.blender.source.${source}`)).join(", ");
}

function addonLabel(item: BlenderInstallation): string {
  return item.addon ? t(`settings.blender.addon.${item.addon.status}`) : "";
}

function canInstall(item: BlenderInstallation): boolean {
  return item.probeState === "ready"
    && (item.support === "supported" || item.support === "unverified")
    && item.addon?.status !== "up_to_date";
}

async function updateLimit(field: "blenderMaxBackgroundInstances" | "blenderBackgroundIdleTimeoutSecs", event: Event) {
  const input = event.target as HTMLInputElement;
  if (!limits.value) return;
  const value = Number(input.value);
  const stored = field === "blenderBackgroundIdleTimeoutSecs" ? value * 60 : value;
  if (!Number.isSafeInteger(stored) || value < 1) {
    input.value = String(limitDisplay(field));
    return;
  }
  try {
    limits.value = (await setWorkspaceResourceLimits({ ...limits.value, [field]: stored })).limits;
  } catch (cause) {
    showError(cause, "background");
    input.value = String(limitDisplay(field));
  }
}

function limitDisplay(field: "blenderMaxBackgroundInstances" | "blenderBackgroundIdleTimeoutSecs"): number {
  const value = limits.value?.[field];
  if (field === "blenderBackgroundIdleTimeoutSecs") return Math.ceil((value ?? 600) / 60);
  return value ?? 2;
}
</script>

<template>
  <div class="settings-section">
    <ServiceSwitchCard
      :label="t('settings.services.blenderEnabled')"
      :description="t('settings.services.blenderEnabledDesc')"
      :model-value="enabled"
      :ready="switches !== null"
      :busy="busy"
      @update:model-value="setEnabled('blender', $event)"
    />

    <template v-if="enabled">
      <section class="blender-settings-block" :aria-busy="loading">
        <div class="blender-settings-head">
          <div class="section-label">{{ t("settings.blender.installations") }}</div>
          <div class="blender-settings-actions">
            <BaseButton size="sm" variant="neutral" :disabled="Boolean(actionBusy)" @click="refresh">
              {{ t("settings.blender.refresh") }}
            </BaseButton>
            <BaseButton size="sm" variant="primary" :disabled="Boolean(actionBusy)" @click="addInstallation">
              {{ actionBusy === "add" ? t("settings.blender.probing") : t("settings.blender.add") }}
            </BaseButton>
          </div>
        </div>
        <div v-if="visibleInstallations.length === 0 && !loading" class="blender-empty">
          {{ t("settings.blender.none") }}
        </div>
        <div v-for="item in visibleInstallations" :key="item.id" class="blender-install-row" :class="{ 'is-hidden': item.hidden }">
          <div class="blender-install-info">
            <span class="blender-install-title">
              {{ versionLabel(item) }}
              <span v-if="item.isDefault" class="blender-badge">{{ t("settings.blender.default") }}</span>
              <span v-if="item.support !== 'supported' && item.probeState === 'ready'" class="blender-badge is-warning">
                {{ t(`settings.blender.support.${item.support}`) }}
              </span>
            </span>
            <span class="blender-install-path" :title="item.path">{{ item.path }}</span>
            <span class="blender-install-meta">
              {{ sourceLabel(item) }}<template v-if="item.probe"> · Python {{ item.probe.python }}</template><template v-if="addonLabel(item)"> · {{ addonLabel(item) }}</template>
            </span>
            <span v-if="item.probeError" class="blender-install-error">{{ item.probeError }}</span>
          </div>
          <div class="blender-install-actions">
            <BaseButton
              v-if="canInstall(item)"
              size="sm"
              variant="primary"
              :disabled="Boolean(actionBusy)"
              @click="run(`install:${item.id}`, () => installAddon(item.id))"
            >
              {{ item.addon?.status === "outdated" ? t("settings.blender.updateAddon") : t("settings.blender.installAddon") }}
            </BaseButton>
            <BaseButton
              v-if="!item.isDefault && !item.hidden && item.probeState === 'ready'"
              size="sm"
              variant="neutral"
              :disabled="Boolean(actionBusy)"
              @click="run(`default:${item.id}`, () => setDefaultBlenderInstallation(item.id))"
            >
              {{ t("settings.blender.setDefault") }}
            </BaseButton>
            <BaseButton
              v-if="item.hidden"
              size="sm"
              variant="neutral"
              :disabled="Boolean(actionBusy)"
              @click="run(`unhide:${item.id}`, () => unhideBlenderInstallation(item.id))"
            >
              {{ t("settings.blender.unhide") }}
            </BaseButton>
            <BaseButton
              v-else
              size="sm"
              variant="neutral"
              :disabled="Boolean(actionBusy)"
              @click="run(`remove:${item.id}`, () => removeBlenderInstallation(item.id))"
            >
              {{ item.sources.includes("manual") ? t("settings.blender.remove") : t("settings.blender.hide") }}
            </BaseButton>
          </div>
        </div>
        <button v-if="hiddenCount > 0" type="button" class="blender-link" @click="showHidden = !showHidden">
          {{ showHidden ? t("settings.blender.hideHidden") : t("settings.blender.showHidden", hiddenCount) }}
        </button>
        <p v-if="error && (errorSection === 'installations' || !limits)" class="blender-error" role="alert">{{ error }}</p>
      </section>

      <section v-if="limits" class="blender-settings-block">
        <div class="section-label">{{ t("settings.blender.background") }}</div>
        <div class="blender-limit-grid">
          <label>
            <span>{{ t("settings.blender.maxBackground") }}</span>
            <input type="number" min="1" max="8" step="1" :value="limitDisplay('blenderMaxBackgroundInstances')"
              @wheel="($event.target as HTMLInputElement).blur()" @change="updateLimit('blenderMaxBackgroundInstances', $event)" />
          </label>
          <label>
            <span>{{ t("settings.blender.idleMinutes") }}</span>
            <input type="number" min="1" step="1" :value="limitDisplay('blenderBackgroundIdleTimeoutSecs')"
              @wheel="($event.target as HTMLInputElement).blur()" @change="updateLimit('blenderBackgroundIdleTimeoutSecs', $event)" />
          </label>
        </div>
        <div v-for="item in background" :key="item.pid" class="blender-install-row">
          <div class="blender-install-info">
            <span class="blender-install-title">Blender {{ item.version }} · PID {{ item.pid }}</span>
            <span class="blender-install-meta">{{ item.file || t("settings.blender.noFile") }}</span>
          </div>
          <BaseButton
            size="sm"
            variant="danger"
            :disabled="Boolean(actionBusy)"
            @click="run(`close:${item.pid}`, async () => { await closeBlenderBackground(item.pid); await reloadBackground(); })"
          >
            {{ t("settings.blender.close") }}
          </BaseButton>
        </div>
        <p v-if="error && errorSection === 'background'" class="blender-error" role="alert">{{ error }}</p>
      </section>
    </template>
  </div>
</template>

<style scoped>
.blender-settings-block {
  max-width: 760px;
  margin-top: 20px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.blender-settings-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.blender-settings-actions,
.blender-install-actions {
  display: flex;
  gap: 6px;
  flex: 0 0 auto;
}

.section-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.blender-empty {
  color: var(--text-secondary);
  font-size: 12px;
}

.blender-install-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
}

.blender-install-row.is-hidden {
  opacity: 0.6;
}

.blender-install-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.blender-install-title {
  font-size: 13px;
  color: var(--text-color);
}

.blender-install-path,
.blender-install-meta {
  color: var(--text-secondary);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.blender-install-path {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.blender-install-error,
.blender-error {
  color: var(--status-danger-fg);
  font-size: 12px;
}

.blender-badge {
  margin-left: 6px;
  padding: 0 5px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--accent-color) 14%, transparent);
  color: var(--accent-color);
  font-size: 11px;
}

.blender-badge.is-warning {
  background: color-mix(in srgb, var(--status-warn-fg) 14%, transparent);
  color: var(--status-warn-fg);
}

.blender-link {
  align-self: flex-start;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  padding: 0;
}

.blender-limit-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 12px 24px;
}

.blender-limit-grid label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  color: var(--text-secondary);
  font-size: 13px;
}

.blender-limit-grid input {
  width: 68px;
  padding: 5px 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--bg-color));
  color: var(--text-color);
  font-family: var(--font-ui);
}
</style>
