// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope } from "vue";
import type { Event, UnlistenFn } from "@tauri-apps/api/event";
import type { RoutedWorkspaceEvent } from "../services/project";
import { workspaceMaterializationMatches } from "../services/project";
import { createWorkspaceEventHub, listenWorkspaceEvent, resetWorkspaceEventHubForTests, snapshotWorkspaceEvents, WORKSPACE_EVENT_NAME } from "../services/workspaceEventHub";
import { getLocusRuntime } from "../services/locusRuntime";
import { useWorkspaceEventScope } from "../composables/useWorkspaceEventScope";

const mocks = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

type WorkspaceEvent = RoutedWorkspaceEvent;

function event(overrides: Partial<WorkspaceEvent> = {}): Event<WorkspaceEvent> {
  return {
    id: 1, event: WORKSPACE_EVENT_NAME,
    payload: {
      eventName: "stream-event", streamRevision: 1,
      projectId: "shared-project", checkoutId: "checkout-a", workspaceGeneration: 3,
      materializationEpoch: 2, serviceInstanceId: "unity-a", serviceGeneration: 4,
      payload: { secret: "do not record payload" }, ...overrides,
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function harness() {
  let dispatch!: (value: Event<WorkspaceEvent>) => unknown;
  const nativeRelease = vi.fn();
  const transport = vi.fn(async (handler: typeof dispatch) => {
    dispatch = handler;
    return nativeRelease;
  });
  const hub = createWorkspaceEventHub(transport);
  return { hub, transport, nativeRelease, emit: (value = event()) => dispatch(value) };
}

afterEach(() => {
  resetWorkspaceEventHubForTests();
  vi.restoreAllMocks();
  mocks.listen.mockReset();
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("window workspace event ownership", () => {
  it("shares one native listener through concurrent subscriptions and repeated idle/remount cycles", async () => {
    const { hub, transport, nativeRelease, emit } = harness();
    const calls = vi.fn();
    const first = hub.subscribe(calls, { owner: "pane-a" });
    const second = hub.subscribe(calls, { owner: "pane-b" });
    await Promise.all([first.ready, second.ready]);
    emit();
    expect(calls).toHaveBeenCalledTimes(2);
    first.unsubscribe(); second.unsubscribe(); first.unsubscribe();
    emit();
    expect(calls).toHaveBeenCalledTimes(2);
    for (let index = 0; index < 100; index += 1) {
      const subscription = hub.subscribe(calls, { owner: `pane-${index}` });
      await subscription.ready;
      subscription.unsubscribe();
    }
    expect(transport).toHaveBeenCalledTimes(1);
    expect(nativeRelease).not.toHaveBeenCalled();
    expect(hub.snapshot().subscribers).toHaveLength(0);
    hub.dispose(); hub.dispose();
    expect(nativeRelease).toHaveBeenCalledTimes(1);
  });

  it("cancels before native registration completes and ignores late registrations after scope disposal", async () => {
    const pending = deferred<UnlistenFn>();
    let dispatch!: (value: Event<WorkspaceEvent>) => unknown;
    const nativeRelease = vi.fn();
    const hub = createWorkspaceEventHub(async (handler) => { dispatch = handler; return pending.promise; });
    const scope = effectScope();
    const signal = scope.run(useWorkspaceEventScope)!;
    const handler = vi.fn();
    const subscription = hub.subscribe(handler, { owner: "unmounted-pane", signal });
    await Promise.resolve();
    scope.stop();
    dispatch(event());
    pending.resolve(nativeRelease);
    await subscription.ready;
    const late = hub.subscribe(handler, { owner: "after-await", signal });
    await late.ready;
    dispatch(event());
    expect(handler).not.toHaveBeenCalled();
    expect(nativeRelease).not.toHaveBeenCalled();
    expect(hub.snapshot().subscribers).toHaveLength(0);
    hub.dispose();
  });

  it("retries failed registration without retaining subscribers from the failed attempt", async () => {
    const pending = deferred<UnlistenFn>();
    const transport = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue(vi.fn());
    const hub = createWorkspaceEventHub(transport);
    const a = hub.subscribe(vi.fn(), { owner: "failed-a" });
    const b = hub.subscribe(vi.fn(), { owner: "failed-b" });
    const settled = Promise.allSettled([a.ready, b.ready]);
    pending.reject(new Error("registration failed"));
    expect((await settled).every((result) => result.status === "rejected")).toBe(true);
    expect(hub.snapshot().subscribers).toHaveLength(0);
    await hub.subscribe(vi.fn(), { owner: "retry" }).ready;
    expect(transport).toHaveBeenCalledTimes(2);
    expect(hub.snapshot().nativeListenerCount).toBe(1);
    hub.dispose();
  });

  it("releases a pending native registration if the test window is destroyed", async () => {
    const pending = deferred<UnlistenFn>();
    const release = vi.fn();
    const hub = createWorkspaceEventHub(() => pending.promise);
    const subscription = hub.subscribe(vi.fn(), { owner: "destroyed-window" });
    hub.dispose();
    pending.resolve(release);
    await subscription.ready;
    expect(release).toHaveBeenCalledTimes(1);
    expect(hub.snapshot().subscribers).toHaveLength(0);
  });

  it("isolates sync and async errors and honors removals during the same dispatch", async () => {
    const { hub, emit } = harness();
    hub.setDiagnosticsEnabled(true);
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    const skipped = vi.fn(); const healthy = vi.fn(); const late = vi.fn();
    let removed!: ReturnType<typeof hub.subscribe>;
    const a = hub.subscribe(() => {
      removed.unsubscribe();
      hub.subscribe(late, { owner: "added-during-dispatch" });
      throw new Error("sync");
    }, { owner: "sync-error" });
    removed = hub.subscribe(skipped, { owner: "removed-during-dispatch" });
    const b = hub.subscribe(async () => { throw new Error("async"); }, { owner: "async-error" });
    const c = hub.subscribe(healthy, { owner: "healthy" });
    await Promise.all([a.ready, b.ready, c.ready]);
    emit();
    await Promise.resolve(); await Promise.resolve();
    expect(skipped).not.toHaveBeenCalled();
    expect(late).not.toHaveBeenCalled();
    expect(healthy).toHaveBeenCalledTimes(1);
    expect(log).toHaveBeenCalledTimes(2);
    expect(hub.snapshot().traces.filter((item) => item.action === "error").map((item) => item.owner))
      .toEqual(["sync-error", "async-error"]);
    hub.dispose();
  });

  it("preserves interleaved worktree envelopes and lets each consumer apply its own generation and materialization guards", async () => {
    const { hub, emit } = harness();
    const receivedA: WorkspaceEvent[] = []; const receivedB: WorkspaceEvent[] = [];
    const all: WorkspaceEvent[] = [];
    const scopeHandler = (checkout: string, generation: number, epoch: number, received: WorkspaceEvent[]) =>
      ({ payload }: Event<WorkspaceEvent>) => {
        if (payload.checkoutId !== checkout || payload.workspaceGeneration !== generation) return;
        if (!workspaceMaterializationMatches(epoch, payload.materializationEpoch)) return;
        received.push(payload);
      };
    await hub.subscribe(scopeHandler("checkout-a", 3, 2, receivedA), { owner: "foreground-a" }).ready;
    await hub.subscribe(scopeHandler("checkout-b", 7, 5, receivedB), { owner: "background-b" }).ready;
    await hub.subscribe(({ payload }) => all.push(payload), { owner: "revision-reducer" }).ready;
    const inputs = [
      event(), event({ checkoutId: "checkout-b", workspaceGeneration: 7, materializationEpoch: 5 }),
      event({ workspaceGeneration: 2 }), event({ materializationEpoch: 1 }),
      event({ streamRevision: 2 }), event({ checkoutId: "checkout-b", workspaceGeneration: 7, materializationEpoch: 4 }),
    ];
    inputs.forEach(emit);
    expect(all).toEqual(inputs.map((input) => input.payload));
    expect(all[0]).toBe(inputs[0].payload);
    expect(receivedA).toEqual([inputs[0].payload, inputs[4].payload]);
    expect(receivedB).toEqual([inputs[1].payload]);
    hub.dispose();
  });

  it("keeps separate native listeners and subscriber lifetimes in separate windows", async () => {
    const main = harness(); const child = harness();
    const a = vi.fn(); const b = vi.fn();
    await main.hub.subscribe(a, { owner: "main" }).ready;
    await child.hub.subscribe(b, { owner: "child" }).ready;
    main.emit();
    expect(a).toHaveBeenCalledTimes(1); expect(b).not.toHaveBeenCalled();
    main.hub.dispose(); child.emit();
    expect(b).toHaveBeenCalledTimes(1); expect(child.nativeRelease).not.toHaveBeenCalled();
    child.hub.dispose();
  });

  it("bounds opt-in diagnostics, omits payloads and returns detached snapshots", async () => {
    const { hub, emit } = harness();
    await hub.subscribe(vi.fn(), { owner: "diagnosed-pane" }).ready;
    emit();
    expect(hub.snapshot().traces).toHaveLength(0);
    hub.setDiagnosticsEnabled(true);
    for (let index = 0; index < 400; index += 1) emit(event({ streamRevision: index }));
    const snapshot = hub.snapshot();
    expect(snapshot.traces).toHaveLength(256);
    expect(JSON.stringify(snapshot)).not.toContain("do not record payload");
    expect(snapshot.traces[255]?.envelope).toMatchObject({ checkoutId: "checkout-a", materializationEpoch: 2 });
    snapshot.traces[255].envelope!.checkoutId = "modified";
    snapshot.subscribers[0].owner = "modified";
    expect(hub.snapshot().subscribers[0].owner).toBe("diagnosed-pane");
    expect(hub.snapshot().traces[255]?.envelope?.checkoutId).toBe("checkout-a");
    hub.setDiagnosticsEnabled(false);
    expect(hub.snapshot().traces).toHaveLength(0);
    hub.dispose();
  });

  it("routes runtime and direct adapters through the same native subscription", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: { invoke: vi.fn() } });
    let dispatch!: (value: Event<WorkspaceEvent>) => void;
    mocks.listen.mockImplementation(async (_event, handler) => { dispatch = handler; return vi.fn(); });
    const raw = vi.fn(); const runtime = vi.fn();
    const [releaseRaw, releaseRuntime] = await Promise.all([
      listenWorkspaceEvent("raw-adapter", raw),
      getLocusRuntime().subscribe(WORKSPACE_EVENT_NAME, runtime, { owner: "runtime-adapter" }),
    ]);
    const value = event(); dispatch(value);
    expect(mocks.listen).toHaveBeenCalledTimes(1);
    expect(raw).toHaveBeenCalledWith(value);
    expect(runtime).toHaveBeenCalledWith(value.payload);
    releaseRaw(); releaseRuntime();
    expect(snapshotWorkspaceEvents().subscribers).toHaveLength(0);
  });
});
