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
    expect(session).toContain("tool_registry_for_agent(tool_registry.inner().as_ref(), spec)");
  });

  it("supports Locus tools and loopback Python callable tools on one agent", () => {
    const tools = read("python/locus/_tools.py");
    const callbacks = read("python/locus/_callbacks.py");
    const bridge = read("src-tauri/src/sdk.rs");

    expect(tools).toContain("get_type_hints(function, include_extras=True)");
    expect(callbacks).toContain('ThreadingHTTPServer(("127.0.0.1", 0), Handler)');
    expect(callbacks).toContain("asyncio.run_coroutine_threadsafe(");
    expect(bridge).toContain("spec.locus_tools = canonical_tool_names(app, spec.locus_tools).await?");
    expect(bridge).toContain("Python tool callback URL must target loopback");
    expect(bridge).toContain("registry.register_runtime(");
  });
});
