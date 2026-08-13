import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("agent parallel tool scheduling safety", () => {
  it("coordinates mutations while bypassing read-only tools", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const acquire = agent.indexOf("process_workspace_execution_lock(", agent.indexOf("let prepared"));
    const checkpoint = agent.indexOf("let pre_checkpoint", acquire);
    const release = agent.lastIndexOf("drop(workspace_round_guard.take())");
    const afterRound = agent.indexOf(".after_round_for_paths(", checkpoint);

    expect(agent).toContain("WorkspaceExecutionLockRequest::PathWrite");
    expect(agent).toContain("WorkspaceExecutionLockRequest::Exclusive");
    expect(agent).toContain("let execute_sequentially = workspace_lock_request.is_some()");
    expect(agent).toContain("workspace_execution_request_for_tool");
    expect(agent).toContain('matches!(target_name.as_str(), "write" | "edit")');
    expect(agent).toContain("if execute_sequentially");
    expect(acquire).toBeGreaterThan(0);
    expect(checkpoint).toBeGreaterThan(acquire);
    expect(release).toBeGreaterThan(afterRound);
  });

  it("serializes same-path writes while allowing distinct paths", () => {
    const lock = read("src-tauri/src/agent/workspace_execution_lock.rs");

    expect(lock).toContain("PathWrite(Vec<String>)");
    expect(lock).toContain("path_gate.lock_owned().await");
    expect(lock).toContain("gate.read_owned().await");
    expect(lock).toContain("gate.write_owned().await");
    expect(lock).toContain("same_path_writes_are_serialized");
    expect(lock).toContain("distinct_path_writes_overlap_and_exclusive_waits_for_all");
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

  it("runs sync subagents before local siblings and isolates other reentrant tool kinds", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const subagentPhase = agent.indexOf("executing foreground subagent phase before local siblings");
    const preconfirm = agent.indexOf("Confirm every local call before taking the workspace lock");
    const acquire = agent.indexOf("process_workspace_execution_lock(", preconfirm);
    const deterministicPhase = agent.indexOf("executing deterministic pre-ask tools in parallel");
    const releaseBeforeAsk = agent.indexOf("drop(workspace_round_guard.take())", deterministicPhase);
    const askPhase = agent.indexOf("executing user-input phase sequentially", deterministicPhase);

    expect(subagentPhase).toBeGreaterThan(0);
    expect(preconfirm).toBeGreaterThan(subagentPhase);
    expect(preconfirm).toBeGreaterThan(0);
    expect(acquire).toBeGreaterThan(preconfirm);
    expect(agent).toContain("user-input rounds only allow deterministic pre-ask tools");
    expect(agent).toContain("is_deterministic_pre_ask_tool");
    expect(deterministicPhase).toBeGreaterThan(acquire);
    expect(releaseBeforeAsk).toBeGreaterThan(deterministicPhase);
    expect(askPhase).toBeGreaterThan(releaseBeforeAsk);
    expect(agent).toContain('"subagent-then-local"');
    expect(agent).toContain("precompleted_results");
    expect(agent).not.toContain("sub-agent calls must run without local sibling tools");
    expect(agent).toContain("external MCP calls must run without local sibling tools");
  });

  it("polls each subagent loop behind an abortable Tokio task boundary", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const runSubagent = agent.indexOf("async fn run_subagent(");
    const childConstruction = agent.indexOf("let mut child = AgentInstance::new(", runSubagent);
    const promptOwnership = agent.indexOf("let child_prompt = prompt.to_owned();", childConstruction);
    const spawn = agent.indexOf("AbortOnDropTask::new(tokio::spawn(async move", promptOwnership);
    const childRun = agent.indexOf("child\n                .run(", spawn);
    const join = agent.indexOf("let child_result = child_task", childRun);

    expect(runSubagent).toBeGreaterThan(0);
    expect(childConstruction).toBeGreaterThan(runSubagent);
    expect(promptOwnership).toBeGreaterThan(childConstruction);
    expect(spawn).toBeGreaterThan(promptOwnership);
    expect(childRun).toBeGreaterThan(spawn);
    expect(join).toBeGreaterThan(childRun);
    expect(agent).toContain("self.handle.abort()");
    expect(agent).toContain("child_store.as_ref()");
    expect(agent).not.toContain("let child_args = args.clone()");
  });

  it("covers the Claude Code CLI host path with the same lock", () => {
    const cli = read("src-tauri/src/agent/instance/claude_code_cli.rs");
    const executeTool = cli.indexOf("fn execute_tool");
    const preconfirm = cli.indexOf("ensure_cli_round_confirmations_prepared().await", executeTool);
    const acquire = cli.indexOf("process_workspace_execution_lock(", preconfirm);

    expect(cli).toContain("workspace_guard: Option<WorkspaceExecutionGuard>");
    expect(cli).toContain("cli_round_workspace_policy");
    expect(cli).toContain("process_workspace_execution_lock(&self.agent.working_dir)");
    expect(cli).toContain("ensure_cli_foreground_subagent_phase().await");
    expect(cli).toContain("precompleted_subagent_results");
    expect(cli).not.toContain("sub-agent calls must run without local sibling tools");
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
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const cli = read("src-tauri/src/agent/instance/claude_code_cli.rs");
    const sdk = read("src-tauri/src/sdk.rs");
    const mcp = read("src-tauri/src/mcp/server/tools.rs");

    for (const event of ["requested", "acquired", "waiting", "cancelled", "abandoned", "released"]) {
      expect(lock).toContain(`[WorkspaceExecutionLock] ${event}`);
    }
    expect(lock).toContain("possible_deadlock=");
    expect(lock).toContain("workspace-execution-lock-diagnostic");
    expect(lock).toContain("WorkspaceExecutionLockDiagnostic");
    expect(lock).toContain("clear_diagnostic");
    expect(lock).toContain("session=");
    expect(lock).toContain("run=");
    expect(lock).toContain("holders=(");
    expect(lock).toContain("PROCESS_WORKSPACE_EXECUTION_LOCKS");
    expect(lock).toContain("normalize_workspace_key");
    for (const source of [agent, cli, sdk, mcp]) {
      expect(source).toContain("acquire_with_diagnostics");
    }
  });

  it("covers the inbound MCP server tool execution path", () => {
    const mcp = read("src-tauri/src/mcp/server/tools.rs");
    const acquire = mcp.indexOf("process_workspace_execution_lock(");
    const execute = mcp.indexOf("execute_workspace_tool(&app", acquire);
    const release = mcp.indexOf("drop(workspace_guard)", execute);

    expect(mcp).toContain("WorkspaceExecutionLockRequest::Exclusive");
    expect(mcp).toContain("let workspace_guard = if let Some(request) = lock_request");
    expect(acquire).toBeGreaterThan(0);
    expect(execute).toBeGreaterThan(acquire);
    expect(release).toBeGreaterThan(execute);
  });
});
