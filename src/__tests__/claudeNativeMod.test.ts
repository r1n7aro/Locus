import { describe, expect, it, vi } from 'vitest';
// This is the same self-contained module embedded in the Rust CLI launcher.
// @ts-expect-error Claude loads this JavaScript module directly.
import { register } from '../../crates/locus-remote-core/src/resources/native_mod.js';

function harness(answer: unknown = { callId: 't1', result: { stdout: 'local', stderr: '', interrupted: false } }) {
  const hooks = new Map<string, { run: Function; fail?: Function; matcher?: object }>();
  const describeHooks: { run: Function; matcher?: object }[] = [];
  register((name: string, matchOrRun: Function | object, handler?: Function) => {
    const run = handler || matchOrRun as Function;
    const entry = { run, fail: undefined as Function | undefined, matcher: handler ? matchOrRun as object : undefined };
    hooks.set(name, entry);
    if (name === 'tool.describe') describeHooks.push(entry);
    return { catch(handler: Function) { entry.fail = handler; } };
  });
  const fetch = vi.fn(async (_url: string, _init: { body: string }) => ({ ok: true, status: 200, text: JSON.stringify(answer) }));
  const api = { env: { get: vi.fn(async (key: string) => key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token') }, http: { fetch }, clock: { every: vi.fn(), now: vi.fn(async () => Date.now()) } };
  const next = vi.fn(async () => ({ result: 'original tool ran' }));
  return { hooks, describeHooks, api, fetch, next };
}

async function inputHarness(options: Record<string, any>) {
  const h = harness(options);
  const draft = { text: '' };
  const fill = vi.fn(async ({ text, mode }: { text: string; mode: string }) => {
    draft.text = mode === 'append' ? draft.text + text : text;
    return { isFilled: true };
  });
  const read = vi.fn(async () => ({ ...draft, cursor: draft.text.length }));
  const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
    session: { id: async () => 'session', model: async () => 'claude-opus-5-5' }, prompt: { fill, read } };
  await h.hooks.get('session.start')!.run(api, {}, h.next);
  const poll = (h.api.clock.every.mock.calls[0] as unknown as [number, () => Promise<void>])[1];
  const events = () => h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).filter(event => event.type);
  return { ...h, api, draft, fill, read, poll, events };
}

describe('Claude native tools Mod', () => {
  it('bridges main TUI spinner transitions without thinking text, animation duplicates, or idle leakage', async () => {
    const h = await inputHarness({ active: true, inputId: 'main', prompt: 'Hello' });
    const api = { ...h.api, session: { ...h.api.session, usage: async () => ({ context: {} }) } };
    const render = h.hooks.get('ui.render')!;
    const next = vi.fn(async (event) => ({ unchanged: event }));
    const spinner = (mode: string, requestId = 'main', surface = 'terminal') =>
      render.run(api, { component: 'Spinner', requestId, surface, props: { mode, word: 'Random verb', message: 'Not a summary' } }, next);
    const phases = () => h.events().filter(event => event.type === 'phase');
    await spinner('thinking');
    expect(phases()).toEqual([]);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'main' }, h.next);
    const step = () => h.hooks.get('turn.step')!.run(api, {}, () => Object.assign((async function* () {
      yield { kind: 'engine', ref: 1 }; // block start, no thinking text
      yield { kind: 'text', text: 'Answer' };
    })(), { result: Promise.resolve({ answer: 'Answer', toolUses: [], usage: {} }) }));
    const first = step();
    await first.next();
    await spinner('requesting');
    await spinner('thinking', 'subagent');
    await spinner('thinking', 'main', 'desktop');
    await spinner('unknown');
    expect(phases()).toEqual([]);
    const result = await spinner('thinking');
    await spinner('thinking');
    expect(result.unchanged.props.mode).toBe('thinking');
    expect(phases()).toEqual([{ protocol: 1, type: 'phase', runId: 'main', stepId: 1, sequence: 1, phase: 'thinking' }]);
    expect(h.events().some(event => event.type === 'thinking')).toBe(false);
    await spinner('tool-input');
    await spinner('tool-use');
    await first.next();
    await first.next();
    expect(phases().map(event => event.phase)).toEqual(['thinking', 'tool-input', 'tool-use', 'responding']);
    expect(phases().map(event => event.sequence)).toEqual([1, 2, 3, 4]);
    const second = step();
    await second.next();
    await spinner('thinking');
    expect(phases()[phases().length - 1]).toMatchObject({ stepId: 2, sequence: 1, phase: 'thinking' });
    await second.next();
    await second.next();
    await h.hooks.get('turn.complete')!.run(api, {}, h.next);
    const count = phases().length;
    await spinner('thinking');
    expect(phases()).toHaveLength(count);
  });

  it('uses progress-only thinking as a phase without inventing summary text', async () => {
    const h = await inputHarness({ active: true, inputId: 'hidden', prompt: 'Hello' });
    const api = { ...h.api, session: { ...h.api.session, usage: async () => ({ context: {} }) } };
    await h.hooks.get('prompt.submit')!.run(api, { text: 'hidden' }, h.next);
    const chunks = [{ kind: 'thinking', index: 0 }, { kind: 'text', text: 'Answer' }];
    const stream = Object.assign((async function* () { yield* chunks; })(), { result: Promise.resolve({ answer: 'Answer', toolUses: [], usage: {} }) });
    const received = [];
    for await (const chunk of h.hooks.get('turn.step')!.run(api, {}, () => stream)) received.push(chunk);
    expect(received).toEqual(chunks);
    expect(h.events().filter(event => event.type === 'phase').map(event => event.phase)).toEqual(['thinking', 'responding']);
    expect(h.events().some(event => event.type === 'thinking')).toBe(false);
    expect(h.events().find(event => event.type === 'assistant').message.content).toEqual([{ type: 'text', text: 'Answer' }]);
  });

  it('does not let a failed status receipt break native rendering or output', async () => {
    const h = await inputHarness({ active: true, inputId: 'main', prompt: 'Hello' });
    const api = { ...h.api, session: { ...h.api.session, usage: async () => ({ context: {} }) } };
    await h.hooks.get('prompt.submit')!.run(api, { text: 'main' }, h.next);
    const fetch = h.fetch.getMockImplementation()!;
    h.fetch.mockImplementation(async (url, init) => {
      if (JSON.parse(init.body).type === 'phase') throw new Error('status unavailable');
      return fetch(url, init);
    });
    const stream = Object.assign((async function* () { yield { kind: 'engine', ref: 1 }; yield { kind: 'text', text: 'Answer' }; })(),
      { result: Promise.resolve({ answer: 'Answer', toolUses: [], usage: {} }) });
    const iterator = h.hooks.get('turn.step')!.run(api, {}, () => stream);
    await iterator.next();
    const next = vi.fn(async () => ({ native: true }));
    await expect(h.hooks.get('ui.render')!.run(api, { surface: 'terminal', requestId: 'main', props: { mode: 'thinking' } }, next)).resolves.toEqual({ native: true });
    expect(next).toHaveBeenCalledTimes(1);
    expect((await iterator.next()).value).toEqual({ kind: 'text', text: 'Answer' });
    await iterator.next();
    expect(h.events().find(event => event.type === 'text')).toMatchObject({ text: 'Answer' });
  });
  it.each(['user', 'attachment'])('persists a checkpoint for consumed mid-turn %s meta rows', async (type) => {
    const options = { active: true, inputId: 'main', insertion: { inputId: 'insert', submissionId: 'draft', prompt: 'New direction' } };
    const h = await inputHarness(options);
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'insert' }, h.next);
    const event = { uuid: 'native-insertion', door: 'note', message: { type, role: 'user', isMeta: true, content: [{ type: 'text', text: 'New direction' }] } };
    await h.hooks.get('session.append')!.run(h.api, event, h.next);
    expect(h.events().find(e => e.type === 'input_consumed')).toMatchObject({ id: 'draft', checkpoint: { version: 1, uuid: 'native-insertion', sessionId: 'session' } });
    expect(h.events().find(e => e.type === 'checkpoint')).toMatchObject({ uuid: 'native-insertion', submissionId: 'draft', user: true });
    await h.hooks.get('session.append')!.run(h.api, { ...event, uuid: 'later-reminder' }, h.next);
    expect(h.events().filter(e => e.type === 'input_consumed')).toHaveLength(1);
  });

  it.each([false, true])('limits interruption to old tools when Claude absorbs input inside the same native turn (during submit: %s)', async (duringSubmit) => {
    const options: Record<string, any> = { active: true, inputId: 'main', prompt: 'Original' };
    const h = await inputHarness(options);
    h.fetch.mockImplementation(async (url, init) => {
      const body = JSON.parse(init.body);
      return { ok: true, status: 200, text: JSON.stringify(url.endsWith('/options') ? options : { ok: true, callId: body.callId, result: 'done' }) };
    });
    const api = { ...h.api, session: { ...h.api.session, usage: async () => ({ context: {} }) } };
    await h.hooks.get('prompt.submit')!.run(api, { text: 'main' }, h.next);
    await h.hooks.get('turn.start')!.run(api, { turnId: 'unchanged-native-turn' }, h.next);
    await h.hooks.get('tool.call')!.run(api, { tool: 'mcp__locus__probe', tool_use_id: 'old' }, h.next);
    options.insertion = { inputId: 'insert', submissionId: 'draft', prompt: 'Continue thoroughly' };
    const absorb = async () => {
      await h.hooks.get('session.append')!.run(api, { uuid: 'receipt', door: 'note', message: { type: 'attachment', role: 'user', isMeta: true, content: [{ type: 'text', text: 'Continue thoroughly' }] } }, h.next);
      const stream = Object.assign((async function* () {})(), { result: Promise.resolve({ answer: '', toolUses: [], usage: {} }) });
      for await (const _ of h.hooks.get('turn.step')!.run(api, { turnId: 'unchanged-native-turn' }, () => stream)) { /* drain */ }
    };
    await h.hooks.get('prompt.submit')!.run(api, { text: 'insert' }, duringSubmit ? async () => { await absorb(); return {}; } : h.next);
    if (!duringSubmit) await absorb();
    await h.hooks.get('tool.call')!.run(api, { tool: 'mcp__locus__probe', tool_use_id: 'new' }, h.next);
    const calls = h.fetch.mock.calls.map(call => JSON.parse(call[1].body));
    const old = calls.find(e => e.callId === 'old');
    const fresh = calls.find(e => e.callId === 'new');
    expect(fresh.turnId).toBe(old.turnId);
    expect(fresh.executionId).not.toBe(old.executionId);
    expect(calls.filter(e => e.type === 'turn_interrupted').map(e => e.id)).toEqual([old.executionId]);
  });
  it('waits for each native image paste before appending text and authorizing Enter', async () => {
    const h = await inputHarness({ active: true, inputId: 'images', imageCount: 2, terminalText: 'images @"/cache/file.txt"' });
    await h.poll();
    expect(h.fill).toHaveBeenCalledExactlyOnceWith({ text: '', mode: 'replace' });
    await h.poll();
    expect(h.events()).toEqual([{ protocol: 1, type: 'input_images', runId: 'images', id: 'images', count: 0 }]);
    h.draft.text = '"/cache/image-1.png"';
    await h.poll();
    expect(h.events()).toHaveLength(1);
    h.draft.text = '[Image #1]';
    await h.poll();
    await h.poll();
    expect(h.events().map(e => e.count)).toEqual([0, 1]);
    expect(h.fill).toHaveBeenCalledTimes(1);
    h.draft.text = '[Image #1] [Image #2]';
    await h.poll();
    expect(h.fill).toHaveBeenLastCalledWith({ text: ' images @"/cache/file.txt" ', mode: 'append' });
    expect(h.draft.text).toBe('[Image #1] [Image #2] images @"/cache/file.txt" ');
    expect(h.events().map(e => [e.type, e.count])).toEqual([
      ['input_images', 0], ['input_images', 1], ['input_images', 2], ['input_ready', undefined],
    ]);
    await h.poll();
    expect(h.fill).toHaveBeenCalledTimes(2);
  });

  it('keeps image readiness scoped to inserted input and does not restage the main prompt', async () => {
    const options: Record<string, any> = { active: true, inputId: 'main', terminalText: 'main' };
    const h = await inputHarness(options);
    await h.poll();
    h.draft.text = '';
    options.insertion = { inputId: 'insert', imageCount: 1 };
    await h.poll();
    await h.poll();
    h.draft.text = '[Image #3]';
    await h.poll();
    expect(h.events().slice(-3)).toEqual([
      { protocol: 1, type: 'input_images', runId: 'main', id: 'insert', count: 0 },
      { protocol: 1, type: 'input_images', runId: 'main', id: 'insert', count: 1 },
      { protocol: 1, type: 'input_ready', runId: 'main', id: 'insert' },
    ]);
    h.draft.text = '';
    delete options.insertion;
    await h.poll();
    expect(h.draft.text).toBe('');
    expect(h.fill).toHaveBeenCalledTimes(3);
    options.inputId = options.terminalText = 'next';
    options.imageCount = 1;
    await h.poll();
    await h.poll();
    h.draft.text = '[Image #4]';
    await h.poll();
    expect(h.events().slice(-1)[0]).toEqual({ protocol: 1, type: 'input_ready', runId: 'next', id: 'next' });
  });

  it('retries a blocked text fill without clearing or pasting the images again', async () => {
    const h = await inputHarness({ active: true, inputId: 'input', imageCount: 1 });
    await h.poll();
    await h.poll();
    h.draft.text = '[Image #1]';
    h.fill.mockResolvedValueOnce({ isFilled: false });
    await h.poll();
    expect(h.events().some(e => e.type === 'input_ready')).toBe(false);
    await h.poll();
    expect(h.draft.text).toBe('[Image #1] input ');
    expect(h.events().filter(e => e.type === 'input_images').map(e => e.count)).toEqual([0, 1]);
    expect(h.events().filter(e => e.type === 'input_ready')).toHaveLength(1);
  });

  it.each(['extra', 'removed', 'read-error'])('fails before Enter when staged images change (%s)', async (failure) => {
    const h = await inputHarness({ active: true, inputId: 'input', imageCount: 2 });
    await h.poll();
    await h.poll();
    h.draft.text = '[Image #1]';
    await h.poll();
    if (failure === 'read-error') h.read.mockRejectedValueOnce(new Error('native image input unavailable'));
    else h.draft.text = failure === 'extra' ? '[Image #1] [Image #2] [Image #3]' : '';
    await h.poll();
    expect(h.events().slice(-1)[0]).toMatchObject({ type: 'input_staging_error', runId: 'input', id: 'input', error: expect.any(String) });
    expect(h.events().some(e => e.type === 'input_ready')).toBe(false);
    await h.poll();
    expect(h.events().filter(e => e.type === 'input_staging_error')).toHaveLength(1);
  });

  it('ends idle main and auxiliary model steps instead of throwing into the CLI fallback', async () => {
    const h = harness({ active: false });
    const abort = vi.fn(async () => ({}));
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { id: async () => 'idle-native', model: async () => 'claude-opus-5-5' }, turn: { abort } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    const inference = vi.fn();
    for (const event of [{ turnId: 'main-turn', index: 0 }, { agentId: 'background', turnId: 'agent-turn', index: 0 }]) {
      const step = h.hooks.get('turn.step')!.run(api, event, inference);
      await expect(step.next()).resolves.toEqual({ done: true, value: expect.objectContaining({ turnId: event.turnId, answer: '', toolUses: [] }) });
    }
    expect(inference).not.toHaveBeenCalled();
    expect(abort.mock.calls).toEqual([[{ turnId: 'main-turn' }], [{ turnId: 'agent-turn' }]]);
  });

  it('ends the step when the input check cannot reach the bridge or the turn is already ending', async () => {
    const h = harness({ active: true });
    const abort = vi.fn(async () => { throw new Error('$.turn.abort: no turn is running'); });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { id: async () => 'native', model: async () => 'claude-opus-5-5' }, turn: { abort } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    h.fetch.mockResolvedValue({ ok: false, status: 502, text: '{}' });
    const inference = vi.fn();
    const step = h.hooks.get('turn.step')!.run(api, { turnId: 'main-turn', index: 1 }, inference);
    await expect(step.next()).resolves.toMatchObject({ done: true, value: { turnId: 'main-turn', answer: '' } });
    expect(inference).not.toHaveBeenCalled();
    expect(abort).toHaveBeenCalledWith({ turnId: 'main-turn' });
  });
  it('does not refill input while an earlier input-ready receipt is pending', async () => {
    const h = harness({ active: true, inputId: 'input', terminalText: 'hello' });
    const fill = vi.fn(async () => ({ isFilled: true }));
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { id: async () => 'session', model: async () => 'claude-opus-5-5' }, prompt: { fill } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    let release!: (answer: any) => void;
    h.fetch.mockImplementation(async (_url, request) => JSON.parse(request.body).type === 'input_ready'
      ? await new Promise(resolve => { release = resolve; })
      : { ok: true, status: 200, text: JSON.stringify({ active: true, inputId: 'input', terminalText: 'hello' }) });
    const poll = (h.api.clock.every.mock.calls[0] as unknown as [number, () => Promise<void>])[1];
    const first = poll();
    while (!release) await Promise.resolve();
    await poll();
    expect(fill).toHaveBeenCalledTimes(1);
    release({ ok: true, status: 200, text: '{"ok":true}' });
    await first;
    await poll();
    expect(fill).toHaveBeenCalledTimes(1);
  });

  it('keeps auxiliary native rows without a checkpoint round trip while retaining user and tool-result barriers', async () => {
    const h = harness({ active: true, inputId: 'input', prompt: 'hello' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { id: async () => 'session', model: async () => 'claude-opus-5-5' } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next);
    const append = vi.fn(async (event) => event);
    const row = { uuid: 'aux', door: 'attachment', message: { type: 'user', role: 'user', content: [{ type: 'text', text: 'Native attachment' }] } };
    await h.hooks.get('session.append')!.run(api, row, append);
    expect(append).toHaveBeenCalledExactlyOnceWith(row);
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).filter(event => event.type === 'checkpoint')).toHaveLength(0);
    await h.hooks.get('session.append')!.run(api, { ...row, uuid: 'user', door: 'prompt' }, append);
    await h.hooks.get('session.append')!.run(api, { ...row, uuid: 'result', message: { ...row.message, content: [{ type: 'tool_result', tool_use_id: 'tool-1' }] } }, append);
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).filter(event => event.type === 'checkpoint')).toEqual([
      expect.objectContaining({ uuid: 'user', user: true }), expect.objectContaining({ uuid: 'result', results: ['tool-1'] }),
    ]);
  });

  it.each([
    { label: 'all eager', deferred: [] as string[] },
    { label: 'mixed', deferred: ['TaskList', 'mcp__locus__skill_action', 'mcp__external__unknown', 'mcp__external__hyphen-name'] },
  ])('corrects only new deferred notices using final placement ($label)', async ({ deferred }) => {
    const header = 'The following deferred tools are now available via ToolSearch. Their schemas are NOT loaded — use ToolSearch first:';
    const original = '<system-reminder>\n' + header + '\n' + ['mcp__locus__edit', ...deferred].join('\n') + '\n</system-reminder>';
    const h = harness({ active: false, tools: [
      { name: 'edit', description: 'Pinned legacy edit', input_schema: {}, defer_loading: false },
      { name: 'skill_action', description: 'Deferred skill', input_schema: {}, defer_loading: true },
    ] });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { id: async () => 'session', model: async () => 'test' }, tool: { register: vi.fn(async () => ({})) } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    const append = vi.fn(async event => event);
    const row = { uuid: 'new-notice', door: 'attachment', message: { type: 'attachment', role: 'user', name: 'deferred_tools_delta', content: [{ type: 'text', text: original }] } };
    await h.hooks.get('session.append')!.run(api, row, append);
    const text = append.mock.calls[0]![0].message.content[0].text;
    expect(text).not.toContain('\nmcp__locus__edit\n');
    expect(text).toContain('Already available directly (no ToolSearch needed): mcp__locus__edit.');
    if (deferred.length) expect(text).toContain(header + '\n' + deferred.join('\n'));
    else expect(text).not.toContain('Their schemas are NOT loaded');
    // Historical rendering and user text must not be rewritten on resume.
    const render = async (event: { text: string }) => ({ text: event.text });
    expect(await h.hooks.get('prompt.attachment')!.run(api, { type: 'deferred_tools_delta', text: original }, render)).toEqual({ text: original });
    append.mockClear();
    const user = { ...row, door: 'prompt', message: { ...row.message, type: 'user' } };
    await h.hooks.get('session.append')!.run(api, user, append);
    expect(append.mock.calls[0]![0].message.content[0].text).toBe(original);
    append.mockClear();
    const futureLayout = original.replace('\nmcp__locus__edit\n', '\nmcp__locus__edit\nunknown future listing format\n');
    await h.hooks.get('session.append')!.run(api, { ...row, message: { ...row.message, content: [{ type: 'text', text: futureLayout }] } }, append);
    expect(append.mock.calls[0]![0].message.content[0].text).toBe(futureLayout);
  });

  it('registers unchanged definitions once and refuses changes to the cached schema', async () => {
    const options = { active: true, inputId: 'input', prompt: 'hello', tools: [
      { name: 'python', description: 'Run locally', input_schema: { type: 'object', properties: {} } },
      { name: 'read', description: 'Native Read', input_schema: { type: 'object' } },
    ] };
    const h = harness(options);
    const registerTool = vi.fn(async () => ({}));
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { id: async () => 'session', model: async () => 'claude-opus-5-5' }, tool: { register: registerTool } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    expect(registerTool).toHaveBeenCalledTimes(1); // rehydrate before native history resumes
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next);
    expect(registerTool).toHaveBeenCalledTimes(1);
    options.tools[0]!.description = 'Updated local tool';
    await expect(h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next)).rejects.toThrow('definition changed');
    expect(registerTool).toHaveBeenCalledTimes(1);
  });

  it('keeps ToolSearch enabled from the first request with no Skill catalog', async () => {
    const h = harness();
    const bootstrap = h.describeHooks.find(entry => (entry.matcher as { tool?: string })?.tool === 'TaskList')!;
    const api = { ...h.api, env: { get: async () => '1' } };
    await expect(bootstrap.run(api, { tool: 'TaskList' }, async () => ({ description: 'List tasks' })))
      .resolves.toEqual({ description: 'List tasks', isDeferred: true });
    await expect(bootstrap.run({ ...api, env: { get: async () => '0' } }, { tool: 'TaskList' }, async () => ({ description: 'List tasks' })))
      .resolves.toEqual({ description: 'List tasks' });
  });

  it('registers a late Skill delta before returning the result, without invalidating descriptions or leaking schemas', async () => {
    const definition = { name: 'skill_action', description: 'Skill tool', input_schema: { type: 'object' }, defer_loading: true };
    const h = harness({ callId: 't1', result: 'Skill loaded', toolCatalogDelta: [definition] });
    const registerTool = vi.fn(async () => ({}));
    const invalidate = vi.fn();
    const api = { ...h.api, tool: { register: registerTool }, ui: { invalidate } };
    const event = { tool: 'Read', tool_use_id: 't1', file_path: 'SKILL.md' };
    expect(await h.hooks.get('tool.call')!.run(api, event, h.next)).toEqual({ result: 'Skill loaded' });
    expect(registerTool).toHaveBeenCalledExactlyOnceWith({ name: 'skill_action', description: 'Skill tool', inputSchema: { type: 'object' } });
    expect(invalidate).not.toHaveBeenCalled();
    const describe = h.hooks.get('tool.describe')!;
    const nativeDescribe = async () => ({ description: 'Skill tool' });
    expect(await describe.run(api, { tool: 'mcp__locus__skill_action' }, nativeDescribe)).toEqual({ description: 'Skill tool', isDeferred: true });
    await h.hooks.get('tool.call')!.run(api, event, h.next);
    expect(registerTool).toHaveBeenCalledTimes(1);
    expect(await describe.run(api, { tool: 'mcp__locus__skill_action' }, nativeDescribe)).toHaveProperty('isDeferred', true);
  });

  it('samples response stop before the native consumer finishes its tools', async () => {
    const h=harness({active:true,inputId:'timing',prompt:'Sample'});
    let now=100;
    const api={...h.api,clock:{...h.api.clock,now:async()=>now},env:{get:async(key:string)=>key==='LOCUS_CLAUDE_TUI'?'1':key.endsWith('_URL')?'http://127.0.0.1:19200':'test-token'},session:{model:async()=>'claude-opus-5-5',id:async()=>'session',usage:async()=>({context:{}})}};
    await h.hooks.get('session.start')!.run(api,{},h.next);
    await h.hooks.get('prompt.submit')!.run(api,{text:'timing'},h.next);
    const usage={model:'test',output_tokens:50};
    const stream=Object.assign((async function*(){now=300;yield {kind:'text',text:'sample'};now=1100;yield {kind:'stop',usage};now=21100;})(),{result:Promise.resolve({answer:'sample',toolUses:[],usage})});
    for await(const _ of h.hooks.get('turn.step')!.run(api,{},()=>stream)){/* consume */}
    expect(h.fetch.mock.calls.map(call=>JSON.parse(call[1].body)).find(event=>event.type==='usage').timing)
      .toEqual({version:1,source:'claude-turn-step',requestStartedAtMs:100,firstOutputAtMs:300,responseStoppedAtMs:1100,resultReadyAtMs:21100,bridgeDurationMs:0,consumerPauseMs:0,outputChunks:1});
  });
  it.each([
    ['WebSearch', false], ['WebSearch', true], ['WebFetch', false], ['WebFetch', true],
  ] as const)('keeps native %s in the CLI and records its outcome (error=%s)', async (tool, fails) => {
    const h=harness({active:true,inputId:'search',prompt:'Search'});
    const api={...h.api,env:{get:async(key:string)=>key==='LOCUS_CLAUDE_TUI'?'1':key.endsWith('_URL')?'http://127.0.0.1:19200':'test-token'},session:{model:async()=>'claude-opus-5-5',id:async()=>'session'}};
    await h.hooks.get('session.start')!.run(api,{},h.next);
    await h.hooks.get('prompt.submit')!.run(api,{text:'search'},h.next);
    const input = tool === 'WebFetch' ? { url: 'https://example.com/', prompt: 'Extract the title' } : { query: 'probe' };
    const output = tool === 'WebFetch' ? { result: { url: input.url, result: 'Example Domain\nExtracted by Claude' } }
      : { result: { query: 'probe', results: [{ content: [{ title: 'Example', url: 'https://example.com/' }] }] } };
    const native=vi.fn(async()=>{if(fails)throw Error('Web endpoint unavailable');return output;});
    const call=h.hooks.get('tool.call')!.run(api,{tool,tool_use_id:'web',...input},native);
    if(fails)await expect(call).rejects.toThrow('Web endpoint unavailable');else await expect(call).resolves.toEqual(output);
    expect(native).toHaveBeenCalledTimes(1);
    const events=h.fetch.mock.calls.map(call=>JSON.parse(call[1].body));
    expect(events.find(event=>event.type==='tool_result')).toMatchObject({id:'web',isError:fails});
    expect(h.fetch.mock.calls.every(call=>!call[0].endsWith('/call'))).toBe(true);
    const check=vi.fn(async()=>({decision:'allow'}));
    await expect(h.hooks.get('tool.check')!.run(api,{tool},check)).resolves.toEqual({decision:'allow'});
    expect(check).toHaveBeenCalledTimes(1);
  });
  it('rejects a remembered Locus web_fetch call without dispatch or fallback', async () => {
    const h = await inputHarness({ active: true, inputId: 'fetch', prompt: 'Fetch' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'fetch' }, h.next);
    const native = vi.fn(async () => ({ result: 'must not run' }));
    const result = await h.hooks.get('tool.call')!.run(h.api,
      { tool: 'mcp__locus__web_fetch', tool_use_id: 'old-fetch', url: 'https://example.com/' }, native);
    expect(result.deny).toContain('Use Claude Code WebFetch');
    expect(native).not.toHaveBeenCalled();
    expect(h.fetch.mock.calls.some(([url]) => url.endsWith('/call'))).toBe(false);
    expect(h.events().find(e => e.type === 'tool_result')).toMatchObject({ id: 'old-fetch', isError: true });
    expect(await h.hooks.get('tool.check')!.run(h.api, { tool: 'mcp__locus__web_fetch' }, native)).toHaveProperty('decision', 'deny');
  });
  it('runs four native searches concurrently even when the first takes longer than the 10 s hook budget', async () => {
    vi.useFakeTimers();
    try {
      const h = await inputHarness({ active: true, inputId: 'searches', prompt: 'Search' });
      await h.hooks.get('prompt.submit')!.run(h.api, { text: 'searches' }, h.next);
      const native = vi.fn(async (event: { tool_use_id: string }) => {
        await new Promise(resolve => setTimeout(resolve, event.tool_use_id === 'a' ? 20000 : 1000));
        return { result: event.tool_use_id };
      });
      const calls = ['a', 'b', 'c', 'd'].map(id => h.hooks.get('tool.call')!.run(h.api,
        { tool: 'WebSearch', tool_use_id: id, query: id }, native));
      await vi.advanceTimersByTimeAsync(11000);
      expect(native).toHaveBeenCalledTimes(4);
      expect(h.events().filter(e => e.type === 'tool_result').map(e => e.id)).toEqual(['b', 'c', 'd']);
      expect(h.events().filter(e => e.type === 'assistant')).toHaveLength(4);
      await vi.advanceTimersByTimeAsync(10000);
      await Promise.all(calls);
      expect(h.events().filter(e => e.type === 'tool_result').map(e => e.id)).toEqual(['b', 'c', 'd', 'a']);
      expect(h.fetch.mock.calls.some(([url]) => url.endsWith('/call'))).toBe(false);
    } finally { vi.useRealTimers(); }
  });

  it('cancels a dispatch before execution without stranding later calls or retrying a write', async () => {
    const h = await inputHarness({ active: true, inputId: 'cancel', prompt: 'Run' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'cancel' }, h.next);
    await h.hooks.get('turn.start')!.run(h.api, { turnId: 'native-turn' }, h.next);
    const abort = new AbortController(); abort.abort();
    const next = Object.assign(vi.fn(async () => ({ result: 'must not execute' })), { signal: abort.signal, called: false });
    const event = { tool: 'Write', tool_use_id: 'cancelled-write', file_path: 'a', content: 'no' };
    await expect(h.hooks.get('tool.call')!.run(h.api, event, next)).rejects.toThrow('interrupted');
    expect(await h.hooks.get('tool.call')!.fail!(h.api, event, next)).toHaveProperty('deny');
    expect(next).not.toHaveBeenCalled();
    expect(h.fetch.mock.calls.some(([url]) => url.endsWith('/call'))).toBe(false);
    expect(h.events().find(e => e.type === 'turn_interrupted')).toMatchObject({ runId: 'cancel', id: 'cancel:1' });
    expect(h.events().find(e => e.type === 'tool_result')).toMatchObject({ id: 'cancelled-write', isError: true });
    expect(h.events().findIndex(e => e.type === 'turn_interrupted')).toBeLessThan(h.events().findIndex(e => e.type === 'tool_result'));
    expect(h.events().some(e => e.type === 'bridge_fault')).toBe(false);
    const native = vi.fn(async () => ({ result: 'later search' }));
    await h.hooks.get('tool.call')!.run(h.api, { tool: 'WebSearch', tool_use_id: 'later', query: 'ok' }, native);
    expect(native).toHaveBeenCalledTimes(1);
  });

  it('never claims an interrupted submitted write was unexecuted and never submits it twice', async () => {
    const h = await inputHarness({ active: true, inputId: 'write', prompt: 'Run' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'write' }, h.next);
    await h.hooks.get('turn.start')!.run(h.api, { turnId: 'native-turn' }, h.next);
    const abort = new AbortController();
    const next = Object.assign(vi.fn(), { signal: abort.signal, called: false });
    const fetch = h.fetch.getMockImplementation()!;
    h.fetch.mockImplementation(async (url, init) => {
      if (url.endsWith('/call')) { abort.abort(); throw Error('dispatch interrupted after submission'); }
      return fetch(url, init);
    });
    const event = { tool: 'Write', tool_use_id: 'write-once', file_path: 'a', content: 'one' };
    await expect(h.hooks.get('tool.call')!.run(h.api, event, next)).rejects.toThrow('after submission');
    const result = await h.hooks.get('tool.call')!.fail!(h.api, event, next);
    expect(result.deny).toContain('outcome may be unknown');
    expect(result.deny).not.toContain('before execution');
    expect(h.fetch.mock.calls.filter(([url]) => url.endsWith('/call'))).toHaveLength(1);
    expect(next).not.toHaveBeenCalled();
    expect(h.events().findIndex(e => e.type === 'turn_interrupted')).toBeLessThan(h.events().findIndex(e => e.type === 'tool_result'));
  });

  it('reports a hook timeout and ends the bridge instead of leaving a silent denial', async () => {
    const h = await inputHarness({ active: true, inputId: 'timeout', prompt: 'Run' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'timeout' }, h.next);
    const abort = new AbortController(); abort.abort();
    const next = Object.assign(vi.fn(), { signal: abort.signal, called: false, error: { kind: 'timeout' } });
    await h.hooks.get('tool.call')!.fail!(h.api, { tool: 'WebSearch', tool_use_id: 'timed-out', query: 'probe' }, next);
    expect(h.events().filter(e => e.type === 'assistant')).toHaveLength(1);
    expect(h.events().filter(e => e.type === 'tool_result')).toHaveLength(1);
    expect(h.events().find(e => e.type === 'bridge_fault')).toMatchObject({ runId: 'timeout' });
    expect(h.events().findIndex(e => e.type === 'bridge_fault')).toBeLessThan(h.events().findIndex(e => e.type === 'tool_result'));
    expect(next).not.toHaveBeenCalled();
    expect(await h.hooks.get('tool.call')!.run(h.api, { tool: 'Write', tool_use_id: 'later' }, next)).toHaveProperty('deny');
  });

  it('rejects an active worker replacement before registering input timers or submitting another prompt', async () => {
    const h = harness();
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { id: async () => 'native', model: async () => 'test' } };
    h.fetch.mockImplementation(async (url) => ({ ok: true, status: 200, text: JSON.stringify(url.endsWith('/ready')
      ? { deny: 'Claude hooks worker restarted during an active turn' } : { active: true, inputId: 'old', tools: [] }) }));
    await expect(h.hooks.get('session.start')!.run(api, {}, h.next)).rejects.toThrow('worker restarted');
    expect(h.api.clock.every).not.toHaveBeenCalled();
    expect(h.next).not.toHaveBeenCalled();
    expect(await h.hooks.get('prompt.submit')!.run(api, { text: 'old' }, h.next)).toHaveProperty('drop');
    expect(await h.hooks.get('tool.call')!.run(api, { tool: 'WebSearch', tool_use_id: 'web' }, h.next)).toHaveProperty('deny');
  });

  it('reports health during a slow native call and a terminal state after a lost completion', async () => {
    const h = await inputHarness({ active: true, inputId: 'health', prompt: 'Run' });
    const health = (h.api.clock.every.mock.calls.find(call => call[0] === 2000) as unknown as [number, () => Promise<void>])[1];
    await health();
    expect(h.events().some(e => e.type === 'bridge_health')).toBe(false);
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'health' }, h.next);
    let finish!: (value: unknown) => void;
    const call = h.hooks.get('tool.call')!.run(h.api, { tool: 'WebSearch', tool_use_id: 'web', query: 'slow' }, () => new Promise(resolve => { finish = resolve; }));
    await vi.waitFor(() => expect(finish).toBeDefined());
    await health();
    expect(h.events().filter(e => e.type === 'bridge_health').slice(-1)[0]).toMatchObject({ runId: 'health', state: 'running' });
    finish({ result: 'done' }); await call;
    const fetch = h.fetch.getMockImplementation()!;
    h.fetch.mockImplementation(async (url, init) => {
      if (JSON.parse(init.body).type === 'complete') throw Error('lost completion receipt');
      return fetch(url, init);
    });
    await expect(h.hooks.get('turn.complete')!.run(h.api, {}, h.next)).rejects.toThrow('lost completion');
    await h.hooks.get('turn.complete')!.fail!(h.api, {}, h.next);
    await health();
    expect(h.events().filter(e => e.type === 'bridge_health').slice(-1)[0]).toMatchObject({ runId: 'health', state: 'complete' });
    expect(h.events().some(e => e.type === 'bridge_fault')).toBe(true);
  });

  it('replays a completed native result from catch and observes its authoritative append without rerunning it', async () => {
    const h = await inputHarness({ active: true, inputId: 'replay', prompt: 'Run' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'replay' }, h.next);
    const event = { tool: 'WebSearch', tool_use_id: 'web', query: 'probe' };
    const native = vi.fn(async () => ({ result: 'actual result' }));
    await h.hooks.get('tool.call')!.run(h.api, event, native);
    const replay = Object.assign(vi.fn(async () => ({ result: 'actual result' })), { called: true });
    await h.hooks.get('tool.call')!.fail!(h.api, event, replay);
    await h.hooks.get('session.append')!.run(h.api, { uuid: 'result', door: 'tool', message: { type: 'user', role: 'user', content: [
      { type: 'tool_result', tool_use_id: 'web', content: 'actual result' },
    ] } }, h.next);
    expect(native).toHaveBeenCalledTimes(1);
    expect(replay).toHaveBeenCalledTimes(1);
    expect(h.events().filter(e => e.type === 'assistant')).toHaveLength(1);
    expect(h.events().filter(e => e.type === 'tool_result')).toHaveLength(3); // host identity ledger deduplicates
    expect(h.events().some(e => e.type === 'bridge_fault')).toBe(false);
  });

  it.each([
    ['Read', true], ['Read', false], ['mcp__locus__read', true], ['mcp__locus__read', false],
  ] as const)('keeps large %s images out of display receipts without changing native history (local summary=%s)', async (tool, hasSummary) => {
    const options = { active: true, inputId: 'large-image', prompt: 'Read the cover' };
    const h = await inputHarness(options);
    const data = 'A'.repeat(1_918_796); // The failing 1.4 MiB JPEG, after base64 expansion.
    const image = { type: 'image', source: { type: 'base64', media_type: 'image/jpeg', data } };
    const summary = '{"file_path":"cover.jpg","image":"attached","byte_size":1439097}';
    const result = tool === 'Read'
      ? { type: 'image', file: { base64: data, type: 'image/jpeg', originalSize: 1_439_097 } }
      : [image];
    h.fetch.mockImplementation(async (url, init) => {
      if (new TextEncoder().encode(init.body).length > 1024 * 1024) {
        return { ok: false, status: 413, text: '{"error":"Invalid bridge body size"}' };
      }
      const body = JSON.parse(init.body);
      return { ok: true, status: 200, text: JSON.stringify(url.endsWith('/options') ? options
        : url.endsWith('/call') ? { callId: body.callId, result, ...(hasSummary ? { locusOutput: summary } : {}) }
        : { ok: true }) };
    });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'large-image' }, h.next);
    const nativeTool = vi.fn();
    const answer = await h.hooks.get('tool.call')!.run(h.api, { tool, tool_use_id: 'image', file_path: 'cover.jpg' }, nativeTool);
    expect(answer.result).toEqual(result);
    const content = [{ type: 'text', text: 'Cover image' }, image];
    const event = { uuid: 'saved-image', door: 'tool', message: { type: 'user', role: 'user', content: [
      { type: 'tool_result', tool_use_id: 'image', content },
    ] } };
    const append = vi.fn(async () => ({}));
    await h.hooks.get('session.append')!.run(h.api, event, append);
    expect(append).toHaveBeenCalledExactlyOnceWith(event);
    expect(image.source.data).toBe(data);
    const receipt = h.events().find(e => e.type === 'tool_result');
    expect(receipt).toMatchObject({ id: 'image', runId: 'large-image', isError: false });
    expect(receipt.output).not.toContain(data);
    if (hasSummary) expect(receipt.output).toBe(summary);
    else {
      expect(receipt.output).toContain('Cover image');
      expect(receipt.output).toContain('image');
    }
    expect(h.events().find(e => e.type === 'checkpoint')).toMatchObject({ uuid: 'saved-image', results: ['image'] });
    const health = (h.api.clock.every.mock.calls.find(call => call[0] === 2000) as unknown as [number, () => Promise<void>])[1];
    await health();
    expect(h.events().slice(-1)[0]).toMatchObject({ type: 'bridge_health', state: 'running' });
    expect(h.fetch.mock.calls.filter(call => call[0].endsWith('/call'))).toHaveLength(1);
    expect(nativeTool).not.toHaveBeenCalled();
  });

  it.each([true, false])('bounds oversized display text and preserves native error verdicts (error=%s)', async (isError) => {
    const options = { active: true, inputId: 'large-text', prompt: 'Run' };
    const h = await inputHarness(options);
    const text = '\u0000\n"\\文😀'.repeat(150_000);
    h.fetch.mockImplementation(async (url, init) => {
      if (new TextEncoder().encode(init.body).length > 1024 * 1024) return { ok: false, status: 413, text: '{}' };
      return { ok: true, status: 200, text: JSON.stringify(url.endsWith('/options') ? options
        : url.endsWith('/call') ? { callId: 'text', result: 'done', locusOutput: isError ? 'local success' : text }
        : { ok: true }) };
    });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'large-text' }, h.next);
    await h.hooks.get('tool.call')!.run(h.api, { tool: 'Read', tool_use_id: 'text', file_path: 'large.txt' }, vi.fn());
    const event = { uuid: 'saved-text', door: 'tool', message: { type: 'user', role: 'user', content: [
      { type: 'tool_result', tool_use_id: 'text', content: text, is_error: isError },
    ] } };
    const append = vi.fn(async () => ({}));
    await h.hooks.get('session.append')!.run(h.api, event, append);
    expect(append).toHaveBeenCalledExactlyOnceWith(event);
    const receipt = h.events().find(e => e.type === 'tool_result');
    expect(receipt.isError).toBe(isError);
    expect(receipt.output.startsWith(text.slice(0, 100))).toBe(true);
    expect(receipt.output).toContain('truncated');
    expect(receipt.output.length).toBeLessThan(70_000);
    expect(h.events().some(e => e.type === 'checkpoint')).toBe(true);
  });

  it('synchronizes the actual native task list without rerunning a mutation', async () => {
    const h=harness({active:true,inputId:'main',prompt:'Tasks'});
    const task={id:'1',subject:'Task A',description:'details',status:'completed',blocks:[],blockedBy:[]};
    const api={...h.api,env:{get:async(key:string)=>key==='LOCUS_CLAUDE_TUI'?'1':key.endsWith('_URL')?'http://127.0.0.1:19200':'test-token'},session:{model:async()=>'claude-opus-5-5',id:async()=>'session'},tool:{call:vi.fn(async(input:{tool:string})=>input.tool==='TaskList'?{result:{tasks:[task]}}:{result:{task}})}};
    await h.hooks.get('session.start')!.run(api,{},h.next);
    await h.hooks.get('prompt.submit')!.run(api,{text:'main'},h.next);
    const mutate=vi.fn(async()=>({result:{success:true}}));
    await h.hooks.get('tool.call')!.run(api,{tool:'TaskUpdate',tool_use_id:'t1',taskId:'1',status:'completed'},mutate);
    expect(mutate).toHaveBeenCalledOnce();
    expect(api.tool.call.mock.calls.map(call=>call[0].tool)).toEqual(['TaskList','TaskGet']);
    const event=h.fetch.mock.calls.map(call=>JSON.parse(call[1].body)).find(event=>event.type==='tool_result');
    expect(JSON.parse(event.output).locusTodoSnapshot).toMatchObject({version:1,tasks:[task],items:[{content:'Task A',status:'completed',priority:'medium'}]});
  });
  it.each(['error', 'result', 'return'])('preserves interrupted response text before the next answer (%s)', async (ending) => {
    const options: Record<string, any> = { active: true, inputId: 'main', prompt: 'Original request' };
    const h = harness(options);
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { model: async () => 'claude-opus-5-5', id: async () => 'session', usage: async () => ({ context: {} }) } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'main' }, h.next);
    const stream = Object.assign((async function* () {
      yield { kind: 'text', text: 'PARTIAL_BEFORE_INSERT' };
      options.insertion = { inputId: 'insert', submissionId: 'draft', prompt: 'New request' };
      await h.hooks.get('prompt.submit')!.run(api, { text: 'insert', turnId: 'old-turn' }, h.next);
      if (ending === 'error') throw new Error('interrupted');
    })(), { result: Promise.resolve({ answer: 'PARTIAL_BEFORE_INSERT', toolUses: [], usage: {} }) });
    const run = async () => { for await (const _ of h.hooks.get('turn.step')!.run(api, { turnId: 'old-turn' }, () => stream)) { /* consume */ } };
    if (ending === 'return') {
      const iterator=h.hooks.get('turn.step')!.run(api,{turnId:'old-turn'},()=>stream);
      await iterator.next();
      options.insertion={inputId:'insert',submissionId:'draft',prompt:'New request'};
      await h.hooks.get('prompt.submit')!.run(api,{text:'insert',turnId:'old-turn'},h.next);
      await iterator.return();
    } else if (ending === 'error') await expect(run()).rejects.toThrow('interrupted');
    else await run();
    const events = h.fetch.mock.calls.map(call => JSON.parse(call[1].body));
    expect(events.filter(event => event.type === 'interrupted_assistant')).toEqual([expect.objectContaining({ message: { content: [{ type: 'text', text: 'PARTIAL_BEFORE_INSERT' }] } })]);
    expect(events.some(event => event.type === 'complete')).toBe(false);
  });

  it('blocks inference when native @image expansion omitted an image', async () => {
    const h = harness({ active: true, inputId: 'input', expectedImageCount: 1, prompt: 'Inspect the image' });
    const abort = vi.fn(async () => ({}));
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { model: async () => 'claude-opus-5-5', id: async () => 'session', messages: async () => [] }, turn: { abort } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next);
    const inference = vi.fn();
    const stream = h.hooks.get('turn.step')!.run(api, { turnId: 'image-turn', index: 0 }, inference);
    // Throwing here would make Claude send the request without the image.
    await expect(stream.next()).resolves.toMatchObject({ done: true, value: { turnId: 'image-turn', answer: '' } });
    expect(inference).not.toHaveBeenCalled();
    expect(abort).toHaveBeenCalledWith({ turnId: 'image-turn' });
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).find(event => event.type === 'complete')).toMatchObject({ reason: 'error', answer: expect.stringContaining('did not load every image') });
  });

  it('preserves native images and rejects a partially staged attachment set', async () => {
    const options = { active: true, inputId: 'input', imageCount: 2, prompt: 'Inspect both images' };
    const h = harness(options);
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { model: async () => 'claude-opus-5-5', id: async () => 'session' } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    const attachments = [{ type: 'image', filename: '1.png' }, { type: 'image', filename: '2.png' }];
    const next = vi.fn(async (input) => input);
    await h.hooks.get('prompt.submit')!.run(api, { text: '[Image #1] [Image #2] input', attachments }, next);
    expect(next).toHaveBeenCalledExactlyOnceWith({ text: options.prompt, attachments });
    next.mockClear();
    expect(await h.hooks.get('prompt.submit')!.run(api, { text: '[Image #1] input', attachments: attachments.slice(0, 1) }, next)).toHaveProperty('drop');
    expect(next).not.toHaveBeenCalled();
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body).type)).toContain('input_error');
  });

  it('acknowledges inserted input only after the native session kept its media row', async () => {
    const h = harness({ active: true, inputId: 'main', insertion: { inputId: 'insert', submissionId: 'draft', prompt: 'New direction', imageCount: 0 }, attachmentMappings: [{ path: '/cache/file.txt', label: 'C:/project/file.txt' }] });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { model: async () => 'claude-opus-5-5', id: async () => 'session' } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'insert' }, h.next);
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body).type)).toContain('input_queued');
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body).type)).not.toContain('input_consumed');
    const media = { type: 'image', source: { type: 'base64', data: 'native-owned' } };
    const queuedRow = { uuid: 'queued-row', door: 'queue', message: { type: 'user', role: 'user', content: [{ type: 'text', text: 'New direction /cache/file.txt' }, media] } };
    const queueNext = vi.fn(async (event) => event);
    await h.hooks.get('session.append')!.run(api, queuedRow, queueNext);
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body).type)).not.toContain('input_consumed');
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body).type)).not.toContain('checkpoint');
    expect(queueNext).toHaveBeenCalledExactlyOnceWith(queuedRow);
    const next = vi.fn(async (event) => {
      expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body).type)).not.toContain('input_consumed');
      return event;
    });
    await h.hooks.get('session.append')!.run(api, { uuid: 'row', door: 'prompt', message: { type: 'user', role: 'user', content: [{ type: 'text', text: 'New direction /cache/file.txt' }, media] } }, next);
    expect(next.mock.calls[0]![0].message.content).toEqual([{ type: 'text', text: 'New direction C:/project/file.txt' }, media]);
    const events = h.fetch.mock.calls.map(call => JSON.parse(call[1].body));
    expect(events.find(event => event.type === 'input_consumed')).toMatchObject({ runId: 'main', id: 'draft' });
    expect(events.find(event => event.type === 'checkpoint')).toMatchObject({ uuid: 'row', user: true, submissionId: 'draft' });
  });

  it('reports the summary returned by native compaction without rewriting its messages', async () => {
    const h = harness({ active: true, inputId: 'input', prompt: 'test' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { model: async () => 'claude-opus-5-5', id: async () => 'session', usage: async () => ({ context: { tokens: 500 } }) } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next);
    const compacted = { messages: [{ role: 'user', text: 'Native summary', toolUses: [], handle: 'opaque' }], tokensAfter: 500 };
    const next = vi.fn(async () => compacted);
    expect(await h.hooks.get('session.compact')!.run(api, { trigger: 'manual', messages: [1, 2, 3] }, next)).toBe(compacted);
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).find(event => event.type === 'compact_done')).toMatchObject({ before: 3, after: 1, summary: 'Native summary', tokens: 500 });
  });

  const compactionHarness = async (prepared: unknown) => {
    const options = { active: true, inputId: 'input', prompt: 'test' };
    const h = harness(options);
    h.fetch.mockImplementation(async (_url, init) => {
      if (JSON.parse(init.body).type !== 'compact_prepare') return { ok: true, status: 200, text: JSON.stringify(options) };
      if (prepared instanceof Error) throw prepared;
      return { ok: true, status: 200, text: JSON.stringify(prepared) };
    });
    const usage = vi.fn(async (args?: { breakdown?: string }) => args?.breakdown
      ? { context: { window: 1_000_000, breakdown: { totalTokens: 48_000 } } } : { context: { tokens: 500, window: 1_000_000 } });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'claude-opus-5-5', id: async () => 'session', usage } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next);
    const events = () => h.fetch.mock.calls.map(call => JSON.parse(call[1].body));
    // Claude runs SessionStart (source "compact") inside the native compaction.
    const sessionStart = (event: object) => h.hooks.get('classic.SessionStart')!.run(api, { hook_event_name: 'SessionStart', ...event }, async () => ({}));
    return { h, api, events, sessionStart };
  };
  const nativeSummary = 'Summary:\nWORK\n\nIf you need details, read the full transcript at: /Users/r/.claude/projects/x/abc.jsonl\nContinue the conversation.';

  it('delivers Locus compaction context as the SessionStart context of the compaction', async () => {
    const prepared = { ok: true, instructions: 'LOCUS_RULES', transcriptPath: 'C:/tmp/compact-transcripts/claude/s/n.jsonl', carryForward: '## Locus Carry-Forward Context\nCARRY' };
    const { h, api, events, sessionStart } = await compactionHarness(prepared);
    expect(await sessionStart({ source: 'resume' })).toEqual({});
    const compacted = { tokensAfter: 900, messages: [
      { role: 'user', text: nativeSummary, toolUses: [], handle: 'summary' },
      { role: 'assistant', text: 'Kept answer', toolUses: [{ tool_use_id: 'toolu_1', tool: 'Bash', input: {} }], handle: 'kept-1' },
      { role: 'user', text: '', toolUses: [], toolResults: [{ tool_use_id: 'toolu_1', text: 'ok', isError: false }], handle: 'kept-2' },
    ] };
    const contexts: unknown[] = [];
    const next = vi.fn(async (_event: { instructions?: string }) => {
      contexts.push(await sessionStart({ source: 'compact', agent_id: 'subagent' }));
      contexts.push(await sessionStart({ source: 'compact' }));
      contexts.push(await sessionStart({ source: 'compact' }));
      return compacted;
    });
    expect(await h.hooks.get('session.compact')!.run(api, { trigger: 'auto', messages: [1, 2, 3, 4], instructions: 'USER_FOCUS' }, next)).toBe(compacted);
    expect(next.mock.calls[0]![0].instructions).toBe('USER_FOCUS\n\nLOCUS_RULES');
    expect(contexts).toEqual([{}, { additionalContext: [prepared.carryForward] }, {}]);
    expect(events().map(event => String(event.type ?? '')).filter(type => type.startsWith('compact_'))).toEqual(['compact_start', 'compact_prepare', 'compact_done']);
    expect(events().find(event => event.type === 'compact_done')).toMatchObject({
      before: 4, after: 3, tokens: 900, estimate: 48_000, summary: nativeSummary, carryForward: prepared.carryForward,
      transcriptPath: prepared.transcriptPath, kept: [
        { role: 'assistant', text: 'Kept answer', toolUseIds: ['toolu_1'], toolResultIds: [] },
        { role: 'user', text: '', toolUseIds: [], toolResultIds: ['toolu_1'] },
      ] });
    expect(events().some(event => event.type === 'diagnostic')).toBe(false);
    // Nothing stays staged for a later compaction or summary row.
    expect(await sessionStart({ source: 'compact' })).toEqual({});
    const append = vi.fn(async (event) => event);
    const row = { uuid: 'summary-row', door: 'compaction', message: { type: 'user', role: 'user', content: [{ type: 'text', text: nativeSummary }] } };
    await h.hooks.get('session.append')!.run(api, row, append);
    expect(append.mock.calls[0]![0].message.content).toEqual(row.message.content);
  });

  it('reports a compaction whose SessionStart context was not requested', async () => {
    const { h, api, events } = await compactionHarness({ ok: true, instructions: 'LOCUS_RULES', transcriptPath: 'C:/tmp/n.jsonl', carryForward: 'CARRY' });
    await h.hooks.get('session.compact')!.run(api, { trigger: 'manual', messages: [1] },
      async () => ({ messages: [{ role: 'user', text: nativeSummary, toolUses: [], handle: 'summary' }] }));
    expect(events().filter(event => event.type === 'diagnostic' && event.kind === 'compaction_context')).toHaveLength(1);
    expect(events().find(event => event.type === 'compact_done')).toMatchObject({ summary: nativeSummary, carryForward: null });
  });

  it('warns when the transcript copy failed but still delivers the carry-forward context', async () => {
    const { h, api, events, sessionStart } = await compactionHarness({ ok: true, instructions: 'LOCUS_RULES', transcriptPath: null, transcriptError: 'archive refresh timed out', carryForward: 'CARRY' });
    let context: unknown;
    await h.hooks.get('session.compact')!.run(api, { trigger: 'auto', messages: [1] }, async () => {
      context = await sessionStart({ source: 'compact' });
      return { messages: [{ role: 'user', text: nativeSummary, toolUses: [], handle: 'summary' }] };
    });
    expect(context).toEqual({ additionalContext: ['CARRY'] });
    const notice = events().find(event => event.type === 'diagnostic');
    expect(notice).toMatchObject({ kind: 'compaction_context' });
    expect(notice.text).toContain('archive refresh timed out');
    expect(events().find(event => event.type === 'compact_done')).toMatchObject({ summary: nativeSummary, carryForward: 'CARRY', transcriptPath: null });
  });

  it('compacts natively when Locus cannot prepare compaction context', async () => {
    const { h, api, events, sessionStart } = await compactionHarness(new Error('bridge unavailable'));
    const compacted = { messages: [{ role: 'user', text: 'Native summary', toolUses: [], handle: 'summary' }] };
    let context: unknown;
    const next = vi.fn(async (_event: object) => { context = await sessionStart({ source: 'compact' }); return compacted; });
    const input = { trigger: 'auto', messages: [1] };
    expect(await h.hooks.get('session.compact')!.run(api, input, next)).toBe(compacted);
    expect(next).toHaveBeenCalledExactlyOnceWith(input);
    expect(context).toEqual({});
    expect(events().filter(event => event.type === 'diagnostic').map(event => event.kind)).toEqual(['compaction_context']);
    expect(events().find(event => event.type === 'compact_done')).toMatchObject({ summary: 'Native summary', carryForward: null, kept: [], transcriptPath: null });
  });

  it('clears staged compaction context when native compaction fails', async () => {
    const { h, api, sessionStart } = await compactionHarness({ ok: true, instructions: 'LOCUS_RULES', transcriptPath: null, carryForward: 'CARRY' });
    await expect(h.hooks.get('session.compact')!.run(api, { trigger: 'auto', messages: [1] }, async () => { throw new Error('summary failed'); }))
      .rejects.toThrow('summary failed');
    expect(await sessionStart({ source: 'compact' })).toEqual({});
  });

  it('keeps synthetic CLI notices in native storage but out of conversation events', async () => {
    const h=harness({active:true,inputId:'input',prompt:'question'});
    const api={...h.api,env:{get:async(key:string)=>key==='LOCUS_CLAUDE_TUI'?'1':key.endsWith('_URL')?'http://127.0.0.1:19200':'test-token'},session:{id:async()=>'session',model:async()=>'claude-opus-5-5'}};
    await h.hooks.get('session.start')!.run(api,{},h.next);
    await h.hooks.get('prompt.submit')!.run(api,{text:'input'},h.next);
    const event={uuid:'error-row',door:'response',origin:{kind:'model',model:'<synthetic>'},message:{type:'assistant',role:'assistant',content:[{type:'text',text:'Run /model'}]}};
    const append=vi.fn(async e=>e);
    await h.hooks.get('session.append')!.run(api,event,append);
    expect(append).toHaveBeenCalledExactlyOnceWith(event);
    let events=h.fetch.mock.calls.map(call=>JSON.parse(call[1].body));
    expect(events.filter(e=>['checkpoint','assistant','text'].includes(e.type))).toEqual([]);
    expect(events.find(e=>e.type==='diagnostic')).toMatchObject({text:'Run /model'});
    // Identical text from the actual model is conversation content.
    await h.hooks.get('session.append')!.run(api,{...event,uuid:'actual-row',origin:{kind:'model',model:'claude-opus-5-5'}},append);
    events=h.fetch.mock.calls.map(call=>JSON.parse(call[1].body));
    expect(events.find(e=>e.type==='checkpoint')).toMatchObject({uuid:'actual-row',text:'Run /model'});
  });

  it.each(['model_refusal_no_fallback', 'model_refusal_fallback', 'model_fallback', 'model_consent_fallback', 'instruction_size_warning', 'permission_denied', 'file_attachments_missing', 'error'])(
    'forwards native %s warnings without changing the native transcript or emitting a checkpoint', async (kind) => {
      const h = await inputHarness({ active: true, inputId: 'input', prompt: 'question' });
      await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, h.next);
      const event = { uuid: 'notice', door: 'note', origin: { kind: 'engine' }, message: {
        type: 'system', name: kind, content: [{ type: 'text', text: 'Native explanation' }],
      } };
      const append = vi.fn(async e => e);
      await h.hooks.get('session.append')!.run(h.api, event, append);
      expect(append).toHaveBeenCalledExactlyOnceWith(event);
      expect(h.events().filter(e => e.type === 'diagnostic')).toEqual([
        { protocol: 1, type: 'diagnostic', runId: 'input', kind, text: 'Native explanation' },
      ]);
      expect(h.events().some(e => ['checkpoint', 'assistant', 'text'].includes(e.type))).toBe(false);
    },
  );

  it('reports retry activity even when the native append projection omits counters and error text', async () => {
    const h = await inputHarness({ active: true, inputId: 'input', prompt: 'question' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, h.next);
    await h.hooks.get('session.append')!.run(h.api, { uuid: 'retry', door: 'note', origin: { kind: 'engine' },
      message: { type: 'system', name: 'api_error', content: [] } }, h.next);
    expect(h.events().find(e => e.type === 'diagnostic')).toMatchObject({ kind: 'api_retry', text: 'Claude Code is retrying an API request.' });
  });

  it.each(['instruction_size_warning', 'file_attachments_missing', 'model_refusal_no_fallback'])(
    'still reports %s when native projection contains no display text', async (name) => {
      const h = await inputHarness({ active: true, inputId: 'input', prompt: 'question' });
      await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, h.next);
      await h.hooks.get('session.append')!.run(h.api, { uuid: 'warning', door: 'note', origin: { kind: 'engine' },
        message: { type: 'system', name, content: [] } }, h.next);
      expect(h.events().find(e => e.type === 'diagnostic')).toMatchObject({ kind: name, text: expect.any(String) });
      expect(h.events().find(e => e.type === 'diagnostic').text.length).toBeGreaterThan(0);
    },
  );

  it('preserves input blocked by a native hook instead of waiting for the generic input timeout', async () => {
    const h = await inputHarness({ active: true, inputId: 'input', prompt: 'question' });
    const denied = { drop: 'Prompt blocked by organization hook' };
    const next = vi.fn(async () => denied);
    expect(await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, next)).toBe(denied);
    expect(h.events().find(e => e.type === 'input_error')).toMatchObject({ error: denied.drop });
    expect(h.events().some(e => e.type === 'input_queued')).toBe(false);
  });

  it('preserves prompt and command hook failure details', async () => {
    const h = await inputHarness({ active: true, inputId: 'input', prompt: 'question' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, h.next);
    const next = Object.assign(vi.fn(), { error: { message: 'Native hook budget exceeded' } });
    expect(await h.hooks.get('prompt.submit')!.fail!(h.api, {}, next)).toEqual({ drop: 'Locus input bridge failed. Native hook budget exceeded' });
    await h.hooks.get('command.run')!.fail!(h.api, { command: 'compact' }, next);
    expect(h.events().filter(e => e.type === 'input_error').map(e => e.error)).toEqual([
      'Locus input bridge failed. Native hook budget exceeded', 'Claude command failed: /compact. Native hook budget exceeded',
    ]);
  });

  it('does not leak idle, subagent, plugin, or ordinary system records as warnings', async () => {
    const options = { active: true, inputId: 'input', prompt: 'question' };
    const h = await inputHarness(options);
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, h.next);
    const row = { uuid: 'row', door: 'note', origin: { kind: 'engine' }, message: {
      type: 'system', name: 'model_fallback', content: [{ type: 'text', text: 'Fallback notice' }],
    } };
    await h.hooks.get('session.append')!.run(h.api, { ...row, agentId: 'subagent' }, h.next);
    await h.hooks.get('session.append')!.run(h.api, { ...row, origin: { kind: 'plugin', name: 'custom' } }, h.next);
    await h.hooks.get('session.append')!.run(h.api, { ...row, message: { ...row.message, name: 'turn_duration' } }, h.next);
    options.active = false;
    await h.hooks.get('session.append')!.run(h.api, row, h.next);
    expect(h.events().filter(e => e.type === 'diagnostic')).toEqual([]);
  });

  it('reports native permission denial without approving or executing the operation', async () => {
    const h = await inputHarness({ active: true, inputId: 'input', prompt: 'question' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, h.next);
    const deny = { decision: 'deny', reason: 'Blocked by managed policy' };
    const next = vi.fn(async () => deny);
    expect(await h.hooks.get('tool.check')!.run(h.api, { tool: 'WebFetch' }, next)).toBe(deny);
    expect(next).toHaveBeenCalledOnce();
    expect(h.events().find(e => e.type === 'diagnostic')).toMatchObject({ kind: 'tool_denied', text: 'WebFetch: Blocked by managed policy' });
    expect(h.events().some(e => ['assistant', 'tool_result'].includes(e.type))).toBe(false);
  });

  it('preserves structured refusal details on completion with unfinished streamed tool input', async () => {
    const h = await inputHarness({ active: true, inputId: 'input', prompt: 'question' });
    await h.hooks.get('prompt.submit')!.run(h.api, { text: 'input' }, h.next);
    const verdict = { reason: 'refusal', isAborted: false, answer: '', refusal: { category: 'policy', explanation: 'Output blocked' } };
    await h.hooks.get('turn.complete')!.run(h.api, verdict, h.next);
    expect(h.events().find(e => e.type === 'complete')).toMatchObject(verdict);
  });

  it('removes only the CLI environment attachment, including on resume and hook failure', async () => {
    const h = harness();
    const api = { ...h.api, env: { get: async () => '1' } };
    const hook = h.hooks.get('prompt.attachment')!;
    const remote = {type:'environment', text:'Platform: linux; cwd: /remote/terminal'};
    // This can run before session.start and can re-render old attachments on resume.
    expect(await hook.run(api, remote, h.next)).toEqual({text:null});
    expect(h.next).not.toHaveBeenCalled();
    expect(await hook.fail!(api, remote, h.next)).toEqual({text:null});
    await hook.run(api, {type:'model',text:'actual model'}, h.next);
    expect(h.next).toHaveBeenCalledOnce();
  });
  it('reports requested and returned models separately and sends per-response usage', async () => {
    const h = harness({ ok: true, active: true });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'claude-opus-5-5', id: async () => 'session', usage: async () => ({ context: { window: 200000, tokens: 999 } }) } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    const usage = { model: 'actual-model', input_tokens: 11, output_tokens: 7, cache_read_input_tokens: 80, cache_creation_input_tokens: 0 };
    const stream = Object.assign((async function* () { yield { kind: 'text', text: 'answer' }; })(), {
      result: Promise.resolve({ answer: 'answer', toolUses: [], usage }),
    });
    const next = vi.fn(() => stream);
    for await (const _chunk of h.hooks.get('turn.step')!.run(api, { model: 'requested-model' }, next)) { /* consume */ }
    const events = h.fetch.mock.calls.map(call => JSON.parse(call[1].body));
    expect(events.find(event => event.type === 'step_start')).toMatchObject({ model: 'requested-model' });
    expect(events.find(event => event.type === 'usage')).toMatchObject({ stepId: 1, usage, context: { window: 200000 } });
    expect(events.findIndex(event => event.type === 'usage')).toBeLessThan(events.findIndex(event => event.type === 'assistant'));
  });

  it('keeps each request usage and identity within one long native turn', async () => {
    const h = harness({ active: true, inputId: 'usage-turn', prompt: 'hello' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'actual-model', id: async () => 'session', usage: async () => ({ context: { window: 200000 } }) } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'usage-turn' }, h.next);
    const samples = [
      { model: 'actual-model', input_tokens: 5, output_tokens: 7, cache_read_input_tokens: 100000, cache_creation_input_tokens: 1000 },
      { model: 'actual-model', input_tokens: 5, output_tokens: 8, cache_read_input_tokens: 0, cache_creation_input_tokens: 102000 },
    ];
    for (const usage of samples) {
      const next = () => Object.assign((async function* () { yield { kind: 'text', text: 'answer' }; })(), {
        result: Promise.resolve({ answer: 'answer', toolUses: [], usage }),
      });
      for await (const _ of h.hooks.get('turn.step')!.run(api, { model: 'actual-model' }, next)) { /* consume */ }
    }
    const receipts = h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).filter(e => e.type === 'usage');
    expect(receipts).toHaveLength(2);
    expect(receipts.map(e => e.stepId)).toEqual([1, 2]);
    expect(receipts.map(e => e.runId)).toEqual(['usage-turn', 'usage-turn']);
    expect(receipts.map(e => e.usage)).toEqual(samples);
  });

  it('preserves the CLI-resolved model instead of sending the GUI alias upstream', async () => {
    const h = harness({ active: true, inputId: 'input-model', model: 'opus', prompt: 'hello' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'claude-opus-5-5', id: async () => 'session', usage: async () => ({ context: {} }) } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input-model' }, h.next);
    const stream = Object.assign((async function* () { yield { kind: 'text', text: 'ok' }; })(),
      { result: Promise.resolve({ answer: 'ok', toolUses: [], usage: { model: 'claude-opus-5-5' } }) });
    const infer = vi.fn(() => stream);
    for await (const _ of h.hooks.get('turn.step')!.run(api, { model: 'claude-opus-5-5' }, infer)) { /* consume */ }
    expect(infer).toHaveBeenCalledExactlyOnceWith({ model: 'claude-opus-5-5' });
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).find(e => e.type === 'step_start')).toMatchObject({ model: 'claude-opus-5-5' });
  });

  it.each(['model', 'help', 'context', 'cost', 'compact'])('does not adapt /%s chat commands into conversation output', async (command) => {
    const h = harness({ active: true, inputId: 'input-model' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { id: async () => 'session', model: async () => 'vendor/actual' } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    h.next.mockClear();
    expect(await h.hooks.get('command.run')!.run(api, { command, args: '' }, h.next)).toEqual({ text: '' });
    expect(h.next).not.toHaveBeenCalled();
    const events = h.fetch.mock.calls.map(call => JSON.parse(call[1].body));
    expect(events.some(e => ['command', 'text', 'assistant'].includes(e.type))).toBe(false);
    expect(events.find(e => e.type === 'input_error')).toBeDefined();
  });

  it('runs explicit GUI compaction without emitting command text as an assistant reply', async () => {
    const h = harness({ active: true, inputId: 'compact', control: 'compact' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { model: async () => 'claude-opus-5-5', id: async () => 'session' } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    h.next.mockClear();
    await h.hooks.get('command.run')!.run(api, { command: 'compact', args: '' }, h.next);
    expect(h.next).toHaveBeenCalledExactlyOnceWith({ command: 'compact', args: '' });
    const event = h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).find(e => e.type === 'command');
    expect(event).toEqual({ protocol: 1, type: 'command', runId: 'compact' });
  });

  it('never executes local or native tools while compacting interrupted history', async () => {
    const h = harness({ active: true, inputId: 'compact', control: 'compact' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'claude-opus-5-5', id: async () => 'session' } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    await h.hooks.get('command.run')!.run(api, { command: 'compact', args: '' }, h.next);
    h.next.mockClear();
    h.fetch.mockClear();
    for (const tool of ['Edit', 'Bash', 'mcp__locus__unity_recompile', 'TaskUpdate', 'WebSearch', 'WebFetch']) {
      expect(await h.hooks.get('tool.call')!.run(api, { tool, tool_use_id: 'interrupted' }, h.next))
        .toEqual({ deny: 'Tool execution is disabled during Locus compaction.' });
      expect(await h.hooks.get('tool.check')!.run(api, { tool, input: {} }, h.next))
        .toMatchObject({ decision: 'deny' });
    }
    expect(h.next).not.toHaveBeenCalled();
    expect(h.fetch).not.toHaveBeenCalled();
  });

  it('preserves the native failure reason when compaction fails', async () => {
    const h = harness();
    const next = Object.assign(vi.fn(async () => ({ skip: 'native failure' })), {
      called: true, error: { kind: 'throw', message: 'Compaction failed: request timed out' },
    });
    await h.hooks.get('session.compact')!.fail!(h.api, { trigger: 'manual' }, next);
    const error = h.fetch.mock.calls.map(call => JSON.parse(call[1].body)).find(event => event.type === 'input_error');
    expect(error.error).toContain('Compaction failed: request timed out');
  });

  it('refuses unadapted commands that could bypass the local tool guard', async () => {
    const h = harness({ inputId: 'command' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' }, session: { model: async () => 'claude-opus-5-5', id: async () => 'session' } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    h.next.mockClear();
    const result = await h.hooks.get('command.run')!.run(api, { command: 'export', args: 'remote-file.md' }, h.next);
    expect(result.text).toBe('');
    expect(h.next).not.toHaveBeenCalled();
    expect(h.fetch.mock.calls.map(call => JSON.parse(call[1].body).type)).toContain('command_start');
    expect((await h.hooks.get('command.run')!.fail!(api, {}, h.next)).text).toContain('未回退');
    expect(h.next).not.toHaveBeenCalled();
  });
  it('intercepts Bash with its exact ID and input without calling the original tool', async () => {
    const h = harness();
    const answer = await h.hooks.get('tool.call')!.run(h.api, { tool: 'Bash', tool_use_id: 't1', command: 'pwd', timeout: 3000 }, h.next);
    expect(answer.result.stdout).toBe('local');
    expect(h.next).not.toHaveBeenCalled();
    expect(JSON.parse(h.fetch.mock.calls[0]![1].body)).toEqual({ protocol: 1, tool: 'Bash', callId: 't1', agentId: null, input: { command: 'pwd', timeout: 3000 } });
  });

  it.each(['Read', 'Write', 'Edit', 'Bash', 'Glob', 'Grep', 'TaskOutput', 'TaskStop', 'AskUserQuestion'])('forwards %s without renaming the native tool', async (tool) => {
    const h = harness({ callId: 't1', deny: 'user rejected' });
    expect(await h.hooks.get('tool.call')!.run(h.api, { tool, tool_use_id: 't1', file_path: 'a' }, h.next)).toEqual({ deny: 'user rejected' });
    expect(h.next).not.toHaveBeenCalled();
  });

  it.each(['Read', 'Write', 'Edit', 'Bash', 'Glob', 'Grep', 'TaskOutput', 'TaskStop', 'AskUserQuestion'])('preserves %s protocol/context without leaking the journal display envelope', async (tool) => {
    const result = { marker: 'native protocol' };
    const context = ['Knowledge index: updated', '<system-reminder>Task completed</system-reminder>'];
    const h = harness({ callId: 't1', result, context, locusOutput: 'compact display only' });
    expect(await h.hooks.get('tool.call')!.run(h.api, { tool, tool_use_id: 't1' }, h.next)).toEqual({ result, context });
    expect(h.next).not.toHaveBeenCalled();
  });

  it('keeps session bookkeeping on its existing path', async () => {
    const h = harness();
    for (const tool of ['TodoWrite', 'ToolSearch']) {
      await h.hooks.get('tool.call')!.run(h.api, { tool }, h.next);
    }
    expect(h.next).toHaveBeenCalledTimes(2);
    expect(h.fetch).not.toHaveBeenCalled();
  });

  it.each([false, true])('routes renamed Mod tools locally even before TUI startup (started=%s)', async (started) => {
    const blocks = [{ type: 'text', text: 'LOCAL' }];
    const h = harness({ callId: 't1', result: blocks });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'claude-opus-5-5', id: async () => 'session' } };
    if (started) await h.hooks.get('session.start')!.run(api, {}, h.next);
    h.next.mockClear();
    h.fetch.mockClear();
    for (const tool of ['mcp__locus__python', 'mcp__locus__unity_execute']) {
      expect(await h.hooks.get('tool.call')!.run(api, { tool, tool_use_id: 't1', code: 'print(await locus.wait_task("t1", timeout=30))', readonly: true }, h.next)).toEqual({ result: blocks });
      const calls = h.fetch.mock.calls.filter(call => call[0].endsWith('/call'));
      const call = calls[calls.length - 1]!;
      expect(JSON.parse(call[1].body)).toMatchObject({ tool, callId: 't1', input: { code: 'print(await locus.wait_task("t1", timeout=30))', readonly: true } });
      expect(await h.hooks.get('tool.check')!.run(api, { tool }, h.next)).toMatchObject({ decision: 'deny' });
    }
    h.fetch.mockRejectedValueOnce(new Error('SSH disconnected'));
    const hook = h.hooks.get('tool.call')!;
    await expect(hook.run(api, { tool: 'mcp__locus__python', tool_use_id: 't1' }, h.next)).rejects.toThrow('SSH disconnected');
    expect(await hook.fail!(api, {}, h.next)).toMatchObject({ deny: expect.stringContaining('no fallback') });
    expect(h.next).not.toHaveBeenCalled();
  });

  it.each([false, true])('converts local MCP images to Claude source blocks before native append (started=%s)', async (started) => {
    const text = { type: 'text', text: 'Captured Unity viewport' };
    const images = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].map((mimeType, index) => ({
      type: 'image', mimeType, data: `image-bytes-${index}`,
    }));
    const nativeImage = { type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'already-native' } };
    const blocks = [text, ...images, nativeImage, { type: 'text', text: 'After the images' }];
    const context = ['Background task completed'];
    // Journal replays use this same bridge envelope; conversion must not depend
    // on fresh execution or rewrite its stored MCP data / GUI display output.
    const h = harness({ callId: 't1', result: blocks, context, locusOutput: 'journal display only' });
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'claude-opus-5-5', id: async () => 'session' } };
    if (started) await h.hooks.get('session.start')!.run(api, {}, h.next);
    h.next.mockClear();
    for (const tool of ['mcp__locus__unity_capture_viewport', 'mcp__locus__unity_execute', 'mcp__locus__python']) {
      const answer = await h.hooks.get('tool.call')!.run(api, { tool, tool_use_id: 't1' }, h.next);
      expect(answer).toEqual({ result: [text, ...images.map(image => ({ type: 'image',
        source: { type: 'base64', media_type: image.mimeType, data: image.data },
      })), nativeImage, blocks[blocks.length - 1]], context });
      expect(answer.result.slice(1, 5).every((block: Record<string, unknown>) => !('mimeType' in block) && !('data' in block))).toBe(true);
    }
    expect(images[0]).toEqual({ type: 'image', mimeType: 'image/png', data: 'image-bytes-0' });
    expect(h.next).not.toHaveBeenCalled();
  });

  it('converts a polled image receipt without resubmitting the tool', async () => {
    const h = harness({ callId: 't1', result: [{ type: 'image', mimeType: 'image/png', data: 'saved-image' }] });
    h.fetch.mockResolvedValueOnce({ ok: true, status: 202, text: JSON.stringify({ pending: 'saved-receipt' }) });
    expect(await h.hooks.get('tool.call')!.run(h.api, { tool: 'mcp__locus__python', tool_use_id: 't1' }, h.next))
      .toEqual({ result: [{ type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'saved-image' } }] });
    expect(h.fetch.mock.calls.map(call => new URL(call[0]).pathname)).toEqual(['/call', '/poll']);
    expect(h.next).not.toHaveBeenCalled();
  });

  it('keeps native Read image envelopes for Claude own renderer', async () => {
    const result = { type: 'image', file: { base64: 'native-read', type: 'image/png', originalSize: 8 } };
    const h = harness({ callId: 't1', result });
    expect(await h.hooks.get('tool.call')!.run(h.api, { tool: 'Read', tool_use_id: 't1' }, h.next)).toEqual({ result });
    expect(h.next).not.toHaveBeenCalled();
  });

  it('registers Python for task control without duplicating native Read or adding a task-output tool', async () => {
    const h = harness({ active: true, inputId: 'input', prompt: 'Read the local file', tools: [
      { name: 'read', description: 'Locus read', input_schema: {} },
      { name: 'web_fetch', description: 'Retired Locus fetch', input_schema: {} },
      { name: 'python', description: 'Locus Python', input_schema: {} },
    ] });
    const register = vi.fn(async () => ({}));
    const api = { ...h.api, env: { get: async (key: string) => key === 'LOCUS_CLAUDE_TUI' ? '1' : key.endsWith('_URL') ? 'http://127.0.0.1:19200' : 'test-token' },
      session: { model: async () => 'claude-opus-5-5', id: async () => 'session' }, tool: { register } };
    await h.hooks.get('session.start')!.run(api, {}, h.next);
    h.next.mockClear();
    await h.hooks.get('prompt.submit')!.run(api, { text: 'input' }, h.next);
    expect(register).toHaveBeenCalledExactlyOnceWith({ name: 'python', description: 'Locus Python', inputSchema: {} });
    const describe = h.hooks.get('tool.describe')!;
    expect(describe.matcher).toEqual({ tool: /^mcp__locus__/ });
    expect(await describe.run(api, { tool: 'mcp__locus__python' }, async () => ({ description: 'Locus Python', isDeferred: true })))
      .toEqual({ description: 'Locus Python', isDeferred: false });
    expect(h.next).toHaveBeenCalledExactlyOnceWith({ text: 'Read the local file' });
  });

  it('refuses unknown tools instead of executing them on the Claude host', async () => {
    const h = harness();
    expect((await h.hooks.get('tool.call')!.run(h.api, { tool: 'FutureNativeTool' }, h.next)).deny).toContain('not adapted');
    expect(h.next).not.toHaveBeenCalled();
  });

  it('does not accept a response from a different call', async () => {
    const h = harness({ callId: 'other', result: 'wrong' });
    await expect(h.hooks.get('tool.call')!.run(h.api, { tool: 'Read', tool_use_id: 't1' }, h.next)).rejects.toThrow('ID mismatch');
    expect(h.next).not.toHaveBeenCalled();
  });

  it('polls slow local results without submitting a tool twice', async () => {
    const h=harness();
    h.fetch.mockResolvedValueOnce({ok:true,status:202,text:JSON.stringify({pending:'receipt1'})});
    h.fetch.mockResolvedValueOnce({ok:true,status:202,text:JSON.stringify({pending:'receipt1'})});
    const answer=await h.hooks.get('tool.call')!.run(h.api,{tool:'Bash',tool_use_id:'t1',command:'sleep 45'},h.next);
    expect(answer.result.stdout).toBe('local');
    expect(h.fetch.mock.calls.map(call=>new URL(call[0]).pathname)).toEqual(['/call','/poll','/poll']);
    expect(JSON.parse(h.fetch.mock.calls[1]![1].body)).toEqual({protocol:1,ticket:'receipt1'});
    expect(h.next).not.toHaveBeenCalled();
  });

  it('fails closed on hook errors, and independently denies native execution', async () => {
    const h = harness();
    expect((await h.hooks.get('tool.call')!.fail!(h.api, {}, { called: false })).deny).toContain('bridge failed');
    expect(await h.hooks.get('tool.check')!.run(h.api, { tool: 'Bash' }, h.next)).toMatchObject({ decision: 'deny' });
    expect(await h.hooks.get('tool.check')!.fail!()).toMatchObject({ decision: 'deny' });
    expect(h.next).not.toHaveBeenCalled();
  });

  it('preserves an already completed passthrough result when an observer fails', async () => {
    const h = harness();
    const completed = Object.assign(vi.fn(async () => ({ result: 'already completed' })), { called: true });
    expect(await h.hooks.get('tool.call')!.fail!(h.api, {}, completed)).toEqual({ result: 'already completed' });
    expect(h.fetch).not.toHaveBeenCalled();
  });

  it('handshakes without an inference request', async () => {
    const h = harness({ ok: true });
    await h.hooks.get('session.start')!.run(h.api, {}, h.next);
    expect(h.fetch.mock.calls[0]![0]).toMatch(/\/ready$/);
    expect(h.next).toHaveBeenCalledOnce();
  });
});
