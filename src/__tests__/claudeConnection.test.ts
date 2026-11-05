// @vitest-environment jsdom
import { createApp, nextTick, type App } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ClaudeConnection from '../components/settings/ClaudeConnection.vue';
import * as service from '../services/claudeConnection';
vi.mock('../services/claudeConnection', async (importOriginal) => ({
  // Constants (the configurable builtin agents) come from the real module.
  ...await importOriginal<typeof import('../services/claudeConnection')>(),
  getClaudeConnection: vi.fn(), inspectClaudeSsh: vi.fn(), authorizeClaudeSsh: vi.fn(), useLocalClaude: vi.fn(), setClaudePermissionMode: vi.fn(),
  clearClaudeSshUnlockPassword: vi.fn(),
  setClaudePrewarm: vi.fn(), setClaudeRuntimeSettings: vi.fn(), setClaudeNativeAgents: vi.fn(), getClaudeNativeAgentsSupport: vi.fn(),
}));
vi.mock('../i18n', () => ({ t: (key: string) => key }));
vi.mock('../services/agent', () => ({ listAgents: vi.fn(), listSubagentDefs: vi.fn() }));
import * as agentService from '../services/agent';
const agentInfo = (id: string, delegatable = true, projectTypes: string[] = []) =>
  ({ id, name: id, description: `${id} agent`, projectTypes, isDefault: false, source: 'builtin', delegatable });
