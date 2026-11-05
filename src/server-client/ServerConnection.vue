<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch } from 'vue';
import BaseButton from '../components/ui/BaseButton.vue';
import { normalizeAppError } from '../services/errors';
import { ipcInvoke } from '../services/ipc';
import type { CodexRateLimitsResponse } from '../services/auth';
import { serverStatus, login, logout, cancelLogin, refreshServer } from './state';

const serverUrl = ref('');
const username = ref('');
const password = ref('');
const busy = ref(false);
const error = ref('');
const notice = ref('');
const quotas = ref<Record<string, string>>({});
let request = 0;

watch(serverUrl, () => { password.value = ''; request++; void cancelLogin(); });
onBeforeUnmount(() => { request++; password.value = ''; void cancelLogin(); });
async function submit() {
  if (busy.value) return;
  const current = ++request;
  busy.value = true; error.value = ''; notice.value = '';
  const secret = password.value;
  password.value = '';
  try { await login(serverUrl.value, username.value, secret); }
  catch (e) { if (current === request) error.value = normalizeAppError(e).message; }
  finally { password.value = ''; busy.value = false; }
}
async function signOut() {
  busy.value = true; error.value = ''; notice.value = ''; quotas.value = {};
  try {
    const result = await logout();
    if (result.serverRevocationConfirmed === false) notice.value = '已退出本机；服务器会话撤销未确认。';
  } catch (e) { error.value = normalizeAppError(e).message; }
  finally { busy.value = false; }
}
async function refresh() {
  busy.value = true; error.value = '';
  try { await refreshServer(); }
  catch (e) { error.value = normalizeAppError(e).message; }
  finally { busy.value = false; }
}
async function queryQuota(bindingId: string) {
  busy.value = true; error.value = '';
  try {
    const result = await ipcInvoke<CodexRateLimitsResponse>('server_client_usage', { bindingId });
    const windows = [result.rateLimits.primary, result.rateLimits.secondary].filter(window => window && Number.isFinite(window.remainingPercent));
    quotas.value[bindingId] = windows.length ? windows.map(window =>
      `${window!.windowMinutes ? `${window!.windowMinutes} 分钟` : '窗口'} · 剩余 ${Math.round(window!.remainingPercent)}%`).join(' / ') : '暂无额度数据';
  } catch (e) { error.value = normalizeAppError(e).message; }
  finally { busy.value = false; }
}
onMounted(async () => {
  try {
    await refreshServer();
    serverUrl.value = serverStatus.value.serverUrl || '';
    username.value = serverStatus.value.user?.username || '';
  } catch (e) { error.value = normalizeAppError(e).message; }
});
</script>

<template>
  <section class="server-section">
    <div class="section-label">Locus Server</div>
    <template v-if="serverStatus.user">
      <div class="account-row">
        <div><strong>{{ serverStatus.user.username }}</strong><span class="server-address">{{ serverStatus.serverUrl }}</span></div>
        <div class="actions"><BaseButton :disabled="busy" @click="refresh">刷新</BaseButton><BaseButton :disabled="busy" @click="signOut">退出登录</BaseButton></div>
      </div>
      <div v-for="upstream in serverStatus.upstreams" :key="upstream.id" class="upstream-row">
        <div><span>{{ upstream.name }}</span><div v-if="quotas[upstream.id]" class="secondary">{{ quotas[upstream.id] }}</div></div>
        <div class="actions"><span class="secondary">{{ upstream.kind === 'codex' ? 'Codex' : 'Claude Code' }}</span><BaseButton v-if="upstream.kind === 'codex'" :disabled="busy" @click="queryQuota(upstream.id)">额度</BaseButton></div>
      </div>
      <div v-if="!serverStatus.upstreams.length" class="secondary">暂无可用上游</div>
    </template>
    <form v-else class="server-form" @submit.prevent="submit">
      <label for="locus-server-url">服务器</label><input id="locus-server-url" v-model="serverUrl" type="url" placeholder="https://locus.example.internal" :disabled="busy" required spellcheck="false" autocomplete="url" />
      <label for="locus-server-user">账号</label><input id="locus-server-user" v-model="username" autocomplete="username" :disabled="busy" required />
      <label for="locus-server-password">密码</label><input id="locus-server-password" v-model="password" type="password" autocomplete="current-password" :disabled="busy" required />
      <div class="form-actions"><BaseButton type="submit" :disabled="busy">{{ busy ? '登录中…' : '登录' }}</BaseButton><BaseButton v-if="busy" @click="request++; cancelLogin()">取消</BaseButton></div>
    </form>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <p v-if="notice" class="secondary" role="status">{{ notice }}</p>
  </section>
</template>

<style scoped>
.server-section { min-width: 0; }
.section-label { color: var(--text-secondary); font-size: 11px; font-weight: 600; letter-spacing: .5px; margin-bottom: 12px; }
.account-row, .upstream-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--border-color); }
.account-row strong { font-size: 13px; font-weight: 500; }
.server-address { display: block; margin-top: 4px; color: var(--text-secondary); font-size: 12px; }
.actions { display: flex; align-items: center; gap: 6px; }
.server-form { display: grid; grid-template-columns: 70px minmax(180px, 420px); align-items: center; gap: 10px; font-size: 13px; }
.server-form input { min-width: 0; padding: 6px 8px; font: inherit; color: var(--text-color); background: var(--bg-color); border: 1px solid var(--border-color); border-radius: 6px; }
.server-form input:focus { outline: 1px solid var(--accent-color); }
.form-actions { grid-column: 2; display: flex; gap: 6px; }
.upstream-row { font-size: 13px; }
.secondary { color: var(--text-secondary); font-size: 12px; }
.error { color: var(--status-danger-fg); font-size: 12px; }
</style>
