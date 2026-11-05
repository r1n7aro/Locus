<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { t } from '../../i18n';
import { normalizeAppError } from '../../services/errors';
import { formatClaudeSshAddress, parseClaudeSshAddress } from '../../utils/claudeSshAddress';
import BaseButton from '../ui/BaseButton.vue';
import BaseSegmented from '../ui/BaseSegmented.vue';
import BaseCheckbox from '../ui/BaseCheckbox.vue';
import BaseDropdown from '../ui/BaseDropdown.vue';
import { CLAUDE_CONFIGURABLE_BUILTIN_AGENTS, CLAUDE_NATIVE_AGENTS_MIN_VERSION, getClaudeNativeAgentsSupport, setClaudeNativeAgents, setClaudePrewarm,
  setClaudeRuntimeSettings, type ClaudeNativeAgentsSupport } from '../../services/claudeConnection';
import { listAgents, listSubagentDefs } from '../../services/agent';
import { agentProjectTypesLabel } from '../../utils/agentProjectTypes';
import type { AgentInfo } from '../../types';
import { authorizeClaudeSsh, clearClaudeSshUnlockPassword, getClaudeConnection, inspectClaudeSsh, useLocalClaude, setClaudePermissionMode,
  type ClaudeSshConfig, type ClaudeRemoteInfo, type ClaudePermissionMode } from '../../services/claudeConnection';

const emit = defineEmits<{ changed: []; modeChange: [mode: 'local' | 'ssh'] }>();
const mode = ref<'local' | 'ssh'>('local');
const permissionMode = ref<ClaudePermissionMode>('auto');
const idleCompaction = ref(false);
const telemetry = ref(false);
const autoCompactWindow = ref<number | null>(null);
const prewarm = ref(false);
const poolCapacity = ref(1);
const idleMinutes = ref(15);
const nativeMultiAgent = ref(true);
const builtinAgents = ref<string[]>(['general-purpose', 'Explore', 'Plan']);
/** Locus agent types left out of native conversations; the rest are offered. */
const excludedLocusAgents = ref<string[]>([]);
/** Delegatable Locus agents of the app catalog, in the Agent list order. */
const locusAgents = ref<AgentInfo[]>([]);
async function updateNativeAgents(enabled: boolean, builtins: readonly string[] = builtinAgents.value,
  excluded: readonly string[] = excludedLocusAgents.value) {
  if (busy.value) return;
  await action(async () => {
    const saved = await setClaudeNativeAgents(enabled, builtins, excluded);
    nativeMultiAgent.value = saved.nativeMultiAgent !== false;
    builtinAgents.value = saved.nativeAgents?.builtins ?? [];
    excludedLocusAgents.value = saved.nativeAgents?.excludedLocusAgents ?? [];
  });
}
function toggleBuiltinAgent(name: string, enabled: boolean) {
  const next = CLAUDE_CONFIGURABLE_BUILTIN_AGENTS.filter(agent => agent === name ? enabled : builtinAgents.value.includes(agent));
  void updateNativeAgents(nativeMultiAgent.value, next);
}
// Ids of agents that are no longer listed stay excluded if they come back.
function toggleLocusAgent(id: string, enabled: boolean) {
  const rest = excludedLocusAgents.value.filter(agent => agent !== id);
  void updateNativeAgents(nativeMultiAgent.value, builtinAgents.value, enabled ? rest : [...rest, id]);
}
/** Version check of the saved connection; new conversations fall back to Locus subagents when it fails. */
const nativeSupport = ref<ClaudeNativeAgentsSupport | null>(null);
async function loadNativeSupport() {
  try {
    nativeSupport.value = await getClaudeNativeAgentsSupport();
  } catch (cause) {
    console.error('Claude native agent support check failed:', cause);
  }
}
const nativeUnsupported = computed(() => nativeMultiAgent.value && !!nativeSupport.value && !nativeSupport.value.supported);
const nativeUnsupportedMessage = computed(() => {
  const support = nativeSupport.value;
  if (!support) return '';
  const version = support.cliVersion?.match(/\d+\.\d+\.\d+/)?.[0];
  const action = !support.remote ? t('settings.claudeCode.nativeUpgradeLocal')
    : support.installable ? t('settings.claudeCode.nativeUpgradeRemote', CLAUDE_NATIVE_AGENTS_MIN_VERSION)
    : t('settings.claudeCode.nativeUpgradeCustomPath');
  return version
    ? t('settings.claudeCode.nativeUnsupportedVersion', version, CLAUDE_NATIVE_AGENTS_MIN_VERSION, action)
    : t('settings.claudeCode.nativeUnsupportedUnknown', CLAUDE_NATIVE_AGENTS_MIN_VERSION, action);
});
/** The saved SSH connection was just tested and can be upgraded for native agents by "Connect and save". */
const nativeUpgrade = computed(() => nativeUnsupported.value && !!nativeSupport.value?.installable
  && mode.value === 'ssh' && !!authorized.value && authorized.value === signature.value);