const info = { platform: 'linux', pythonInstalled: true, cliSupported: true, cliVersion: '2.1.299', model: 'test-model', authenticated: true, home: '/home/test' };
let app: App | undefined;
const flushPromises = async () => { await Promise.resolve(); await nextTick(); await Promise.resolve(); await nextTick(); };
function mount(_component: unknown) {
  const host = document.createElement('div'); document.body.append(host);
  const changes: unknown[] = [];
  app = createApp(ClaudeConnection, { onChanged: () => changes.push(true) }); app.mount(host);
  const node = (element: Element) => ({
    text: () => element.textContent?.trim() || '',
    attr: (name: string) => element.getAttribute(name),
    value: () => (element as HTMLInputElement).value,
    trigger: async (name: string) => {
      if (name === 'click') (element as HTMLElement).click();
      else element.dispatchEvent(new Event(name, { bubbles: true }));
      await nextTick();
    },
    setValue: async (value: string) => { (element as HTMLInputElement).value = value; element.dispatchEvent(new Event('input', { bubbles: true })); await nextTick(); },
  });
  return { findAll: (selector: string) => [...host.querySelectorAll(selector)].map(node),
    find: (selector: string) => node(host.querySelector(selector)!), emitted: (_name: string) => changes };
}
afterEach(() => { app?.unmount(); document.body.innerHTML = ''; });
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(service.getClaudeConnection).mockResolvedValue({ version: 1, permissionMode: 'auto', remote: null, authorizedTarget: null, info: null });
  vi.mocked(service.inspectClaudeSsh).mockResolvedValue(info);
  vi.mocked(service.getClaudeNativeAgentsSupport).mockResolvedValue({ cliVersion: '2.1.312 (Claude Code)', supported: true, remote: false, installable: false });
  vi.mocked(agentService.listAgents).mockResolvedValue([agentInfo('general', false), agentInfo('unity', true, ['unity'])]);
  vi.mocked(agentService.listSubagentDefs).mockResolvedValue([agentInfo('explorer', true, ['unity']), agentInfo('researcher')]);
});
describe('Claude SSH authorization', () => {
  it('saves optional spare capacity without probing SSH or sending messages', async () => {
    const remote = { target: 'user@server', port: null, identityFile: null, knownHostsFile: null, cliPath: null };
    const saved = { version: 1, permissionMode: 'auto' as const, remote, authorizedTarget: 'bound', info, prewarm: false, poolCapacity: 2, idleMinutes: 30 };
    vi.mocked(service.getClaudeConnection).mockResolvedValueOnce(saved);
    vi.mocked(service.setClaudePrewarm).mockResolvedValueOnce({ ...saved, prewarm: true });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    await wrapper.find('[role="checkbox"]').trigger('click'); await flushPromises();
    expect(service.setClaudePrewarm).toHaveBeenCalledExactlyOnceWith(true, 2, 30);
    expect(wrapper.find('[role="checkbox"]').attr('aria-checked')).toBe('true');
    expect(service.inspectClaudeSsh).not.toHaveBeenCalled();
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
  });
  it('starts with only an address and tests an inline port before asking for setup', async () => {
    const wrapper=mount(ClaudeConnection);await flushPromises();
    await wrapper.findAll('button').find(b=>b.text()==='SSH')!.trigger('click');
    expect(wrapper.findAll('input:not([type="number"])')).toHaveLength(1);
    expect(wrapper.findAll('details')).toHaveLength(0);
    await wrapper.find('input').setValue('user@server:2222');
    await wrapper.findAll('button').find(b=>b.text().endsWith('.inspect'))!.trigger('click');await flushPromises();
    expect(service.inspectClaudeSsh).toHaveBeenCalledWith({target:'user@server',port:2222,identityFile:null,knownHostsFile:null,cliPath:null});
    expect(wrapper.find('[role="status"]').text()).toContain('.ready');
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    expect(wrapper.findAll('details')).toHaveLength(1);
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
    // Editing optional paths invalidates the test without hiding its form.
    await wrapper.findAll('input')[1]!.setValue('C:/keys/id_ed25519');
    expect(wrapper.findAll('details')).toHaveLength(1);
    expect(wrapper.findAll('[role="status"]')).toHaveLength(0);
  });
  it('reports SSH failures and reveals optional connection settings without deploying', async () => {
    vi.mocked(service.inspectClaudeSsh).mockRejectedValueOnce({message:'Permission denied',code:'ssh',retryable:false});
    const wrapper=mount(ClaudeConnection);await flushPromises();
    await wrapper.findAll('button').find(b=>b.text()==='SSH')!.trigger('click');
    await wrapper.find('input').setValue('user@server');
    await wrapper.findAll('button').find(b=>b.text().endsWith('.inspect'))!.trigger('click');await flushPromises();
    expect(wrapper.find('[role="alert"]').text()).toBe('Permission denied');
    expect(wrapper.findAll('[role="status"]')).toHaveLength(0);
    expect(wrapper.findAll('details')).toHaveLength(1);
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
  });
  it('requires setup authorization then reports native login as the next step', async () => {
    vi.mocked(service.inspectClaudeSsh).mockResolvedValueOnce({...info,pythonInstalled:false,cliSupported:false,authenticated:false,cliVersion:null});
    const wrapper=mount(ClaudeConnection);await flushPromises();
    await wrapper.findAll('button').find(b=>b.text()==='SSH')!.trigger('click');
    await wrapper.find('input').setValue('user@server');
    await wrapper.findAll('button').find(b=>b.text().endsWith('.inspect'))!.trigger('click');await flushPromises();
    expect(wrapper.find('[role="status"]').text()).toContain('.setupRequired');
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
    vi.mocked(service.authorizeClaudeSsh).mockRejectedValueOnce('Remote Claude authentication check failed');
    vi.mocked(service.inspectClaudeSsh).mockResolvedValueOnce({...info,authenticated:false});
    await wrapper.findAll('button').find(b=>b.text().endsWith('.installContinue'))!.trigger('click');await flushPromises();await flushPromises();
    expect(service.authorizeClaudeSsh).toHaveBeenCalledOnce();
    expect(wrapper.find('[role="status"]').text()).toContain('.loginRequired');
    expect(wrapper.findAll('[role="alert"]')).toHaveLength(0);
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    expect(wrapper.emitted('changed')).toHaveLength(0);
  });
  it('does not deploy on mode selection or inspection and invalidates inspection when target changes', async () => {
    const wrapper = mount(ClaudeConnection);
    await flushPromises();
    await wrapper.findAll('button').find(b => b.text() === 'SSH')!.trigger('click');
    await wrapper.find('input').setValue('user@first');
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
    await wrapper.findAll('button').find(b => b.text().endsWith('.inspect'))!.trigger('click');
    await flushPromises();
    expect(service.inspectClaudeSsh).toHaveBeenCalledOnce();
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
    expect(wrapper.findAll('button').some(b => b.text().endsWith('.authorize'))).toBe(true);
    await wrapper.find('input').setValue('user@second');
    expect(wrapper.findAll('button').some(b => b.text().endsWith('.authorize'))).toBe(false);
  });
  it('deploys only after the explicit authorization action', async () => {
    const wrapper = mount(ClaudeConnection);
    await flushPromises();
    await wrapper.findAll('button').find(b => b.text() === 'SSH')!.trigger('click');
    await wrapper.find('input').setValue('user@host');
    await wrapper.findAll('button').find(b => b.text().endsWith('.inspect'))!.trigger('click');
    await flushPromises();
    vi.mocked(service.authorizeClaudeSsh).mockResolvedValue({ version: 1, permissionMode: 'auto', remote: null, authorizedTarget: 'bound', info });
    await wrapper.findAll('button').find(b => b.text().endsWith('.authorize'))!.trigger('click');
    await flushPromises();
    expect(service.authorizeClaudeSsh).toHaveBeenCalledWith({ target: 'user@host', port: null, identityFile: null, knownHostsFile: null, cliPath: null }, undefined);
    expect(wrapper.emitted('changed')).toHaveLength(1);
  });
});

