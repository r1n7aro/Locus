import { beforeEach, describe, expect, it, vi } from 'vitest';
const ipc = vi.hoisted(() => vi.fn());
vi.mock('../services/ipc', () => ({ ipcInvoke: ipc }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
const loggedIn = { serverUrl: 'https://server.internal', user: { id: 'one', username: 'user', role: 'member' }, upstreams: [] };

beforeEach(() => { vi.resetModules(); ipc.mockReset(); });
describe('internal server account state', () => {
  it('does not accept a login response after cancellation', async () => {
    const pending = deferred<unknown>();
    ipc.mockImplementation(command => command === 'server_client_login' ? pending.promise : Promise.resolve());
    const state = await import('../server-client/state');
    const login = state.login('https://server.internal', 'user', 'ephemeral-password');
    await state.cancelLogin();
    pending.resolve(loggedIn);
    await login;
    expect(state.serverStatus.value.user).toBeNull();
    expect(ipc).not.toHaveBeenCalledWith('server_client_models');
  });
  it('does not restore models after logout while an earlier list is in flight', async () => {
    const pending = deferred<unknown>();
    ipc.mockImplementation(command => {
      if (command === 'server_client_status') return Promise.resolve(loggedIn);
      if (command === 'server_client_models') return pending.promise;
      return Promise.resolve({ serverUrl: null, user: null, upstreams: [], serverRevocationConfirmed: false });
    });
    const state = await import('../server-client/state');
    const refresh = state.refreshServer();
    await Promise.resolve();
    await state.logout();
    pending.resolve([{ id: 'locus-server/binding/openai/model', name: 'model', provider: 'openai_codex' }]);
    await refresh;
    expect(state.additionalModels.value).toEqual([]);
    expect(state.serverStatus.value.user).toBeNull();
  });
});