async function loadLocusAgents() {
  try {
    const [agents, subagents] = await Promise.all([listAgents(), listSubagentDefs()]);
    const seen = new Set<string>();
    locusAgents.value = [...agents, ...subagents].filter(agent => agent.delegatable && !seen.has(agent.id) && !!seen.add(agent.id));
  } catch (cause) {
    console.error('Locus agent list failed:', cause);
  }
}
async function updatePrewarm(enabled: boolean, capacity = poolCapacity.value, idle = idleMinutes.value) {
  if (busy.value) return;
  await action(async () => {
    const saved = await setClaudePrewarm(enabled, capacity, idle);
    prewarm.value = !!saved.prewarm; poolCapacity.value = saved.poolCapacity ?? 1; idleMinutes.value = saved.idleMinutes ?? 15;
  });
}
const busy = ref(false);
const error = ref('');
const address = ref('');
const identityFile = ref('');
const knownHostsFile = ref('');
const cliPath = ref('');
const inspected = ref('');
const authorized = ref('');
const attempted = ref(false);
const info = ref<ClaudeRemoteInfo | null>(null);
const unlockPassword = ref('');
const savedPasswordFor = ref('');
const editPassword = ref(false);
const parsedAddress = computed(() => parseClaudeSshAddress(address.value));
const config = computed<ClaudeSshConfig>(() => ({
  target: parsedAddress.value?.target || address.value.trim(), port: parsedAddress.value?.port ?? null,
  identityFile: identityFile.value.trim() || null, knownHostsFile: knownHostsFile.value.trim() || null, cliPath: cliPath.value.trim() || null,
}));
const signature = computed(() => JSON.stringify(config.value));
const valid = computed(() => !!parsedAddress.value);
const hasUnlockPassword = computed(() => savedPasswordFor.value === signature.value);
const needsSetup = computed(() => !!info.value && (!info.value.pythonInstalled || !info.value.cliSupported));
const needsUnlock = computed(() => !needsSetup.value && info.value?.platform === 'darwin' && info.value.keychainUnlocked === false && !info.value.authenticated);
const showPassword = computed(() => needsUnlock.value || (info.value?.platform === 'darwin' && editPassword.value));
const ready = computed(() => !!info.value?.authenticated && !needsSetup.value);
const canAuthorize = computed(() => inspected.value === signature.value && (needsSetup.value || ready.value || (needsUnlock.value && (!!unlockPassword.value || hasUnlockPassword.value))));
const showAuthorize = computed(() => !!info.value && (needsSetup.value || needsUnlock.value || editPassword.value
  || (ready.value && (authorized.value !== signature.value || nativeUpgrade.value))));
const platformLabel = computed(() => info.value?.platform === 'darwin' ? 'macOS' : info.value?.platform?.startsWith('linux') ? 'Linux' : info.value?.platform);
const statusLabel = computed(() => t(needsSetup.value ? 'settings.claudeSsh.setupRequired' : needsUnlock.value ? 'settings.claudeSsh.keychainLocked' : !info.value?.authenticated ? 'settings.claudeSsh.loginRequired' : 'settings.claudeSsh.ready'));
const authorizeLabel = computed(() => t(needsSetup.value ? 'settings.claudeSsh.installContinue' : showPassword.value ? 'settings.claudeSsh.unlockSave'
  : nativeUpgrade.value ? 'settings.claudeSsh.upgradeSave' : 'settings.claudeSsh.authorize'));

