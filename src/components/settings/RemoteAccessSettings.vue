<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import BaseButton from "../ui/BaseButton.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import RemotePairingDialog from "./RemotePairingDialog.vue";
import { t } from "../../i18n";
import { formatSessionTime } from "../../composables/useFormatters";
import { useCopyFeedback } from "../../composables/useCopyFeedback";
import { normalizeAppError } from "../../services/errors";
import { getLocusRuntime } from "../../services/locusRuntime";
import {
  REMOTE_ACCESS_CHANGED_EVENT,
  getRemoteAccessState,
  openRemoteFirewallSettings,
  renameRemoteDevice,
  resetRemoteCertificate,
  revokeAllRemoteDevices,
  revokeRemoteDevice,
  saveRemoteAccessSettings,
  setRemoteDevicePermission,
  settingsInput,
  type RemoteAccessSettingsInput,
  type RemoteAccessState,
  type RemoteDevice,
  type RemoteNetworkInterface,
} from "../../services/remoteAccess";

type Tone = "ok" | "error" | "idle";

const state = ref<RemoteAccessState | null>(null);
const saving = ref(false);
const confirmingEnable = ref(false);
const error = ref("");
const pairingOpen = ref(false);
const portDraft = ref("");
const idleDaysDraft = ref("");
const hostnameDraft = ref("");
const renamingId = ref<string | null>(null);
const renameDraft = ref("");
const now = ref(Date.now());
const commandCopy = useCopyFeedback();
const fingerprintCopy = useCopyFeedback();

const settings = computed(() => state.value?.settings ?? null);
const enabled = computed(() => settings.value?.enabled === true);
const onlineCount = computed(() => state.value?.devices.filter((device) => device.connected).length ?? 0);
const usableInterfaces = computed(() => state.value?.interfaces.filter((item) => item.kind !== "other") ?? []);
const otherInterfaces = computed(() => state.value?.interfaces.filter((item) => item.kind === "other") ?? []);
const hasDirectListener = computed(() => state.value?.listeners.some((listener) => listener.kind !== "loopback") ?? false);
const canPair = computed(() => enabled.value
  && ((state.value?.listeners.some((listener) => listener.secure) ?? false)
    || (settings.value?.hostnames.length ?? 0) > 0));
const appPackage = computed(() => state.value?.appPackage ?? null);

const status = computed<{ text: string; tone: Tone }>(() => {
  if (!state.value || !enabled.value) return { text: t("remoteAccess.status.off"), tone: "idle" };
  if (state.value.lastError) return { text: t("remoteAccess.status.failed"), tone: "error" };
  if (!state.value.running) return { text: t("remoteAccess.status.noListener"), tone: "error" };
  return onlineCount.value > 0
    ? { text: t("remoteAccess.status.onWithDevices", String(onlineCount.value)), tone: "ok" }
    : { text: t("remoteAccess.status.on"), tone: "ok" };
});

const pairHint = computed(() => {
  if (!enabled.value) return t("remoteAccess.pair.enableFirst");
  if (!canPair.value) return t("remoteAccess.pair.needAddress");
  return t("remoteAccess.pair.description");
});

