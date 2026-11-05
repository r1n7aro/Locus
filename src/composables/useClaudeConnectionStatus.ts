import { computed, effectScope, onScopeDispose, readonly, shallowRef, watch, type EffectScope } from 'vue';
import {
  claudePrewarmSettings, getClaudeSessionConnection, isRemoteClaudeCode, subscribeClaudeConnectionStatus,
  type ClaudeSessionConnection,
} from '../services/claudeConnection';
import { claudeCodeCliEnabled } from '../services/claudeCodeCli';
import { normalizeAppError } from '../services/errors';

// All composers in this renderer share one process-level snapshot. The backend
// pushes the overview whenever it changes (`claude-connection-status`); one
// request fills the gap until the first push. Focus, selected model and
// conversation identity never own connection health.
const status = shallowRef<ClaudeSessionConnection | null>(null);
const pollError = shallowRef('');
const error = computed(() => status.value?.lastError || pollError.value);
const phase = computed(() => status.value?.phase ?? 'unknown');
const visible = computed(() => claudeCodeCliEnabled.value && isRemoteClaudeCode.value);
const shared = { status: readonly(status), visible, phase, error };
let consumers = 0;
let updates: EffectScope | undefined;

function startUpdates() {
  // Detached from the first composer's lifetime: closing it must not stop
  // updates for other panes that still display the same process state.
  updates = effectScope(true);
  updates.run(() => {
    let generation = 0;
    let release: (() => void) | undefined;
    const stop = () => { generation++; release?.(); release = undefined; };
    watch([() => visible.value, () => JSON.stringify(claudePrewarmSettings.value)], () => {
      stop();
      status.value = null;
      pollError.value = '';
      if (!visible.value) return;
      const version = generation;
      async function snapshot() {
        try {
          // Null session + no model/cwd requests the configured server's whole
          // process pool. This reads local state; it never opens SSH or sends input.
          const result = await getClaudeSessionConnection(null);
          if (version === generation) { status.value = result; pollError.value = ''; }
        } catch (error) {
          // Invalidate health, latency and pool counts for every pane together.
          if (version === generation) { status.value = null; pollError.value = normalizeAppError(error).message; }
        }
      }
      // Subscribe before the snapshot so no change falls between the two.
      void subscribeClaudeConnectionStatus((event) => {
        if (version !== generation) return;
        status.value = event.status;
        pollError.value = event.error ?? '';
      })
        .then((unsubscribe) => { if (version === generation) release = unsubscribe; else unsubscribe(); })
        .catch(() => {})
        .finally(() => { if (version === generation) void snapshot(); });
    }, { immediate: true, flush: 'sync' });
    onScopeDispose(() => { stop(); status.value = null; pollError.value = ''; });
  });
}

export function useClaudeConnectionStatus() {
  if (consumers++ === 0) startUpdates();
  onScopeDispose(() => {
    if (--consumers === 0) {
      updates?.stop();
      updates = undefined;
    }
  });
  return shared;
}