watch(signature, () => { inspected.value = ''; info.value = null; error.value = ''; unlockPassword.value = ''; editPassword.value = false; });
watch(address, () => { attempted.value = false; });
watch(mode, value => emit('modeChange', value), { immediate: true });
async function action(work: () => Promise<void>) {
  busy.value = true; error.value = '';
  try { await work(); } catch (e) { error.value = normalizeAppError(e).message; } finally { busy.value = false; }
}
async function updateMode(value: string) {
  if (busy.value) return;
  unlockPassword.value = ''; editPassword.value = false;
  if (value === 'local') {
    await action(async () => { await useLocalClaude(); mode.value = 'local'; authorized.value = ''; emit('changed'); void loadNativeSupport(); });
  } else if (value === 'ssh') { mode.value = value; }
}
async function updatePermissionMode(value: string) {
  if (busy.value || (value !== 'auto' && value !== 'bypassPermissions')) return;
  await action(async () => { permissionMode.value = (await setClaudePermissionMode(value)).permissionMode; });
}
async function updateRuntimeSettings(idle: boolean, telemetryEnabled: boolean, window = autoCompactWindow.value) {
  if (busy.value) return;
  await action(async () => {
    const saved = await setClaudeRuntimeSettings(idle, telemetryEnabled, window);
    idleCompaction.value = !!saved.idleCompaction;
    telemetry.value = !!saved.telemetry;
    autoCompactWindow.value = saved.autoCompactWindow ?? null;
  });
}
async function updateAutoCompactWindow(event: Event) {
  const input = event.target as HTMLInputElement;
  const value = input.value.trim();
  const window = value ? Number(value) : null;
  if (!input.validity.valid || (window !== null && (!Number.isInteger(window) || window < 100_000 || window > 1_000_000))) {
    error.value = t('settings.claudeCode.autoCompactWindowInvalid');
  } else if (window !== autoCompactWindow.value) {
    await updateRuntimeSettings(idleCompaction.value, telemetry.value, window);
  }
  input.value = autoCompactWindow.value === null ? '' : String(autoCompactWindow.value);
}
async function inspect() {
  if (!valid.value || busy.value) return;
  const snapshot = signature.value, connection = config.value;
  attempted.value = true; inspected.value = ''; info.value = null; editPassword.value = false; unlockPassword.value = '';
  await action(async () => {
    const result = await inspectClaudeSsh(connection);
    if (signature.value !== snapshot) return;
    info.value = result; inspected.value = snapshot;
    // Testing the saved connection records its current CLI version.
    if (authorized.value === snapshot) { emit('changed'); await loadNativeSupport(); }
  });
}
async function authorize() {
  if (!canAuthorize.value || busy.value) return;
  const snapshot = signature.value, connection = config.value, installing = needsSetup.value;
  await action(async () => {
    try {
      const result = await authorizeClaudeSsh(connection, showPassword.value && unlockPassword.value ? unlockPassword.value : undefined);
      if (signature.value !== snapshot) return;
      authorized.value = snapshot; info.value = result.info; editPassword.value = false;
      savedPasswordFor.value = result.hasUnlockPassword ? snapshot : '';
      emit('changed');
      await loadNativeSupport();
    } catch (failure) {
      // Setup may succeed while native login still needs the user's action.
      if (installing) {
        const result = await inspectClaudeSsh(connection).catch(() => null);
        if (result && signature.value === snapshot) {
          info.value = result; inspected.value = snapshot;
          if (result.pythonInstalled && result.cliSupported && !result.authenticated) return;
        }
      }
      throw failure;
    } finally { unlockPassword.value = ''; }
  });
}
async function clearUnlockPassword() {
  if (busy.value) return;
  await action(async () => {
    await clearClaudeSshUnlockPassword(config.value);
    savedPasswordFor.value = ''; unlockPassword.value = ''; editPassword.value = false;
    info.value = null; inspected.value = '';
  });
}
onMounted(() => action(async () => {
  const saved = await getClaudeConnection();
  permissionMode.value = saved.permissionMode;
  idleCompaction.value = !!saved.idleCompaction;
  telemetry.value = !!saved.telemetry;
  autoCompactWindow.value = saved.autoCompactWindow ?? null;
  prewarm.value = !!saved.prewarm; poolCapacity.value = saved.poolCapacity ?? 1; idleMinutes.value = saved.idleMinutes ?? 15;
  nativeMultiAgent.value = saved.nativeMultiAgent !== false;
  builtinAgents.value = saved.nativeAgents?.builtins ?? ['general-purpose', 'Explore', 'Plan'];
  excludedLocusAgents.value = saved.nativeAgents?.excludedLocusAgents ?? [];
  void loadLocusAgents();
  void loadNativeSupport();
  if (saved.remote) {
    address.value = formatClaudeSshAddress(saved.remote);
    identityFile.value = saved.remote.identityFile || ''; knownHostsFile.value = saved.remote.knownHostsFile || '';
    cliPath.value = saved.remote.cliPath || ''; mode.value = 'ssh';
    await Promise.resolve();
    authorized.value = signature.value;
    savedPasswordFor.value = saved.hasUnlockPassword ? signature.value : '';
    // Cached platform/login information is not a fresh connection test.
  }
}));
</script>

