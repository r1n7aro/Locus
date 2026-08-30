import type { ChangeSpec } from "@codemirror/state";
import { createBoundedTextDiffs } from "../../../utils/boundedTextDiff";

const DIFF_DELETE = -1;
const DIFF_EQUAL = 0;
const DIFF_INSERT = 1;

/**
 * Build disjoint replacements that transform one document into another while
 * retaining unchanged ranges between external edits. Preserving those ranges
 * lets CodeMirror map selections and local undo history through Agent edits,
 * filesystem updates, and save acknowledgements without corrupting content.
 */
export function createMinimalTextChange(
  currentText: string,
  nextText: string,
): ChangeSpec | null {
  if (currentText === nextText) return null;

  const changes: Array<{ from: number; to: number; insert: string }> = [];
  let currentOffset = 0;
  let pendingFrom: number | null = null;
  let pendingTo = 0;
  let pendingInsert = "";

  const flushPending = () => {
    if (pendingFrom === null) return;
    changes.push({
      from: pendingFrom,
      to: pendingTo,
      insert: pendingInsert,
    });
    pendingFrom = null;
    pendingTo = 0;
    pendingInsert = "";
  };

  for (const [operation, text] of createBoundedTextDiffs(currentText, nextText)) {
    if (operation === DIFF_EQUAL) {
      flushPending();
      currentOffset += text.length;
      continue;
    }
    if (pendingFrom === null) {
      pendingFrom = currentOffset;
      pendingTo = currentOffset;
    }
    if (operation === DIFF_DELETE) {
      currentOffset += text.length;
      pendingTo = currentOffset;
    } else if (operation === DIFF_INSERT) {
      pendingInsert += text;
    }
  }
  flushPending();

  if (changes.length === 1) return changes[0];
  return changes;
}
