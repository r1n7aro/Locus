import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("workspace event scope guard", () => {
  it("keeps generationless root publishers out of the backend", () => {
    const router = read("src-tauri/src/workspace_service/event.rs");
    expect(router).not.toContain("publish_for_root");
    expect(router).not.toContain("emit_for_workspace_root");
  });

  it("validates the workspace envelope before projecting payload state", () => {
    const bootstrap = read("src/composables/useAppBootstrap.ts");
    const listener = bootstrap.indexOf("WORKSPACE_EVENT_NAME,");
    const validation = bootstrap.indexOf(
      "workspaceContextStore.applyWorkspaceEvent(event)",
      listener,
    );
    const firstProjection = bootstrap.indexOf(
      'event.eventName === "unity-connection-status"',
      listener,
    );

    expect(listener).toBeGreaterThan(-1);
    expect(validation).toBeGreaterThan(listener);
    expect(firstProjection).toBeGreaterThan(validation);
  });

  it("requires the registered runtime generation before accepting an event", () => {
    const store = read("src/stores/workspaceContext.ts");
    expect(store).toContain("runtime.workspaceGeneration !== event.workspaceGeneration");
    expect(store).toContain("event.streamRevision <= currentEvent.streamRevision");
  });

  it("routes workspace lock diagnostics through the scoped event envelope", () => {
    const lock = read("src-tauri/src/agent/workspace_execution_lock.rs");
    const bootstrap = read("src/composables/useAppBootstrap.ts");

    expect(lock).toContain("publish_for_scope(");
    expect(lock).not.toContain("app_handle.emit(WORKSPACE_EXECUTION_LOCK_DIAGNOSTIC_EVENT");
    expect(bootstrap).toContain(
      "event.eventName === WORKSPACE_EXECUTION_LOCK_DIAGNOSTIC_EVENT",
    );
    expect(bootstrap).not.toContain(
      "runtime.subscribe<WorkspaceExecutionLockDiagnostic>(",
    );
  });
});
