<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import BaseButton from "./ui/BaseButton.vue";
import { t } from "../i18n";
import { useRemotePairingRequests } from "../composables/useRemotePairingRequests";

// App-level approval prompt for a phone pairing request. The pairing dialog
// in Settings answers inline while it is open; this covers every other case.
const { current, dialogOpen, responding, respond } = useRemotePairingRequests();
const now = ref(Date.now());
let ticker: ReturnType<typeof setInterval> | null = null;

const visible = computed(() => !!current.value && !dialogOpen.value);
const secondsLeft = computed(() =>
  current.value ? Math.max(0, Math.ceil((current.value.expiresAtMs - now.value) / 1000)) : 0);

function stopTicker() {
  if (ticker) clearInterval(ticker);
  ticker = null;
}

// Ticks only while a request is pending.
watch(current, (request) => {
  stopTicker();
  if (!request) return;
  now.value = Date.now();
  ticker = setInterval(() => {
    now.value = Date.now();
    // The desktop denies on timeout; drop the prompt with it.
    if (current.value && current.value.expiresAtMs <= now.value) current.value = null;
  }, 1000);
}, { immediate: true });

onBeforeUnmount(stopTicker);
</script>

<template>
  <div v-if="visible && current" class="rpp-backdrop">
    <div class="rpp-card" role="alertdialog" aria-modal="true" :aria-label="t('remoteAccess.pairing.requestTitle', current.deviceName)">
      <div class="rpp-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24"><rect x="7" y="2.5" width="10" height="19" rx="2.5" /><path d="M11 18.5h2" /></svg>
      </div>
      <h3>{{ t("remoteAccess.pairing.requestTitle", current.deviceName) }}</h3>
      <p class="rpp-meta">{{ t("remoteAccess.pairing.requestFrom", current.address, current.platform) }}</p>
      <p class="rpp-warning">{{ t("remoteAccess.pairing.requestWarning") }}</p>
      <div class="rpp-actions">
        <BaseButton :disabled="responding" @click="respond(false)">{{ t("remoteAccess.pairing.deny") }}</BaseButton>
        <BaseButton variant="primary" :disabled="responding" @click="respond(true)">
          {{ t("remoteAccess.pairing.allow") }} ({{ secondsLeft }})
        </BaseButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.rpp-backdrop {
  position: fixed;
  inset: 0;
  z-index: 2000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgb(0 0 0 / 45%);
}

.rpp-card {
  display: flex;
  width: min(380px, calc(100vw - 48px));
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 22px 20px 18px;
  border: 1px solid var(--border-color);
  border-radius: 14px;
  background: var(--surface-elevated);
  box-shadow: 0 18px 48px rgb(0 0 0 / 30%);
  text-align: center;
}

.rpp-icon {
  display: flex;
  width: 52px;
  height: 52px;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--accent-soft);
  color: var(--accent-color);
}

.rpp-icon svg {
  width: 26px;
  height: 26px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.8;
  stroke-linecap: round;
}

h3 {
  margin: 8px 0 0;
  font-size: 15px;
  font-weight: 600;
}

.rpp-meta {
  margin: 0;
  color: var(--text-secondary);
  font-size: 13px;
}

.rpp-warning {
  margin: 4px 0 0;
  color: var(--status-warn-fg);
  font-size: 12px;
}

.rpp-actions {
  display: flex;
  gap: 10px;
  margin-top: 12px;
}
</style>