function formatSize(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function syncDrafts() {
  if (!settings.value) return;
  portDraft.value = String(settings.value.port);
  idleDaysDraft.value = String(settings.value.tokenIdleDays);
}

async function load() {
  try {
    state.value = await getRemoteAccessState();
    syncDrafts();
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  }
}

async function save(patch: Partial<RemoteAccessSettingsInput>) {
  if (!settings.value || saving.value) return;
  saving.value = true;
  error.value = "";
  try {
    state.value = await saveRemoteAccessSettings({ ...settingsInput(settings.value), ...patch });
    syncDrafts();
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
    await load();
  } finally {
    saving.value = false;
  }
}

/** Turning remote access on is experimental and exposes this computer; ask first. */
async function setEnabled(next: boolean) {
  if (next && !enabled.value) {
    if (confirmingEnable.value) return;
    confirmingEnable.value = true;
    try {
      const confirmed = await confirm(t("remoteAccess.enableWarning.message"), {
        title: t("remoteAccess.enableWarning.title"),
        kind: "warning",
        okLabel: t("remoteAccess.enableWarning.confirm"),
        cancelLabel: t("common.cancel"),
      });
      if (!confirmed) return;
    } finally {
      confirmingEnable.value = false;
    }
  }
  await save({ enabled: next });
}

async function run(action: () => Promise<unknown>) {
  error.value = "";
  try {
    await action();
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  }
  await load();
}

// ----- network
function interfaceSelected(item: RemoteNetworkInterface): boolean {
  return settings.value?.interfaces.includes(item.name) ?? false;
}

function setInterface(item: RemoteNetworkInterface, selected: boolean) {
  const current = settings.value?.interfaces ?? [];
  const next = selected
    ? [...current.filter((name) => name !== item.name), item.name]
    : current.filter((name) => name !== item.name);
  void save({ interfaces: next });
}

/** Listener state of one bound address; null while not expected to listen. */
function listenerState(address: string): { tone: Tone; text: string } | null {
  if (!state.value || !enabled.value) return null;
  const socket = `${address}:${settings.value?.port}`;
  if (state.value.listeners.some((listener) => listener.address === socket)) {
    return { tone: "ok", text: t("remoteAccess.network.listening") };
  }
  const failure = state.value.listenerErrors.find((item) => item.address === socket);
  return failure ? { tone: "error", text: failure.error } : null;
}

const loopbackUrl = computed(() => `http://127.0.0.1:${settings.value?.port ?? ""}`);
const loopbackState = computed(() => (settings.value?.loopback ? listenerState("127.0.0.1") : null));

function commitPort() {
  const port = Number(portDraft.value.trim());
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    error.value = t("remoteAccess.network.portInvalid");
    syncDrafts();
    return;
  }
  if (port !== settings.value?.port) void save({ port });
}

function addHostname() {
  const value = hostnameDraft.value.trim();
  if (!value || !settings.value) return;
  hostnameDraft.value = "";
  void save({ hostnames: [...settings.value.hostnames, value] });
}

function removeHostname(hostname: string) {
  if (!settings.value) return;
  void save({ hostnames: settings.value.hostnames.filter((item) => item !== hostname) });
}

// ----- security
function commitIdleDays() {
  const days = Number(idleDaysDraft.value.trim());
  if (!Number.isInteger(days) || days < 1 || days > 365) {
    error.value = t("remoteAccess.security.idleInvalid");
    syncDrafts();
    return;
  }
  if (days !== settings.value?.tokenIdleDays) void save({ tokenIdleDays: days });
}

async function resetCertificate() {
  const confirmed = await confirm(t("remoteAccess.security.resetConfirm"), {
    title: t("remoteAccess.security.reset"),
    kind: "warning",
  });
  if (!confirmed) return;
  await run(async () => { state.value = await resetRemoteCertificate(); });
}

// ----- devices
function lastSeen(device: RemoteDevice): string {
  if (device.connected) return t("remoteAccess.devices.online");
  void now.value;
  return t("remoteAccess.devices.lastSeen", formatSessionTime(device.lastSeenAtMs / 1000));
}

function startRename(device: RemoteDevice) {
  renamingId.value = device.id;
  renameDraft.value = device.name;
}

function commitRename(device: RemoteDevice) {
  if (renamingId.value !== device.id) return;
  const name = renameDraft.value.trim();
  renamingId.value = null;
  if (!name || name === device.name) return;
  void run(() => renameRemoteDevice(device.id, name));
}

function setPermission(device: RemoteDevice, allowed: boolean) {
  void run(() => setRemoteDevicePermission(device.id, allowed));
}

async function revoke(device: RemoteDevice) {
  const confirmed = await confirm(t("remoteAccess.devices.revokeConfirm", device.name), {
    title: t("remoteAccess.devices.revoke"),
    kind: "warning",
  });
  if (confirmed) await run(() => revokeRemoteDevice(device.id));
}

async function revokeAll() {
  const confirmed = await confirm(t("remoteAccess.devices.revokeAllConfirm"), {
    title: t("remoteAccess.devices.revokeAll"),
    kind: "warning",
  });
  if (confirmed) await run(() => revokeAllRemoteDevices());
}

