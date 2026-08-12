import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("agent parallel tool scheduling safety", () => {
  it("keeps mutating rounds under the process workspace write lock", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const acquire = agent.indexOf("process_workspace_execution_lock()", agent.indexOf("let prepared"));
    const checkpoint = agent.indexOf("let pre_checkpoint", acquire);
    const release = agent.lastIndexOf("drop(workspace_round_guard.take())");
    const afterRound = agent.indexOf(".after_round(", checkpoint);

    expect(agent).toContain("WorkspaceExecutionLockMode::Write");
    expect(agent).toContain("let execute_sequentially = workspace_lock_mode");
    expect(agent).toContain("if execute_sequentially");
    expect(acquire).toBeGreaterThan(0);
    expect(checkpoint).toBeGreaterThan(acquire);
    expect(release).toBeGreaterThan(afterRound);
  });

  it("batches same-file edits and runs distinct file batches in parallel", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const plan = agent.indexOf("Self::plan_parallel_edit_batches(&prepared");
    const branch = agent.indexOf("} else if let Some(edit_batches) = parallel_edit_batches");
    const pending = agent.indexOf("FuturesUnordered::new()", branch);
    const execute = agent.indexOf(".execute_single_tool(", pending);

    expect(plan).toBeGreaterThan(0);
    expect(branch).toBeGreaterThan(plan);
    expect(pending).toBeGreaterThan(branch);
    expect(execute).toBeGreaterThan(pending);
    expect(agent).toContain('"parallel-edit-batches"');
    expect(agent).toContain("member_indices.push(index)");
    expect(agent).toContain('"edits": operations');
  });

  it("preconfirms before locking and isolates reentrant tool kinds", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const preconfirm = agent.indexOf("Confirm every local call before taking the process-wide lock");
    const acquire = agent.indexOf("process_workspace_execution_lock()", preconfirm);
    const deterministicPhase = agent.indexOf("executing deterministic pre-ask tools in parallel");
    const releaseBeforeAsk = agent.indexOf("drop(workspace_round_guard.take())", deterministicPhase);
    const askPhase = agent.indexOf("executing user-input phase sequentially", deterministicPhase);

    expect(preconfirm).toBeGreaterThan(0);
    expect(acquire).toBeGreaterThan(preconfirm);
    expect(agent).toContain("user-input rounds only allow deterministic pre-ask tools");
    expect(agent).toContain("is_deterministic_pre_ask_tool");
    expect(deterministicPhase).toBeGreaterThan(acquire);
    expect(releaseBeforeAsk).toBeGreaterThan(deterministicPhase);
    expect(askPhase).toBeGreaterThan(releaseBeforeAsk);
    expect(agent).toContain("sub-agent calls must run without local sibling tools");
    expect(agent).toContain("external MCP calls must run without local sibling tools");
  });

  it("covers the Claude Code CLI host path with the same lock", () => {
    const cli = read("src-tauri/src/agent/instance/claude_code_cli.rs");
    const executeTool = cli.indexOf("fn execute_tool");
    const preconfirm = cli.indexOf("ensure_cli_round_confirmations_prepared().await", executeTool);
    const acquire = cli.indexOf("process_workspace_execution_lock()", preconfirm);

    expect(cli).toContain("workspace_guard: Option<WorkspaceExecutionGuard>");
    expect(cli).toContain("cli_round_workspace_policy");
    expect(cli).toContain("process_workspace_execution_lock()");
    expect(cli).toContain("confirmation_preapproved");
    expect(cli).toContain("is_deterministic_pre_ask_tool");
    expect(cli).toContain("is_deterministic_pre_ask_call");
    expect(cli).toContain("_single_tool_workspace_guard = Some(guard)");
    expect(preconfirm).toBeGreaterThan(executeTool);
    expect(acquire).toBeGreaterThan(preconfirm);
  });

  it("uses conflict-aware atomic filesystem mutations", () => {
    const filesystem = read("src-tauri/src/tool/builtins/filesystem.rs");

    expect(filesystem).toContain(".create_new(true)");
    expect(filesystem).toContain("ensure_edit_base_is_current");
    expect(filesystem).toContain("replace_file_atomically");
    expect(filesystem).toContain("current_content = op.new_string.clone()");
    expect(filesystem).toContain("set_permissions(&temp_path, metadata.permissions())");
    expect(filesystem).toContain("[FilesystemEdit] conflict");
    expect(filesystem).toContain("MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH");
  });

  it("emits actionable lock lifecycle and possible-deadlock logs", () => {
    const lock = read("src-tauri/src/agent/workspace_execution_lock.rs");

    for (const event of ["requested", "acquired", "waiting", "cancelled", "abandoned", "released"]) {
      expect(lock).toContain(`[WorkspaceExecutionLock] ${event}`);
    }
    expect(lock).toContain("possible_deadlock=");
    expect(lock).toContain("session=");
    expect(lock).toContain("run=");
    expect(lock).toContain("holders=(");
  });

  it("covers the inbound MCP server tool execution path", () => {
    const mcp = read("src-tauri/src/mcp/server/tools.rs");
    const acquire = mcp.indexOf("process_workspace_execution_lock()");
    const execute = mcp.indexOf("execute_workspace_tool(&app", acquire);
    const release = mcp.indexOf("drop(workspace_guard)", execute);

    expect(mcp).toContain("WorkspaceExecutionLockMode::Write");
    expect(acquire).toBeGreaterThan(0);
    expect(execute).toBeGreaterThan(acquire);
    expect(release).toBeGreaterThan(execute);
  });
});
