import { CdpClient, findLocusWebViewTarget, sleep } from "./locus-webview2-stress-client";

// Run against a fresh isolated locus:test:app instance before opening Knowledge:
// bun run scripts/locus-skill-startup-cdp-test.ts --browser-url <url> --workspace <isolated-workspace>
const args = process.argv.slice(2);
const browserUrl = args[args.indexOf("--browser-url") + 1];
const workspaceRoot = args[args.indexOf("--workspace") + 1];
if (!args.includes("--browser-url") || !args.includes("--workspace") || !browserUrl || !workspaceRoot) {
  throw new Error("Supply --browser-url and --workspace for the isolated test instance.");
}
await fetch(`${browserUrl.replace(/\/$/, "")}/json/version`).then(response => {
  if (!response.ok) throw new Error("DevTools endpoint unavailable");
});
const target = await findLocusWebViewTarget(browserUrl, 30_000);
const client = await CdpClient.connect(target.webSocketDebuggerUrl!);
const expectedRoot = workspaceRoot.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
const inspect = `(() => {
  const app = document.querySelector('#app')?.__vue_app__;
  if (!app?._instance) return null;
  const components = [];
  function visit(node) {
    if (!node) return;
    if (node.component) { components.push(node.component); visit(node.component.subTree); }
    else if (Array.isArray(node.children)) node.children.forEach(visit);
  }
  visit(app._instance.subTree);
  const stores = app.config.globalProperties.$pinia._s;
  const workspace = stores.get('workspaceContext');
  const composer = components.find(c => c.type.__name === 'RichChatInput');
  return JSON.parse(JSON.stringify({
    root: workspace?.focusedRoot, windowId: workspace?.windowId, workspaceRef: workspace?.focusedWorkspaceRef,
    knowledgeMounted: components.some(c => c.type.__name === 'KnowledgeView'),
    poolMounted: components.some(c => c.type.__name === 'WorkbenchWindow' && c.props.sharedHost?.pooled),
    manifestCount: composer?.props.skills?.length ?? 0,
    agentCheckoutId: stores.get('agent')?.workspaceCheckoutId,
    startupError: app._instance.setupState.mainBootstrapError,
  }));
})()`;

try {
  let state: any;
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    state = await client.evaluate(inspect);
    if (state?.startupError) throw new Error(state.startupError);
    if (state?.poolMounted && state.manifestCount > 0) break;
    await sleep(250);
  }
  if (!state?.poolMounted) throw new Error("Shared window pool did not finish prewarming");
  if (state.knowledgeMounted) throw new Error("Knowledge was already mounted; this is not a cold-start skill check");
  if (state.windowId !== "main" || !state.workspaceRef) throw new Error("Prewarming stole the main workspace focus");
  if (state.root.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase() !== expectedRoot) {
    throw new Error("The connected instance is not bound to the supplied test workspace");
  }
  if (state.agentCheckoutId !== state.workspaceRef.checkoutId) throw new Error("Scoped Agents did not load");

  const popup = await client.evaluate(`(async () => {
    const input = document.querySelector('textarea.chat-composer-input');
    if (!input) throw new Error('Composer not found');
    const original = input.value;
    const start = input.selectionStart, end = input.selectionEnd;
    const focused = document.activeElement;
    function update(value, from, to) {
      input.value = value; input.setSelectionRange(from, to);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }
    try {
      input.focus(); update('/', 1, 1);
      await new Promise(resolve => setTimeout(resolve, 150));
      let component = input.__vueParentComponent;
      while (component && component.type.__name !== 'RichChatInput') component = component.parent;
      if (!component) throw new Error('RichChatInput owner not found');
      const commands = component.setupState.filteredCommands;
      if (!component.setupState.showCommandPopup || !commands.some(c => c.commandType === 'skill')) {
        throw new Error('Slash popup contains no skill commands');
      }
      return { visible: true, commands: commands.map(c => c.name), skillCommands: commands.filter(c => c.commandType === 'skill').length };
    } finally {
      update(original, start, end);
      if (focused instanceof HTMLElement) focused.focus();
    }
  })()`);
  console.log(`LOCUS_SKILL_STARTUP_JSON ${JSON.stringify({ ...state, popup, passed: true })}`);
} finally {
  client.close();
}
