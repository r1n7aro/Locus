import DiffMatchPatch from "diff-match-patch";

export type TextDiffOperation = -1 | 0 | 1;
export type TextDiff = readonly [operation: TextDiffOperation, text: string];

const DIFF_DELETE: TextDiffOperation = -1;
const DIFF_EQUAL: TextDiffOperation = 0;
const DIFF_INSERT: TextDiffOperation = 1;
const SMALL_DIFF_LIMIT = 16 * 1024;
const SMALL_DIFF_TIMEOUT_SECONDS = 0.02;
const MAX_RECURSION_DEPTH = 12;
const MAX_EQUAL_LENGTH_HUNKS = 2_048;
const MIN_ANCHOR_LENGTH = 64;
const MAX_ANCHOR_LENGTH = 1_024;

function appendDiff(target: TextDiff[], operation: TextDiffOperation, text: string): void {
  if (!text) return;
  const previous = target[target.length - 1];
  if (previous?.[0] === operation) {
    target[target.length - 1] = [operation, previous[1] + text];
    return;
  }
  target.push([operation, text]);
}

function commonPrefixLength(left: string, right: string): number {
  const limit = Math.min(left.length, right.length);
  let index = 0;
  while (index < limit && left.charCodeAt(index) === right.charCodeAt(index)) index += 1;
  return index;
}

function commonSuffixLength(left: string, right: string, prefixLength: number): number {
  const limit = Math.min(left.length, right.length) - prefixLength;
  let length = 0;
  while (
    length < limit
    && left.charCodeAt(left.length - length - 1) === right.charCodeAt(right.length - length - 1)
  ) length += 1;
  return length;
}

function equalLengthDiff(left: string, right: string): TextDiff[] | null {
  if (left.length !== right.length) return null;
  const diffs: TextDiff[] = [];
  let cursor = 0;
  let changedCharacters = 0;
  let changedHunks = 0;
  while (cursor < left.length) {
    const equal = left.charCodeAt(cursor) === right.charCodeAt(cursor);
    let end = cursor + 1;
    while (
      end < left.length
      && (left.charCodeAt(end) === right.charCodeAt(end)) === equal
    ) end += 1;
    if (equal) {
      appendDiff(diffs, DIFF_EQUAL, left.slice(cursor, end));
    } else {
      changedHunks += 1;
      changedCharacters += end - cursor;
      if (changedHunks > MAX_EQUAL_LENGTH_HUNKS) return null;
      appendDiff(diffs, DIFF_DELETE, left.slice(cursor, end));
      appendDiff(diffs, DIFF_INSERT, right.slice(cursor, end));
    }
    cursor = end;
  }

  // A same-length insertion/deletion pair can shift most of the document.
  // In that case positional comparison would hide large unchanged spans, so
  // let the anchor path recover them instead.
  const acceptableChanges = Math.max(4_096, Math.floor(left.length * 0.12));
  return changedCharacters <= acceptableChanges ? diffs : null;
}

interface CommonAnchor {
  leftFrom: number;
  rightFrom: number;
  length: number;
}

function nearestAnchorOccurrence(
  content: string,
  anchor: string,
  expected: number,
  radius: number,
): number {
  const lower = Math.max(0, expected - radius);
  const upper = Math.min(content.length - anchor.length, expected + radius);
  let best = -1;
  let bestDistance = Number.POSITIVE_INFINITY;
  const consider = (position: number) => {
    if (position < lower || position > upper) return;
    const distance = Math.abs(position - expected);
    if (distance < bestDistance) {
      best = position;
      bestDistance = distance;
    }
  };

  consider(content.lastIndexOf(anchor, Math.min(expected, upper)));
  consider(content.indexOf(anchor, Math.max(expected, lower)));
  consider(content.indexOf(anchor, lower));
  if (best >= 0) return best;

  // Large insertions can move the only stable block beyond the local window.
  // One native full-string search keeps that case bounded without invoking a
  // quadratic character diff.
  return content.indexOf(anchor);
}