describe('macOS SSH keychain password', () => {
  const remote = { target: 'user@mac', port: null, identityFile: null, knownHostsFile: null, cliPath: null };
  const saved = { version: 1, permissionMode: 'auto' as const, remote, authorizedTarget: 'bound',
    info: { ...info, platform: 'darwin', keychainUnlocked: false, authenticated: false }, hasUnlockPassword: true };
  async function inspectMac(wrapper: ReturnType<typeof mount>) {
    vi.mocked(service.inspectClaudeSsh).mockResolvedValueOnce(saved.info);
    await wrapper.findAll('button').find(b => b.text().endsWith('.inspect'))!.trigger('click');
    await flushPromises();
  }
  it('asks for a password only after a fresh connection detects a keychain issue', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValue(saved);
    const wrapper = mount(ClaudeConnection); await flushPromises();
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    expect(wrapper.findAll('input:not([type="number"])')).toHaveLength(1);
    expect(service.inspectClaudeSsh).not.toHaveBeenCalled();
    await inspectMac(wrapper);
    expect(wrapper.find('input[type="password"]').value()).toBe('');
    expect(wrapper.find('input[type="password"]').attr('placeholder')).toContain('passwordSaved');
    await wrapper.find('input').setValue('user@linux');
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    await wrapper.findAll('button').find(b => b.text().endsWith('.inspect'))!.trigger('click'); await flushPromises();
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
  });
  it('passes a replacement separately from config and clears the input on success', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValue(saved);
    vi.mocked(service.authorizeClaudeSsh).mockResolvedValue({ ...saved, info: { ...saved.info, authenticated: true } });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    await inspectMac(wrapper);
    await wrapper.find('input[type="password"]').setValue('  test password  ');
    await wrapper.findAll('button').find(b => b.text().endsWith('.unlockSave'))!.trigger('click'); await flushPromises();
    expect(service.authorizeClaudeSsh).toHaveBeenCalledWith(remote, '  test password  ');
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    await wrapper.findAll('button').find(b => b.text().endsWith('.changePassword'))!.trigger('click');
    expect(wrapper.find('input[type="password"]').value()).toBe('');
    expect(wrapper.emitted('changed')).toHaveLength(2);
  });
  it('keeps the saved status on test failure, clears draft secrets, and supports explicit deletion', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValue(saved);
    vi.mocked(service.authorizeClaudeSsh).mockRejectedValue(new Error('unlock failed'));
    const wrapper = mount(ClaudeConnection); await flushPromises();
    await inspectMac(wrapper);
    await wrapper.find('input[type="password"]').setValue('incorrect');
    await wrapper.findAll('button').find(b => b.text().endsWith('.unlockSave'))!.trigger('click'); await flushPromises();
    expect(wrapper.find('input[type="password"]').value()).toBe('');
    expect(wrapper.find('input[type="password"]').attr('placeholder')).toContain('passwordSaved');
    expect(wrapper.emitted('changed')).toHaveLength(1);
    await wrapper.findAll('button').find(b => b.text().endsWith('.clearPassword'))!.trigger('click'); await flushPromises();
    expect(service.clearClaudeSshUnlockPassword).toHaveBeenCalledWith(remote);
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    await inspectMac(wrapper);
    expect(wrapper.find('input[type="password"]').attr('placeholder')).toContain('passwordPlaceholder');
  });
  it('retains the stored secret when testing with a blank input', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValue(saved);
    vi.mocked(service.authorizeClaudeSsh).mockResolvedValue({ ...saved, info: { ...saved.info, authenticated: true } });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    await inspectMac(wrapper);
    await wrapper.findAll('button').find(b => b.text().endsWith('.unlockSave'))!.trigger('click'); await flushPromises();
    expect(service.authorizeClaudeSsh).toHaveBeenCalledWith(remote, undefined);
    expect(service.clearClaudeSshUnlockPassword).not.toHaveBeenCalled();
  });
  it.each([
    { ...info, platform: 'darwin', keychainUnlocked: true, authenticated: true },
    { ...info, platform: 'darwin', keychainUnlocked: true, authenticated: false },
    { ...info, platform: 'darwin', keychainUnlocked: false, authenticated: true },
    { ...info, platform: 'linux', keychainUnlocked: false, authenticated: false },
  ])('does not ask for a keychain password without a detected Mac login blockage', async result => {
    vi.mocked(service.getClaudeConnection).mockResolvedValue(saved);
    vi.mocked(service.inspectClaudeSsh).mockResolvedValueOnce(result);
    const wrapper=mount(ClaudeConnection);await flushPromises();
    await wrapper.findAll('button').find(b=>b.text().endsWith('.inspect'))!.trigger('click');await flushPromises();
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(0);
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
  });
});

