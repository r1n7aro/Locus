// @vitest-environment jsdom
import { createPinia } from 'pinia';
import { createApp, h, nextTick, ref, type App } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ClaudeSessionConnection, ClaudeTransportSnapshot } from '../services/claudeConnection';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), status: vi.fn(), connect: vi.fn(),
  pushes: new Set<(event: { status: unknown; error: string | null }) => void>() }));
vi.mock('../services/ipc', () => ({ ipcInvoke: mocks.invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ confirm: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../services/claudeConnection', async (importOriginal) => ({
  ...await importOriginal<typeof import('../services/claudeConnection')>(),
  getClaudeSessionConnection: mocks.status,
  subscribeClaudeConnectionStatus: async (handler: (event: { status: unknown; error: string | null }) => void) => {
    mocks.pushes.add(handler);
    return () => mocks.pushes.delete(handler);
  },
  connectClaudeSession: mocks.connect,
  isRemoteClaudeCode: ref(true),
  claudePrewarmSettings: ref({ enabled: true, capacity: 2, idleMinutes: 15, scope: 'ssh-a' }),
}));
vi.mock('../services/claudeCodeCli', async (importOriginal) => ({
  ...await importOriginal<typeof import('../services/claudeCodeCli')>(),
  claudeCodeCliEnabled: ref(true),
}));
vi.mock('../composables/useClaudePrewarm', () => ({ useClaudePrewarm: () => {} }));
vi.mock('../services/locusRuntime', () => ({ getLocusRuntime: () => ({
  invoke: mocks.invoke, subscribe: async () => () => {},
}) }));

import ChatStatusIndicators from '../components/chat/ChatStatusIndicators.vue';
import { t } from '../i18n';

let app: App | undefined;
let host: HTMLDivElement;
const cause = 'Claude plugin validation failed.\nregister.js:116: invalid helper <script>bad</script>';
const failed: ClaudeSessionConnection = { phase: 'disconnected', target: 'user@server', port: null,
  latencyMs: null, lastError: cause,
  pool: { state: 'retrying', capacity: 2, ready: 0, starting: 0, active: 0, retained: 0 } };
async function flush() { for (let i = 0; i < 25; i++) await nextTick(); }
/** The backend pushes the connection overview whenever it changes. */
async function push(status: ClaudeSessionConnection | null) {
  for (const handler of [...mocks.pushes]) handler({ status, error: null });
  await flush();
}
function icon() { return [...host.querySelectorAll<HTMLButtonElement>('button')]
  .find(button => button.getAttribute('aria-label')?.startsWith(`${t('model.provider.claude_code.remote')}:`))!; }
async function mount() {
  app = createApp(() => h(ChatStatusIndicators, { workspaceRef: { checkoutId: 'test', expectedGeneration: 1, expectedMaterializationEpoch: 0 },
    projectServices: [], selectedModelId: 'claude_code/opus' }));
  app.use(createPinia()); app.mount(host); await flush();
  await vi.advanceTimersByTimeAsync(1);
  icon().click(); await flush();
}
beforeEach(() => {
  vi.useFakeTimers();
  mocks.invoke.mockReset().mockImplementation(async (command: string) => command === 'mcp_get_status' || command === 'list_models' ? [] : null);
  mocks.status.mockReset().mockResolvedValue(failed);
  mocks.connect.mockReset().mockResolvedValue(undefined);
  mocks.pushes.clear();
  host = document.createElement('div'); document.body.appendChild(host);
});
afterEach(() => { app?.unmount(); app = undefined; host.remove(); vi.useRealTimers(); });

