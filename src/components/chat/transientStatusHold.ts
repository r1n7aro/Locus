/**
 * Live status rows ("Thinking…", "Waiting for response…", the row under a
 * running tool batch, an active thinking header) end when the backend changes
 * phase, which is often a few hundred milliseconds before the next tool block
 * or text actually renders. Removing the row on the phase change shrinks the
 * bottom of the transcript and the following output grows it back, so a
 * bottom-following transcript visibly jumps twice per tool call.
 *
 * The last displayed status is therefore kept until the transient region
 * renders an output unit (tool call or visible text) that it did not render
 * while the status was shown, or until the stream ends.
 */

export interface TransientStatusMemory<S> {
  status: S;
  /** Output units rendered while the status was last shown. */
  units: ReadonlySet<string>;
}

export interface TransientStatusResolution<S> {
  status: S | null;
  /** True when `status` is a retained status that is no longer current. */
  held: boolean;
  memory: TransientStatusMemory<S> | null;
}

function hasNewUnit(units: ReadonlySet<string>, previous: ReadonlySet<string>) {
  for (const unit of units) {
    if (!previous.has(unit)) return true;
  }
  return false;
}

export function resolveTransientStatus<S>(params: {
  /** The status the current state asks for, or null when none applies. */
  current: S | null;
  /**
   * False for statuses of a discrete operation (context compaction): once it
   * finishes, showing it longer would misreport the state.
   */
  currentRetainable?: boolean;
  /** Output units the transient region renders now. */
  units: ReadonlySet<string>;
  streaming: boolean;
  memory: TransientStatusMemory<S> | null;
}): TransientStatusResolution<S> {
  const { current, currentRetainable = true, units, streaming, memory } = params;
  if (!streaming) {
    return { status: current, held: false, memory: null };
  }
  if (current !== null) {
    return {
      status: current,
      held: false,
      memory: currentRetainable ? { status: current, units } : null,
    };
  }
  // Units that only left the transient region (moved into history) are not
  // new output; the retained set keeps them from releasing the status later.
  if (memory && !hasNewUnit(units, memory.units)) {
    return { status: memory.status, held: true, memory };
  }
  return { status: null, held: false, memory: null };
}

/**
 * A status that disappears together with the arrival of shorter output (for
 * example a 38px status row replaced by a 28px tool row) still shrinks the
 * transcript bottom by the difference. Returns the minimum height to hold so
 * the swap is not smaller than before, capped by the removed status' own
 * footprint so intentional collapses are never absorbed. Returns null when no
 * hold is needed.
 */
export function transientShrinkHoldHeight(params: {
  heightBefore: number;
  heightAfter: number;
  statusFootprint: number;
}): number | null {
  const { heightBefore, heightAfter, statusFootprint } = params;
  if (!(statusFootprint > 0)) return null;
  const deficit = Math.min(heightBefore - heightAfter, statusFootprint);
  if (!(deficit >= 1)) return null;
  return Math.ceil(heightAfter + deficit);
}
