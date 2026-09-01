import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("Unity modal dialog readiness barrier", () => {
  it("returns the captured dialog choices before waiting for Agent service readiness", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const preflight = agent.indexOf("crate::unity_bridge::dialog::blocked_error(", agent.indexOf("let requires_ready"));
    const barrier = agent.indexOf(".resolve_service_ready(owner, UNITY_SERVICE_READY_TIMEOUT)", preflight);
    const retry = agent.indexOf("crate::unity_bridge::dialog::blocked_error(", barrier);

    expect(preflight).toBeGreaterThan(0);
    expect(barrier).toBeGreaterThan(preflight);
    expect(retry).toBeGreaterThan(barrier);
    expect(agent.slice(preflight, barrier)).toContain('"not_sent"');
  });

  it("uses the same fail-fast dialog result for MCP Unity tools", () => {
    const mcp = read("src-tauri/src/mcp/server/tools.rs");
    const execute = mcp.indexOf("async fn execute_workspace_tool(");
    const preflight = mcp.indexOf("crate::unity_bridge::dialog::blocked_error(", execute);
    const barrier = mcp.indexOf("resolve_owned_service_for_mcp_tool(", preflight);
    const retry = mcp.indexOf("crate::unity_bridge::dialog::blocked_error(", barrier);

    expect(execute).toBeGreaterThan(0);
    expect(preflight).toBeGreaterThan(execute);
    expect(barrier).toBeGreaterThan(preflight);
    expect(retry).toBeGreaterThan(barrier);
  });

  it("formats every captured choice in the original blocked-tool message", () => {
    const dialog = read("src-tauri/src/unity_bridge/dialog.rs");
    const formatStart = dialog.indexOf("fn format_blocked_error(");
    const formatEnd = dialog.indexOf("fn display_or_empty(", formatStart);
    const format = dialog.slice(formatStart, formatEnd);

    expect(format).toContain('lines.push(format!("- {}: {}", choice.id, choice.label))');
    expect(dialog).toContain("choice-0: Save");
    expect(dialog).toContain("choice-1: Don't Save");
    expect(dialog).toContain("choice-2: Cancel");
    expect(format).toContain("locus.choose_unity_dialog");
    expect(format).toContain("内置 python 工具");
    expect(format).toContain("choice = await locus.choose_unity_dialog");
    expect(format).toContain("output = await locus.wait_unity_execution");
    expect(format).not.toContain("python -c");
  });

  it("waits for the selected dialog to close before returning to Python", () => {
    const dialog = read("src-tauri/src/unity_bridge/dialog.rs");
    const choose = dialog.indexOf("pub async fn choose_dialog(");
    const subscribe = dialog.indexOf("let mut dialog_updates = subscribe();", choose);
    const invoke = dialog.indexOf("tokio::task::spawn_blocking", subscribe);
    const wait = dialog.indexOf("wait_for_dialog_dismissal(", invoke);

    expect(choose).toBeGreaterThan(0);
    expect(subscribe).toBeGreaterThan(choose);
    expect(invoke).toBeGreaterThan(subscribe);
    expect(wait).toBeGreaterThan(invoke);
  });

  it("returns a normal result when the user already handled the dialog", () => {
    const dialog = read("src-tauri/src/unity_bridge/dialog.rs");
    const choose = dialog.indexOf("fn choose_dialog_blocking(");
    const unavailable = dialog.indexOf("fn dialog_unavailable_result(", choose);
    const implementation = dialog.slice(choose, unavailable);

    expect(implementation).toContain("let Some(record) = records.get_mut(&key) else");
    expect(implementation).toContain("return Ok(dialog_unavailable_result(");
    expect(dialog).toContain('"dialog_not_found"');
    expect(dialog).toContain("it may already have been handled manually");
  });

  it("covers immediate detached-execution recovery in the Unity integration suite", () => {
    const driver = read("src-tauri/src/cli_driver.rs");
    const resolver = driver.indexOf("async fn resolve_modal_dialog_through_python_sdk(");
    const choose = driver.indexOf("await locus.choose_unity_dialog(", resolver);
    const wait = driver.indexOf("await locus.wait_unity_execution(", choose);
    const recoveredOutput = driver.indexOf('"executionOutput": execution_output', wait);

    expect(resolver).toBeGreaterThan(0);
    expect(choose).toBeGreaterThan(resolver);
    expect(wait).toBeGreaterThan(choose);
    expect(recoveredOutput).toBeGreaterThan(wait);
  });

  it("subscribes before the transport preflight without adding another window scan", () => {
    const transport = read("src-tauri/src/unity_bridge/transport.rs");
    const send = transport.indexOf("async fn send_message_inner(");
    const subscribe = transport.indexOf("dialog::subscribe()", send);
    const preflight = transport.indexOf("dialog::blocked_error(project_path", subscribe);
    const connect = transport.indexOf("get_or_connect(project_path)", preflight);

    expect(send).toBeGreaterThan(0);
    expect(subscribe).toBeGreaterThan(send);
    expect(preflight).toBeGreaterThan(subscribe);
    expect(connect).toBeGreaterThan(preflight);
    expect(transport.slice(subscribe, connect).match(/dialog::blocked_error/g)).toHaveLength(1);
  });
});
