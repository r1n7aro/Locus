
<script setup lang="ts">
import { ref, onUnmounted } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../i18n";
import { useCopyFeedback } from "../composables/useCopyFeedback";
import { normalizeAppError } from "../services/errors";
import { saveApiKey, codexStartLogin, codexPollLogin } from "../services/auth";

const emit = defineEmits<{
  loggedIn: [];
  skip: [];
}>();

const activeTab = ref<"apikey" | "codex">("apikey");

// ── OpenRouter API Key ──
const apiKey = ref("");
const apiKeyError = ref("");
const apiKeyLoading = ref(false);

async function submitKey() {
  const key = apiKey.value.trim();
  if (!key) { apiKeyError.value = t("login.enterApiKey"); return; }
  apiKeyError.value = "";
  apiKeyLoading.value = true;
  try {
    await saveApiKey(key);
    emit("loggedIn");
  } catch (e) {
    apiKeyError.value = t("login.saveFailed", normalizeAppError(e).message);
  } finally {
    apiKeyLoading.value = false;
  }
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submitKey(); }
}


type CodexStep = "idle" | "opening" | "waiting" | "success" | "error";

const codexStep = ref<CodexStep>("idle");
const codexUserCode = ref("");
const codexUrl = ref("");
const codexDeviceAuthId = ref("");
const codexInterval = ref(5);
const codexError = ref("");
const { copied: codexCodeCopied, copyText: copyCodexText, reset: resetCodexCopyState } = useCopyFeedback();
let codexPollTimer: ReturnType<typeof setTimeout> | null = null;
let codexPollInFlight = false;

function stopPoll() {
  if (codexPollTimer) {
    clearTimeout(codexPollTimer);
    codexPollTimer = null;
  }
  codexPollInFlight = false;
}

function schedulePoll(delayMs = codexInterval.value * 1000) {
  if (codexPollTimer) clearTimeout(codexPollTimer);
  codexPollTimer = setTimeout(() => {
    codexPollTimer = null;
    void pollCodex();
  }, delayMs);
}

async function startCodexLogin() {
  if (codexStep.value === "opening" || codexStep.value === "waiting") return;
  stopPoll();
  resetCodexCopyState();
  codexError.value = "";
  codexStep.value = "opening";
  try {
    const info = await codexStartLogin();

    codexUserCode.value = info.userCode;
    codexUrl.value = info.url;
    codexDeviceAuthId.value = info.deviceAuthId;
    codexInterval.value = Math.max(info.interval, 5);
    codexStep.value = "waiting";

    void openUrl(info.url).catch(() => undefined);

    schedulePoll();
  } catch (e) {
    codexStep.value = "error";
    codexError.value = t("login.loginFailed", normalizeAppError(e).message);
  }
}

async function pollCodex() {
  if (codexPollInFlight || codexStep.value !== "waiting") return;
  codexPollInFlight = true;
  try {
    const result = await codexPollLogin(codexDeviceAuthId.value, codexUserCode.value);

    switch (result.status) {
      case "success":
        stopPoll();
        codexStep.value = "success";
        setTimeout(() => emit("loggedIn"), 1200);
        break;
      case "failed":
        stopPoll();
        codexError.value = result.message ?? t("login.authFailed");
        codexStep.value = "error";
        break;
      default:
        if (codexStep.value === "waiting") schedulePoll();
        break;
    }
  } catch (e) {
    console.warn("[Codex] poll error:", e);
    if (codexStep.value === "waiting") schedulePoll();
  } finally {
    codexPollInFlight = false;
  }
}

function cancelCodexLogin() {
  stopPoll();
  resetCodexCopyState();
  codexStep.value = "idle";
  codexError.value = "";
}

async function copyUserCode() {
  await copyCodexText(codexUserCode.value);
}

onUnmounted(() => {
  stopPoll();
});
</script>