describe('Claude permission mode', () => {
  it('defaults to Auto and saves an explicit Bypass selection', async () => {
    const wrapper = mount(ClaudeConnection);
    await flushPromises();
    expect(wrapper.findAll('button').find(b => b.text() === 'Auto')!.attr('aria-checked')).toBe('true');
    expect(service.setClaudePermissionMode).not.toHaveBeenCalled();
    vi.mocked(service.setClaudePermissionMode).mockResolvedValue({ version: 1, permissionMode: 'bypassPermissions', remote: null, authorizedTarget: null, info: null });
    await wrapper.findAll('button').find(b => b.text() === 'Bypass')!.trigger('click');
    await flushPromises();
    expect(service.setClaudePermissionMode).toHaveBeenCalledWith('bypassPermissions');
    expect(wrapper.findAll('button').find(b => b.text() === 'Bypass')!.attr('aria-checked')).toBe('true');
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
  });
  it('restores Bypass and keeps the saved selection when an update fails', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValue({ version: 1, permissionMode: 'bypassPermissions', remote: null, authorizedTarget: null, info: null });
    const wrapper = mount(ClaudeConnection);
    await flushPromises();
    expect(wrapper.findAll('button').find(b => b.text() === 'Bypass')!.attr('aria-checked')).toBe('true');
    vi.mocked(service.setClaudePermissionMode).mockRejectedValue(new Error('save failed'));
    await wrapper.findAll('button').find(b => b.text() === 'Auto')!.trigger('click');
    await flushPromises();
    expect(wrapper.find('[role="alert"]').text()).toContain('save failed');
    expect(wrapper.findAll('button').find(b => b.text() === 'Bypass')!.attr('aria-checked')).toBe('true');
  });
});

