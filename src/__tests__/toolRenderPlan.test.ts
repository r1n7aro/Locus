import { describe, expect, it } from "vitest";
import type { ToolCallDisplay } from "../types";
import {
  collectRenderedToolCalls,
  planPromotedToolVisibility,
} from "../composables/toolRenderPlan";
import { consumeToolCallDisplayFromMatchState, collectToolCallDisplayMatchState } from "../composables/toolCallBatches";

function tc(
  id: string,
  name: string,
  args: Record<string, unknown> = {},
  extra: Partial<ToolCallDisplay> = {},
): ToolCallDisplay {
  return {
    id,
    name,
    arguments: JSON.stringify(args),
    order: 0,
    status: "done",
    ...extra,
  };
}

function segment(...toolCalls: ToolCallDisplay[]) {
  return { toolCalls };
}

function hiddenIds(plan: ReturnType<typeof planPromotedToolVisibility>) {
  return [...plan.hiddenMatchState.ids].sort();
}

describe("collectRenderedToolCalls", () => {
  it("flattens tool calls across transient tool segments in order", () => {
    const a = tc("a", "read", { path: "a.ts" });
    const b = tc("b", "edit", { path: "b.ts" });
    const c = tc("c", "grep", { pattern: "x" });

    expect(collectRenderedToolCalls([segment(a, b), segment(c)])).toEqual([a, b, c]);
  });

  it("returns an empty list when nothing is rendered", () => {
    expect(collectRenderedToolCalls([])).toEqual([]);
  });
});

