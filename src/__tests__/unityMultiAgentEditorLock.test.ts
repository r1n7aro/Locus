import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("Unity multi-Agent single Editor coordination", () => {
  it("defines wait and try acquisition around only minimal Editor-state control", () => {
    const definition = JSON.parse(read("tools/unity_lock.json"));
    const releaseDefinition = JSON.parse(read("tools/unity_release.json"));
    const builtin = read("src-tauri/src/tool/builtins/unity.rs");
    const lockStart = builtin.indexOf("pub(super) fn unity_lock()");
    const lockEnd = builtin.indexOf("pub(super) fn unity_release()", lockStart);
    const lockImplementation = builtin.slice(lockStart, lockEnd);

    expect(definition.parameters.properties.mode.enum).toEqual(["wait", "try"]);
    expect(definition.parameters.properties.mode.default).toBe("wait");
    expect(definition.description).toContain("smallest critical section");
    expect(definition.description).toContain("Never hold the lock for an entire task or workflow");
    expect(definition.description).toContain("Asset and code work does not require this lock");
    expect(definition.description).toContain("only the recompile/state-dependent portion may need the lock");
    expect(definition.description).toContain("not on whether an operation mutates assets or on `unity_execute.readonly`");
    expect(definition.description).toContain("mutating `unity_execute` runs without this lock");
    expect(definition.parameters.properties.reason.description).toContain("Do not list asset or code edits");
    expect(releaseDefinition.description).toContain("immediately after the last direct Editor-state control");
    expect(releaseDefinition.description).toContain("asset/code edits and ordinary verification outside the lock");
    expect(definition.description).toContain("complete holder session and reason");
    expect(lockImplementation).toContain("UnityEditorLockAcquireError::Busy");
    expect(lockImplementation).toMatch(/UnityEditorLockAcquireError::Busy[\s\S]*is_error: false/);
  });

  it("injects the protocol dynamically and keeps it outside Unity execution barriers", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");

    expect(agent).toContain('push_unique_tool_name(&mut tools, "unity_lock")');
    expect(agent).toContain('push_unique_tool_name(&mut tools, "unity_release")');
    expect(agent).toContain("crate::unity_editor_lock::is_enabled()");
    expect(agent).toContain("release_for_session(&self.working_dir, &self.session_id)");

    const barrierStart = agent.indexOf("pub(crate) fn is_unity_execution_barrier_tool");
    const barrierEnd = agent.indexOf("fn tool_call_has_unity_execution_barrier", barrierStart);
    const barrier = agent.slice(barrierStart, barrierEnd);
    expect(barrier).not.toContain("unity_lock");
    expect(barrier).not.toContain("unity_release");
  });

  it("reclaims inactive holders and reports the complete owner session", () => {
    const manager = read("src-tauri/src/unity_editor_lock.rs");

    expect(manager).toContain("activity.is_active(&holder.session_id)");
    expect(manager).toContain("release_if_holder_matches(&project_key, &holder.session_id)");
    expect(manager).toContain("current holder: {}");
    expect(manager).toContain("holder.summary()");
    expect(manager).toContain("UnityEditorLockAcquireMode::Try");
    expect(manager).toContain("try mode did not jump the queue");
  });

  it("persists the opt-in setting and exposes both built-in tools", () => {
    const config = read("src-tauri/src/config.rs");
    const builtins = read("src-tauri/src/tool/builtins/mod.rs");
    const prompt = read("src-tauri/src/prompt.rs");

    expect(config).toContain("default_unity_multi_agent_editor_enabled");
    expect(config).toContain("set_unity_multi_agent_editor_enabled");
    expect(builtins).toContain("unity::unity_lock()");
    expect(builtins).toContain("unity::unity_release()");
    expect(prompt).toContain('(\"unity_lock\", UNITY_LOCK)');
    expect(prompt).toContain('(\"unity_release\", UNITY_RELEASE)');
  });
});
