import { effectScope, ref, type Ref } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useClaudeConnectionStatus } from '../composables/useClaudeConnectionStatus';
import * as service from '../services/claudeConnection';
import * as cliGate from '../services/claudeCodeCli';

const pushed = vi.hoisted(() => new Set<(event: { status: unknown; error: string | null }) => void>());
vi.mock('../services/claudeCodeCli', () => ({ claudeCodeCliEnabled: ref(true) }));
vi.mock('../services/claudeConnection', () => ({
  getClaudeSessionConnection: vi.fn(),
  subscribeClaudeConnectionStatus: vi.fn(async (handler: (event: { status: unknown; error: string | null }) => void) => {
    pushed.add(handler);
    return () => pushed.delete(handler);
  }),
  isRemoteClaudeCode: ref(true),
  claudePrewarmSettings: ref({ enabled: true, capacity: 2, idleMinutes: 15, scope: 'ssh-a' }),
}));
const ready: service.ClaudeSessionConnection = { phase: 'ready', target: 'user@server', port: null, latencyMs: 42,
  pool: { state: 'ready', capacity: 2, ready: 2, starting: 0, active: 1, retained: 3 } };
const scopes: ReturnType<typeof effectScope>[] = [];
function harness() {
  const scope = effectScope();
  scopes.push(scope);
  return { ...scope.run(useClaudeConnectionStatus)!, stop: () => scope.stop() };
}
const settle = async () => { for (let index = 0; index < 8; index += 1) await Promise.resolve(); };
function push(status: service.ClaudeSessionConnection | null, error: string | null = null) {
  for (const handler of [...pushed]) handler({ status, error });
}
const calls = () => vi.mocked(service.getClaudeSessionConnection).mock.calls.length;
const settings = service.claudePrewarmSettings as Ref<{ enabled: boolean; capacity: number; idleMinutes: number; scope: string }>;
beforeEach(() => {
  vi.useFakeTimers();
  pushed.clear();
  (cliGate.claudeCodeCliEnabled as unknown as Ref<boolean>).value = true;
  (service.isRemoteClaudeCode as Ref<boolean>).value = true;
  settings.value = { enabled: true, capacity: 2, idleMinutes: 15, scope: 'ssh-a' };
  vi.mocked(service.getClaudeSessionConnection).mockReset().mockResolvedValue(ready);
});
afterEach(() => { scopes.splice(0).forEach(scope => scope.stop()); vi.useRealTimers(); });

