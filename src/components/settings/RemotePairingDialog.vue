<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import BaseButton from "../ui/BaseButton.vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import {
  cancelRemotePairing,
  startRemotePairing,
  type RemoteAccessState,
  type RemotePairingView,
} from "../../services/remoteAccess";
import { useRemotePairingRequests } from "../../composables/useRemotePairingRequests";

const props = defineProps<{ state: RemoteAccessState }>();
const emit = defineEmits<{ close: [] }>();

const { current: request, dialogOpen, responding, respond } = useRemotePairingRequests();
const view = ref<RemotePairingView | null>(null);
const error = ref("");
const now = ref(Date.now());
const knownDeviceIds = ref(new Set(props.state.devices.map((device) => device.id)));
let ticker: ReturnType<typeof setInterval> | null = null;
let closeTimer: ReturnType<typeof setTimeout> | null = null;

const remainingSeconds = computed(() =>
  view.value ? Math.max(0, Math.ceil((view.value.expiresAtMs - now.value) / 1000)) : 0);
const pairedDevice = computed(() =>
  props.state.devices.find((device) => !knownDeviceIds.value.has(device.id)) ?? null);
const groupedCode = computed(() =>
  view.value ? `${view.value.code.slice(0, 3)} ${view.value.code.slice(3)}` : "");

type Phase = "loading" | "waiting" | "confirm" | "done" | "expired" | "error";
const phase = computed<Phase>(() => {
  if (pairedDevice.value) return "done";
  if (error.value) return "error";
  if (!view.value) return "loading";
  if (request.value) return "confirm";
  const sessionOpen = props.state.pairing?.sessionId === view.value.sessionId;
  if (!sessionOpen || remainingSeconds.value === 0) return "expired";
  return "waiting";
});

async function start() {
  error.value = "";
  view.value = null;
  knownDeviceIds.value = new Set(props.state.devices.map((device) => device.id));
  try {
    view.value = await startRemotePairing();
    now.value = Date.now();
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  }
}

function close() {
  emit("close");
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") close();
}

watch(phase, (next) => {
  if (next === "done") closeTimer = setTimeout(close, 1800);
});

onMounted(() => {
  dialogOpen.value = true;
  ticker = setInterval(() => { now.value = Date.now(); }, 500);
  window.addEventListener("keydown", onKeydown);
  void start();
});

onBeforeUnmount(() => {
  dialogOpen.value = false;
  if (ticker) clearInterval(ticker);
  if (closeTimer) clearTimeout(closeTimer);
  window.removeEventListener("keydown", onKeydown);
  // Leaving mid-pairing withdraws the credentials shown on screen.
  if (phase.value !== "done") void cancelRemotePairing().catch(() => {});
});
</script>

<template>
  <div class="ra-dialog-backdrop" @click.self="close">
    <div class="ra-dialog" role="dialog" aria-modal="true" :aria-label="t('remoteAccess.pairing.title')">
      <header class="ra-dialog-header">
        <h3>{{ t("remoteAccess.pairing.title") }}</h3>
        <button class="ra-dialog-close" type="button" :aria-label="t('common.close')" @click="close">
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
        </button>
      </header>

      <div v-if="phase === 'loading'" class="ra-dialog-body ra-center">
        <span class="ra-muted">{{ t("remoteAccess.pairing.preparing") }}</span>
      </div>

      <div v-else-if="phase === 'error'" class="ra-dialog-body ra-center">
        <p class="ra-error">{{ error }}</p>
        <BaseButton variant="primary" @click="start">{{ t("remoteAccess.pairing.retry") }}</BaseButton>
      </div>

      <div v-else-if="phase === 'waiting' && view" class="ra-dialog-body">
        <div class="ra-qr-block">
          <!-- SVG rendered by the desktop from its own pairing link. -->
          <div class="ra-qr" v-html="view.qrSvg" />
          <div class="ra-qr-side">
            <ol class="ra-steps">
              <li>{{ t("remoteAccess.pairing.step1") }}</li>
              <li>{{ t("remoteAccess.pairing.step2") }}</li>
              <li>{{ t("remoteAccess.pairing.step3") }}</li>
            </ol>
            <p v-if="view.appDownload" class="ra-no-app">{{ t("remoteAccess.pairing.noApp") }}</p>
            <div class="ra-countdown" :class="{ 'is-urgent': remainingSeconds <= 20 }">
              {{ t("remoteAccess.pairing.expiresIn", String(remainingSeconds)) }}
            </div>
          </div>
        </div>
        <div v-if="view.codeUsable" class="ra-code-block">
          <span class="ra-code-label">{{ t("remoteAccess.pairing.codeLabel") }}</span>
          <span class="ra-code">{{ groupedCode }}</span>
        </div>
        <details class="ra-details">
          <summary>{{ t("remoteAccess.pairing.details") }}</summary>
          <dl>
            <dt>{{ t("remoteAccess.fingerprint") }}</dt>
            <dd><code class="ra-fingerprint">{{ view.fingerprint }}</code></dd>
            <dt>{{ t("remoteAccess.pairing.addresses") }}</dt>
            <dd>
              <ul class="ra-address-list">
                <li v-for="address in view.addresses" :key="address.u">
                  <code>{{ address.u }}</code>
                  <span class="ra-tag">{{ address.pin ? t("remoteAccess.pairing.pinned") : t("remoteAccess.pairing.caTrusted") }}</span>
                </li>
              </ul>
            </dd>
          </dl>
        </details>
      </div>

      <div v-else-if="phase === 'confirm' && request" class="ra-dialog-body ra-confirm">
        <div class="ra-device-avatar" aria-hidden="true">
          <svg viewBox="0 0 24 24"><rect x="7" y="2.5" width="10" height="19" rx="2.5" /><path d="M11 18.5h2" /></svg>
        </div>
        <p class="ra-confirm-title">{{ t("remoteAccess.pairing.requestTitle", request.deviceName) }}</p>
        <p class="ra-muted">{{ t("remoteAccess.pairing.requestFrom", request.address, request.platform) }}</p>
        <p class="ra-warning">{{ t("remoteAccess.pairing.requestWarning") }}</p>
        <div class="ra-actions">
          <BaseButton :disabled="responding" @click="respond(false)">{{ t("remoteAccess.pairing.deny") }}</BaseButton>
          <BaseButton variant="primary" :disabled="responding" @click="respond(true)">{{ t("remoteAccess.pairing.allow") }}</BaseButton>
        </div>
      </div>

      <div v-else-if="phase === 'done'" class="ra-dialog-body ra-center">
        <div class="ra-success" aria-hidden="true">
          <svg viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" /></svg>
        </div>
        <p class="ra-confirm-title">{{ t("remoteAccess.pairing.done", pairedDevice?.name ?? "") }}</p>
      </div>

      <div v-else class="ra-dialog-body ra-center">
        <p class="ra-muted">{{ t("remoteAccess.pairing.expired") }}</p>
        <BaseButton variant="primary" @click="start">{{ t("remoteAccess.pairing.regenerate") }}</BaseButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ra-dialog-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgb(0 0 0 / 45%);
}