describe('Claude connection error panel', () => {
  it('displays background errors without a manual connection and clears them after recovery', async () => {
    await mount();
    expect(mocks.connect).not.toHaveBeenCalled();
    expect(icon().classList.contains('tone-danger')).toBe(true);
    expect(host.querySelector('.chat-status-popover-summary')?.textContent).toBe(t('chat.status.claude.failed'));
    expect(host.querySelector('[role="alert"]')?.textContent?.trim()).toBe(cause);
    expect(host.querySelector('script')).toBeNull();
    expect(host.textContent).toContain(t('chat.status.claude.pool.retrying'));
    await push({ ...failed, phase: 'starting' });
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('register.js:116');
    await push({ ...failed, phase: 'ready', lastError: null, pool: { ...failed.pool, state: 'ready', ready: 2 } });
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(icon().classList.contains('tone-success')).toBe(true);
  });

  it('shows passive progress and retains failure evidence without running checks from the panel', async () => {
    const trace: ClaudeTransportSnapshot = {
      id: 7, pid: 123, operation: 'history', stage: 'historyTransfer', elapsedMs: 45000, stageElapsedMs: 40000,
      receivedBytes: 262144, sentBytes: 128, lastReceiveAgeMs: 500, partialFrameBytes: 262144,
      expectedFrameBytes: 1048576, stageReceivedBytes: 262144, stageBytesPerSecond: 6553.6,
      heartbeatAgeMs: null, pendingHeartbeats: 4,
      helper: { helperVersion: '2', sourceHash: 'ab'.repeat(32), hostPid: 61196, claudePid: 61210,
        startedAt: 1000, updatedAt: 1100, event: 'ready', exitReason: null, exitCode: null,
        claudeExitCode: null, errorType: null, reportingError: null, cleanupError: null },
      timeline: [{ stage: 'ssh', elapsedMs: 0 }, { stage: 'historyTransfer', elapsedMs: 5000 }],
    };
    const diagnostics = {
      active: trace,
      lastFailure: { at: 1700000000000, kind: 'historyTransferTimeout', message: '45s expired',
        stderr: 'SSH <script>not executable</script>', exitCode: null, transport: { ...trace,
          helper: { ...trace.helper!, event: 'exit', exitReason: 'keyboard_interrupt', exitCode: 130,
            reportingError: 'OSError', updatedAt: 2000 } } },
      probe: { failureId: 6, at: 1700000000000, state: 'complete', checks: [
        { name: 'sshBanner', outcome: 'ok', elapsedMs: 600, detail: 'SSH-2.0-test; no authentication attempted' },
      ] },
    };
    mocks.status.mockResolvedValue({ ...failed, diagnostics });
    await mount();
    expect(host.textContent).toContain(t('chat.status.claude.diagnostics.waitingHeartbeat'));
    const details = host.querySelector<HTMLDetailsElement>('details.chat-status-diagnostics')!;
    expect(details).not.toBeNull();
    expect(details.open).toBe(false);
    details.open = true;
    expect(details.textContent).toContain('256.0 KiB / 1.00 MiB');
    expect(details.textContent).toContain('historyTransferTimeout');
    expect(details.textContent).toContain('sshBanner: ok');
    expect(details.textContent).toContain('Python PID 61196 · Claude PID 61210');
    expect(details.textContent).toContain('SHA-256 ' + 'ab'.repeat(32));
    expect(details.textContent).toContain('keyboard_interrupt · exit 130');
    expect(details.textContent).toContain('reportingError: OSError');
    expect(details.textContent).toContain('#6'); // probe belongs to an earlier failure, not active #7
    expect(host.querySelector('script')).toBeNull();
    expect(mocks.connect).not.toHaveBeenCalled();
    await push({ ...failed, phase: 'ready', lastError: null, diagnostics: {
      ...diagnostics, active: { ...trace, id: 8, operation: 'session', stage: 'ready', heartbeatAgeMs: 200 },
    } });
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(icon().classList.contains('tone-success')).toBe(true);
    expect(host.textContent).toContain(t('chat.status.claude.diagnostics.lastFailure'));
    expect(host.textContent).toContain('historyTransferTimeout');
    expect(mocks.connect).not.toHaveBeenCalled();
  });

  it('reports the independent backup pool without changing chat health or opening connections', async () => {
    const archivePool = { ready: 1, busy: 1, queued: 2, starting: 0, lastError: 'Backup transfer stalled <script>bad</script>' };
    mocks.status.mockResolvedValue({ ...failed, phase: 'ready', lastError: null, archivePool });
    await mount();
    expect(host.textContent).toContain(t('chat.status.claude.archivePool'));
    expect(host.textContent).toContain(t('chat.status.claude.archiveCounts', 1, 1, 2, 0));
    expect(host.textContent).toContain(archivePool.lastError);
    expect(icon().classList.contains('tone-success')).toBe(true);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('script')).toBeNull();
    expect(mocks.connect).not.toHaveBeenCalled();
    await push({ ...failed, phase: 'ready', lastError: null, archivePool: null });
    expect(host.textContent).not.toContain(t('chat.status.claude.archivePool'));
  });

  it('preserves manual failures when a later disconnected snapshot arrives', async () => {
    mocks.status.mockResolvedValue(null);
    mocks.connect.mockRejectedValue(new Error('SSH permission denied'));
    await mount();
    await vi.advanceTimersByTimeAsync(1);
    host.querySelector<HTMLButtonElement>('.chat-status-action')!.click(); await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('SSH permission denied');
    await push({ ...failed, lastError: null });
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('SSH permission denied');
  });
});