<template>
  <div class="claude-connection">
    <div class="connection-row connection-header">
      <div class="connection-heading">
        <span id="claude-connection-mode-label" class="connection-title">{{ t('settings.claudeSsh.connection') }}</span>
        <span class="connection-hint">{{ t(mode === 'ssh' ? 'settings.claudeSsh.remoteHint' : 'settings.claudeSsh.localHint') }}</span>
      </div>
      <BaseSegmented class="connection-mode" :model-value="mode" size="sm" aria-labelledby="claude-connection-mode-label" :options="[
        { value: 'local', label: t('settings.claudeSsh.local'), disabled: busy },
        { value: 'ssh', label: 'SSH', disabled: busy },
      ]" @update:model-value="updateMode" />
    </div>
    <template v-if="mode === 'ssh'">
      <form class="connection-server" @submit.prevent="inspect">
        <label class="connection-field">
          <span>{{ t('settings.claudeSsh.server') }}</span>
          <input v-model="address" :disabled="busy" placeholder="user@hostname:22" spellcheck="false" autocomplete="off" />
        </label>
        <BaseButton type="submit" size="md" variant="neutral" :disabled="busy || !valid">{{ busy ? t('settings.claudeSsh.working') : t('settings.claudeSsh.inspect') }}</BaseButton>
      </form>
      <p v-if="address.trim() && !valid" class="connection-hint">{{ t('settings.claudeSsh.addressHint') }}</p>
      <div v-if="info" class="connection-result" role="status" aria-live="polite">
        <div class="connection-row">
          <span>{{ statusLabel }}</span>
          <span class="connection-hint">{{ [platformLabel, info.cliVersion].filter(Boolean).join(' · ') }}</span>
        </div>
        <p v-if="needsSetup" class="connection-hint">{{ t('settings.claudeSsh.installHint') }}</p>
        <p v-else-if="!info.authenticated && !needsUnlock" class="connection-hint">{{ t('settings.claudeSsh.login') }}</p>
      </div>
      <template v-if="showPassword">
        <label class="connection-field">
          <span>{{ t('settings.claudeSsh.unlockPassword') }}</span>
          <input v-model="unlockPassword" type="password" :disabled="busy" autocomplete="new-password" spellcheck="false"
            :placeholder="hasUnlockPassword ? t('settings.claudeSsh.passwordSaved') : t('settings.claudeSsh.passwordPlaceholder')"
            @keydown.enter.prevent="authorize" />
        </label>
        <p class="connection-hint">{{ t('settings.claudeSsh.passwordHint') }}</p>
      </template>
      <p v-if="showAuthorize && authorized !== signature" class="connection-hint">{{ t('settings.claudeSsh.authorizeHint') }}</p>
      <div v-if="info && (showAuthorize || ready)" class="connection-row">
        <span class="connection-hint">{{ ready && authorized === signature ? t('settings.claudeSsh.saved') : '' }}</span>
        <BaseButton v-if="showAuthorize" size="md" variant="primary" :disabled="busy || !canAuthorize" @click="authorize">{{ authorizeLabel }}</BaseButton>
      </div>
      <details v-if="attempted" class="connection-options">
        <summary>{{ t('settings.claudeSsh.advanced') }}</summary>
        <div class="connection-fields">
          <label class="connection-field"><span>{{ t('settings.claudeSsh.identity') }}</span><input v-model="identityFile" :disabled="busy" spellcheck="false" /></label>
          <label class="connection-field"><span>known_hosts</span><input v-model="knownHostsFile" :disabled="busy" spellcheck="false" /></label>
          <label class="connection-field"><span>{{ t('settings.claudeSsh.cliPath') }}</span><input v-model="cliPath" :disabled="busy" spellcheck="false" /></label>
        </div>
        <p class="connection-hint">{{ t('settings.claudeSsh.sshHint') }}</p>
        <div v-if="info?.platform === 'darwin' && hasUnlockPassword" class="connection-actions">
          <BaseButton size="sm" variant="neutral" :disabled="busy" @click="editPassword = true">{{ t('settings.claudeSsh.changePassword') }}</BaseButton>
          <BaseButton size="sm" variant="neutral" :disabled="busy" @click="clearUnlockPassword">{{ t('settings.claudeSsh.clearPassword') }}</BaseButton>
        </div>
      </details>
    </template>
    <template v-if="mode === 'ssh' && authorized && authorized === signature">
      <div class="connection-field">
        <span>{{ t('settings.claudeSsh.pool') }}</span>
        <label class="connection-pool-toggle"><BaseCheckbox :model-value="prewarm" :disabled="busy" :aria-label="t('settings.claudeSsh.prewarm')" @update:model-value="updatePrewarm($event)" /><span>{{ t('settings.claudeSsh.prewarm') }}</span></label>
      </div>
      <template v-if="prewarm">
        <div class="connection-field"><span>{{ t('settings.claudeSsh.capacity') }}</span>
          <BaseDropdown class="connection-pool-options" :model-value="String(poolCapacity)" :disabled="busy" :options="[1, 2, 3, 4].map(n => ({ value: String(n), label: String(n) }))" @update:model-value="updatePrewarm(true, Number($event))" />
        </div>
        <div class="connection-field"><span>{{ t('settings.claudeSsh.idleTimeout') }}</span>
          <BaseDropdown class="connection-pool-options" :model-value="String(idleMinutes)" :disabled="busy" :options="[5, 15, 30, 60].map(n => ({ value: String(n), label: t('settings.claudeSsh.minutes', n) }))" @update:model-value="updatePrewarm(true, poolCapacity, Number($event))" />
        </div>
        <p class="connection-hint">{{ t('settings.claudeSsh.poolHint') }}</p>
      </template>
    </template>
    <p v-if="error" class="connection-error" role="alert">{{ error }}</p>
    <div class="connection-field connection-permissions" :class="{ 'is-local': mode === 'local' }">
      <span id="claude-permission-mode-label">{{ t('settings.claudeCode.permissionMode') }}</span>
      <BaseSegmented class="connection-permission-options" :model-value="permissionMode" size="sm" aria-labelledby="claude-permission-mode-label" :options="[
        { value: 'auto', label: 'Auto', hint: t('settings.claudeCode.autoModeHint'), disabled: busy },
        { value: 'bypassPermissions', label: 'Bypass', hint: t('settings.claudeCode.bypassModeHint'), disabled: busy },
      ]" @update:model-value="updatePermissionMode" />
    </div>
    <label class="connection-field">
      <span>{{ t('settings.claudeCode.autoCompactWindow') }}</span>
      <div class="connection-compact-window">
        <input type="number" :value="autoCompactWindow ?? ''" :disabled="busy" min="100000" max="1000000" step="1"
          :aria-label="t('settings.claudeCode.autoCompactWindow')" aria-describedby="claude-compact-window-hint"
          :placeholder="t('settings.claudeCode.autoCompactWindowDefault')" @change="updateAutoCompactWindow"
          @keydown.enter.prevent="($event.target as HTMLInputElement).blur()" />
        <span>tokens</span>
      </div>
    </label>
    <p id="claude-compact-window-hint" class="connection-hint">{{ t('settings.claudeCode.autoCompactWindowHint') }}</p>
    <div class="connection-field">
      <span>{{ t('settings.claudeCode.idleCompaction') }}</span>
      <BaseCheckbox :model-value="idleCompaction" :disabled="busy" :aria-label="t('settings.claudeCode.idleCompaction')"
        @update:model-value="updateRuntimeSettings($event, telemetry)" />
    </div>
    <div class="connection-field">
      <span>{{ t('settings.claudeCode.telemetry') }}</span>
      <BaseCheckbox :model-value="telemetry" :disabled="busy" :aria-label="t('settings.claudeCode.telemetry')"
        @update:model-value="updateRuntimeSettings(idleCompaction, $event)" />
    </div>
    <div class="connection-field connection-native-agents">
      <span>{{ t('settings.claudeCode.nativeMultiAgent') }}</span>
      <BaseCheckbox :model-value="nativeMultiAgent" :disabled="busy" :aria-label="t('settings.claudeCode.nativeMultiAgent')"
        aria-describedby="claude-native-agents-hint" @update:model-value="updateNativeAgents($event)" />
    </div>
    <p id="claude-native-agents-hint" class="connection-hint">{{ t('settings.claudeCode.nativeMultiAgentHint') }}</p>
    <p v-if="nativeUnsupported" class="connection-warning">{{ nativeUnsupportedMessage }}</p>
    <template v-if="nativeMultiAgent">
      <div class="connection-field">
        <span id="claude-builtin-agents-label">{{ t('settings.claudeCode.builtinAgents') }}</span>
        <div class="connection-builtin-agents" role="group" aria-labelledby="claude-builtin-agents-label" aria-describedby="claude-builtin-agents-hint">
          <label v-for="agent in CLAUDE_CONFIGURABLE_BUILTIN_AGENTS" :key="agent" class="connection-pool-toggle">
            <BaseCheckbox :model-value="builtinAgents.includes(agent)" :disabled="busy" :aria-label="agent"
              @update:model-value="toggleBuiltinAgent(agent, $event)" /><span>{{ agent }}</span>
          </label>
        </div>
      </div>
      <p id="claude-builtin-agents-hint" class="connection-hint">{{ t('settings.claudeCode.builtinAgentsHint') }}</p>
      <div class="connection-field">
        <span id="claude-locus-agents-label">{{ t('settings.claudeCode.locusAgents') }}</span>
        <div v-if="locusAgents.length" class="connection-locus-agents" role="group"
          aria-labelledby="claude-locus-agents-label" aria-describedby="claude-locus-agents-hint">
          <label v-for="agent in locusAgents" :key="agent.id" class="connection-pool-toggle" :title="agent.description || undefined">
            <BaseCheckbox :model-value="!excludedLocusAgents.includes(agent.id)" :disabled="busy" :aria-label="agent.id"
              @update:model-value="toggleLocusAgent(agent.id, $event)" /><span>{{ agent.name || agent.id }}</span>
            <span v-if="agentProjectTypesLabel(agent)" class="connection-agent-scope">{{ agentProjectTypesLabel(agent) }}</span>
          </label>
        </div>
        <span v-else class="connection-hint">{{ t('settings.claudeCode.locusAgentsNone') }}</span>
      </div>
      <p id="claude-locus-agents-hint" class="connection-hint">{{ t('settings.claudeCode.locusAgentsHint') }}</p>
    </template>
  </div>
