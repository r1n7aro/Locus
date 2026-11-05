import { describe, expect, it, vi } from 'vitest';
// The self-contained module embedded in the Rust CLI launcher.
// @ts-expect-error Claude loads this JavaScript module directly.
import { register } from '../../crates/locus-remote-core/src/resources/native_mod.js';

const ENVIRONMENT = '# Environment\nYour tools run through Locus.';
const agents = {
  version: 1,
  builtins: ['Explore'],
  environment: ENVIRONMENT,
  definitions: [
    { name: 'reviewer', backend: 'native', description: 'Reviews code', prompt: 'You review.', tools: ['Read'], model: 'claude-sonnet-4-6' },
    { name: 'researcher', backend: 'locus', description: 'Researches', prompt: 'Locus runs this agent type.', tools: [], model: 'inherit' },
  ],
};

// The host's registry: a1 runs, a2 finished, a3 was stopped by the user.
const ROUTES: Record<string, object> = {
  a1: { ok: true, native: true, running: true, stopped: false },
  a2: { ok: true, native: true, running: false, stopped: false },
  a3: { ok: true, native: true, running: false, stopped: true },
};

// One native delegation conversation: the published directory, an active
// Locus turn, and a bridge that answers each endpoint like the host.
async function delegation() {
  const hooks = new Map<string, { run: Function; fail?: Function }>();
  register((name: string, matchOrRun: Function | object, handler?: Function) => {
    const entry = { run: (handler || matchOrRun) as Function, fail: undefined as Function | undefined };
    hooks.set(name, entry);
    return { catch(fail: Function) { entry.fail = fail; } };
  });
  const state: { options: Record<string, unknown>; step: object } = {
    options: { active: true, inputId: 'run-1', messageId: 'm1', prompt: 'Delegate the work', tools: [], agents },
    step: { ok: true },
  };
  const fetch = vi.fn(async (url: string, init: { body: string }) => {
    const body = JSON.parse(init.body);
    const answer = url.endsWith('/options') ? state.options
      : url.endsWith('/call') ? { callId: body.callId, result: body.tool === 'SendMessage'
        ? { success: true, message: 'Message queued.' } : { status: 'completed', content: [{ type: 'text', text: 'LOCUS' }] } }
        : body.type === 'agent_route' ? ROUTES[body.id] ?? { ok: true, native: false, running: false, stopped: false }
          : body.type === 'agent_step' ? state.step : { ok: true };
    return { ok: true, status: 200, text: JSON.stringify(answer) };
  });
  const registerAgent = vi.fn(async ({ name }: { name: string }) => ({ agent: 'locus:' + name }));
  const api = {
    env: { get: vi.fn(async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token') },
    http: { fetch },
    clock: { every: vi.fn(), now: vi.fn(async () => Date.now()) },
    session: { id: async () => 'session', model: async () => 'claude-opus-5-5' },
    agent: { register: registerAgent },
    tool: { call: vi.fn(async (_input: object) => ({ result: { message: 'Successfully stopped task', task_type: 'local_agent' } })) },
    turn: { abort: vi.fn(async () => ({})) },
  };
  await hooks.get('session.start')!.run(api, {}, vi.fn(async () => ({})));
  // The active Locus input starts the run that owns the delegation.
  await hooks.get('prompt.submit')!.run(api, { text: 'run-1' }, vi.fn(async () => ({})));
  const sent = (path: string) => fetch.mock.calls.filter(call => call[0].endsWith(path)).map(call => JSON.parse(call[1].body));
  const events = (type: string) => sent('/event').filter(event => event.type === type);
  const poll = (api.clock.every.mock.calls.find(call => call[0] === 150) as unknown as [number, () => Promise<void>])[1];
  return { hooks, api, state, registerAgent, sent, events, poll, run: (hook: string, e: object, next: Function) => hooks.get(hook)!.run(api, e, next) };
}

// A subagent's model step: its stream, then its result with usage.
async function subagentStep(h: Awaited<ReturnType<typeof delegation>>, e: object, inference = vi.fn(async function* () {
  yield { kind: 'text', text: 'working' };
  return { answer: 'working', toolUses: [], usage: { input_tokens: 10, output_tokens: 2, model: 'claude-sonnet-4-6' } };
})) {
  const step = h.run('turn.step', e, inference) as AsyncGenerator<unknown, Record<string, unknown>>;
  for (;;) {
    const item = await step.next();
    if (item.done) return { value: item.value, inference };
  }
}

describe('Claude native delegation Mod', () => {
  it('registers the published types and offers only configured builtins and registered types', async () => {
    const h = await delegation();
    expect(h.registerAgent.mock.calls.map(([definition]) => definition)).toEqual([
      { name: 'reviewer', description: 'Reviews code', prompt: 'You review.', tools: ['Read'], model: 'claude-sonnet-4-6' },
      { name: 'researcher', description: 'Researches', prompt: 'Locus runs this agent type.', tools: [], model: 'inherit' },
    ]);
    const next = vi.fn();
    const offered = async (agent: string, source: string) => (await h.run('agent.offer', { agent, source }, next)).isOffered;
    expect(await offered('Explore', 'built-in')).toBe(true);
    expect(await offered('general-purpose', 'built-in')).toBe(false);
    expect(await offered('fork', 'built-in')).toBe(false);
    expect(await offered('locus:reviewer', 'plugin')).toBe(true);
    expect(await offered('locus:researcher', 'plugin')).toBe(true);
    expect(await offered('locus:other', 'plugin')).toBe(false);
    expect(await offered('locus:reviewer', 'flagSettings')).toBe(false);
    expect(next).not.toHaveBeenCalled();
  });

  it('decides a main Agent from its nested input and never defers to the CLI prompt', async () => {
    const h = await delegation();
    const next = vi.fn(async () => ({ decision: 'ask' }));
    const check = (input: object, agentId?: string) => h.run('tool.check', { tool: 'Agent', input, tool_use_id: 't1', ...(agentId ? { agentId } : {}) }, next);
    expect(await check({ prompt: 'p', description: 'd', subagent_type: 'locus:reviewer', run_in_background: false })).toEqual({ decision: 'allow' });
    expect(await check({ prompt: 'p', description: 'd', subagent_type: 'Explore' })).toEqual({ decision: 'allow' });
    expect(await check({ prompt: 'p', description: 'd', subagent_type: 'Explore', run_in_background: true })).toEqual({ decision: 'allow' });
    for (const input of [
      { prompt: 'p', subagent_type: 'locus:researcher' },
      { prompt: 'p' },
      { prompt: 'p', subagent_type: 'general-purpose' },
      { prompt: 'p', subagent_type: 'locus:reviewer', model: 'opus' },
    ]) {
      expect(await check(input)).toMatchObject({ decision: 'deny' });
    }
    // A subagent's Agent call never reaches this branch as an allowed start.
    expect(await check({ prompt: 'p', subagent_type: 'locus:reviewer' }, 'a1')).toMatchObject({ decision: 'deny' });
    expect(next).not.toHaveBeenCalled();
  });

  it('runs a native type in the foreground or, when asked, the background and forwards a Locus type to the host', async () => {
    const h = await delegation();
    const result = { result: { status: 'completed', content: [
      { type: 'text', text: '<tools-used-before-summary>Read</tools-used-before-summary>' }, { type: 'text', text: 'REVIEWED' },
    ] } };
    const next = vi.fn(async () => result);
    const native = { tool: 'Agent', tool_use_id: 'toolu_native', prompt: 'p', description: 'd', subagent_type: 'locus:reviewer' };
    expect(await h.run('tool.call', native, next)).toBe(result);
    expect(next).toHaveBeenCalledWith({ ...native, run_in_background: false });
    expect(h.events('tool_result').find(event => event.id === 'toolu_native')).toMatchObject({ output: 'REVIEWED', isError: false });

    // In the background, Claude's own spawn of this call is admitted.
    const spawn = vi.fn(async () => ({ model: 'claude-sonnet-4-6', agentId: 'a7' }));
    const background = { tool: 'Agent', tool_use_id: 'toolu_bg', prompt: 'p', description: 'Scan', subagent_type: 'Explore', run_in_background: true };
    const launched = { result: { isAsync: true, status: 'async_launched', agentId: 'a7', description: 'Scan' } };
    const start = vi.fn(async () => {
      expect(await h.run('agent.spawn', { tool_use_id: 'toolu_bg', subagentType: 'Explore', prompt: 'p', description: 'Scan', background: true }, spawn))
        .toEqual({ model: 'claude-sonnet-4-6', agentId: 'a7' });
      return launched;
    });
    expect(await h.run('tool.call', background, start)).toBe(launched);
    expect(start).toHaveBeenCalledWith(background);
    expect(h.events('agent_spawn')).toEqual([{ protocol: 1, type: 'agent_spawn', runId: 'run-1', agentId: 'a7',
      parentToolUseId: 'toolu_bg', name: 'Explore', description: 'Scan', background: true, resume: false }]);
    expect(h.events('tool_result').find(event => event.id === 'toolu_bg')!.output).toContain('Running in the background as agent a7');
    expect(h.sent('/call')).toEqual([]);

    const locus = { tool: 'Agent', tool_use_id: 'toolu_locus', prompt: 'p', description: 'd', subagent_type: 'locus:researcher', run_in_background: true };
    const untouched = vi.fn();
    expect(await h.run('tool.call', locus, untouched)).toEqual({ result: { status: 'completed', content: [{ type: 'text', text: 'LOCUS' }] } });
    expect(untouched).not.toHaveBeenCalled();
    expect(h.sent('/call')).toEqual([expect.objectContaining({ tool: 'Agent', callId: 'toolu_locus', agentId: null,
      input: { prompt: 'p', description: 'd', subagent_type: 'locus:researcher', run_in_background: true } })]);
  });

  it('refuses unknown types and unsupported options without starting anything', async () => {
    const h = await delegation();
    const next = vi.fn();
    const deny = async (e: object) => (await h.run('tool.call', { tool_use_id: 't1', ...e }, next)).deny as string;
    expect(await deny({ tool: 'Agent', prompt: 'p', subagent_type: 'general-purpose' }))
      .toBe('Unknown subagent_type "general-purpose". Available subagent_type values: `locus:reviewer`, `locus:researcher`, `Explore`.');
    expect(await deny({ tool: 'Agent', prompt: 'p', subagent_type: 'Explore', isolation: 'worktree' })).toContain('isolation is not supported');
    expect(await deny({ tool: 'Agent', prompt: 'p', subagent_type: 'Explore', cwd: '/tmp' })).toContain('cwd is not supported');
    expect(next).not.toHaveBeenCalled();
    expect(h.sent('/call')).toEqual([]);
  });

  it('sends messages, output reads and stops for its own agents through Claude and the rest through Locus', async () => {
    const h = await delegation();
    const check = (tool: string, id: string, input: object) => h.run('tool.check', { tool, input, tool_use_id: id }, vi.fn());
    // Nothing reaches Claude's SendMessage without the routing below.
    expect(await check('SendMessage', 'toolu_x', { to: 'a1', message: 'hi' })).toMatchObject({ decision: 'deny' });
    const message = { tool: 'SendMessage', tool_use_id: 'toolu_msg', to: 'a1', message: 'Also check the tests', summary: 'tests' };
    const queued = { result: { success: true, message: 'Message queued for delivery to a1 at its next tool round.' } };
    const deliver = vi.fn(async () => {
      expect(await check('SendMessage', 'toolu_msg', { to: 'a1', message: 'Also check the tests' })).toEqual({ decision: 'allow' });
      return queued;
    });
    expect(await h.run('tool.call', message, deliver)).toBe(queued);
    expect(deliver).toHaveBeenCalledWith(message);
    expect(await check('SendMessage', 'toolu_msg', { to: 'a1' })).toMatchObject({ decision: 'deny' });
    expect(h.events('tool_result').find(event => event.id === 'toolu_msg')).toMatchObject({ isError: false });
    // A finished agent continues through Claude (its spawn is a resume).
    expect(await h.run('tool.call', { ...message, tool_use_id: 'toolu_resume', to: 'a2' }, vi.fn(async () => queued))).toBe(queued);

    const next = vi.fn();
    const deny = async (e: object) => (await h.run('tool.call', { tool: 'SendMessage', tool_use_id: 't1', message: 'hi', ...e }, next)).deny as string;
    expect(await deny({ to: 'main' })).toContain('You are the main agent');
    expect(await deny({ to: 'nobody' })).toContain('No agent "nobody" belongs to this conversation');
    expect(await deny({ to: 'a3' })).toContain('The user stopped agent a3');
    expect(next).not.toHaveBeenCalled();
    // A Locus agent's mailbox is the host's.
    expect(await h.run('tool.call', { tool: 'SendMessage', tool_use_id: 'toolu_locus_msg', to: 'locus:t3', message: 'hi' }, next))
      .toEqual({ result: { success: true, message: 'Message queued.' } });
    expect(h.sent('/call').map(call => [call.tool, call.callId, call.input.to])).toEqual([['SendMessage', 'toolu_locus_msg', 'locus:t3']]);

    // Claude stops its own agent; the agent's pending Locus call ends with it.
    await subagentStep(h, { agentId: 'a1', turnId: 'sub-turn', index: 1 });
    const stopped = { result: { message: 'Successfully stopped task: a1', task_id: 'a1', task_type: 'local_agent' } };
    expect(await h.run('tool.call', { tool: 'TaskStop', tool_use_id: 'toolu_stop', task_id: 'a1' }, vi.fn(async () => stopped))).toBe(stopped);
    expect(h.events('turn_interrupted')).toEqual([{ protocol: 1, type: 'turn_interrupted', runId: 'run-1', agentId: 'a1', id: 'agent:a1:1' }]);
    const output = { result: { retrieval_status: 'success' } };
    expect(await h.run('tool.call', { tool: 'TaskOutput', tool_use_id: 'toolu_out', task_id: 'a1' }, vi.fn(async () => output))).toBe(output);
    // Locus tasks keep their Locus tools.
    await h.run('tool.call', { tool: 'TaskStop', tool_use_id: 'toolu_task', task_id: 't3' }, next);
    expect(next).not.toHaveBeenCalled();
    expect(h.sent('/call').at(-1)).toMatchObject({ tool: 'TaskStop', callId: 'toolu_task', input: { task_id: 't3' } });
  });

  it('keeps subagents from delegating or using the parent task list and lets them message the main agent', async () => {
    const h = await delegation();
    const next = vi.fn();
    const deny = async (e: object) => (await h.run('tool.call', { tool_use_id: 'child_1', agentId: 'a1', ...e }, next)).deny as string;
    expect(await deny({ tool: 'Agent', prompt: 'p', subagent_type: 'Explore' })).toContain('cannot start other agents');
    expect(await deny({ tool: 'SendMessage', to: 'a2', message: 'hi' })).toContain('only to the main agent');
    expect(await deny({ tool: 'TodoWrite', todos: [] })).toContain('not available to Claude Code subagents');
    expect(await deny({ tool: 'TaskStop', task_id: 'a2' })).toContain('not available to Claude Code subagents');
    expect(await deny({ tool: 'Bash', command: 'sleep 1', run_in_background: true })).toContain('Background commands');
    expect(next).not.toHaveBeenCalled();
    // Allowed subagent calls are bridged with the agent's identity and the
    // cancellation scope of its current model step.
    await subagentStep(h, { agentId: 'a1', turnId: 'sub-turn', index: 1 });
    await h.run('tool.call', { tool: 'Read', tool_use_id: 'child_2', agentId: 'a1', file_path: 'a.txt' }, next);
    await h.run('tool.call', { tool: 'SendMessage', tool_use_id: 'child_3', agentId: 'a1', to: 'main', message: 'Halfway there' }, next);
    expect(next).not.toHaveBeenCalled();
    expect(h.sent('/call')).toEqual([
      expect.objectContaining({ tool: 'Read', callId: 'child_2', agentId: 'a1', input: { file_path: 'a.txt' }, executionId: 'agent:a1:1' }),
      expect.objectContaining({ tool: 'SendMessage', callId: 'child_3', agentId: 'a1', input: { to: 'main', message: 'Halfway there' } }),
    ]);
    // Its announcement belongs to the subagent, apart from the parent's text.
    expect(h.events('assistant').find(event => event.message.content[0].id === 'child_2'))
      .toMatchObject({ runId: 'run-1', agentId: 'a1', message: { content: [{ type: 'tool_use', id: 'child_2', name: 'Read' }] } });
  });

  it('starts only ordinary subagents of native types and continues only this conversation\'s unstopped agents', async () => {
    const h = await delegation();
    const next = vi.fn(async () => ({ model: 'claude-sonnet-4-6', agentId: 'a1' }));
    const spawn = { tool_use_id: 'toolu_native', subagentType: 'locus:reviewer', prompt: 'p', description: 'Review' };
    for (const e of [{ fork: true }, { isTeammate: true }, { workflow: 'w' }, { background: true }, { parentAgentId: 'a0' },
      { subagentType: 'locus:researcher' }, { subagentType: 'general-purpose' }, { resume: { agentId: 'a3' } }, { resume: { agentId: 'nobody' } }]) {
      expect((await h.run('agent.spawn', { ...spawn, ...e }, next)).deny).toBeTypeOf('string');
    }
    expect(next).not.toHaveBeenCalled();
    expect(await h.run('agent.spawn', spawn, next)).toEqual({ model: 'claude-sonnet-4-6', agentId: 'a1' });
    // SendMessage continues a finished agent as a background run.
    const resumed = vi.fn(async () => ({ model: 'claude-sonnet-4-6', agentId: 'a2' }));
    await h.run('agent.spawn', { ...spawn, tool_use_id: 'toolu_send', resume: { agentId: 'a2' }, background: true }, resumed);
    expect(resumed).toHaveBeenCalledOnce();
    expect(h.events('agent_spawn')).toEqual([
      { protocol: 1, type: 'agent_spawn', runId: 'run-1', agentId: 'a1', parentToolUseId: 'toolu_native', name: 'locus:reviewer',
        description: 'Review', background: false, resume: false },
      { protocol: 1, type: 'agent_spawn', runId: 'run-1', agentId: 'a2', parentToolUseId: 'toolu_send', name: 'locus:reviewer',
        description: 'Review', background: true, resume: true },
    ]);
  });

  it('hands task notifications to Locus and drops peer messages without starting a turn', async () => {
    const h = await delegation();
    const submit = vi.fn(async () => ({}));
    const notice = '<task-notification>\n<task-id>a1</task-id>\n<tool-use-id>toolu_bg</tool-use-id>\n<status>completed</status>\n<summary>Agent "Scan" finished</summary>\n<result>DONE</result>\n</task-notification>';
    h.state.options = { ...h.state.options, active: false };
    expect((await h.run('prompt.submit', { text: notice, wait: false, origin: { kind: 'task-notification' } }, submit)).drop).toBeTypeOf('string');
    expect(h.events('agent_notification')).toEqual([{ protocol: 1, type: 'agent_notification', text: notice }]);
    expect((await h.run('prompt.submit', { text: '<agent-message>hi</agent-message>', origin: { kind: 'peer' } }, submit)).drop).toBeTypeOf('string');
    expect(submit).not.toHaveBeenCalled();
  });

  it('gates subagent steps by their registration, also while idle, and records their usage', async () => {
    const h = await delegation();
    h.state.options = { ...h.state.options, active: false };
    const { value, inference } = await subagentStep(h, { agentId: 'a1', turnId: 'sub-turn', index: 2, model: 'claude-sonnet-4-6' });
    expect(inference).toHaveBeenCalledOnce();
    expect(value).toMatchObject({ answer: 'working' });
    expect(h.events('agent_step')).toEqual([{ protocol: 1, type: 'agent_step', agentId: 'a1' }]);
    expect(h.events('usage')).toEqual([{ protocol: 1, type: 'usage', agentId: 'a1',
      usage: { input_tokens: 10, output_tokens: 2, model: 'claude-sonnet-4-6', stepKey: 'sub-turn:2' } }]);
    // A stopped (or unregistered) agent sends nothing more.
    h.state.step = { deny: 'The user stopped this agent.' };
    const refused = await subagentStep(h, { agentId: 'a1', turnId: 'sub-turn', index: 3 });
    expect(refused.inference).not.toHaveBeenCalled();
    expect(refused.value).toMatchObject({ turnId: 'sub-turn', answer: '', toolUses: [] });
    expect(h.api.turn.abort).toHaveBeenCalledWith({ turnId: 'sub-turn' });
    // Its text reaches its card or task while the conversation is idle.
    const append = vi.fn(async () => ({}));
    await h.run('session.append', { door: 'transcript', uuid: 'u1', agentId: 'a1', origin: { kind: 'model' },
      message: { type: 'assistant', role: 'assistant', content: [{ type: 'text', text: 'Found 3 files' }] } }, append);
    expect(h.events('assistant').at(-1)).toMatchObject({ agentId: 'a1', message: { content: [{ type: 'text', text: 'Found 3 files' }] } });
    const complete = vi.fn(async () => ({ text: '' }));
    await h.run('turn.complete', { agentId: 'a1', turnId: 'sub-turn', isAborted: false }, complete);
    expect(complete).toHaveBeenCalledOnce();
    expect(h.events('agent_finished')).toEqual([{ protocol: 1, type: 'agent_finished', agentId: 'a1' }]);
    expect(h.events('complete')).toEqual([]);
  });

  it('runs a user stop from the input poll, also while idle, and acknowledges a repeat without stopping again', async () => {
    const h = await delegation();
    const command = { id: 'stop:a1:1', type: 'stop', agentId: 'a1' };
    h.state.options = { active: false, inputId: null, agentControl: { commands: [command], stopped: ['a1'] } };
    const checks: unknown[] = [];
    h.api.tool.call.mockImplementation(async (input: object) => {
      checks.push(await h.run('tool.check', { tool: 'TaskStop', input: { task_id: (input as { task_id: string }).task_id } }, vi.fn()));
      return { result: { message: 'Successfully stopped task: a1', task_type: 'local_agent' } };
    });
    await h.poll();
    await vi.waitFor(() => expect(h.events('control_done')).toHaveLength(1));
    expect(h.api.tool.call.mock.calls).toEqual([[{ tool: 'TaskStop', task_id: 'a1' }]]);
    expect(checks).toEqual([{ decision: 'allow' }]);
    expect(h.events('control_done')).toEqual([{ protocol: 1, type: 'control_done', id: 'stop:a1:1', agentId: 'a1', ok: true }]);
    // Only while it runs: Claude's TaskStop is otherwise the model's, routed by tool.call.
    expect(await h.run('tool.check', { tool: 'TaskStop', input: { task_id: 'a1' } }, vi.fn())).toMatchObject({ decision: 'deny' });
    // This Mod's own TaskStop re-enters tool.call and passes through.
    const reentry = Object.assign(vi.fn(async () => ({ result: 'stopped' })), { error: { kind: 're-entry' }, called: false });
    expect(await h.hooks.get('tool.call')!.fail!(h.api, { tool: 'TaskStop', task_id: 'a1' }, reentry)).toEqual({ result: 'stopped' });
    // A lost acknowledgement repeats the command; Claude is not asked twice.
    await h.poll();
    await vi.waitFor(() => expect(h.events('control_done')).toHaveLength(2));
    expect(h.api.tool.call).toHaveBeenCalledOnce();
  });

  it('gives subagents the Locus environment instead of the CLI host environment', async () => {
    const h = await delegation();
    const next = vi.fn();
    expect(await h.run('prompt.attachment', { type: 'environment', agentId: 'a1' }, next)).toEqual({ text: ENVIRONMENT });
    expect(await h.run('prompt.attachment', { type: 'environment' }, next)).toEqual({ text: null });
    expect(next).not.toHaveBeenCalled();
  });
});