describe("planPromotedToolVisibility (fail-open hide/show reconciliation)", () => {
  it("hides every promoted call in the healthy path where transient renders all of them", () => {
    const a = tc("a", "read", { path: "a.ts" });
    const b = tc("b", "edit", { path: "b.ts" });

    // Healthy: transient's promoted prefix contains the very same call objects.
    const plan = planPromotedToolVisibility({
      promotedToolCalls: [a, b],
      transientRenderedToolCalls: [a, b],
    });

    expect(plan.uncoveredToolCalls).toEqual([]);
    expect(plan.coveredToolCalls).toEqual([a, b]);
    expect(hiddenIds(plan)).toEqual(["a", "b"]);
  });

  it("REGRESSION: keeps a promoted call visible in history when transient drops it", () => {
    // This is the original bug: the last tool batch is hidden from history but
    // never rendered in transient -> it vanishes from both regions. Fail-open
    // must keep the dropped call (`b`) in history.
    const a = tc("a", "read", { path: "a.ts" });
    const b = tc("b", "edit", { path: "b.ts" });

    const plan = planPromotedToolVisibility({
      promotedToolCalls: [a, b],
      transientRenderedToolCalls: [a], // transient failed to render `b`
    });

    expect(plan.coveredToolCalls).toEqual([a]);
    expect(plan.uncoveredToolCalls).toEqual([b]);
    expect(hiddenIds(plan)).toEqual(["a"]); // `b` stays visible in history
    expect(plan.hiddenMatchState.ids.has("b")).toBe(false);
  });

  it("contrast: the previous full-set hide would have hidden the dropped call (the bug)", () => {
    const a = tc("a", "read", { path: "a.ts" });
    const b = tc("b", "edit", { path: "b.ts" });

    // Old behavior merged the ENTIRE promoted match state into the hidden set,
    // regardless of what transient actually rendered -> `b` vanished from both.
    const oldHidden = collectToolCallDisplayMatchState([a, b]);
    expect(oldHidden.ids.has("b")).toBe(true);

    // New behavior only hides what transient renders, so `b` survives in history.
    const plan = planPromotedToolVisibility({
      promotedToolCalls: [a, b],
      transientRenderedToolCalls: [a],
    });
    expect(plan.hiddenMatchState.ids.has("b")).toBe(false);
  });

  it("hides nothing when the transient region renders no tool calls at all", () => {
    const a = tc("a", "read", { path: "a.ts" });

    const plan = planPromotedToolVisibility({
      promotedToolCalls: [a],
      transientRenderedToolCalls: [],
    });

    expect(plan.coveredToolCalls).toEqual([]);
    expect(plan.uncoveredToolCalls).toEqual([a]);
    expect(hiddenIds(plan)).toEqual([]);
  });

  it("reconciles ids that differ across the transient->history boundary by fingerprint", () => {
    // History copy and transient copy of the same logical call carry different
    // ids (assigned by different sources) but identical name + args.
    const historyCopy = tc("history-1", "unity_execute", { code: "Foo()" });
    const transientCopy = tc("transient-1", "unity_execute", { code: "Foo()" });

    const plan = planPromotedToolVisibility({
      promotedToolCalls: [historyCopy],
      transientRenderedToolCalls: [transientCopy],
    });

    expect(plan.uncoveredToolCalls).toEqual([]);
    expect(hiddenIds(plan)).toEqual(["history-1"]);
  });

  it("reconciles path-alias differences (camelCase / snake_case / path) by fingerprint", () => {
    const historyCopy = tc("h", "read", { file_path: "C:\\a\\b.cs" });
    const transientCopy = tc("t", "read", { path: "C:/a/b.cs" });

    const plan = planPromotedToolVisibility({
      promotedToolCalls: [historyCopy],
      transientRenderedToolCalls: [transientCopy],
    });

    expect(plan.uncoveredToolCalls).toEqual([]);
    expect(hiddenIds(plan)).toEqual(["h"]);
  });

  it("matches duplicate same-name/same-arg calls by multiplicity, not set membership", () => {
    // Two identical-fingerprint calls promoted; transient renders BOTH -> both hidden.
    const d1 = tc("d1", "read", { path: "dup.ts" });
    const d2 = tc("d2", "read", { path: "dup.ts" });

    const plan = planPromotedToolVisibility({
      promotedToolCalls: [d1, d2],
      transientRenderedToolCalls: [d1, d2],
    });

    expect(plan.uncoveredToolCalls).toEqual([]);
    expect(hiddenIds(plan)).toEqual(["d1", "d2"]);
  });

  it("REGRESSION: hides only one of two identical calls when transient renders only one", () => {
    // The subtle duplicate-fingerprint trap: set-based coverage would treat the
    // fingerprint as "covered" and hide BOTH, re-vanishing the second. Count-based
    // coverage hides exactly one and keeps the unrendered duplicate in history.
    const d1 = tc("d1", "read", { path: "dup.ts" });
    const d2 = tc("d2", "read", { path: "dup.ts" });

    const plan = planPromotedToolVisibility({
      promotedToolCalls: [d1, d2],
      transientRenderedToolCalls: [d1], // only one instance rendered
    });

    expect(plan.coveredToolCalls).toHaveLength(1);
    expect(plan.uncoveredToolCalls).toHaveLength(1);
    // Exactly one id hidden, the other preserved in history.
    expect(plan.hiddenMatchState.ids.size).toBe(1);
  });

  it("covers a parent tool call together with its nested children", () => {
    const nested = tc("n1", "read", { path: "nested.ts" });
    const parent = tc("p1", "tool_call", {}, { nestedToolCalls: [nested] });
    const transientParent = tc("p1", "tool_call", {}, { nestedToolCalls: [nested] });

    const plan = planPromotedToolVisibility({
      promotedToolCalls: [parent],
      transientRenderedToolCalls: [transientParent],
    });

    expect(plan.uncoveredToolCalls).toEqual([]);
    expect(plan.hiddenMatchState.ids.has("p1")).toBe(true);
  });

  it("screenshot scenario: a 20-call batch with repeated event functions", () => {
    // Mirrors the reported batch: many calls, several sharing a fingerprint.
    const names = ["Awake", "OnEnable", "Start", "Update", "FixedUpdate", "LateUpdate"];
    const batch: ToolCallDisplay[] = [];
    for (let i = 0; i < 20; i += 1) {
      // Deliberately reuse args so some calls share a fingerprint.
      batch.push(tc(`call-${i}`, "unity_execute", { fn: names[i % names.length] }));
    }

    // Fully covered -> all hidden (no leftover, no duplicate).
    const full = planPromotedToolVisibility({
      promotedToolCalls: batch,
      transientRenderedToolCalls: batch,
    });
    expect(full.uncoveredToolCalls).toEqual([]);
    expect(full.coveredToolCalls).toHaveLength(20);

    // Transient drops the final call -> only that one stays in history.
    const dropped = batch.slice(0, -1);
    const partial = planPromotedToolVisibility({
      promotedToolCalls: batch,
      transientRenderedToolCalls: dropped,
    });
    expect(partial.uncoveredToolCalls).toHaveLength(1);
    expect(partial.uncoveredToolCalls[0]!.id).toBe("call-19");
  });
});

describe("consumeToolCallDisplayFromMatchState", () => {
  it("consumes by id, then by remaining fingerprint instance, then fails", () => {
    const state = collectToolCallDisplayMatchState([
      tc("x", "read", { path: "f.ts" }),
      tc("y", "read", { path: "f.ts" }),
    ]);

    // First identical call: matched by id `x`.
    expect(consumeToolCallDisplayFromMatchState(tc("x", "read", { path: "f.ts" }), state)).toBe(true);
    // Second: id `z` not present, but one fingerprint instance remains.
    expect(consumeToolCallDisplayFromMatchState(tc("z", "read", { path: "f.ts" }), state)).toBe(true);
    // Third identical call: nothing left to match.
    expect(consumeToolCallDisplayFromMatchState(tc("w", "read", { path: "f.ts" }), state)).toBe(false);
  });
});