// ----- live updates
let releaseChanged: (() => void) | null = null;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;
let clock: ReturnType<typeof setInterval> | null = null;

onMounted(async () => {
  await load();
  releaseChanged = await getLocusRuntime().subscribe(REMOTE_ACCESS_CHANGED_EVENT, () => {
    if (reloadTimer) return;
    reloadTimer = setTimeout(() => {
      reloadTimer = null;
      void load();
    }, 150);
  });
  clock = setInterval(() => { now.value = Date.now(); }, 30_000);
});

onBeforeUnmount(() => {
  releaseChanged?.();
  if (reloadTimer) clearTimeout(reloadTimer);
  if (clock) clearInterval(clock);
});
</script>

<template>
  <!-- Access and pairing -->
  <div class="settings-section ra-section">
    <div class="section-label">
      <span>{{ t("remoteAccess.title") }}</span>
      <span class="settings-badge settings-badge-experimental">{{ t("settings.badge.experimental") }}</span>
    </div>
    <p class="section-desc">{{ t("remoteAccess.subtitle") }}</p>

    <div class="ra-card">
      <div class="ra-row">
        <div class="ra-info">
          <span class="ra-name">{{ t("remoteAccess.enable") }}</span>
          <span class="ra-state" :data-tone="status.tone">{{ status.text }}</span>
        </div>
        <div class="ra-controls">
          <BaseSwitch
            :model-value="enabled"
            :disabled="!state || saving || confirmingEnable"
            :aria-label="t('remoteAccess.enable')"
            @update:model-value="setEnabled"
          />
        </div>
      </div>

      <div class="ra-row" :class="{ 'is-muted': !enabled }">
        <div class="ra-info">
          <span class="ra-name">{{ t("remoteAccess.pair.title") }}</span>
          <span class="ra-desc" :class="{ 'is-warn': enabled && !canPair }">{{ pairHint }}</span>
        </div>
        <div class="ra-controls">
          <BaseButton variant="primary" :disabled="!canPair" @click="pairingOpen = true">
            {{ t("remoteAccess.pair.button") }}
          </BaseButton>
        </div>
      </div>

      <div v-if="appPackage && settings" class="ra-row">
        <div class="ra-info">
          <span class="ra-name">{{ t("remoteAccess.download.title") }}</span>
          <span class="ra-desc">
            {{ t("remoteAccess.download.description", appPackage.version, formatSize(appPackage.sizeBytes)) }}
          </span>
        </div>
        <div class="ra-controls">
          <BaseSwitch
            :model-value="settings.appDownload"
            :disabled="saving"
            :aria-label="t('remoteAccess.download.title')"
            @update:model-value="save({ appDownload: $event })"
          />
        </div>
      </div>
    </div>

    <p v-if="error" class="ra-alert" role="alert">{{ error }}</p>
    <p v-if="enabled && state?.lastError" class="ra-alert">{{ state.lastError }}</p>
  </div>

  <template v-if="state && settings">
    <!-- Devices -->
    <div class="settings-section ra-section">
      <div class="ra-section-head">
        <div class="section-label">{{ t("remoteAccess.devices.title") }}</div>
        <button v-if="state.devices.length > 1" class="ra-link is-danger" type="button" @click="revokeAll">
          {{ t("remoteAccess.devices.revokeAll") }}
        </button>
      </div>
      <p class="section-desc">{{ t("remoteAccess.devices.description") }}</p>

      <div class="ra-card">
        <div v-if="state.devices.length === 0" class="ra-row">
          <span class="ra-desc">{{ t("remoteAccess.devices.empty") }}</span>
        </div>
        <div v-for="device in state.devices" :key="device.id" class="ra-row">
          <div class="ra-info">
            <input
              v-if="renamingId === device.id"
              v-model="renameDraft"
              class="ra-input ra-rename"
              maxlength="64"
              :aria-label="t('common.rename')"
              @keydown.enter="commitRename(device)"
              @keydown.esc="renamingId = null"
              @blur="commitRename(device)"
            >
            <button
              v-else
              class="ra-name ra-device-name"
              type="button"
              :title="t('common.rename')"
              @click="startRename(device)"
            >
              {{ device.name }}
            </button>
            <span class="ra-state" :data-tone="device.connected ? 'ok' : 'idle'">
              {{ lastSeen(device) }}<template v-if="device.lastAddress"> · {{ device.lastAddress }}</template>
            </span>
          </div>
          <div class="ra-controls">
            <label class="ra-inline-switch" :title="t('remoteAccess.devices.fullControlHint')">
              <span>{{ t("remoteAccess.devices.fullControl") }}</span>
              <BaseSwitch
                :model-value="device.fullControl"
                :aria-label="t('remoteAccess.devices.fullControl')"
                @update:model-value="setPermission(device, $event)"
              />
            </label>
            <BaseButton variant="danger" @click="revoke(device)">{{ t("remoteAccess.devices.revoke") }}</BaseButton>
          </div>
        </div>
      </div>
    </div>

    <!-- Network -->
    <div class="settings-section ra-section">
      <div class="section-label">{{ t("remoteAccess.network.title") }}</div>
      <p class="section-desc">{{ t("remoteAccess.network.description") }}</p>

      <div class="ra-card">
        <div class="ra-group">
          <span class="ra-group-title">{{ t("remoteAccess.network.direct") }}</span>
          <span class="ra-desc">{{ t("remoteAccess.network.directHint") }}</span>
        </div>
        <div v-if="usableInterfaces.length === 0" class="ra-row">
          <span class="ra-desc">{{ t("remoteAccess.network.noInterfaces") }}</span>
        </div>
        <div v-for="item in usableInterfaces" :key="item.name" class="ra-row">
          <div class="ra-info">
            <span class="ra-name">
              {{ item.name }}
              <span class="ra-tag" :data-kind="item.kind">
                {{ item.kind === "tailscale" ? "Tailscale" : t("remoteAccess.network.lan") }}
              </span>
            </span>
            <span v-for="address in item.addresses" :key="address" class="ra-address">
              <code>https://{{ address }}:{{ settings.port }}</code>
              <span
                v-if="interfaceSelected(item) && listenerState(address)"
                class="ra-state"
                :data-tone="listenerState(address)!.tone"
              >{{ listenerState(address)!.text }}</span>
            </span>
          </div>
          <div class="ra-controls">
            <BaseSwitch
              :model-value="interfaceSelected(item)"
              :disabled="saving"
              :aria-label="item.name"
              @update:model-value="setInterface(item, $event)"
            />
          </div>
        </div>
        <div v-if="hasDirectListener" class="ra-row">
          <span class="ra-desc is-warn">{{ t("remoteAccess.network.firewall") }}</span>
          <div class="ra-controls">
            <BaseButton @click="run(openRemoteFirewallSettings)">{{ t("remoteAccess.network.openFirewall") }}</BaseButton>
          </div>
        </div>
        <div v-if="otherInterfaces.length > 0" class="ra-row">
          <span class="ra-desc">
            {{ t("remoteAccess.network.otherInterfaces", otherInterfaces.map((item) => item.name).join(", ")) }}
          </span>
        </div>
      </div>

      <div class="ra-card">
        <div class="ra-group">
          <span class="ra-group-title">{{ t("remoteAccess.network.tunnel") }}</span>
          <span class="ra-desc">{{ t("remoteAccess.network.tunnelHint") }}</span>
        </div>
        <div class="ra-row">
          <div class="ra-info">
            <span class="ra-name">{{ t("remoteAccess.network.loopback") }}</span>
            <span class="ra-desc">{{ t("remoteAccess.network.loopbackHint") }}</span>
            <span v-if="settings.loopback" class="ra-address">
              <code>{{ loopbackUrl }}</code>
              <span v-if="loopbackState" class="ra-state" :data-tone="loopbackState.tone">{{ loopbackState.text }}</span>
            </span>
          </div>
          <div class="ra-controls">
            <BaseSwitch
              :model-value="settings.loopback"
              :disabled="saving"
              :aria-label="t('remoteAccess.network.loopback')"
              @update:model-value="save({ loopback: $event })"
            />
          </div>
        </div>
        <div v-if="settings.loopback" class="ra-row">
          <div class="ra-info">
            <span class="ra-name">{{ t("remoteAccess.network.serveCommand") }}</span>
            <code class="ra-code">{{ state.tailscaleServeCommand }}</code>
          </div>
          <div class="ra-controls">
            <BaseButton @click="commandCopy.copyText(state.tailscaleServeCommand)">
              {{ commandCopy.copied.value ? t("common.copied") : t("remoteAccess.copy") }}
            </BaseButton>
          </div>
        </div>
        <div class="ra-row ra-row-stack">
          <div class="ra-info">
            <span class="ra-name">{{ t("remoteAccess.network.hostnames") }}</span>
            <span class="ra-desc">{{ t("remoteAccess.network.hostnamesHint") }}</span>
          </div>
          <ul v-if="settings.hostnames.length > 0" class="ra-chips">
            <li v-for="hostname in settings.hostnames" :key="hostname" class="ra-chip">
              <code>https://{{ hostname }}</code>
              <button type="button" :aria-label="t('remoteAccess.remove')" @click="removeHostname(hostname)">
                <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
              </button>
            </li>
          </ul>
          <form class="ra-add" @submit.prevent="addHostname">
            <input
              v-model="hostnameDraft"
              class="ra-input"
              :placeholder="t('remoteAccess.network.hostnamePlaceholder')"
              :aria-label="t('remoteAccess.network.hostnames')"
              spellcheck="false"
              autocapitalize="off"
            >
            <BaseButton type="submit" :disabled="!hostnameDraft.trim() || saving">{{ t("remoteAccess.add") }}</BaseButton>
          </form>
        </div>
      </div>

      <div class="ra-card">
        <div class="ra-row">
          <div class="ra-info">
            <span class="ra-name">{{ t("remoteAccess.network.port") }}</span>
            <span class="ra-desc">{{ t("remoteAccess.network.portHint") }}</span>
          </div>
          <div class="ra-controls">
            <input
              v-model="portDraft"
              class="ra-input ra-number"
              inputmode="numeric"
              :aria-label="t('remoteAccess.network.port')"
              @keydown.enter="($event.target as HTMLInputElement).blur()"
              @blur="commitPort"
            >
          </div>
        </div>
      </div>
    </div>

    <!-- Security -->
    <div class="settings-section ra-section">
      <div class="section-label">{{ t("remoteAccess.security.title") }}</div>

      <div class="ra-card">
        <div class="ra-row">
          <div class="ra-info">
            <span class="ra-name">{{ t("remoteAccess.security.confirm") }}</span>
            <span class="ra-desc">{{ t("remoteAccess.security.confirmHint") }}</span>
          </div>
          <div class="ra-controls">
            <BaseSwitch
              :model-value="settings.requireConfirmation"
              :disabled="saving"
              :aria-label="t('remoteAccess.security.confirm')"
              @update:model-value="save({ requireConfirmation: $event })"
            />
          </div>
        </div>
        <div class="ra-row">
          <div class="ra-info">
            <span class="ra-name">{{ t("remoteAccess.security.idle") }}</span>
            <span class="ra-desc">{{ t("remoteAccess.security.idleHint") }}</span>
          </div>
          <div class="ra-controls">
            <input
              v-model="idleDaysDraft"
              class="ra-input ra-number"
              inputmode="numeric"
              :aria-label="t('remoteAccess.security.idle')"
              @keydown.enter="($event.target as HTMLInputElement).blur()"
              @blur="commitIdleDays"
            >
          </div>
        </div>
        <div class="ra-row">
          <div class="ra-info">
            <span class="ra-name">{{ t("remoteAccess.fingerprint") }}</span>
            <span class="ra-desc">{{ t("remoteAccess.security.fingerprintHint") }}</span>
            <code v-if="state.fingerprint" class="ra-code ra-fingerprint">{{ state.fingerprint }}</code>
            <span v-else class="ra-desc">{{ t("remoteAccess.security.noCertificate") }}</span>
          </div>
          <div class="ra-controls">
            <BaseButton v-if="state.fingerprint" @click="fingerprintCopy.copyText(state.fingerprint)">
              {{ fingerprintCopy.copied.value ? t("common.copied") : t("remoteAccess.copy") }}
            </BaseButton>
            <BaseButton variant="danger" @click="resetCertificate">{{ t("remoteAccess.security.reset") }}</BaseButton>
          </div>
        </div>
      </div>
    </div>
  </template>

  <RemotePairingDialog v-if="pairingOpen && state" :state="state" @close="pairingOpen = false; load()" />