describe('shared remote Claude process status', () => {
  it('shares one snapshot across panes and applies pushes without polling', async () => {
    const left = harness(), right = harness();
    await settle();
    expect(service.getClaudeSessionConnection).toHaveBeenCalledExactlyOnceWith(null);
    expect(pushed.size).toBe(1);
    expect(left.status).toBe(right.status);
    expect(left.status.value).toEqual(ready);
    const third = harness();
    expect(third.phase.value).toBe('ready');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(calls()).toBe(1);
    push({ ...ready, latencyMs: 85, pool: { ...ready.pool, active: 2 } });
    for (const pane of [left, right, third]) {
      expect(pane.visible.value).toBe(true);
      expect(pane.status.value?.latencyMs).toBe(85);
      expect(pane.status.value?.pool.active).toBe(2);
    }
  });

  it('keeps the detached subscription until the last pane closes', async () => {
    const first = harness(), second = harness();
    await settle();
    first.stop();
    push({ ...ready, phase: 'disconnected', latencyMs: null });
    expect(second.phase.value).toBe('disconnected');
    second.stop();
    expect(pushed.size).toBe(0);
  });

  it('shows a snapshot failure in every pane and recovers from the next push', async () => {
    vi.mocked(service.getClaudeSessionConnection).mockRejectedValueOnce(new Error('IPC unavailable'));
    const left = harness(), right = harness();
    await settle();
    for (const pane of [left, right]) {
      expect(pane.phase.value).toBe('unknown');
      expect(pane.status.value).toBeNull();
      expect(pane.error.value).toBe('IPC unavailable');
    }
    push(ready);
    expect(left.phase.value).toBe('ready');
    expect(right.error.value).toBe('');
  });

  it('shows pushed startup and settings errors and clears them on recovery or a server change', async () => {
    const error = 'Claude plugin validation failed.\nregister.js:116: invalid helper';
    const failed = { ...ready, phase: 'disconnected' as const, lastError: error,
      pool: { ...ready.pool, state: 'retrying' as const, ready: 0 } };
    const left = harness(), right = harness();
    await settle();
    push(failed);
    expect(left.error.value).toBe(error);
    push(ready);
    expect(right.error.value).toBe('');
    push(null, 'Authorize environment setup for this SSH target in Settings');
    expect(left.status.value).toBeNull();
    expect(left.error.value).toBe('Authorize environment setup for this SSH target in Settings');
    vi.mocked(service.getClaudeSessionConnection).mockResolvedValue({ ...ready, target: 'server-b' });
    settings.value = { ...settings.value, scope: 'ssh-b' };
    expect(left.error.value).toBe('');
    await settle();
    expect(right.status.value?.target).toBe('server-b');
  });

  it('hides all icons and unsubscribes in local mode, then resumes once for all panes', async () => {
    const left = harness(), right = harness();
    await settle();
    (service.isRemoteClaudeCode as Ref<boolean>).value = false;
    for (const pane of [left, right]) {
      expect(pane.visible.value).toBe(false);
      expect(pane.status.value).toBeNull();
    }
    expect(pushed.size).toBe(0);
    const count = calls();
    await vi.advanceTimersByTimeAsync(3000);
    expect(calls()).toBe(count);
    (service.isRemoteClaudeCode as Ref<boolean>).value = true;
    await settle();
    expect(calls()).toBe(count + 1);
    expect(right.phase.value).toBe('ready');
  });

  it('hides remote status while the experimental provider is off', async () => {
    const pane = harness();
    await settle();
    (cliGate.claudeCodeCliEnabled as unknown as Ref<boolean>).value = false;
    expect(pane.visible.value).toBe(false);
    expect(pane.status.value).toBeNull();
    expect(pushed.size).toBe(0);
  });

  it('ignores late snapshots from a previous server for all panes', async () => {
    let old!: (value: service.ClaudeSessionConnection) => void;
    vi.mocked(service.getClaudeSessionConnection).mockImplementationOnce(() => new Promise(resolve => { old = resolve; }));
    const left = harness(), right = harness();
    await settle();
    vi.mocked(service.getClaudeSessionConnection).mockResolvedValue({ ...ready, target: 'server-b' });
    settings.value = { ...settings.value, scope: 'ssh-b' };
    await settle();
    old({ ...ready, target: 'server-a' });
    await settle();
    expect(left.status.value?.target).toBe('server-b');
    expect(right.status.value?.target).toBe('server-b');
  });

  it('does not let a disposed subscription overwrite a new one', async () => {
    let old!: (value: service.ClaudeSessionConnection) => void;
    vi.mocked(service.getClaudeSessionConnection).mockImplementationOnce(() => new Promise(resolve => { old = resolve; }));
    const first = harness();
    await settle();
    first.stop();
    const next = harness();
    await settle();
    old({ ...ready, phase: 'connecting' });
    await settle();
    expect(next.phase.value).toBe('ready');
    expect(pushed.size).toBe(1);
  });

  it('refreshes pool setting changes immediately but not equivalent configuration reads', async () => {
    const pane = harness();
    await settle();
    settings.value = { ...settings.value };
    await settle();
    expect(calls()).toBe(1);
    vi.mocked(service.getClaudeSessionConnection).mockResolvedValue({ ...ready, pool: { ...ready.pool, capacity: 3 } });
    settings.value = { ...settings.value, capacity: 3 };
    await settle();
    expect(calls()).toBe(2);
    expect(pane.status.value?.pool.capacity).toBe(3);
  });
});
