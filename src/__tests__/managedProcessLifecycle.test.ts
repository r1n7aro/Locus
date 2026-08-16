import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("managed tool process lifecycle", () => {
  it("creates process trees before user commands can start", () => {
    const processUtil = read("src-tauri/src/process_util.rs");

    expect(processUtil).toContain("CREATE_NO_WINDOW | CREATE_SUSPENDED");
    expect(processUtil).toContain("AssignProcessToJobObject");
    expect(processUtil).toContain("JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE");
    expect(processUtil).toContain("resume_managed_process(&child)");
    expect(processUtil).toContain("command.process_group(0)");
  });

  it("routes shell and Skill processes through the managed process owner", () => {
    const shell = read("src-tauri/src/tool/builtins/shell.rs");
    const skill = read("src-tauri/src/commands/skill.rs");
    const tool = read("src-tauri/src/tool/mod.rs");

    expect(tool).toContain("pub process_owner: Option<crate::process_util::ProcessOwner>");
    expect(shell).toContain("spawn_managed(command, process_owner)");
    expect(skill).toContain("let mut child = spawn_managed(cmd, owner)");
  });

  it("cascades cancellation through sessions, workspaces, async tasks, and app exit", () => {
    const session = read("src-tauri/src/commands/session.rs");
    const workspace = read("src-tauri/src/commands/workspace.rs");
    const system = read("src-tauri/src/commands/system.rs");

    expect(session).toContain("async_tasks.cancel_session(&session_id)");
    expect(session).toContain("terminate_managed_processes_for_session(&session_id)");
    expect(workspace).toContain("async_task_manager.cancel_workspace(&prev_cwd)");
    expect(workspace).toContain("terminate_managed_processes_for_workspace(&prev_cwd)");
    expect(system).toContain("tasks.cancel_all()");
    expect(system).toContain("begin_managed_process_shutdown()");
    expect(system).toContain("wait_for_managed_processes(");
  });
});
