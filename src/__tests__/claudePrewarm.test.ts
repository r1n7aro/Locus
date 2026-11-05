import { effectScope, reactive, ref } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useClaudePrewarm } from '../composables/useClaudePrewarm';
import * as service from '../services/claudeConnection';
vi.mock('@locus/server-client', () => ({
  protocolModelId: (model: string) => model.startsWith('locus-server/') ? model.split('/').slice(2).join('/') : model,
}));
vi.mock('../services/claudeConnection', () => ({
  claudePrewarmSettings: ref({ enabled: true, capacity: 1, idleMinutes: 15, scope: 'server-a' }),
  prewarmClaudeSession: vi.fn(),
}));
const scopes: ReturnType<typeof effectScope>[] = [];
function harness() {
  const context = reactive({ sessionId: null as string | null, modelId: 'claude_code/claude-opus-5-5', active: true, busy: false,
    workspaceRef: { checkoutId: 'checkout-a', expectedGeneration: 1 } });
  const scope = effectScope(); scopes.push(scope);
  scope.run(() => useClaudePrewarm(() => context));
  return { context, stop: () => scope.stop() };
}
beforeEach(() => { vi.useFakeTimers(); vi.mocked(service.prewarmClaudeSession).mockReset().mockResolvedValue(); });
afterEach(() => { scopes.splice(0).forEach(s => s.stop()); vi.useRealTimers(); });
describe('remote Claude prewarm interest', () => {
  it('warms server models with the full binding and leaves Codex models alone', async () => {
    const h=harness();h.context.modelId='locus-server/binding/claude_code/sonnet';
    await vi.advanceTimersByTimeAsync(350);
    expect(service.prewarmClaudeSession).toHaveBeenCalledExactlyOnceWith(null,h.context.modelId,h.context.workspaceRef,true);
    h.context.modelId='locus-server/binding/openai/gpt-test';
    await vi.advanceTimersByTimeAsync(31000);
    expect(service.prewarmClaudeSession).toHaveBeenCalledTimes(1);
  });
  it('defers selection changes during a turn and warms the selected model after completion', async () => {
    const h = harness(); h.context.busy = true;
    h.context.modelId = 'claude_code/claude-sonnet-5-5';
    await vi.advanceTimersByTimeAsync(1000);
    expect(service.prewarmClaudeSession).not.toHaveBeenCalled();
    h.context.busy = false;
    await vi.advanceTimersByTimeAsync(350);
    expect(service.prewarmClaudeSession).toHaveBeenCalledExactlyOnceWith(null, h.context.modelId, h.context.workspaceRef, true);
  });
  it('debounces selection, warms once, then only maintains spare capacity', async () => {
    const h = harness();
    h.context.modelId = 'claude_code/claude-sonnet-5-5';
    await vi.advanceTimersByTimeAsync(349);
    expect(service.prewarmClaudeSession).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(service.prewarmClaudeSession).toHaveBeenCalledExactlyOnceWith(null, h.context.modelId, h.context.workspaceRef, true);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(service.prewarmClaudeSession).toHaveBeenLastCalledWith(null, h.context.modelId, h.context.workspaceRef, false);
  });
  it('stops maintaining a hidden composer or API selection, without killing a sticky session', async () => {
    const h = harness(); h.context.active = false;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(service.prewarmClaudeSession).not.toHaveBeenCalled();
    h.context.active = true; h.context.modelId = 'openai/gpt';
    await vi.advanceTimersByTimeAsync(60_000);
    expect(service.prewarmClaudeSession).not.toHaveBeenCalled();
    h.context.modelId = 'claude_code/claude-opus-5-5'; h.context.sessionId = 'sticky';
    await vi.advanceTimersByTimeAsync(350);
    expect(service.prewarmClaudeSession).toHaveBeenCalledExactlyOnceWith('sticky', h.context.modelId, h.context.workspaceRef, true);
    h.stop(); await vi.advanceTimersByTimeAsync(60_000);
    expect(service.prewarmClaudeSession).toHaveBeenCalledTimes(1);
  });
  it('does not overlap slow heartbeats or resurrect an old workspace after switching', async () => {
    let finish!: () => void;
    vi.mocked(service.prewarmClaudeSession).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    const h = harness(); await vi.advanceTimersByTimeAsync(90_000);
    expect(service.prewarmClaudeSession).toHaveBeenCalledTimes(1);
    h.context.workspaceRef.checkoutId = 'checkout-b'; await vi.advanceTimersByTimeAsync(350);
    finish(); await vi.advanceTimersByTimeAsync(30_000);
    expect(service.prewarmClaudeSession).toHaveBeenCalledTimes(3);
    expect(service.prewarmClaudeSession).toHaveBeenLastCalledWith(null, h.context.modelId, h.context.workspaceRef, false);
  });
});