.ra-dialog {
  width: min(520px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  overflow: auto;
  border: 1px solid var(--border-color);
  border-radius: 14px;
  background: var(--surface-elevated);
  box-shadow: 0 18px 48px rgb(0 0 0 / 30%);
}

.ra-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 18px 4px;
}

.ra-dialog-header h3 {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}

.ra-dialog-close {
  display: inline-flex;
  width: 28px;
  height: 28px;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.ra-dialog-close:hover {
  background: var(--hover-bg);
}

.ra-dialog-close svg {
  width: 14px;
  height: 14px;
  stroke: currentColor;
  stroke-width: 1.6;
  stroke-linecap: round;
}

.ra-dialog-body {
  padding: 12px 18px 18px;
}

.ra-center {
  display: flex;
  min-height: 180px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  text-align: center;
}

.ra-qr-block {
  display: flex;
  align-items: center;
  gap: 18px;
}

.ra-qr {
  flex-shrink: 0;
  width: 200px;
  height: 200px;
  padding: 8px;
  border-radius: 10px;
  background: #fff;
}

.ra-qr :deep(svg) {
  display: block;
  width: 100%;
  height: 100%;
}

.ra-qr-side {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.ra-steps {
  margin: 0;
  padding-left: 18px;
  color: var(--text-secondary);
  font-size: 13px;
  line-height: 1.7;
}

.ra-no-app {
  margin: 0;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.ra-countdown {
  color: var(--text-secondary);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

.ra-countdown.is-urgent {
  color: var(--status-warn-fg);
}

.ra-code-block {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 14px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--panel-bg);
}

.ra-code-label {
  color: var(--text-secondary);
  font-size: 12px;
}

.ra-code {
  font-family: var(--font-mono-display);
  font-size: 22px;
  font-weight: 600;
  letter-spacing: 0.12em;
  font-variant-numeric: tabular-nums;
}

.ra-details {
  margin-top: 14px;
  color: var(--text-secondary);
  font-size: 12px;
}

.ra-details summary {
  cursor: pointer;
}

.ra-details dl {
  margin: 8px 0 0;
}

.ra-details dt {
  margin-top: 8px;
  font-weight: 500;
}

.ra-details dd {
  margin: 4px 0 0;
}

.ra-fingerprint {
  word-break: break-all;
  font-size: 11px;
}

.ra-address-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.ra-address-list li {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 0;
}

.ra-tag {
  padding: 0 6px;
  border-radius: 999px;
  background: var(--hover-bg);
  font-size: 11px;
}

.ra-confirm {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  text-align: center;
}

.ra-device-avatar,
.ra-success {
  display: flex;
  width: 52px;
  height: 52px;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--accent-soft);
  color: var(--accent-color);
}

.ra-success {
  background: var(--status-good-bg);
  color: var(--status-good-fg);
}

.ra-device-avatar svg,
.ra-success svg {
  width: 26px;
  height: 26px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.8;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.ra-confirm-title {
  margin: 6px 0 0;
  font-size: 15px;
  font-weight: 600;
}

.ra-muted {
  margin: 0;
  color: var(--text-secondary);
  font-size: 13px;
}

.ra-warning {
  margin: 6px 0 0;
  color: var(--status-warn-fg);
  font-size: 12px;
}

.ra-error {
  margin: 0;
  color: var(--status-danger-fg);
  font-size: 13px;
}

.ra-actions {
  display: flex;
  gap: 10px;
  margin-top: 12px;
}
</style>