</template>

<style scoped>
/* Row-card layout shared with the other settings pages (see McpServerSettings). */
.ra-section {
  display: flex;
  flex-direction: column;
}

.ra-section + .ra-section {
  padding-top: 4px;
}

.ra-section-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  max-width: 760px;
}

.ra-card {
  display: flex;
  flex-direction: column;
  max-width: 760px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  overflow: hidden;
}

.ra-card + .ra-card {
  margin-top: 10px;
}

.ra-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 40px;
  padding: 11px 16px;
}

.ra-row + .ra-row,
.ra-group + .ra-row {
  border-top: 1px solid var(--border-color);
}

.ra-row.is-muted .ra-info {
  opacity: 0.6;
}

.ra-row-stack {
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
}

.ra-group {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 16px;
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--sidebar-bg) 30%);
}

.ra-group-title {
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.ra-info {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
}

.ra-name {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--text-color);
  font-size: 12.5px;
  font-weight: 600;
}

.ra-desc {
  color: var(--text-secondary);
  font-size: 11.5px;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.ra-desc.is-warn {
  color: var(--status-warn-fg);
}

.ra-controls {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 10px;
}

/* Status: colored dot + text. */
.ra-state {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--text-secondary);
  font-size: 11.5px;
}

.ra-state::before {
  content: "";
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-tertiary);
}