<template>
  <div class="login-container">
    <div class="login-card">
      <div class="login-header">
        <h1 class="login-title">Locus</h1>
      </div>

      <div class="tab-bar">
        <button :class="['tab-btn', { active: activeTab === 'apikey' }]" @click="activeTab = 'apikey'">
          API Key
        </button>
        <button :class="['tab-btn', { active: activeTab === 'codex' }]" @click="activeTab = 'codex'">
          {{ t("login.tabSubscription") }}
        </button>
      </div>

      <div v-if="activeTab === 'apikey'" class="login-body">
        <p class="login-desc">
          <a href="https://openrouter.ai/keys" target="_blank" class="login-link">openrouter.ai/keys</a> — {{ t("login.apiKeyDesc") }}
        </p>
        <div class="key-input-group">
          <input
            v-model="apiKey"
            class="key-input"
            type="password"
            placeholder="sk-or-..."
            autofocus
            @keydown="handleKeydown"
          />
          <button class="login-btn primary" :disabled="apiKeyLoading || !apiKey.trim()" @click="submitKey">
            {{ apiKeyLoading ? t("login.saving") : t("login.confirm") }}
          </button>
        </div>
        <div v-if="apiKeyError" class="error-msg">{{ apiKeyError }}</div>
        <button class="login-btn secondary" @click="emit('skip')">{{ t("login.skipForNow") }}</button>
      </div>

      <div v-else class="login-body">
        <!-- idle / error -->
        <template v-if="codexStep === 'idle' || codexStep === 'error'">
          <p class="login-desc">
            {{ t("login.codexDesc") }}
          </p>
          <div v-if="codexError" class="error-msg">{{ codexError }}</div>
          <button class="login-btn primary" @click="startCodexLogin">
            {{ codexStep === 'error' ? t("login.reLogin") : t("login.startLogin") }}
          </button>
          <button class="login-btn secondary" @click="emit('skip')">{{ t("login.skipForNow") }}</button>
        </template>

        <template v-else-if="codexStep === 'opening'">
          <p class="login-desc">
            {{ t("login.codexDesc") }}
          </p>
          <button class="login-btn primary" type="button" disabled>
            {{ t("settings.codex.opening") }}
          </button>
        </template>

        <!-- waiting -->
        <template v-else-if="codexStep === 'waiting'">
          <p class="login-desc">{{ t("login.waitingInstruction") }}</p>
          <button
            class="code-block"
            :class="{ copied: codexCodeCopied }"
            type="button"
            :title="codexCodeCopied ? t('common.copied') : t('common.clickToCopy')"
            @click="copyUserCode"
          >
            <span class="user-code">{{ codexUserCode }}</span>
            <span class="copy-indicator">
              {{ codexCodeCopied ? t("common.copied") : t("common.clickToCopy") }}
            </span>
          </button>
          <a :href="codexUrl" target="_blank" class="login-link url-text">{{ codexUrl }}</a>
          <div class="poll-hint">
            <span class="spinner"></span>
            {{ t("login.waitingAuth") }}
          </div>
          <button class="login-btn secondary" @click="cancelCodexLogin">{{ t("login.cancel") }}</button>
        </template>

        <!-- success -->
        <template v-else-if="codexStep === 'success'">
          <div class="success-msg">
            <span class="checkmark">✓</span>
            {{ t("settings.codex.loginSuccess") }}
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.login-container {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-color);
  height: 100vh;
}

.login-card {
  width: 420px;
  max-width: 90vw;
  padding: 40px;
  border-radius: 16px;
  border: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.login-header {
  text-align: center;
  margin-bottom: 24px;
}

.login-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -1px;
  margin-bottom: 0;
}

.tab-bar {
  display: flex;
  gap: 4px;
  margin-bottom: 24px;
  border-bottom: 1px solid var(--border-color);
}

.tab-btn {
  flex: 1;
  padding: 8px 12px;
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--text-secondary);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s;
  margin-bottom: -1px;
}

.tab-btn:hover {
  color: var(--text-color);
}

.tab-btn.active {
  color: var(--accent-color);
  border-bottom-color: var(--accent-color);
}

.login-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.login-desc {
  font-size: 13px;
  color: var(--text-secondary);
  line-height: 1.6;
  margin: 0;
}

.login-link {
  color: var(--accent-color);
  text-decoration: underline;
  word-break: break-all;
}

.url-text {
  font-size: 12px;
}

.login-btn {
  padding: 10px 20px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s;
}

.login-btn.primary {
  background: var(--accent-color);
  color: var(--bg-color);
  border-color: var(--accent-color);
}

.login-btn.primary:hover:not(:disabled) { opacity: 0.85; }
.login-btn.primary:disabled { opacity: 0.5; cursor: not-allowed; }

.login-btn.secondary {
  background: transparent;
  color: var(--text-secondary);
}
.login-btn.secondary:hover { background: var(--hover-bg); }

.key-input-group {
  display: flex;
  gap: 8px;
}

.key-input {
  flex: 1;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 14px;
  font-family: var(--font-mono-editor);
  outline: none;
  transition: border-color 0.15s;
}
.key-input:focus { border-color: var(--accent-color); }

.code-block {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border-radius: 8px;
  background: var(--input-bg);
  border: 1px solid var(--border-color);
  color: inherit;
  cursor: pointer;
  text-align: left;
  box-shadow: none;
  transition: border-color 0.15s, background 0.15s;
}

.code-block:hover {
  background: var(--hover-bg);
  border-color: var(--accent-color);
}

.code-block:focus-visible {
  outline: none;
  border-color: var(--accent-color);
}

.code-block.copied {
  border-color: var(--status-good-border, #32c864);
  background: var(--status-good-bg, rgba(50, 200, 100, 0.1));
}

.user-code {
  flex: 1;
  font-size: 22px;
  font-weight: 700;
  font-family: var(--font-mono-display);
  letter-spacing: 3px;
  color: var(--accent-color);
}

.copy-indicator {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-size: 12px;
}

.code-block.copied .copy-indicator {
  color: var(--status-good-fg, #32c864);
}

.poll-hint {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--text-secondary);
}

.spinner {
  width: 14px;
  height: 14px;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}
@keyframes spin { to { transform: rotate(360deg); } }

.success-msg {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px;
  border-radius: 8px;
  background: rgba(50, 200, 100, 0.1);
  color: #32c864;
  font-size: 14px;
}

.checkmark {
  font-size: 18px;
  font-weight: bold;
}

.error-msg {
  padding: 10px 14px;
  border-radius: 8px;
  background: rgba(220, 50, 50, 0.1);
  color: #dc3232;
  font-size: 13px;
  line-height: 1.5;
}
</style>
