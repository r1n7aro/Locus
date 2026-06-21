import type { ToolCallDisplay } from "../types";
import {
  cloneToolCallMatchState,
  collectToolCallDisplayMatchState,
  consumeToolCallDisplayFromMatchState,
  type ToolCallMatchState,
} from "./toolCallBatches";

/**
 * Pure decision layer for reconciling which trailing tool calls the transcript
 * shows in the settled "history" region versus the live "transient" region.
 *
 * The transcript promotes the trailing tool batch of the last assistant turn
 * out of history and into the transient region during the active->settled
 * transition. The historical failure mode ("the last message sometimes
 * disappears") happens when history hides a promoted tool call that the
 * transient region never actually renders — so it vanishes from both places.
 *
 * Keeping this reconciliation in a small, dependency-free module (rather than
 * inline in the 3k-line transcript component) makes the invariant statically
 * testable with plain data fixtures: no DOM, no timers, no component mount.
 */

/** Any object that carries a flat batch of rendered tool calls (e.g. a transient tool segment). */
export interface ToolCallSegmentLike {
  toolCalls: ToolCallDisplay[];
}

export interface PromotedToolVisibilityPlan {
  /**
   * Match state covering exactly the promoted tool calls that are provably
   * rendered in the transient region — and therefore safe to hide from history.
   */
  hiddenMatchState: ToolCallMatchState;
  /** Promoted calls confirmed rendered in transient (hidden from history). */
  coveredToolCalls: ToolCallDisplay[];
  /** Promoted calls NOT rendered in transient — kept visible in history (fail-open). */
  uncoveredToolCalls: ToolCallDisplay[];
}

/** Flatten the tool calls rendered by a set of transient tool segments. */
export function collectRenderedToolCalls(
  segments: readonly ToolCallSegmentLike[],
): ToolCallDisplay[] {
  const rendered: ToolCallDisplay[] = [];
  for (const segment of segments) {
    for (const toolCall of segment.toolCalls) {
      rendered.push(toolCall);
    }
  }
  return rendered;
}

/**
 * Fail-open reconciliation between history hiding and transient rendering.
 *
 * A promoted history tool call is hidden from history ONLY when an equivalent
 * call is provably rendered in the transient region. Equivalence is matched by
 * id first, then by a per-instance fingerprint count, so:
 *   - ids that differ across the transient->history boundary still reconcile
 *     (same name + args => same fingerprint), and
 *   - duplicate same-name/same-arg calls reconcile by multiplicity (rendering
 *     one of two identical calls hides exactly one, never both).
 *
 * Anything the transient region does not render stays in history. The trailing
 * tool batch can therefore never disappear from both regions at once: the worst
 * case is a call shown in history that could have been shown in transient — the
 * opposite of the original "blank last message" bug, and self-correcting on the
 * next frame once transient catches up.
 *
 * In the healthy path the transient region's promoted prefix contains the very
 * same promoted tool-call objects, so every promoted call is covered by id and
 * the result is identical to hiding the full promoted set.
 */
export function planPromotedToolVisibility(params: {
  promotedToolCalls: readonly ToolCallDisplay[];
  transientRenderedToolCalls: readonly ToolCallDisplay[];
}): PromotedToolVisibilityPlan {
  const { promotedToolCalls, transientRenderedToolCalls } = params;

  const coverage = cloneToolCallMatchState(
    collectToolCallDisplayMatchState([...transientRenderedToolCalls]),
  );

  const coveredToolCalls: ToolCallDisplay[] = [];
  const uncoveredToolCalls: ToolCallDisplay[] = [];
  for (const toolCall of promotedToolCalls) {
    if (consumeToolCallDisplayFromMatchState(toolCall, coverage)) {
      coveredToolCalls.push(toolCall);
    } else {
      uncoveredToolCalls.push(toolCall);
    }
  }

  return {
    hiddenMatchState: collectToolCallDisplayMatchState(coveredToolCalls),
    coveredToolCalls,
    uncoveredToolCalls,
  };
}