.ra-state[data-tone="ok"] {
  color: var(--status-good-fg);
}

.ra-state[data-tone="ok"]::before {
  background: var(--status-good-fg);
}

.ra-state[data-tone="error"] {
  color: var(--status-danger-fg);
}

.ra-state[data-tone="error"]::before {
  background: var(--status-danger-fg);
}

.ra-address {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}

.ra-address code,
.ra-code,
.ra-chip code {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11.5px;
}

.ra-code {
  max-width: 100%;
  overflow-wrap: anywhere;
}

.ra-fingerprint {
  margin-top: 2px;
  color: var(--text-color);
}

.ra-tag {
  padding: 0 6px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  font-size: 10.5px;
  font-weight: 500;
  line-height: 16px;
}

.ra-tag[data-kind="tailscale"] {
  background: var(--accent-soft);
  color: var(--accent-color);
}

.ra-device-name {
  max-width: 100%;
  overflow: hidden;
  padding: 0;
  border: 0;
  border-bottom: 1px dashed transparent;
  background: none;
  text-align: left;
  white-space: nowrap;
  text-overflow: ellipsis;
  cursor: text;
}

.ra-device-name:hover {
  border-bottom-color: var(--text-tertiary);
}

.ra-inline-switch {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--text-secondary);
  font-size: 12px;
  white-space: nowrap;
  cursor: pointer;
}

.ra-link {
  padding: 0;
  border: 0;
  background: none;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
}

.ra-link.is-danger:hover {
  color: var(--status-danger-fg);
}

.ra-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.ra-chip {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px 2px 2px 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
}

.ra-chip button {
  display: inline-flex;
  width: 20px;
  height: 20px;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.ra-chip button:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.ra-chip svg {
  width: 10px;
  height: 10px;
  stroke: currentColor;
  stroke-width: 1.8;
  stroke-linecap: round;
}

.ra-add {
  display: flex;
  gap: 8px;
}

.ra-input {
  min-width: 0;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 12px;
}

.ra-add .ra-input {
  flex: 1;
}

.ra-input:focus {
  border-color: var(--accent-color);
  outline: none;
}

.ra-number {
  width: 92px;
  font-family: var(--font-mono-identifier);
  text-align: right;
}

.ra-rename {
  width: min(280px, 100%);
}

.ra-alert {
  max-width: 760px;
  margin: 10px 0 0;
  padding: 8px 12px;
  border: 1px solid var(--status-danger-border);
  border-radius: 8px;
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
  overflow-wrap: anywhere;
}
</style>
