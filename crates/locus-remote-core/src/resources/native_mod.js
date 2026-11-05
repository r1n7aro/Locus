// Runs inside Claude Code >= 2.1.299. All filesystem/process work stays in Locus.
// Keep this module self-contained: Claude statically checks every mods API call.
async function callBridge($, path, body) {
  const url = await $.env.get('LOCUS_CLAUDE_BRIDGE_URL');
  const token = await $.env.get('LOCUS_CLAUDE_BRIDGE_TOKEN');
  if (!url || !/^http:\/\/127\.0\.0\.1:[0-9]+$/.test(url) || !token) {
    throw new Error('Locus tool bridge is unavailable');
  }
  // Claude caps one HTTP operation at 30 seconds. Only submission may execute
  // a tool; subsequent requests poll its receipt and never submit it again.
  for (;;) {
  const response = await $.http.fetch(url + path, {
    method: 'POST',
    headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error('Locus tool bridge returned HTTP ' + response.status);
  const answer = JSON.parse(response.text);
  if (!answer.pending) return answer;
  path = '/poll'; body = { protocol: 1, ticket: answer.pending };
  }
}

async function reportInterruption($, turnId, runId, text, thinking) {
  await callBridge($, '/event', { protocol: 1, type: 'turn_interrupted', runId, id: turnId });
  const content = [];
  if (text) content.push({ type: 'text', text });
  if (thinking) content.push({ type: 'thinking', thinking });
  if (content.length) await callBridge($, '/event', { protocol: 1, type: 'interrupted_assistant', runId, message: { content } });
}

// A turn.step hook that throws before next() makes Claude run the original
// step, so the request still reaches the model. Refuse by ending the turn and
// returning an empty step instead; the abort may fail if the turn is ending.
async function refuseStep($, e) {
  try { await $.turn.abort({ turnId: e.turnId }); } catch { /* already ending */ }
  return { turnId: e.turnId, index: e.index, answer: '', toolUses: [], stopReason: null, usage: null };
}

// Mods only allows passing $ to file-level helpers; keep session state explicit.
async function reportPhase($, phase, activity) {
  if (!activity || activity.phase === phase) return;
  activity.phase = phase;
  const event = { protocol: 1, type: 'phase', runId: activity.runId,
    stepId: activity.stepId, sequence: ++activity.sequence, phase };
  // The host rejects out-of-order sequences. Never await another hook's
  // Promise: that wait spends this hook's 10 s budget, unlike its own $ calls.
  try { await callBridge($, '/event', event); } catch { /* display only */ }
}

async function registerTools($, tools, registeredTools, toolPlacements) {
  // Validate first: replacing even one eager/previously-referenced schema
  // changes the cache prefix. Registration is append-only for this session.
  for (const tool of tools || []) {
    if (tool.name === 'read' || tool.name === 'web_fetch') continue;
    const prior = registeredTools.get(tool.name);
    if (prior && prior !== JSON.stringify(tool)) throw new Error('Claude tool definition changed: ' + tool.name + '. Start a new conversation.');
  }
  for (const tool of tools || []) {
    if (tool.name === 'read' || tool.name === 'web_fetch' || registeredTools.has(tool.name)) continue;
    toolPlacements.set('mcp__locus__' + tool.name, tool.defer_loading === true);
    // Reserve synchronously: concurrent tool receipts may contain the same
    // append-only delta. The owning hook still awaits the actual registration.
    registeredTools.set(tool.name, JSON.stringify(tool));
    try { await $.tool.register({ name: tool.name, description: tool.description, inputSchema: tool.input_schema }); }
    catch (error) {
      registeredTools.delete(tool.name);
      toolPlacements.delete('mcp__locus__' + tool.name);
      throw error;
    }
  }
}

// The host/journal uses MCP content, but tools registered through Mods bypass
// Claude's MCP renderer: tool.call returns API content blocks directly. Convert
// here for both fresh and replayed receipts, before native history stores them.
// Native Read has its own file envelope/renderer and must not use this adapter.
function nativeToolContent(result) {
  if (!Array.isArray(result)) return result;
  return result.map(block => {
    if (block.type !== 'image' || block.source) return block;
    return { type: 'image', source: { type: 'base64', media_type: block.mimeType, data: block.data } };
  });
}

// Display receipts are not model results. Never echo media back through the
// 1 MiB HTTP event endpoint after it has already reached native history. The
// character cap also leaves room for JSON's worst-case six-byte escaping.
function toolResultDisplay(content) {
  const text = typeof content === 'string' ? content : JSON.stringify(content, (_key, block) =>
    block && ['image', 'document'].includes(block.type)
      ? { type: block.type, omitted: 'Media retained in native history' } : block) ?? '';
  const limit = 64 * 1024;
  return text.length <= limit ? text : text.slice(0, limit) + '\n[Display truncated; full result retained in native history.]';
}

// Native delegation (one Agent entry). Configured builtins and Locus-
// registered Claude types run as native subagents; the other registered
// types run as Locus subagents through the host. Unknown types are refused,
// never replaced by another type or by an inherited model. Agents run in the
// foreground unless the call explicitly asks for the background.
function agentRoute(delegation, type) {
  if (!delegation || typeof type !== 'string' || !type) return null;
  if (delegation.builtins.includes(type)) return 'native';
  const definition = delegation.definitions.find(item => 'locus:' + item.name === type);
  return definition ? (definition.backend === 'native' ? 'native' : 'locus') : null;
}

function agentInputRefusal(delegation, e) {
  if (!delegation) return 'Agent is not available in this conversation.';
  if (e.isolation) return 'Agent isolation is not supported; subagent tools run in this conversation\'s workspace.';
  if (e.cwd) return 'Agent cwd is not supported; subagent tools run in this conversation\'s workspace.';
  const type = e.subagent_type || 'general-purpose';
  if (!agentRoute(delegation, type)) {
    const choices = [...delegation.definitions.map(item => '`locus:' + item.name + '`'), ...delegation.builtins.map(name => '`' + name + '`')];
    return 'Unknown subagent_type ' + JSON.stringify(type) + '. Available subagent_type values: ' + (choices.join(', ') || 'none') + '.';
  }
  if (type.startsWith('locus:') && e.model) return 'Agent type ' + type + ' always uses its configured model. Omit model, or choose another subagent_type.';
  return null;
}

// Subagents run with the delegating agent's authority but never delegate,
// keep the parent's task list or start background work of their own. They
// may message the main agent only.
function subagentToolRefusal(e) {
  if (e.tool === 'Agent') return 'Claude Code subagents cannot start other agents in Locus.';
  if (e.tool === 'SendMessage' && e.to !== 'main') return 'Claude Code subagents can send messages only to the main agent (to: "main").';
  if (['TodoWrite', 'TaskCreate', 'TaskGet', 'TaskList', 'TaskUpdate', 'TaskOutput', 'TaskStop'].includes(e.tool)) {
    return e.tool + ' is not available to Claude Code subagents. Report progress in your final answer.';
  }
  if (e.tool === 'Bash' && e.run_in_background === true) return 'Background commands are not available to Claude Code subagents. Run the command in the foreground.';
  return null;
}

// Claude owns its agents (addressed by agentId); Locus agents are addressed
// as locus:<task id>. The host answers from its registry, also for agents of
// an earlier Claude process of this conversation.
async function nativeAgentTarget($, id) {
  if (typeof id !== 'string' || !id || id === 'main' || id.startsWith('locus:')) return null;
  const route = await callBridge($, '/event', { protocol: 1, type: 'agent_route', id });
  return route?.native ? route : null;
}

// A subagent's calls share one cancellation scope per model step.
function agentScope(agentSteps, agentId) {
  return 'agent:' + agentId + ':' + (agentSteps.get(agentId) || 0);
}

// A stopped agent's pending Locus call ends now; it is never repeated.
async function releaseAgentCalls($, agentSteps, agentId, runId) {
  if (!agentSteps.has(agentId)) return;
  try { await callBridge($, '/event', { protocol: 1, type: 'turn_interrupted', runId, agentId, id: agentScope(agentSteps, agentId) }); }
  catch { /* The host's stop flag also ends a user-stopped agent's calls. */ }
}

// The user's stop from Locus: the host persisted it first; this runs
// Claude's TaskStop and acknowledges. A repeated command only acknowledges.
async function runAgentStop($, command, control, agentSteps, runId) {
  let ok = true;
  if (!control.done.has(command.id)) {
    control.stopping.add(command.agentId);
    let result;
    try { result = await $.tool.call({ tool: 'TaskStop', task_id: command.agentId }); }
    catch (error) { result = { deny: String(error) }; }
    finally { control.stopping.delete(command.agentId); }
    control.done.add(command.id);
    ok = !result.deny && !result.isError;
    await releaseAgentCalls($, agentSteps, command.agentId, runId);
  }
  await callBridge($, '/event', { protocol: 1, type: 'control_done', id: command.id, agentId: command.agentId, ok });
}

function toolResultEvent(e, runId, output, isError) {
  return { protocol: 1, type: 'tool_result', runId, ...(e.agentId ? { agentId: e.agentId } : {}), id: e.tool_use_id, output, isError };
}

// The transcript keeps a subagent's answer, not the result's protocol data.
function agentResultDisplay(result) {
  const value = result?.result ?? result;
  if (value?.status === 'async_launched' && typeof value.agentId === 'string') {
    return 'Running in the background as agent ' + value.agentId + '. Its result is delivered when it finishes.';
  }
  if (value?.status === 'completed' && Array.isArray(value.content)) {
    const text = value.content.filter(block => block.type === 'text' && typeof block.text === 'string'
      && !/^\s*<tools-used-before-summary>[\s\S]*<\/tools-used-before-summary>\s*$/.test(block.text))
      .map(block => block.text).join('\n');
    if (text.trim()) return toolResultDisplay(text);
  }
  return toolResultDisplay(value);
}

// Definitions are re-registered in every process; the published directory is
// pinned by the host, so a resumed conversation registers the same types.
async function registerAgents($, agents, registeredAgents) {
  if (!agents || !Array.isArray(agents.definitions) || !Array.isArray(agents.builtins)) return null;
  for (const definition of agents.definitions) {
    const id = 'locus:' + definition.name;
    if (registeredAgents.has(id)) continue;
    const registered = await $.agent.register({ name: definition.name, description: definition.description,
      prompt: definition.prompt, tools: definition.tools, model: definition.model });
    if (registered?.agent !== id) throw new Error('Claude registered agent ' + definition.name + ' as ' + String(registered?.agent));
    registeredAgents.add(id);
  }
  return { builtins: agents.builtins, definitions: agents.definitions,
    environment: typeof agents.environment === 'string' ? agents.environment : null };
}

// Correct only NEW attachment rows (see session.append below). Re-rendering
// old prompt.attachment rows on resume would change an already-cached prefix.
function correctDeferredToolNotice(text, toolPlacements) {
  const lines = text.split('\n');
  const output = [];
  for (let index = 0; index < lines.length; index++) {
    const header = lines[index];
    if (!header.startsWith('The following deferred tools are now available via ToolSearch.')) {
      output.push(header);
      continue;
    }
    const entries = [];
    let end = index + 1;
    while (end < lines.length && /^[A-Za-z_][A-Za-z0-9_.-]*$/.test(lines[end])) entries.push(lines[end++]);
    // Unknown future layouts pass through verbatim rather than losing their
    // header or accidentally changing removal/server-status sections.
    if (end < lines.length && lines[end] !== '' && lines[end] !== '</system-reminder>') {
      output.push(header);
      continue;
    }
    const eager = entries.filter(name => toolPlacements.get(name) === false);
    if (!eager.length) { output.push(header); continue; }
    const deferred = entries.filter(name => toolPlacements.get(name) !== false);
    if (deferred.length) output.push(header, ...deferred);
    output.push('Already available directly (no ToolSearch needed): ' + eager.join(', ') + '. Prefer an equivalent native CLI tool when available.');
    index = end - 1;
  }
  return output.join('\n');
}

// session.append exposes synthetic API errors and engine system rows separately
// from model output. Its system projection retains name/content, not level or
// retry counters. Never infer diagnostics by scanning ordinary assistant text.
function nativeDiagnostic(event, content) {
  const text = content.filter(block => block.type === 'text').map(block => block.text).join('\n');
  if (event.origin?.kind === 'model' && event.origin.model === '<synthetic>') {
    return { kind: 'api_error', text };
  }
  if (event.origin?.kind !== 'engine' || event.message.type !== 'system') return null;
  const kind = event.message.name;
  if (kind === 'api_error' || kind === 'api_retry') {
    return { kind: 'api_retry', text: text || 'Claude Code is retrying an API request.' };
  }
  const notices = {
    model_refusal_no_fallback: 'Claude Code refused the request; no fallback was used.',
    model_refusal_fallback: 'Claude Code is using a configured fallback after a model refusal.',
    model_fallback: 'Claude Code switched to a configured fallback model.',
    model_consent_fallback: 'Claude Code reported a model consent/fallback change.',
    instruction_size_warning: 'Claude Code reports that the instruction files exceed its recommended size.',
    permission_denied: 'Claude Code denied permission for an operation.',
    file_attachments_missing: 'Claude Code could not load one or more file attachments.',
    error: 'Claude Code reported an error.',
  };
  if (Object.hasOwn(notices, kind)) return { kind, text: text.trim() ? text : notices[kind] };
  return null;
}

export function register(on) {
  let tui = false;
  let runId = null;
  let settings = {};
  let pendingText = '';
  let pendingThinking = '';
  let pendingTurnId = null;
  let bridgeFailed = false;
  let bridgeState = 'idle';
  let reportingHealth = false;
  const announcedTools = new Set();
  const toolScopes = new Map();
  let stagedRunId = null;
  const stagedInputs = new Set();
  let imageStaging = null;
  let pollingInput = false;
  let currentTurnId = null;
  let executionSequence = 0;
  let executionId = null;
  let insertion = null;
  let insertionExecutionId = null;
  let insertionAppended = false;
  let interruptionReported = false;
  let lastCheckpoint = null;
  let imageCheck = null;
  let appendingPending = null;
  let activity = null;
  let activityStep = 0;
  // Locus context staged for the compaction in progress. Interactive Claude
  // stores compaction rows without session.append, so it is delivered as the
  // compaction's SessionStart additional context instead.
  let compaction = null;
  // Published delegation directory; null keeps Claude's agent hooks inert.
  let delegation = null;
  const registeredAgents = new Set();
  // Agent calls that asked for the background; their spawn may run there.
  const backgroundCalls = new Set();
  // SendMessage/TaskStop/TaskOutput calls routed to Claude's own agents;
  // tool.check allows exactly these.
  const nativeCalls = new Set();
  // Model steps per subagent: the cancellation scope of its calls.
  const agentSteps = new Map();
  // User stops in flight (by command id and by agent) and completed ones.
  const agentControl = { running: new Set(), stopping: new Set(), done: new Set() };
  on('ui.render', { component: 'Spinner' }, async ($, e, next) => {
    if (tui && e.surface === 'terminal' && e.requestId === 'main'
        && ['requesting', 'thinking', 'tool-input', 'tool-use', 'responding'].includes(e.props?.mode)) {
      await reportPhase($, e.props.mode, activity);
    }
    return next(e);
  });
  const registeredTools = new Map();
  const toolPlacements = new Map();
  on('session.start', async ($, e, next) => {
    tui = await $.env.get('LOCUS_CLAUDE_TUI') === '1';
    if (tui) {
      // Restore the pinned catalog before the engine resumes native history,
      // not only when the next user prompt is submitted.
      const options = await callBridge($, '/options', { protocol: 1 });
      await registerTools($, options.tools, registeredTools, toolPlacements);
      delegation = await registerAgents($, options.agents, registeredAgents);
    }
    // A worker replacement must be authorized before it can refill/submit the
    // current input. The host rejects reloads during an active turn.
    const ready = await callBridge($, '/ready', { protocol: 1, ...(tui ? { sessionId: await $.session.id(), model: await $.session.model() } : {}) });
    if (ready.deny) {
      bridgeFailed = true;
      throw new Error(ready.deny);
    }
    if (tui) $.clock.every(150, async () => {
      if (pollingInput || bridgeFailed) return;
      pollingInput = true;
      let options;
      let input;
      try {
      options = await callBridge($, '/options', { protocol: 1, view: 'input' });
      // User stops also arrive while the conversation is idle. TaskStop runs
      // on its own, so input staging never waits for it.
      for (const command of (delegation && options.agentControl?.commands) || []) {
        if (command?.type !== 'stop' || typeof command.id !== 'string' || typeof command.agentId !== 'string'
            || agentControl.running.has(command.id)) continue;
        agentControl.running.add(command.id);
        runAgentStop($, command, agentControl, agentSteps, runId)
          .catch(() => { /* Still pending: the next poll acknowledges it. */ })
          .finally(() => agentControl.running.delete(command.id));
      }
      input = options.insertion || options;
      if (!options.active || !input.inputId) return;
      const key = input.inputId;
      if (stagedRunId !== options.inputId) {
        stagedRunId = options.inputId;
        stagedInputs.clear();
        imageStaging = null;
      }
      // Clearing an insertion must not refill the already submitted main input.
      if (stagedInputs.has(key)) return;
      const imageCount = input.imageCount || 0;
      if (imageCount) {
        if (imageStaging?.key !== key) {
          const cleared = await $.prompt.fill({ text: '', mode: 'replace' });
          if (!cleared.isFilled) return;
          imageStaging = { key, count: -1 };
          return;
        }
        const draft = await $.prompt.read();
        const markers = draft.text.match(/\[Image #\d+\]/g) || [];
        if (markers.length > imageCount || markers.length < imageStaging.count) {
          throw new Error('Claude image attachments changed while preparing the input.');
        }
        // A path still visible as text is not a loaded image. The host waits
        // for a receipt and times out without submitting it as plain text.
        if (draft.text.replace(/\[Image #\d+\]/g, '').trim()) return;
        if (markers.length !== imageStaging.count) {
          imageStaging.count = markers.length;
          await callBridge($, '/event', { protocol: 1, type: 'input_images', runId: options.inputId, id: key, count: markers.length });
        }
        if (markers.length !== imageCount) return;
      }
      // A trailing space closes native @file completion, so Enter submits
      // instead of selecting the highlighted path on Linux terminals.
      // Append preserves the native media markers and their attachment IDs.
      const filled = await $.prompt.fill({ text: (imageCount ? ' ' : '') + (input.terminalText || input.inputId) + ' ', mode: imageCount ? 'append' : 'replace' });
      if (!filled.isFilled) return;
      // Enter can arrive before the SSH receipt returns. Do not refill or
      // announce the same input again while that acknowledgement is in flight.
      stagedInputs.add(key);
      await callBridge($, '/event', { protocol: 1, type: 'input_ready', runId: options.inputId, id: input.inputId });
      } catch (error) {
        if (options?.active && input?.inputId) {
          stagedInputs.add(input.inputId);
          await callBridge($, '/event', { protocol: 1, type: 'input_staging_error', runId: options.inputId, id: input.inputId, error: String(error) });
        }
      } finally { pollingInput = false; }
    });
    if (tui) $.clock.every(2000, async () => {
      if (reportingHealth || !runId || bridgeFailed) return;
      reportingHealth = true;
      try {
        await callBridge($, '/event', { protocol: 1, type: 'bridge_health', runId, state: bridgeState });
      } catch { /* The host's independent lease detects a missing heartbeat. */ }
      finally { reportingHealth = false; }
    });
    return next(e);
  });
  on('prompt.compose', async ($, e, next) => {
    if (!tui) return next(e);
    const options = await callBridge($, '/options', { protocol: 1, view: 'prompt' });
    return { sections: [{ id: 'locus:system', text: options.systemPrompt || 'You are Locus.', scope: 'session' }] };
  });
  // CLI environment is a separate attachment, not part of prompt.compose.
  // Locus already supplies the tool host's OS/shell/workspace in its prompt.
  // Advertising the PTY host here gives the model two conflicting machines.
  on('prompt.attachment', { type: 'environment' }, async ($, e, next) => {
    if (await $.env.get('LOCUS_CLAUDE_TUI') === '1' && e.type === 'environment') {
      // A subagent has no Locus system prompt: tell it where its tools run.
      return { text: e.agentId && delegation?.environment ? delegation.environment : null };
    }
    return next(e);
  }).catch(async ($, e, next) => {
    if (e.type === 'environment') return { text: null };
    if (next.called) return next(e);
    throw new Error('Claude prompt attachment could not be rendered');
  });
  on('prompt.submit', async ($, e, next) => {
    if (!tui) return next(e);
    if (bridgeFailed) return { drop: 'Locus bridge recovery is required.' };
    // A background agent's result: Locus records it on the agent's task and
    // delivers it as a task notification (waking an idle conversation), so
    // Claude's own copy never starts a turn without a Locus input.
    if (delegation && e.origin?.kind === 'task-notification') {
      try { await callBridge($, '/event', { protocol: 1, type: 'agent_notification', text: e.text }); }
      catch { /* A Claude exit still closes the agent's task. */ }
      return { drop: 'Locus delivers agent results.' };
    }
    // Subagents message the main agent through Locus (SendMessage to main).
    if (delegation && e.origin?.kind === 'peer') return { drop: 'Locus delivers messages from other agents.' };
    settings = await callBridge($, '/options', { protocol: 1 });
    if (!settings.active) return { drop: 'The Locus turn is no longer active.' };
    const inputToken = e.text.replace(/\[Image #\d+\]/g, '').trim();
    const input = (settings.insertion?.terminalText || settings.insertion?.inputId) === inputToken ? settings.insertion : settings;
    const inputExecutionId = executionId;
    if (!e.command && inputToken !== (input.terminalText || input.inputId)) return { drop: 'Input must come from the active Locus conversation.' };
    if (!e.command && (e.attachments || []).filter(item => item.type === 'image').length !== (input.imageCount || 0)) {
      await callBridge($, '/event', { protocol: 1, type: 'input_error', runId: settings.inputId, error: 'Claude did not accept every image attachment.' });
      return { drop: 'Claude did not accept every image attachment.' };
    }
    runId = settings.inputId;
    bridgeState = 'running';
    if (input === settings) { announcedTools.clear(); toolScopes.clear(); }
    if (input.expectedImageCount) {
      const messages = await $.session.messages({ as: 'api' });
      imageCheck = { before: messages.reduce((count, message) => count + message.content.filter(block => block.type === 'image').length, 0), expected: input.expectedImageCount };
    }
    if (input !== settings) { insertion = input; insertionExecutionId = inputExecutionId; insertionAppended = false; interruptionReported = false; }
    await registerTools($, settings.tools, registeredTools, toolPlacements);
    if (!e.command && input === settings) {
      for (const prior of settings.pendingUsers || []) {
        // Recovery may find the native append after its receipt was lost.
        // The stable token makes retry idempotent; it is never text matching.
        const marker = `<locus-pending-user id="${prior.token}">\n`;
        if (typeof prior.id !== 'string' || typeof prior.text !== 'string' || !prior.text.startsWith(marker)
            || !/^[0-9a-f-]{36}$/i.test(prior.token)) throw new Error('Invalid pending user input');
        const existing = await $.session.messages({ as: 'api' });
        if (existing.some(message => message.role === 'user' && message.content.some(block => block.type === 'text' && block.text.startsWith(marker)))) continue;
        appendingPending = prior;
        try {
          const appended = await $.session.append({ message: { type: 'user', content: [{ type: 'text', text: prior.text }] } });
          if (appended.deny) throw new Error(appended.deny);
        } finally { appendingPending = null; }
      }
    }
    const result = await next(e.command ? e : { ...e, text: input.prompt });
    if (result.drop) {
      await callBridge($, '/event', { protocol: 1, type: 'input_error', runId, error: result.drop });
      return result;
    }
    if (input !== settings) {
      // Unblock a local tool even when the native turn waits for its result
      // before raising turn.complete. The GUI is sending this queue now.
      await callBridge($, '/event', { protocol: 1, type: 'turn_interrupted', runId, id: inputExecutionId });
      await callBridge($, '/event', { protocol: 1, type: 'input_queued', runId, id: input.submissionId });
    }
    return result;
  }).catch(async ($, e, next) => {
    const error = 'Locus input bridge failed.' + (next.error?.message ? ' ' + next.error.message : '');
    if (tui && runId) {
      try { await callBridge($, '/event', { protocol: 1, type: 'input_error', runId, error }); }
      catch { /* The host's input timeout still fails closed if the bridge is unavailable. */ }
    }
    return { drop: error };
  });
  // Keep a real, low-frequency builtin deferred from the FIRST request, even
  // with zero Skill tools. Otherwise the first Skill adds eager ToolSearch and
  // invalidates the entire prefix. Never toggle placement after discovery.
  on('tool.describe', { tool: 'TaskList' }, async ($, e, next) => {
    const result = await next(e);
    return await $.env.get('LOCUS_CLAUDE_NATIVE_TOOL_SEARCH') === '1'
      ? { ...result, isDeferred: true } : result;
  });
  on('tool.describe', { tool: /^mcp__locus__/ }, async ($, e, next) => ({
    ...(await next(e)), isDeferred: toolPlacements.get(e.tool) === true,
  }));
  // The Agent catalog: configured builtins and the published Locus types.
  // Unknown and internal builtins (fork, worker, ...) stay hidden.
  on('agent.offer', async ($, e, next) => {
    if (!delegation) return next(e);
    if (e.source === 'built-in') return { isOffered: delegation.builtins.includes(e.agent) };
    return { isOffered: e.source === 'plugin' && registeredAgents.has(e.agent) };
  }).catch(async () => ({ isOffered: false }));
  // Backstop behind tool.call routing: only an ordinary subagent of a native
  // type may start, in the background only when its Agent call asked for it.
  // Claude continues a finished agent (SendMessage, or a message queued for
  // it) only if it belongs to this conversation and the user did not stop
  // it. The host learns each run's identity before any of its calls execute.
  on('agent.spawn', async ($, e, next) => {
    if (!delegation) return next(e);
    if (e.fork || e.isTeammate || e.workflow) return { deny: 'Locus runs ordinary Claude subagents only (no fork, teammate or workflow agents).' };
    if (e.parentAgentId) return { deny: 'Claude Code subagents cannot start other agents in Locus.' };
    if (agentRoute(delegation, e.subagentType) !== 'native') {
      return { deny: 'Agent type ' + JSON.stringify(e.subagentType) + ' does not run as a Claude subagent; no agent was started.' };
    }
    if (e.resume) {
      const target = await nativeAgentTarget($, e.resume.agentId);
      if (!target) return { deny: 'Claude tried to continue an agent this conversation did not start; it was not resumed.' };
      if (target.stopped) return { deny: 'The user stopped this agent; it stays stopped in this conversation. Start a new Agent if the work is still needed.' };
    } else if (e.background && !backgroundCalls.has(e.tool_use_id)) {
      return { deny: 'This agent runs in the foreground. Pass run_in_background: true to start it in the background.' };
    }
    const spawnRunId = runId;
    const result = await next(e);
    if (result.agentId) {
      // Registration is handled at the host's ingress, never behind a call.
      // A refused registration (stale turn) leaves the subagent without any
      // executable tool; its next model step is refused.
      await callBridge($, '/event', { protocol: 1, type: 'agent_spawn', runId: spawnRunId,
        agentId: result.agentId, parentToolUseId: e.tool_use_id, name: e.subagentType,
        description: typeof e.description === 'string' ? e.description : '',
        background: e.background === true, resume: !!e.resume });
    }
    return result;
  }).catch(async ($, e, next) => (next.called
    ? { deny: 'Locus could not register the Claude subagent; its tools will not run.' }
    : { deny: 'Locus could not start the Claude subagent.' }));
  on('session.append', async ($, e, next) => {
    if (!tui) return next(e);
    // Since 2.1.299, queue records also pass through this hook. They are
    // transcript previews, not consumed input or native conversation checkpoints.
    if (e.door === 'queue') return next(e);
    const options = await callBridge($, '/options', { protocol: 1, view: 'session' });
    const pendingUser = appendingPending && e.door === 'note' && e.message.role === 'user'
      && e.message.content.some(block => block.type === 'text' && block.text === appendingPending.text) ? appendingPending : null;
    const inserted = insertion && !insertionAppended && e.message.role === 'user' && e.message.content.some(block => block.type === 'text' && block.text.includes(insertion.prompt)) ? insertion : null;
    // Native attachments retain their media blocks. Only their runtime cache
    // paths are replaced with client labels in model-visible text.
    const content = e.message.content.map(block => {
      if (block.type !== 'text') return block;
      let text = block.text;
      if (e.door === 'attachment' && e.message.type === 'attachment' && e.message.name === 'deferred_tools_delta') {
        text = correctDeferredToolNotice(text, toolPlacements);
      }
      for (const file of options.attachmentMappings || []) {
        text = text.split(file.path).join(file.label);
        const windowsPath = file.path.replaceAll('/', '\\');
        text = text.split(JSON.stringify(windowsPath).slice(1, -1)).join(JSON.stringify(file.label).slice(1, -1));
        text = text.split(windowsPath).join(file.label);
      }
      return { ...block, text };
    });
    const result = await next({ ...e, message: { ...e.message, content } });
    if (result.deny) return result;
    const diagnostic = nativeDiagnostic(e, content);
    if (diagnostic) {
      if (runId && options.active && !e.agentId) {
        await callBridge($, '/event', { protocol: 1, type: 'diagnostic', runId, ...diagnostic });
      }
      return result;
    }
    // A subagent's text is display only, on its Agent card (or its task, in
    // the background, also while the conversation is idle). tool.call
    // announces its calls; only the text travels here.
    if (e.agentId && delegation && !bridgeFailed
        && e.message.type === 'assistant' && e.origin?.kind === 'model') {
      const text = content.filter(block => block.type === 'text').map(block => block.text).join('\n');
      if (text.trim()) {
        try {
          await callBridge($, '/event', { protocol: 1, type: 'assistant', runId, agentId: e.agentId,
            message: { content: [{ type: 'text', text }] } });
        } catch { /* display only */ }
      }
    }
    // Native history is authoritative. This second observation closes the
    // display even if tool.call's post-processing/catch lost its receipt. The
    // host deduplicates by tool_use_id and never re-executes an operation here.
    if (runId && options.active && !e.agentId && !bridgeFailed) {
      for (const block of content) {
        if (block.type !== 'tool_result' || !toolScopes.has(block.tool_use_id)) continue;
        const scope = toolScopes.get(block.tool_use_id);
        await callBridge($, '/event', { protocol: 1, type: 'tool_result', runId: scope.runId,
          id: block.tool_use_id, output: toolResultDisplay(!block.is_error && typeof scope.displayOutput === 'string' ? scope.displayOutput : block.content),
          isError: block.is_error === true });
        toolScopes.delete(block.tool_use_id);
      }
    }
    if (!e.agentId) lastCheckpoint = { version: 1, uuid: e.uuid, sessionId: await $.session.id() };
    if (inserted) {
      insertionAppended = true;
      await callBridge($, '/event', { protocol: 1, type: 'input_consumed', runId, id: inserted.submissionId,
        checkpoint: { version: 1, uuid: e.uuid, sessionId: await $.session.id() } });
    }
    const checkpointUser = !!pendingUser || !!inserted || (e.door === 'prompt' && e.message.type === 'user' && !e.message.isMeta);
    const results = content.filter(block => block.type === 'tool_result').map(block => block.tool_use_id);
    // Auxiliary native user rows have no Locus message to bind. Keep them in
    // native storage and lastCheckpoint, without an unused SSH round trip.
    // Consumed mid-turn inputs can be native attachments/meta rows, not a new
    // user turn. Their actual append UUID is still the delivery checkpoint.
    if (runId && options.active && !e.agentId && (inserted || pendingUser || (!e.message.isMeta && ['user', 'assistant'].includes(e.message.type))) && (checkpointUser || e.message.role === 'assistant' || results.length)) {
      await callBridge($, '/event', { protocol: 1, type: 'checkpoint', runId,
        version: 1, uuid: e.uuid, sessionId: await $.session.id(), role: e.message.role,
        user: checkpointUser, submissionId: inserted ? inserted.submissionId : null,
        messageId: pendingUser ? pendingUser.id : (checkpointUser && !inserted ? settings.messageId : null),
        text: e.message.role === 'assistant' ? content.filter(block => block.type === 'text').map(block => block.text).join('') : '',
        tools: content.filter(block => block.type === 'tool_use').map(block => block.id),
        results });
    }
    return result;
  });
  on('turn.start', async ($, e, next) => {
    if (!e.agentId) {
      currentTurnId = e.turnId;
      executionId = `${runId}:${++executionSequence}`;
    }
    if (tui && !e.agentId) await callBridge($, '/event', { protocol: 1, type: 'turn_start', runId, turnId: e.turnId });
    return next(e);
  });
  on('turn.step', async function* ($, e, next) {
    if (!tui) return yield* next(e);
    // Recovery is required, or Claude is idle: model requests need an active Locus input.
    if (bridgeFailed) return await refuseStep($, e);
    // A subagent's request needs its live host registration instead: a
    // background agent keeps working while the conversation is idle, and a
    // stopped one sends nothing more.
    if (e.agentId && delegation) {
      agentSteps.set(e.agentId, (agentSteps.get(e.agentId) || 0) + 1);
      let gate;
      try { gate = await callBridge($, '/event', { protocol: 1, type: 'agent_step', agentId: e.agentId }); }
      catch { return await refuseStep($, e); }
      if (!gate?.ok) return await refuseStep($, e);
      const result = yield* next(e);
      try {
        await callBridge($, '/event', { protocol: 1, type: 'usage', agentId: e.agentId,
          usage: { ...(result?.usage || {}), model: result?.usage?.model || e.model || null, stepKey: e.turnId + ':' + e.index } });
      } catch { /* Billing record only. */ }
      return result;
    }
    let inputState;
    try { inputState = await callBridge($, '/options', { protocol: 1, view: 'input' }); }
    catch { return await refuseStep($, e); }
    if (!inputState.active) return await refuseStep($, e);
    if (e.agentId) return yield* next(e);
    // Claude may absorb input without starting another native turn. Tools
    // produced after that input need a fresh cancellation scope nonetheless.
    if (insertion && insertionAppended) executionId = `${runId}:${++executionSequence}`;
    const stepTurnId = executionId;
    let inferenceStarted = false;
    try {
    if (insertion && insertionAppended) {
      if (pendingText || pendingThinking) {
        await reportInterruption($, pendingTurnId, runId, pendingText, pendingThinking);
        pendingText = ''; pendingThinking = '';
      }
      insertion = null; insertionAppended = false;
    }
    pendingTurnId = stepTurnId;
    if (imageCheck) {
      const messages = await $.session.messages({ as: 'api' });
      const images = messages.reduce((count, message) => count + message.content.filter(block => block.type === 'image').length, 0);
      if (images < imageCheck.before + imageCheck.expected) throw new Error('Claude did not load every image attachment; no model request was sent.');
      imageCheck = null;
    }
    // --model is owned by the GUI selection; a changed selection restarts the
    // TUI with --resume. The engine has already resolved aliases here. Writing
    // settings.model back would send e.g. "opus" directly to the provider.
    const request = { ...e, ...(settings.effort ? { effort: settings.effort } : {}) };
    const stepId = ++activityStep;
    const stepRunId = runId;
    activity = null;
    await callBridge($, '/event', { protocol: 1, type: 'step_start', runId, stepId, model: request.model });
    activity = { runId, stepId, phase: 'requesting', sequence: 0 };
    const timing = { version: 1, source: 'claude-turn-step', requestStartedAtMs: await $.clock.now(), firstOutputAtMs: null, responseStoppedAtMs: null, resultReadyAtMs: null, bridgeDurationMs: 0, consumerPauseMs: 0, outputChunks: 0 };
    inferenceStarted = true;
    const stream = next(request);
    const tools = [];
    let thinking = '';
    for await (const chunk of stream) {
      if (timing.firstOutputAtMs === null && ['text', 'thinking', 'tool'].includes(chunk.kind)) timing.firstOutputAtMs = await $.clock.now();
      // Stop belongs to this model response. stream.result / turn.complete can
      // arrive after a tool has finished, so neither is the output clock's end.
      if (chunk.kind === 'stop') timing.responseStoppedAtMs = await $.clock.now();
      if (chunk.kind === 'text' || chunk.kind === 'thinking') {
        const beforeBridge = await $.clock.now();
        await reportPhase($, chunk.kind === 'text' ? 'responding' : 'thinking', activity);
        // Hidden thinking may report progress without a text payload.
        if (typeof chunk.text === 'string') {
          timing.outputChunks++;
          if (chunk.kind === 'text') pendingText += chunk.text;
          else pendingThinking += chunk.text;
          if (chunk.kind === 'thinking') thinking += chunk.text;
          await callBridge($, '/event', { protocol: 1, type: chunk.kind, runId, text: chunk.text });
        }
        timing.bridgeDurationMs += Math.max(0, (await $.clock.now()) - beforeBridge);
      } else if (chunk.kind === 'tool') {
        const beforeBridge = await $.clock.now();
        await reportPhase($, 'tool-input', activity);
        tools.push({ id: chunk.id, name: chunk.name });
        await callBridge($, '/event', { protocol: 1, type: 'tool_start', runId, id: chunk.id, name: chunk.name });
        timing.bridgeDurationMs += Math.max(0, (await $.clock.now()) - beforeBridge);
      }
      const beforeYield = timing.responseStoppedAtMs === null ? await $.clock.now() : null;
      yield chunk;
      if (beforeYield !== null) timing.consumerPauseMs += Math.max(0, (await $.clock.now()) - beforeYield);
    }
    const result = await stream.result;
    timing.resultReadyAtMs = await $.clock.now();
    const context = (await $.session.usage()).context;
    await callBridge($, '/event', { protocol: 1, type: 'usage', runId: stepRunId, stepId, usage: result.usage, context, timing });
    const content = [];
    if (result.answer) content.push({ type: 'text', text: result.answer });
    if (thinking) content.push({ type: 'thinking', thinking });
    for (let i = 0; i < result.toolUses.length; i++) {
      const tool = result.toolUses[i];
      if (!tools[i]) throw new Error('Missing native tool ID');
      content.push({ type: 'tool_use', id: tools[i].id, name: tool.name, input: tool.input });
    }
    // Native TUI starts tools while the model stream is still open. Their
    // authoritative envelopes were persisted by tool.call before execution.
    if (!result.toolUses.length) {
      await callBridge($, '/event', { protocol: 1, type: insertion ? 'interrupted_assistant' : 'assistant', runId, message: { content } });
      pendingText = ''; pendingThinking = '';
    }
    return result;
    } catch (error) {
      activity = null;
      try {
        if (insertion) {
          if (!interruptionReported) {
            interruptionReported = true;
            await reportInterruption($, stepTurnId, runId, pendingText, pendingThinking);
            pendingText = ''; pendingThinking = '';
          }
        } else {
          bridgeState = 'complete';
          await callBridge($, '/event', { protocol: 1, type: 'complete', runId, reason: 'error', answer: String(error) });
        }
      } catch (reportError) {
        if (inferenceStarted) throw reportError;
      }
      // Before inference a rethrow would let Claude send the request anyway.
      if (!inferenceStarted) return await refuseStep($, e);
      throw error;
    } finally {
      if (insertion) activity = null;
      // Native queue interruption can close a generator with return(), which
      // bypasses catch and has no stream.result. Keep already-visible output.
      if (insertion && (pendingText || pendingThinking)) {
        await reportInterruption($, stepTurnId, runId, pendingText, pendingThinking);
        pendingText = ''; pendingThinking = ''; interruptionReported = true;
      }
    }
  });
  on('turn.complete', async ($, e, next) => {
    // A subagent's run ended. Its result reaches Locus as its Agent result
    // (foreground) or its task notification (background).
    if (tui && e.agentId && delegation) {
      try { await callBridge($, '/event', { protocol: 1, type: 'agent_finished', agentId: e.agentId }); }
      catch { /* Its notification and Claude's exit still settle the run. */ }
      return next(e);
    }
    if (tui && !e.agentId && insertion) {
      if (e.isAborted && !interruptionReported) {
        interruptionReported = true;
        await reportInterruption($, insertionExecutionId, runId, pendingText, pendingThinking);
        pendingText = ''; pendingThinking = '';
      }
      return next(e);
    }
    if (tui && !e.agentId) {
      activity = null;
      bridgeState = 'complete';
      await callBridge($, '/event', { protocol: 1, type: 'complete', runId, ...e });
    }
    return next(e);
  }).catch(async ($, e, next) => {
    if (tui && !e.agentId && !insertion && runId) {
      bridgeState = 'complete';
      try {
        await callBridge($, '/event', { protocol: 1, type: 'bridge_fault', runId,
          error: 'Claude completion could not be delivered. Resume from native history.' });
      } catch { /* Terminal heartbeat bounds the wait even if this receipt fails. */ }
    }
    return next(e);
  });
  on('classic.SessionStart', async ($, e, next) => {
    const base = await next(e);
    if (!tui || e.source !== 'compact' || e.agent_id || !compaction || compaction.delivered) return base;
    compaction.delivered = true;
    return { ...base, additionalContext: [...(base?.additionalContext || []), compaction.carryForward] };
  });
  on('session.compact', async ($, e, next) => {
    if (!tui || e.agentId || e.trigger === 'precompute') return next(e);
    await callBridge($, '/event', { protocol: 1, type: 'compact_start', runId, trigger: e.trigger });
    // Locus copies the transcript to the client and captures what repeated
    // summaries lose (user messages, tasks, files read through Locus) while
    // the conversation is still intact. Without it Claude compacts natively.
    let prepared = null;
    try {
      prepared = await callBridge($, '/event', { protocol: 1, type: 'compact_prepare', runId, trigger: e.trigger });
    } catch (error) {
      try {
        await callBridge($, '/event', { protocol: 1, type: 'diagnostic', runId, kind: 'compaction_context',
          text: 'Locus could not prepare compaction context; Claude compacted without it. ' + String(error) });
      } catch { /* The compaction itself must not depend on this notice. */ }
    }
    const prepareNotice = prepared?.ok === false
      ? 'Locus could not prepare compaction context; Claude compacted with its own summary only. ' + String(prepared.error ?? '')
      : prepared?.transcriptError
        ? 'Locus could not save a local copy of the Claude transcript; the summary keeps the path on the Claude host. ' + String(prepared.transcriptError)
        : null;
    if (prepareNotice) {
      try {
        await callBridge($, '/event', { protocol: 1, type: 'diagnostic', runId, kind: 'compaction_context', text: prepareNotice.trim() });
      } catch { /* The compaction itself must not depend on this notice. */ }
    }
    const instructions = [e.instructions, prepared?.instructions]
      .filter(text => typeof text === 'string' && text.trim()).join('\n\n');
    // Claude runs SessionStart (source "compact") inside the compaction and
    // stores its additional context after the summary and kept messages.
    compaction = prepared?.carryForward
      ? { transcriptPath: prepared.transcriptPath || null, carryForward: prepared.carryForward, delivered: false } : null;
    const staged = compaction;
    let result;
    try {
      result = await next(instructions ? { ...e, instructions } : e);
    } finally {
      compaction = null;
    }
    if (result.skip) {
      await callBridge($, '/event', { protocol: 1, type: 'input_error', runId, error: result.skip });
      return result;
    }
    const carryForward = staged?.delivered ? staged.carryForward : null;
    if (staged && !staged.delivered) {
      try {
        await callBridge($, '/event', { protocol: 1, type: 'diagnostic', runId, kind: 'compaction_context',
          text: 'Claude did not run the post-compaction SessionStart hook; the Locus carry-forward context was not added.' });
      } catch { /* The compaction itself must not depend on this notice. */ }
    }
    const context = (await $.session.usage()).context;
    // Claude's tokensAfter excludes the system prompt and tools.
    let estimate = null;
    try { estimate = (await $.session.usage({ breakdown: 'summary' })).context?.breakdown?.totalTokens ?? null; }
    catch { /* Display estimate only. */ }
    const [summaryMessage, ...keptMessages] = result.messages;
    // The summary stays exactly as Claude stored it; Locus's additions travel
    // separately as the SessionStart context.
    const summary = summaryMessage?.text || '';
    // Kept rows are the most recent messages, verbatim after the summary.
    // Tool ids anchor them to Locus rows; text is a bounded fallback.
    const kept = keptMessages.map(message => ({ role: message.role, text: (message.text || '').slice(0, 4000),
      toolUseIds: (message.toolUses || []).map(use => use.tool_use_id),
      toolResultIds: (message.toolResults || []).map(item => item.tool_use_id) }));
    await callBridge($, '/event', { protocol: 1, type: 'compact_done', runId,
      before: e.messages.length, after: result.messages.length, tokens: result.tokensAfter, estimate, context,
      summary, carryForward, kept, transcriptPath: staged?.transcriptPath ?? null, checkpoint: lastCheckpoint });
    return result;
  }).catch(async ($, e, next) => {
    compaction = null;
    const detail = next.error?.message;
    await callBridge($, '/event', { protocol: 1, type: 'input_error', runId, error: 'Locus could not record Claude compaction.' + (detail ? ' ' + detail : '') });
    return next.called ? next(e) : { skip: 'Locus could not record Claude compaction.' };
  });
  on('command.run', async ($, e, next) => {
    if (!tui) return next(e);
    settings = await callBridge($, '/options', { protocol: 1 });
    const commandRunId = settings.inputId;
    runId = commandRunId;
    bridgeState = 'running';
    await callBridge($, '/event', { protocol: 1, type: 'command_start', runId: commandRunId });
    // Only the GUI's explicit compact action may run a native command. Model
    // selection and diagnostics never become chat input or assistant output.
    if (!settings.active || settings.control !== 'compact' || e.command !== 'compact') {
      await callBridge($, '/event', { protocol: 1, type: 'input_error', runId: commandRunId, error: 'Use the corresponding Locus control for Claude commands.' });
      return { text: '' };
    }
    const result = await next(e);
    bridgeState = 'complete';
    await callBridge($, '/event', { protocol: 1, type: 'command', runId: commandRunId });
    return result;
  }).catch(async ($, e, next) => {
    const detail = next.error?.message;
    await callBridge($, '/event', { protocol: 1, type: 'input_error', runId,
      error: 'Claude command failed: /' + e.command + (detail ? '. ' + detail : '') });
    return { text: 'Locus 命令桥接失败，未回退执行命令。' };
  });
  on('tool.call', async ($, e, next) => {
    // A background agent's calls are not part of the parent's compaction
    // (settings keeps the last control while the conversation is idle).
    const subagent = !!(delegation && e.agentId);
    if (tui && settings.control === 'compact' && !subagent) {
      return { deny: 'Tool execution is disabled during Locus compaction.' };
    }
    const callTurnId = currentTurnId;
    const callExecutionId = subagent ? agentScope(agentSteps, e.agentId) : executionId;
    const callRunId = runId;
    toolScopes.set(e.tool_use_id, { runId: callRunId, executionId: callExecutionId, agentId: subagent ? e.agentId : null, dispatched: false });
    if (bridgeFailed) return { deny: 'Locus bridge recovery is required.' };
    if (next.signal?.aborted) throw new Error('Claude tool dispatch was interrupted before execution');
    // Native tools retain Claude's concurrency. Bridged tools are serialized by
    // the host, where waiting for execution cannot exhaust a Mod hook's budget.
    if (tui) {
      if (!e.agentId) await reportPhase($, 'tool-use', activity);
      const { tool, tool_use_id, agentId, ...input } = e;
      const content = [];
      // A subagent's call belongs to its Agent card, never to the parent's
      // streamed text or assistant round.
      if (!agentId) {
        if (pendingText) content.push({ type: 'text', text: pendingText });
        if (pendingThinking) content.push({ type: 'thinking', thinking: pendingThinking });
        pendingText = ''; pendingThinking = '';
      }
      content.push({ type: 'tool_use', id: tool_use_id, name: tool, input });
      await callBridge($, '/event', { protocol: 1, type: 'assistant', runId: callRunId, ...(agentId ? { agentId } : {}), message: { content } });
      announcedTools.add(tool_use_id);
    }
    if (next.signal?.aborted) throw new Error('Claude tool dispatch was interrupted before execution');
    const subagentRefusal = e.agentId ? subagentToolRefusal(e) : null;
    if (subagentRefusal) {
      if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, subagentRefusal, true));
      return { deny: subagentRefusal };
    }
    if (!e.agentId && e.tool === 'SendMessage' && !delegation) {
      const deny = 'SendMessage is not available in this conversation.';
      if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, deny, true));
      return { deny };
    }
    // The main agent's messages, output reads and stops for Claude's own
    // agents run in Claude. Locus agents (locus:<task id>) and Locus tasks
    // stay with the host below.
    if (!e.agentId && delegation && ['SendMessage', 'TaskStop', 'TaskOutput'].includes(e.tool)) {
      const target = e.tool === 'SendMessage' ? e.to : e.task_id;
      const toLocus = typeof target === 'string' && target.startsWith('locus:');
      const native = toLocus ? null : await nativeAgentTarget($, target);
      let refusal = null;
      if (e.tool === 'SendMessage' && !toLocus) {
        if (target === 'main') refusal = 'You are the main agent. Address a subagent by the agentId its Agent call returned.';
        else if (!native) refusal = 'No agent ' + JSON.stringify(target) + ' belongs to this conversation. Use the agentId an Agent call returned, or locus:<task id> for a Locus agent.';
        else if (native.stopped) refusal = 'The user stopped agent ' + target + '; it stays stopped in this conversation. Start a new Agent if the work is still needed.';
      }
      if (refusal) {
        if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, refusal, true));
        return { deny: refusal };
      }
      if (native) {
        // Never retried: a second next() could deliver a message twice.
        nativeCalls.add(e.tool_use_id);
        let result;
        try {
          result = await next(e);
        } catch (error) {
          const deny = next.signal?.aborted ? 'The call was interrupted.' : e.tool + ' failed: ' + String(error);
          if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, deny, true));
          return { deny };
        } finally { nativeCalls.delete(e.tool_use_id); }
        if (e.tool === 'TaskStop' && !result.deny && !result.isError) await releaseAgentCalls($, agentSteps, target, callRunId);
        if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, toolResultDisplay(result.deny ?? result.result ?? result), !!(result.deny || result.isError)));
        return result;
      }
    }
    let agentToLocus = false;
    if (!e.agentId && e.tool === 'Agent') {
      const refusal = agentInputRefusal(delegation, e);
      if (refusal) {
        if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, refusal, true));
        return { deny: refusal };
      }
      if (agentRoute(delegation, e.subagent_type || 'general-purpose') === 'native') {
        // Claude runs it, in the foreground unless the call asks for the
        // background. Never retried: a second next() could start another agent.
        const background = e.run_in_background === true;
        if (background) backgroundCalls.add(e.tool_use_id);
        let result;
        try {
          result = await next({ ...e, run_in_background: background });
        } catch (error) {
          const deny = next.signal?.aborted ? 'The agent was interrupted.' : 'The agent failed: ' + String(error);
          if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, deny, true));
          return { deny };
        } finally { backgroundCalls.delete(e.tool_use_id); }
        if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, agentResultDisplay(result), !!(result.deny || result.isError)));
        return result;
      }
      agentToLocus = true;
    }
    const locusTool = e.tool.startsWith('mcp__locus__');
    // Session bookkeeping and web tools retain Claude's implementation.
    // Search/fetch requests belong to the CLI, never to Locus.
    if (['TodoWrite',
      'TaskCreate', 'TaskGet', 'TaskList', 'TaskUpdate', 'ToolSearch', 'WebSearch', 'WebFetch'].includes(e.tool)) {
      let result;
      try {
        result = await next(e);
      } catch (error) {
        if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, String(error), true));
        throw error;
      }
      let observed = result;
      if (tui && !result.deny && !result.isError) {
        try {
        let snapshot;
        if (e.tool === 'TodoWrite') {
          snapshot = { version: 1, kind: 'todos', items: e.todos.map(todo => ({ content: todo.content, status: todo.status, priority: todo.priority || 'medium' })) };
        } else if (['TaskCreate', 'TaskUpdate', 'TaskList'].includes(e.tool)) {
          const listed = e.tool === 'TaskList' ? result : await $.tool.call({ tool: 'TaskList' });
          const listedTasks = listed.result?.tasks || listed.result;
          if (listed.deny || listed.isError || !Array.isArray(listedTasks)) {
            await callBridge($, '/event', { protocol: 1, type: 'input_error', runId, error: 'Claude updated its tasks but could not synchronize the task list. Resume to inspect the saved tasks; do not repeat the mutation.' });
            return result;
          }
          const tasks = [...listedTasks];
          for (let index = 0; index < tasks.length; index++) {
            const detail = await $.tool.call({ tool: 'TaskGet', taskId: tasks[index].id });
            if (detail.deny || detail.isError || !(detail.result?.task || detail.result)?.id) throw new Error('Claude task detail could not be synchronized');
            tasks[index] = detail.result.task || detail.result;
          }
          snapshot = { version: 1, kind: 'tasks', tasks, items: tasks.filter(task => task.status !== 'deleted').map(task => ({ content: task.subject, status: task.status, priority: 'medium' })) };
        }
        if (snapshot) observed = { ...result, locusTodoSnapshot: snapshot };
        } catch (error) {
          await callBridge($, '/event', { protocol: 1, type: 'input_error', runId, error: 'Claude task synchronization failed after native execution: ' + String(error) + '. Inspect the tasks before retrying.' });
          return result;
        }
      }
      if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, JSON.stringify(observed), !!(result.deny || result.isError)));
      return result;
    }
    if (e.tool === 'mcp__locus__web_fetch') {
      const deny = 'Use Claude Code WebFetch. Locus web_fetch is unavailable in Claude Code sessions.';
      if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, deny, true));
      return { deny };
    }
    // A Locus-routed Agent runs in the host as a Locus subagent. SendMessage
    // reaching here addresses a Locus agent, or (from a subagent) the main one.
    if (!locusTool && !agentToLocus && !['Read', 'Write', 'Edit', 'Bash', 'Glob', 'Grep', 'TaskOutput', 'TaskStop', 'AskUserQuestion',
      ...(delegation ? ['SendMessage'] : [])].includes(e.tool)) {
      const deny = 'Locus has not adapted ' + e.tool + ' for local execution yet.';
      if (tui) await callBridge($, '/event', toolResultEvent(e, callRunId, deny, true));
      return { deny };
    }
    // Do not call next for a bridged tool, including on failure. Retrying a
    // transport failure could run the same write twice, so never retry here.
    const { tool, tool_use_id, agentId, ...input } = e;
    toolScopes.get(tool_use_id).dispatched = true;
    const answer = await callBridge($, '/call', {
      protocol: 1, tool, callId: tool_use_id, agentId: agentId ?? null, input, ...(tui ? { runId: callRunId, turnId: callTurnId, executionId: callExecutionId } : {}),
    });
    if (answer.callId !== tool_use_id) throw new Error('Locus tool call ID mismatch');
    if (answer.deny) return { deny: answer.deny };
    if (!Object.hasOwn(answer, 'result')) throw new Error('Locus tool result is missing');
    const scope = toolScopes.get(tool_use_id);
    if (scope && typeof answer.locusOutput === 'string') scope.displayOutput = toolResultDisplay(answer.locusOutput);
    // Registration must settle before returning the result: the engine then
    // appends its deferred-tools delta and handles ToolSearch/tool_reference.
    // Schemas stay in bridge metadata, never in result text or system prompts.
    await registerTools($, answer.toolCatalogDelta, registeredTools, toolPlacements);
    return { result: locusTool ? nativeToolContent(answer.result) : answer.result, ...(answer.context ? { context: answer.context } : {}) };
  }).catch(async ($, e, next) => {
    // This Mod's own calls: task synchronization and the user's agent stops.
    if (next.error?.kind === 're-entry' && ['TaskList', 'TaskGet', 'TaskStop'].includes(e.tool)) return next(e);
    // Replay a completed native result, never execute a bridged write again.
    const scope = toolScopes.get(e.tool_use_id) || { runId, executionId };
    let result;
    if (next.called && ['Agent', 'SendMessage'].includes(e.tool)) {
      // Never start a second subagent, or deliver a message twice.
      result = { deny: e.tool === 'Agent' ? 'The agent ended without a result; it was not run again.' : 'The message outcome is unknown; it was not sent again.' };
    } else if (next.called) {
      try { result = await next(e); }
      catch (error) { result = { deny: String(error) }; }
    } else {
      result = { deny: next.signal?.aborted
        ? (scope.dispatched
          ? 'Locus tool dispatch was interrupted; execution outcome may be unknown. Verify local state before retrying.'
          : 'Claude tool dispatch was interrupted before execution.')
        : 'Locus tool bridge failed; no fallback tool was run. Verify local state before retrying a write.' };
    }
    if (tui && !bridgeFailed && (scope?.runId || scope?.agentId)) {
      try {
        // These receipts bypass the ordered tool lane. Send them FIRST: a
        // tool_result behind a pending /call cannot unblock that same call.
        // A subagent's interruption ends only its own scope.
        if (!next.called && next.signal?.aborted && scope.executionId) {
          await callBridge($, '/event', { protocol: 1, type: 'turn_interrupted', runId: scope.runId,
            ...(scope.agentId ? { agentId: scope.agentId } : {}), id: scope.executionId });
        }
        if (!next.called && (!next.signal?.aborted || next.error?.kind === 'timeout')) {
          await callBridge($, '/event', { protocol: 1, type: 'bridge_fault', runId: scope.runId,
            error: 'Claude tool hook failed. Resume from native history; no tool will be automatically repeated.' });
          bridgeFailed = true;
        }
        if (!announcedTools.has(e.tool_use_id)) {
          const { tool, tool_use_id, agentId, ...input } = e;
          await callBridge($, '/event', { protocol: 1, type: 'assistant', runId: scope.runId, ...(agentId ? { agentId } : {}),
            message: { content: [{ type: 'tool_use', id: tool_use_id, name: tool, input }] } });
          announcedTools.add(tool_use_id);
        }
        await callBridge($, '/event', toolResultEvent(e, scope.runId, JSON.stringify(result), !!(result.deny || result.isError)));
      } catch { bridgeFailed = true; /* Host heartbeat lease will fail closed. */ }
    }
    return result;
  });

  // Independent final guard: if the forwarding hook is skipped, native
  // filesystem/process tools must not execute on the Claude host.
  on('tool.check', async ($, e, next) => {
    if (bridgeFailed) return { decision: 'deny', reason: 'Locus bridge recovery is required.' };
    // tool.check names no agent; tool.call recorded the call's scope.
    const subagent = !!(e.tool_use_id && toolScopes.get(e.tool_use_id)?.agentId);
    if (tui && settings.control === 'compact' && !subagent) {
      return { decision: 'deny', reason: 'Tool execution is disabled during Locus compaction.' };
    }
    // Only tool.call's native route reaches the engine with an Agent call.
    // Unlike tool.call, tool.check nests the call's arguments in { input }.
    // Locus decides here: auto mode ignores the CLI's Agent allow rule and
    // would classify or prompt in a TUI nobody answers. Starting the agent
    // has no effect of its own; each of its calls runs through Locus.
    if (e.tool === 'Agent' && !e.agentId) {
      const input = e.input || {};
      return agentInputRefusal(delegation, input) || agentRoute(delegation, input.subagent_type || 'general-purpose') !== 'native'
        ? { decision: 'deny', reason: 'This agent must start through the Locus delegation route.' } : { decision: 'allow' };
    }
    // Exactly the calls tool.call routed to Claude's own agents, and the
    // user's stops this Mod runs (they carry no tool_use_id of the model).
    if (['SendMessage', 'TaskStop', 'TaskOutput'].includes(e.tool)) {
      const routed = (e.tool_use_id && nativeCalls.has(e.tool_use_id))
        || (e.tool === 'TaskStop' && agentControl.stopping.has(e.input?.task_id));
      return routed ? { decision: 'allow' } : { decision: 'deny', reason: 'This tool must execute through the Locus bridge.' };
    }
    if (['TodoWrite',
        'TaskCreate', 'TaskGet', 'TaskList', 'TaskUpdate', 'ToolSearch', 'WebSearch', 'WebFetch'].includes(e.tool)) {
      const result = await next(e);
      if (tui && settings.active && runId && !e.agentId && result.decision === 'deny') {
        await callBridge($, '/event', { protocol: 1, type: 'diagnostic', runId,
          kind: 'tool_denied', text: e.tool + ': ' + (result.reason || 'Claude Code denied this tool operation.') });
      }
      return result;
    }
    return { decision: 'deny', reason: 'This tool must execute through the Locus bridge.' };
  }).catch(async () => ({ decision: 'deny', reason: 'Locus tool guard failed.' }));
}