</template>

<style scoped>
.claude-connection { display: grid; gap: 12px; font-size: 13px; }
.connection-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.connection-heading { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.connection-title { font-size: 14px; font-weight: 600; }
.connection-mode :deep(.base-segmented-item) { min-width: 88px; }
.connection-mode { flex-shrink: 0; }
.connection-pool-toggle { display: flex; align-items: center; gap: 8px; cursor: pointer; }
.connection-compact-window { display: flex; align-items: center; gap: 8px; }
.connection-compact-window input { width: 160px; max-width: 100%; }
.connection-pool-options { width: 120px; justify-self: start; }
.connection-native-agents { padding-top: 12px; border-top: 1px solid var(--border-color); }
.connection-builtin-agents, .connection-locus-agents { display: flex; flex-wrap: wrap; gap: 8px 16px; }
.connection-agent-scope { color: var(--text-secondary); font-size: 12px; }
.connection-server { display: flex; align-items: center; gap: 10px; border-top: 1px solid var(--border-color); padding-top: 12px; }
.connection-server .connection-field { flex: 1; min-width: 0; }
.connection-field { display: grid; grid-template-columns: 80px minmax(0, 1fr); align-items: center; gap: 12px; }
.connection-field span, summary { color: var(--text-secondary); }
.connection-field input { min-width: 0; min-height: 32px; box-sizing: border-box; padding: 5px 8px; border: 1px solid var(--border-color); border-radius: 6px; background: var(--input-bg); color: var(--text-color); font: inherit; }
.connection-field input:focus-visible { outline: 1px solid var(--accent-color); }
.connection-fields { display: grid; gap: 10px; padding-top: 12px; }
.connection-hint { margin: 0; color: var(--text-secondary); font-size: 12px; line-height: 1.6; }
.connection-result { display: grid; gap: 6px; }
.connection-options .connection-hint, .connection-actions { margin-top: 10px; }
summary { cursor: pointer; font-size: 12px; }
.connection-actions { display: flex; gap: 8px; }
.connection-permission-options { justify-self: start; }
.connection-permissions.is-local { padding-top: 12px; border-top: 1px solid var(--border-color); }
.connection-error { margin: 0; font-size: 12px; color: var(--status-danger-fg); overflow-wrap: anywhere; }
.connection-warning { margin: 0; font-size: 12px; line-height: 1.6; color: var(--status-warn-fg); overflow-wrap: anywhere; }
@media (max-width: 600px) {
  .connection-header { flex-wrap: wrap; }
  .connection-server { flex-wrap: wrap; }
  .connection-server .connection-field { flex-basis: 100%; }
  .connection-field { grid-template-columns: 72px minmax(0, 1fr); }
}
</style>
