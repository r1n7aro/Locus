import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8").replace(/\r\n?/g, "\n");
}

describe("Locus Python API", () => {
  it("bundles the package as locus and injects the local bridge into Python runtimes", () => {
    const baseConfig = read("src-tauri/tauri.conf.json");
    const embeddedConfig = read("src-tauri/tauri.with_embed_python_git.conf.json");
    const externalConfig = read("src-tauri/tauri.without_embed_python_git.conf.json");
    const runtime = read("src-tauri/src/python_runtime.rs");
    const skill = read("src-tauri/src/commands/skill.rs");

    for (const config of [baseConfig, embeddedConfig, externalConfig]) {
      expect(config).toContain('"../python/locus": "locus-python-sdk/locus/"');
    }
    expect(runtime).toContain('const LOCUS_SDK_RESOURCE_DIR: &str = "locus-python-sdk"');
    expect(runtime).toContain('("LOCUS_SDK_URL".to_string(), connection.url)');
    expect(runtime).toContain('("LOCUS_SDK_TOKEN".to_string(), connection.token)');
    expect(runtime).toContain("if let Some(sdk_dir) = runtime.sdk_dir.as_ref()");
    expect(skill).toContain("managed_python_path_env(");
  });

  it("keeps Python agent definitions inline and reuses the Locus session by default", () => {
    const models = read("python/locus/_models.py");
    const client = read("python/locus/_client.py");
    const bridge = read("src-tauri/src/sdk.rs");
    const session = read("src-tauri/src/commands/session.rs");

    expect(models).toContain("effective_session = None if new_session else (session_id or self.session_id)");
    expect(models).toContain("self.session_id = run.session_id");
    expect(models).toContain("callback_key = self._callback_keys[builtins.id(binding)]");
    expect(client).toContain('"agentSpec": agent_spec');
    expect(bridge).toContain('"agents.prompt" => prompt_agent');
    expect(bridge).not.toContain('"agents.define"');
    expect(session).toContain("Python-defined agents resend the full");
    expect(session).toContain("provider conversation/prompt cache remains reusable");
    expect(session).toContain("tool_registry_for_agent(effective_tool_registry.as_ref(), spec)");
  });

  it("supports Locus tools and loopback Python callable tools on one agent", () => {
    const tools = read("python/locus/_tools.py");
    const callbacks = read("python/locus/_callbacks.py");
    const bridge = read("src-tauri/src/sdk.rs");

    expect(tools).toContain("get_type_hints(function, include_extras=True)");
    expect(callbacks).toContain('ThreadingHTTPServer(("127.0.0.1", 0), Handler)');
    expect(callbacks).toContain("asyncio.run_coroutine_threadsafe(");
    expect(bridge).toContain("spec.locus_tools = canonical_tool_names(app, spec.locus_tools, workspace_ref).await?");
    expect(bridge).toContain("Python tool callback URL must target loopback");
    expect(bridge).toContain("registry.register_runtime(");
  });

  it("covers model discovery, direct tools, workspace state, sessions, and run streams", () => {
    const api = read("python/locus/__init__.py");
    const client = read("python/locus/_client.py");
    const models = read("python/locus/_models.py");
    const bridge = read("src-tauri/src/sdk.rs");
    const example = read("python/examples/custom_workflow.py");

    for (const method of [
      '"models.list"',
      '"tools.call"',
      '"workspace.get"',
      '"unity.editor.status"',
      '"unity.editor.ensure"',
      '"unity.editor.restart"',
      '"sessions.list"',
      '"sessions.get"',
      '"sessions.send"',
      '"sessions.events"',
    ]) {
      expect(bridge).toContain(method);
    }
    expect(api).toContain("async def list_models(");
    expect(api).toContain("async def call_tool(");
    expect(api).toContain("async def get_session(");
    expect(api).toContain("async def list_running_sessions(");
    expect(api).toContain("async def send_session_message(");
    expect(api).toContain("async def get_unity_editor_status(");
    expect(api).toContain("async def ensure_unity_editor(");
    expect(api).toContain("async def restart_unity_editor(");
    expect(client).toContain("async def get_workspace(");
    expect(client).toContain('"workspaceRef": None if workspace_ref is None else workspace_ref.to_payload()');
    expect(client).toContain("wait_until not in {\"process\", \"connected\", \"ready\"}");
    expect(client).toContain('mode not in {"interactive", "headless"}');
    expect(models).toContain("class WorkspaceRef:");
    expect(models).toContain("class UnityEditorStatus:");
    expect(models).toContain("main_thread_blocked: bool");
    expect(models).toContain("blocking_dialog: UnityModalDialog | None");
    expect(models).toContain("class UnityEditorEnsureResult:");
    expect(models).toContain("class UnityEditorRestartResult:");
    expect(models).toContain("class SessionMessageDelivery:");
    expect(models).toContain("async def event_stream(");
    expect(models).toContain("def raise_for_error(self)");
    expect(example).toContain("await asyncio.gather(");
    expect(example).toContain("project_tree = await locus.call_tool(");
  });

  it("registers Python as a direct checkout-scoped tool with progressive SDK help", () => {
    const builtins = read("src-tauri/src/tool/builtins/mod.rs");
    const pythonTool = read("src-tauri/src/tool/builtins/python.rs");
    const prompt = read("tools/python.json");
    const unityAgent = read("agent/unity/config.json");
    const unityPrompt = read("agent/unity/tools/python.json");
    const simpleAgent = read("agent/simple/config.json");

    expect(builtins).toContain("registry.register_builtin(python::python())");
    expect(unityAgent).toContain('"python"');
    expect(simpleAgent).toContain('"python"');
    expect(prompt).not.toContain("get_unity_editor_status");
    expect(prompt).not.toContain("restart_unity_editor");
    expect(unityPrompt).toContain("get_unity_editor_status");
    expect(unityPrompt).toContain("restart_unity_editor");
    expect(unityPrompt).toContain("status.safe_mode");
    expect(unityPrompt).toContain("status.editor_log_path");
    expect(prompt).toContain('"help"');
    expect(pythonTool).toContain("workspace_ref = locus.WorkspaceRef");
    expect(pythonTool).toContain("HELP_CALLBACKS");
    expect(pythonTool).toContain("python_process_env");
    expect(pythonTool).toContain('command.env("LOCUS_SESSION_ID"');
  });
});
