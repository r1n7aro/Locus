// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  initWebviewBridgeDiagnostics,
  snapshotPendingInvokes,
  teardownWebviewBridgeDiagnostics,
} from "../services/webviewBridgeDiagnostics";
import {
  invokeLocusRuntime,
  subscribeLocusRuntimeInvokeActivity,
  type LocusRuntimeInvokeActivity,
} from "../services/locusRuntime";
import {
  isRuntimePerformanceDiagnosticsRunning,
} from "../services/runtimePerformanceDiagnostics";

function installReadOnlyTauriInvoke(
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>,
): Record<string, unknown> {
  const internals: Record<string, unknown> = {};
  Object.defineProperty(internals, "invoke", {
    value: invoke,
    writable: false,
    enumerable: false,
    configurable: false,
  });
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    value: internals,
    writable: true,
    configurable: true,
  });
  return internals;
}

function enableStartupDiagnostics(): void {
  Object.defineProperty(window, "__LOCUS_DEBUG_ENABLED__", {
    value: true,
    writable: true,
    configurable: true,
  });
}

afterEach(() => {
  teardownWebviewBridgeDiagnostics();
  window.localStorage.clear();
  delete (window as unknown as { __LOCUS_DEBUG_ENABLED__?: boolean }).__LOCUS_DEBUG_ENABLED__;
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("webview bridge diagnostics", () => {
  it("orders pending invokes by age and bounds command names", () => {
    const snapshot = snapshotPendingInvokes([
      { id: 2, command: "newer", startedAtMs: 9_500 },
      { id: 1, command: "x".repeat(200), startedAtMs: 8_000 },
    ], 10_000);

    expect(snapshot).toEqual([
      { command: "x".repeat(160), ageMs: 2_000 },
      { command: "newer", ageMs: 500 },
    ]);
  });

  it("caps the diagnostic payload to the oldest pending calls", () => {
    const pending = Array.from({ length: 30 }, (_, index) => ({
      id: index,
      command: `command-${index}`,
      startedAtMs: index,
    }));

    const snapshot = snapshotPendingInvokes(pending, 100);

    expect(snapshot).toHaveLength(24);
    expect(snapshot[0]?.command).toBe("command-0");
    expect(snapshot[snapshot.length - 1]?.command).toBe("command-23");
  });

  it("starts with Tauri's production read-only invoke property", () => {
    const nativeInvoke = vi.fn(async () => undefined);
    const internals = installReadOnlyTauriInvoke(nativeInvoke);
    const descriptorBefore = Object.getOwnPropertyDescriptor(internals, "invoke");
    enableStartupDiagnostics();

    expect(() => initWebviewBridgeDiagnostics()).not.toThrow();

    expect(Object.getOwnPropertyDescriptor(internals, "invoke")).toEqual(descriptorBefore);
    expect(internals.invoke).toBe(nativeInvoke);
  });

  it("keeps runtime performance diagnostics stopped when debug mode is disabled", async () => {
    installReadOnlyTauriInvoke(async () => undefined);

    initWebviewBridgeDiagnostics();
    await vi.dynamicImportSettled();

    expect(isRuntimePerformanceDiagnosticsRunning()).toBe(false);
  });

  it("starts and stops runtime performance diagnostics with debug mode", async () => {
    vi.spyOn(console, "info").mockImplementation(() => {});
    installReadOnlyTauriInvoke(async () => undefined);
    enableStartupDiagnostics();

    initWebviewBridgeDiagnostics();
    await vi.dynamicImportSettled();
    expect(isRuntimePerformanceDiagnosticsRunning()).toBe(true);

    teardownWebviewBridgeDiagnostics();
    await vi.dynamicImportSettled();
    expect(isRuntimePerformanceDiagnosticsRunning()).toBe(false);
  });

  it("tracks project-owned runtime invokes without mutating Tauri internals", async () => {
    let resolveRequest!: (value: string) => void;
    const nativeRequest = new Promise<string>((resolve) => {
      resolveRequest = resolve;
    });
    const nativeInvoke = vi.fn(() => nativeRequest);
    const internals = installReadOnlyTauriInvoke(nativeInvoke);
    const descriptorBefore = Object.getOwnPropertyDescriptor(internals, "invoke");
    const activities: LocusRuntimeInvokeActivity[] = [];
    const unsubscribe = subscribeLocusRuntimeInvokeActivity((activity) => {
      activities.push(activity);
    });

    const request = invokeLocusRuntime<string>("slow_command", { value: 1 });

    expect(activities).toHaveLength(1);
    expect(activities[0]).toMatchObject({ phase: "started", command: "slow_command" });
    expect(Object.getOwnPropertyDescriptor(internals, "invoke")).toEqual(descriptorBefore);

    resolveRequest("done");
    await expect(request).resolves.toBe("done");
    await Promise.resolve();

    expect(activities).toHaveLength(2);
    expect(activities[1]).toMatchObject({
      phase: "settled",
      id: activities[0]?.id,
      command: "slow_command",
    });
    unsubscribe();
  });

  it("isolates runtime observer failures from IPC results", async () => {
    installReadOnlyTauriInvoke(async () => "ok");
    const unsubscribe = subscribeLocusRuntimeInvokeActivity(() => {
      throw new Error("observer failed");
    });

    await expect(invokeLocusRuntime<string>("safe_command")).resolves.toBe("ok");
    unsubscribe();
  });

  it("settles tracked activity when the native invoke rejects", async () => {
    installReadOnlyTauriInvoke(async () => {
      throw new Error("native failure");
    });
    const activities: LocusRuntimeInvokeActivity[] = [];
    const unsubscribe = subscribeLocusRuntimeInvokeActivity((activity) => {
      activities.push(activity);
    });

    await expect(invokeLocusRuntime("failing_command")).rejects.toThrow("native failure");
    await Promise.resolve();

    expect(activities.map(({ phase, command }) => ({ phase, command }))).toEqual([
      { phase: "started", command: "failing_command" },
      { phase: "settled", command: "failing_command" },
    ]);
    unsubscribe();
  });
});