describe('Claude native multi-agent', () => {
  const saved = { version: 1, permissionMode: 'auto' as const, remote: null, authorizedTarget: null, info: null };
  it('defaults on with the three builtins and saves the switch and each builtin', async () => {
    const wrapper = mount(ClaudeConnection); await flushPromises();
    const native = wrapper.find('[aria-label="settings.claudeCode.nativeMultiAgent"]');
    expect(native.attr('aria-checked')).toBe('true');
    const checked = () => wrapper.findAll('.connection-builtin-agents [role="checkbox"]')
      .filter(box => box.attr('aria-checked') === 'true').map(box => box.attr('aria-label'));
    expect(checked()).toEqual(['general-purpose', 'Explore', 'Plan']);
    expect(service.setClaudeNativeAgents).not.toHaveBeenCalled();

    // A builtin is added in the published order, never appended.
    vi.mocked(service.setClaudeNativeAgents).mockResolvedValueOnce({ ...saved, nativeMultiAgent: true, nativeAgents: { builtins: ['general-purpose', 'Explore', 'Plan', 'claude'] } });
    await wrapper.find('[aria-label="claude"]').trigger('click'); await flushPromises();
    expect(service.setClaudeNativeAgents).toHaveBeenLastCalledWith(true, ['general-purpose', 'Explore', 'Plan', 'claude'], []);
    expect(checked()).toEqual(['general-purpose', 'Explore', 'Plan', 'claude']);

    // Turning the mode off keeps the builtins and hides their picker.
    vi.mocked(service.setClaudeNativeAgents).mockResolvedValueOnce({ ...saved, nativeMultiAgent: false, nativeAgents: { builtins: ['general-purpose', 'Explore', 'Plan', 'claude'] } });
    await native.trigger('click'); await flushPromises();
    expect(service.setClaudeNativeAgents).toHaveBeenLastCalledWith(false, ['general-purpose', 'Explore', 'Plan', 'claude'], []);
    expect(native.attr('aria-checked')).toBe('false');
    expect(wrapper.findAll('.connection-builtin-agents')).toEqual([]);
    expect(wrapper.findAll('.connection-locus-agents')).toEqual([]);
    expect(service.inspectClaudeSsh).not.toHaveBeenCalled();
  });

  it('offers every delegatable Locus agent by default and saves exclusions', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValue({ ...saved, nativeAgents: { builtins: ['Explore'], excludedLocusAgents: ['gone'] } });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    const boxes = () => wrapper.findAll('.connection-locus-agents [role="checkbox"]');
    // Non-delegatable agents (general) are not listed; a type limited to a
    // project type shows it.
    expect(boxes().map(box => box.attr('aria-label'))).toEqual(['unity', 'explorer', 'researcher']);
    expect(boxes().every(box => box.attr('aria-checked') === 'true')).toBe(true);
    expect(wrapper.findAll('.connection-agent-scope').map(scope => scope.text())).toEqual(['Unity', 'Unity']);

    // Unchecking adds the id; an unlisted saved id stays excluded.
    vi.mocked(service.setClaudeNativeAgents).mockResolvedValueOnce({ ...saved, nativeMultiAgent: true, nativeAgents: { builtins: ['Explore'], excludedLocusAgents: ['gone', 'explorer'] } });
    await wrapper.find('.connection-locus-agents [aria-label="explorer"]').trigger('click'); await flushPromises();
    expect(service.setClaudeNativeAgents).toHaveBeenLastCalledWith(true, ['Explore'], ['gone', 'explorer']);
    expect(wrapper.find('.connection-locus-agents [aria-label="explorer"]').attr('aria-checked')).toBe('false');

    // A builtin change keeps the exclusions; rechecking removes the id.
    vi.mocked(service.setClaudeNativeAgents).mockResolvedValueOnce({ ...saved, nativeMultiAgent: true, nativeAgents: { builtins: ['Explore', 'Plan'], excludedLocusAgents: ['gone', 'explorer'] } });
    await wrapper.find('[aria-label="Plan"]').trigger('click'); await flushPromises();
    expect(service.setClaudeNativeAgents).toHaveBeenLastCalledWith(true, ['Explore', 'Plan'], ['gone', 'explorer']);
    vi.mocked(service.setClaudeNativeAgents).mockResolvedValueOnce({ ...saved, nativeMultiAgent: true, nativeAgents: { builtins: ['Explore', 'Plan'], excludedLocusAgents: ['gone'] } });
    await wrapper.find('.connection-locus-agents [aria-label="explorer"]').trigger('click'); await flushPromises();
    expect(service.setClaudeNativeAgents).toHaveBeenLastCalledWith(true, ['Explore', 'Plan'], ['gone']);
  });

  it('warns when the local CLI is too old and hides the warning while the mode is off', async () => {
    vi.mocked(service.getClaudeNativeAgentsSupport).mockResolvedValue({ cliVersion: '2.1.302 (Claude Code)', supported: false, remote: false, installable: false });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    expect(wrapper.find('.connection-warning').text()).toBe('settings.claudeCode.nativeUnsupportedVersion');
    expect(wrapper.findAll('button').some(b => b.text() === 'settings.claudeSsh.upgradeSave')).toBe(false);
    vi.mocked(service.setClaudeNativeAgents).mockResolvedValueOnce({ ...saved, nativeMultiAgent: false, nativeAgents: { builtins: ['general-purpose', 'Explore', 'Plan'] } });
    await wrapper.find('[aria-label="settings.claudeCode.nativeMultiAgent"]').trigger('click'); await flushPromises();
    expect(wrapper.findAll('.connection-warning')).toEqual([]);
  });

  it('upgrades a saved SSH connection whose Claude Code is too old after a fresh test', async () => {
    const remote = { target: 'user@server', port: null, identityFile: null, knownHostsFile: null, cliPath: null };
    const old = { ...info, cliVersion: '2.1.302 (Claude Code)' };
    const savedRemote = { ...saved, remote, authorizedTarget: 'bound', info: old };
    vi.mocked(service.getClaudeConnection).mockResolvedValue(savedRemote);
    vi.mocked(service.inspectClaudeSsh).mockResolvedValue(old);
    vi.mocked(service.getClaudeNativeAgentsSupport).mockResolvedValue({ cliVersion: old.cliVersion, supported: false, remote: true, installable: true });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    expect(wrapper.find('.connection-warning').text()).toBe('settings.claudeCode.nativeUnsupportedVersion');
    // Cached information is not a connection test: no button until tested.
    expect(wrapper.findAll('button').some(b => b.text() === 'settings.claudeSsh.upgradeSave')).toBe(false);
    await wrapper.findAll('button').find(b => b.text().endsWith('.inspect'))!.trigger('click'); await flushPromises();
    const upgrade = () => wrapper.findAll('button').find(b => b.text() === 'settings.claudeSsh.upgradeSave');
    expect(upgrade()).toBeDefined();

    vi.mocked(service.authorizeClaudeSsh).mockResolvedValue({ ...savedRemote, info: { ...old, cliVersion: '2.1.312 (Claude Code)' } });
    vi.mocked(service.getClaudeNativeAgentsSupport).mockResolvedValue({ cliVersion: '2.1.312 (Claude Code)', supported: true, remote: true, installable: true });
    await upgrade()!.trigger('click'); await flushPromises();
    expect(service.authorizeClaudeSsh).toHaveBeenCalledExactlyOnceWith(remote, undefined);
    expect(wrapper.findAll('.connection-warning')).toEqual([]);
    expect(upgrade()).toBeUndefined();
  });

  it('does not offer an upgrade for a custom remote Claude path', async () => {
    const remote = { target: 'user@server', port: null, identityFile: null, knownHostsFile: null, cliPath: '/opt/claude' };
    const old = { ...info, cliVersion: '2.1.302 (Claude Code)' };
    vi.mocked(service.getClaudeConnection).mockResolvedValue({ ...saved, remote, authorizedTarget: 'bound', info: old });
    vi.mocked(service.inspectClaudeSsh).mockResolvedValue(old);
    vi.mocked(service.getClaudeNativeAgentsSupport).mockResolvedValue({ cliVersion: old.cliVersion, supported: false, remote: true, installable: false });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    await wrapper.findAll('button').find(b => b.text().endsWith('.inspect'))!.trigger('click'); await flushPromises();
    expect(wrapper.find('.connection-warning').text()).toBe('settings.claudeCode.nativeUnsupportedVersion');
    expect(wrapper.findAll('button').some(b => b.text() === 'settings.claudeSsh.upgradeSave')).toBe(false);
  });
});