function findCommonAnchor(left: string, right: string): CommonAnchor | null {
  const shortest = Math.min(left.length, right.length);
  if (shortest < MIN_ANCHOR_LENGTH * 2) return null;
  const length = Math.min(
    MAX_ANCHOR_LENGTH,
    Math.max(MIN_ANCHOR_LENGTH, Math.floor(shortest / 32)),
  );
  const availableLeft = left.length - length;
  const availableRight = right.length - length;
  const fractions = [0.5, 0.25, 0.75, 0.125, 0.875, 0.375, 0.625];
  let best: CommonAnchor | null = null;
  let bestScore = Number.NEGATIVE_INFINITY;

  for (const fraction of fractions) {
    const leftFrom = Math.max(0, Math.min(availableLeft, Math.round(availableLeft * fraction)));
    const anchor = left.slice(leftFrom, leftFrom + length);
    const expected = availableLeft > 0
      ? Math.round((leftFrom / availableLeft) * Math.max(0, availableRight))
      : 0;
    const radius = Math.min(
      right.length,
      Math.max(64 * 1024, Math.abs(right.length - left.length) * 2 + length * 2),
    );
    const rightFrom = nearestAnchorOccurrence(right, anchor, expected, radius);
    if (rightFrom < 0) continue;
    const balance = Math.min(
      leftFrom,
      left.length - leftFrom - length,
      rightFrom,
      right.length - rightFrom - length,
    );
    const score = balance - Math.abs(rightFrom - expected) * 0.25;
    if (score > bestScore) {
      best = { leftFrom, rightFrom, length };
      bestScore = score;
    }
  }
  return best;
}

function appendSmallDiff(target: TextDiff[], left: string, right: string): void {
  const diff = new DiffMatchPatch();
  diff.Diff_Timeout = SMALL_DIFF_TIMEOUT_SECONDS;
  for (const [operation, text] of diff.diff_main(left, right, true)) {
    appendDiff(target, operation as TextDiffOperation, text);
  }
}

function appendBoundedDiff(
  target: TextDiff[],
  left: string,
  right: string,
  depth: number,
): void {
  if (left === right) {
    appendDiff(target, DIFF_EQUAL, left);
    return;
  }

  const prefixLength = commonPrefixLength(left, right);
  if (prefixLength > 0) appendDiff(target, DIFF_EQUAL, left.slice(0, prefixLength));
  const suffixLength = commonSuffixLength(left, right, prefixLength);
  const leftEnd = left.length - suffixLength;
  const rightEnd = right.length - suffixLength;
  const leftMiddle = left.slice(prefixLength, leftEnd);
  const rightMiddle = right.slice(prefixLength, rightEnd);

  if (!leftMiddle) appendDiff(target, DIFF_INSERT, rightMiddle);
  else if (!rightMiddle) appendDiff(target, DIFF_DELETE, leftMiddle);
  else {
    const positional = equalLengthDiff(leftMiddle, rightMiddle);
    if (positional) {
      for (const [operation, text] of positional) appendDiff(target, operation, text);
    } else if (Math.max(leftMiddle.length, rightMiddle.length) <= SMALL_DIFF_LIMIT) {
      appendSmallDiff(target, leftMiddle, rightMiddle);
    } else if (depth < MAX_RECURSION_DEPTH) {
      const anchor = findCommonAnchor(leftMiddle, rightMiddle);
      if (anchor) {
        appendBoundedDiff(
          target,
          leftMiddle.slice(0, anchor.leftFrom),
          rightMiddle.slice(0, anchor.rightFrom),
          depth + 1,
        );
        appendDiff(
          target,
          DIFF_EQUAL,
          leftMiddle.slice(anchor.leftFrom, anchor.leftFrom + anchor.length),
        );
        appendBoundedDiff(
          target,
          leftMiddle.slice(anchor.leftFrom + anchor.length),
          rightMiddle.slice(anchor.rightFrom + anchor.length),
          depth + 1,
        );
      } else {
        appendDiff(target, DIFF_DELETE, leftMiddle);
        appendDiff(target, DIFF_INSERT, rightMiddle);
      }
    } else {
      appendDiff(target, DIFF_DELETE, leftMiddle);
      appendDiff(target, DIFF_INSERT, rightMiddle);
    }
  }

  if (suffixLength > 0) appendDiff(target, DIFF_EQUAL, left.slice(leftEnd));
}

/**
 * Produces a multi-hunk diff with a bounded large-document path. Small ranges
 * retain diff-match-patch precision; large ranges use linear scans and stable
 * substring anchors, avoiding quadratic stalls on repetitive Markdown.
 */
export function createBoundedTextDiffs(left: string, right: string): TextDiff[] {
  const diffs: TextDiff[] = [];
  appendBoundedDiff(diffs, left, right, 0);
  return diffs;
}
