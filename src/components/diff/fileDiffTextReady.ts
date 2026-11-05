import type { TextDiff } from "../../types";

/**
 * Text diffs at or below this many lines render in the mounting frame. The
 * deferred "Loading…" gate only helps when rendering takes long enough for a
 * placeholder to be seen; for small diffs it just collapses the viewer for a
 * frame and shifts everything around it.
 */
export const SYNC_TEXT_RENDER_LINE_LIMIT = 200;

// Text diffs that have already been rendered by some viewer. A viewer that is
// remounted with the same diff (a transcript re-render, a pane move) renders
// it immediately instead of collapsing to the loading placeholder again.
const renderedTextDiffs = new WeakSet<TextDiff>();

export function canRenderDiffTextImmediately(text: TextDiff | null | undefined, lineCount: number): boolean {
  if (!text) return true;
  return lineCount <= SYNC_TEXT_RENDER_LINE_LIMIT || renderedTextDiffs.has(text);
}

export function markDiffTextRendered(text: TextDiff | null | undefined) {
  if (text) renderedTextDiffs.add(text);
}