describe('Claude runtime preferences', () => {
  const saved = { version: 1, permissionMode: 'auto' as const, remote: null, authorizedTarget: null, info: null };
  it('defaults both preferences off for old settings and saves independently', async () => {
    const wrapper = mount(ClaudeConnection);
    await flushPromises();
    const idle = wrapper.find('[aria-label="settings.claudeCode.idleCompaction"]');
    const telemetry = wrapper.find('[aria-label="settings.claudeCode.telemetry"]');
    expect(idle.attr('aria-checked')).toBe('false');
    expect(telemetry.attr('aria-checked')).toBe('false');
    expect(service.setClaudeRuntimeSettings).not.toHaveBeenCalled();
    vi.mocked(service.setClaudeRuntimeSettings).mockResolvedValueOnce({ ...saved, idleCompaction: true, telemetry: false });
    await idle.trigger('click'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).toHaveBeenLastCalledWith(true, false, null);
    expect(idle.attr('aria-checked')).toBe('true');
    vi.mocked(service.setClaudeRuntimeSettings).mockResolvedValueOnce({ ...saved, idleCompaction: true, telemetry: true });
    await telemetry.trigger('click'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).toHaveBeenLastCalledWith(true, true, null);
    expect(telemetry.attr('aria-checked')).toBe('true');
    expect(service.inspectClaudeSsh).not.toHaveBeenCalled();
  });
  it('edits and clears the remote native compaction window while preserving other preferences', async () => {
    const remote = { target: 'user@server', port: null, identityFile: null, knownHostsFile: null, cliPath: null };
    const connection = { ...saved, remote, idleCompaction: true, telemetry: true, autoCompactWindow: 200_000 };
    vi.mocked(service.getClaudeConnection).mockResolvedValueOnce(connection);
    const wrapper = mount(ClaudeConnection); await flushPromises();
    const input = wrapper.find('[aria-label="settings.claudeCode.autoCompactWindow"]');
    expect(input.value()).toBe('200000');
    expect(input.attr('disabled')).toBeNull();
    vi.mocked(service.setClaudeRuntimeSettings).mockResolvedValueOnce({ ...connection, autoCompactWindow: 350_000 });
    await input.setValue('350000'); await input.trigger('change'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).toHaveBeenLastCalledWith(true, true, 350_000);
    expect(input.value()).toBe('350000');
    // Updating another preference must not silently reset the threshold.
    vi.mocked(service.setClaudeRuntimeSettings).mockResolvedValueOnce({ ...connection, telemetry: false, autoCompactWindow: 350_000 });
    await wrapper.find('[aria-label="settings.claudeCode.telemetry"]').trigger('click'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).toHaveBeenLastCalledWith(true, false, 350_000);
    vi.mocked(service.setClaudeRuntimeSettings).mockResolvedValueOnce({ ...connection, telemetry: false, autoCompactWindow: null });
    await input.setValue(''); await input.trigger('change'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).toHaveBeenLastCalledWith(true, false, null);
    expect(input.value()).toBe('');
    expect(service.inspectClaudeSsh).not.toHaveBeenCalled();
    expect(service.authorizeClaudeSsh).not.toHaveBeenCalled();
  });
  it.each(['0', '99999', '1000001', '200000.5'])('rejects invalid compaction windows without saving: %s', async value => {
    const wrapper = mount(ClaudeConnection); await flushPromises();
    const input = wrapper.find('[aria-label="settings.claudeCode.autoCompactWindow"]');
    await input.setValue(value); await input.trigger('change'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).not.toHaveBeenCalled();
    expect(input.value()).toBe('');
    expect(wrapper.find('[role="alert"]').text()).toContain('autoCompactWindowInvalid');
  });
  it.each([100_000, 1_000_000])('accepts native compaction window boundary %s', async value => {
    const wrapper = mount(ClaudeConnection); await flushPromises();
    vi.mocked(service.setClaudeRuntimeSettings).mockResolvedValueOnce({ ...saved, autoCompactWindow: value });
    const input = wrapper.find('[aria-label="settings.claudeCode.autoCompactWindow"]');
    await input.setValue(String(value)); await input.trigger('change'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).toHaveBeenLastCalledWith(false, false, value);
    expect(input.value()).toBe(String(value));
  });
  it('restores the saved compaction window after a failed save', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValueOnce({ ...saved, autoCompactWindow: 200_000 });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    vi.mocked(service.setClaudeRuntimeSettings).mockRejectedValueOnce(new Error('save failed'));
    const input = wrapper.find('[aria-label="settings.claudeCode.autoCompactWindow"]');
    await input.setValue('350000'); await input.trigger('change'); await flushPromises();
    expect(input.value()).toBe('200000');
    expect(wrapper.find('[role="alert"]').text()).toContain('save failed');
  });
  it('restores saved preferences and leaves them intact when saving fails', async () => {
    vi.mocked(service.getClaudeConnection).mockResolvedValueOnce({ ...saved, idleCompaction: true, telemetry: true });
    const wrapper = mount(ClaudeConnection); await flushPromises();
    vi.mocked(service.setClaudeRuntimeSettings).mockRejectedValueOnce(new Error('save failed'));
    await wrapper.find('[aria-label="settings.claudeCode.telemetry"]').trigger('click'); await flushPromises();
    expect(service.setClaudeRuntimeSettings).toHaveBeenLastCalledWith(true, false, null);
    expect(wrapper.find('[aria-label="settings.claudeCode.telemetry"]').attr('aria-checked')).toBe('true');
    expect(wrapper.find('[role="alert"]').text()).toContain('save failed');
  });
});
