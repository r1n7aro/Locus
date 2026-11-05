import { onScopeDispose, watch } from 'vue';
import { protocolModelId } from '@locus/server-client';
import { claudePrewarmSettings, prewarmClaudeSession } from '../services/claudeConnection';
import type { WorkspaceRef } from '../services/project';

export function useClaudePrewarm(context: () => { sessionId: string | null; modelId: string; active: boolean; busy?: boolean; workspaceRef?: WorkspaceRef | null }) {
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const clear = () => { generation++; clearTimeout(timer); };
  watch(() => JSON.stringify([context(), claudePrewarmSettings.value]), () => {
    clear();
    const current = context();
    if (!current.active || current.busy || !current.workspaceRef || !protocolModelId(current.modelId).startsWith('claude_code/') || !claudePrewarmSettings.value.enabled) return;
    const version = generation;
    async function refresh(first: boolean) {
      try { await prewarmClaudeSession(current.sessionId, current.modelId, current.workspaceRef!, first); }
      catch { /* Startup failures are exposed by the shared connection status snapshot. */ }
      finally { if (generation === version) timer = setTimeout(() => void refresh(false), 30_000); }
    }
    // Debounce rapid model/tab selections; the backend owns deduplication and capacity.
    timer = setTimeout(() => void refresh(true), 350);
  }, { immediate: true, flush: 'sync' });
  onScopeDispose(clear);
}
