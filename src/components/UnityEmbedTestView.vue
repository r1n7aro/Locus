<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { getLocusRuntime } from "../services/locusRuntime";
import { t } from "../i18n";
import BaseButton from "./ui/BaseButton.vue";

interface PingResponse {
  ok: boolean;
  runtime: string;
  message: string;
  port?: number;
  pipeName?: string;
  windowLabel?: string;
  control?: {
    updateCount: number;
    lastType: string;
    lastRect: string;
    lastParentHwnd: number;
    lastChildHwnd: number;
    lastVisible: boolean;
    lastMounted: boolean;
    lastError: string;
    lastUpdateMsAgo?: number;
  };
}

const runtime = getLocusRuntime();
const loading = ref(false);
const error = ref<string | null>(null);
const ping = ref<PingResponse | null>(null);
const invokePing = ref<PingResponse | null>(null);

const statusText = computed(() => {
  if (error.value) return t("unity.embed.status.failed");
  if (loading.value) return t("unity.embed.status.checking");
  if (ping.value?.ok) return t("unity.embed.status.connected");
  return t("unity.embed.status.pending");
});

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    ping.value = await runtime.invoke<PingResponse>("unity_embed_status");
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
    ping.value = null;
  } finally {
    loading.value = false;
  }
}

async function runInvokePing() {
  error.value = null;
  try {
    invokePing.value = await runtime.invoke<PingResponse>("unity_embed_status");
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
    invokePing.value = null;
  }
}

onMounted(() => {
  void refresh();
});
</script>

<template>
  <main class="unity-embed-test">
    <header class="uet-header">
      <div class="uet-title-block">
        <h1>Locus Unity Embed Test</h1>
        <span>{{ statusText }}</span>
      </div>
      <div class="uet-actions">
        <BaseButton @click="runInvokePing">Invoke Ping</BaseButton>
        <BaseButton @click="refresh">Refresh</BaseButton>
      </div>
    </header>

    <section class="uet-body">
      <aside class="uet-pane uet-side">
        <div class="uet-row">
          <span>Runtime</span>
          <strong>{{ runtime.kind }}</strong>
        </div>
        <div class="uet-row">
          <span>Bridge</span>
          <strong>{{ ping?.pipeName || "empty" }}</strong>
        </div>
        <div class="uet-row">
          <span>Status</span>
          <strong>{{ ping?.message || "empty" }}</strong>
        </div>
        <div class="uet-row">
          <span>Invoke Ping</span>
          <strong>{{ invokePing?.message || "empty" }}</strong>
        </div>
        <div class="uet-row" v-if="ping?.control">
          <span>Rect</span>
          <strong>{{ ping.control.lastRect || "empty" }}</strong>
        </div>
        <div class="uet-row" v-if="ping?.control">
          <span>Mounted</span>
          <strong>{{ ping.control.lastMounted ? "yes" : "no" }}</strong>
        </div>
      </aside>

      <section class="uet-pane uet-main">
        <div class="uet-section">
          <h2>Overlay</h2>
          <dl>
            <div v-if="ping?.control">
              <dt>Updates</dt>
              <dd>{{ ping.control.updateCount }}</dd>
            </div>
            <div v-if="ping?.control">
              <dt>Parent HWND</dt>
              <dd>{{ ping.control.lastParentHwnd || "empty" }}</dd>
            </div>
            <div v-if="ping?.control">
              <dt>Child HWND</dt>
              <dd>{{ ping.control.lastChildHwnd || "empty" }}</dd>
            </div>
            <div v-if="ping?.control">
              <dt>Last Error</dt>
              <dd>{{ ping.control.lastError || "empty" }}</dd>
            </div>
          </dl>
        </div>

        <div class="uet-section" v-if="error">
          <h2>Error</h2>
          <pre>{{ error }}</pre>
        </div>
      </section>
    </section>
  </main>
</template>

<style scoped>
.unity-embed-test {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  background: var(--bg-color);
  color: var(--text-color);
}

.uet-header {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 42px;
  padding: 0 12px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.uet-title-block {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
}

.uet-title-block h1 {
  font-size: 13px;
  font-weight: 650;
  line-height: 1;
}

.uet-title-block span {
  color: var(--text-secondary);
  font-size: 12px;
}

.uet-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: auto;
}

.uet-body {
  display: grid;
  grid-template-columns: minmax(220px, 280px) minmax(0, 1fr);
  flex: 1;
  min-height: 0;
}

.uet-pane {
  min-width: 0;
  min-height: 0;
}

.uet-side {
  padding: 10px;
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 92%, var(--bg-color) 8%);
}

.uet-main {
  overflow: auto;
  padding: 14px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
}

.uet-row {
  display: grid;
  grid-template-columns: 78px minmax(0, 1fr);
  gap: 8px;
  padding: 7px 4px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  font-size: 12px;
}

.uet-row span,
.uet-section dt {
  color: var(--text-secondary);
}

.uet-row strong {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}

.uet-section {
  max-width: 860px;
}

.uet-section + .uet-section {
  margin-top: 16px;
}

.uet-section h2 {
  margin-bottom: 10px;
  font-size: 13px;
  font-weight: 650;
}

.uet-section dl {
  display: grid;
  gap: 0;
  border-top: 1px solid var(--border-color);
}

.uet-section dl > div {
  display: grid;
  grid-template-columns: 110px minmax(0, 1fr);
  gap: 10px;
  padding: 8px 0;
  border-bottom: 1px solid var(--border-color);
  font-size: 12px;
}

.uet-section dd {
  min-width: 0;
  overflow-wrap: anywhere;
}

.uet-section pre {
  margin: 0;
  padding: 10px;
  overflow: auto;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
  user-select: text;
}

@media (max-width: 620px) {
  .uet-body {
    grid-template-columns: 1fr;
  }

  .uet-side {
    border-right: 0;
    border-bottom: 1px solid var(--border-color);
  }
}
</style>
