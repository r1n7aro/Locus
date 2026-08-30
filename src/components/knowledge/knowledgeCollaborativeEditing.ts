import type {
  KnowledgeDocumentEditOperation,
  KnowledgeDocumentSection,
} from "../../types";
import { createBoundedTextDiffs, type TextDiff } from "../../utils/boundedTextDiff";
import { normalizeKnowledgeEditorValue } from "./knowledgeEditorDrafts";

const DIFF_DELETE = -1;
const DIFF_EQUAL = 0;
const DIFF_INSERT = 1;

export interface KnowledgeTextHunk {
  start: number;
  end: number;
  oldText: string;
  newText: string;
}

export interface KnowledgeTextConflict {
  local: KnowledgeTextHunk;
  remote: KnowledgeTextHunk;
}

export interface KnowledgeTextRebaseResult {
  text: string;
  remotePreferredText: string;
  conflicts: KnowledgeTextConflict[];
  localHunks: KnowledgeTextHunk[];
  remoteHunks: KnowledgeTextHunk[];
}

function createDiffs(base: string, next: string): TextDiff[] {
  return createBoundedTextDiffs(base, next);
}

export function buildKnowledgeTextHunks(baseValue: string, nextValue: string): KnowledgeTextHunk[] {
  const base = normalizeKnowledgeEditorValue(baseValue);
  const next = normalizeKnowledgeEditorValue(nextValue);
  if (base === next) return [];

  const hunks: KnowledgeTextHunk[] = [];
  let baseOffset = 0;
  let hunkStart: number | null = null;
  let hunkEnd = 0;
  let replacement = "";

  const flush = () => {
    if (hunkStart === null) return;
    hunks.push({
      start: hunkStart,
      end: hunkEnd,
      oldText: base.slice(hunkStart, hunkEnd),
      newText: replacement,
    });
    hunkStart = null;
    hunkEnd = 0;
    replacement = "";
  };

  for (const [operation, text] of createDiffs(base, next)) {
    if (operation === DIFF_EQUAL) {
      flush();
      baseOffset += text.length;
      continue;
    }
    if (hunkStart === null) {
      hunkStart = baseOffset;
      hunkEnd = baseOffset;
    }
    if (operation === DIFF_DELETE) {
      baseOffset += text.length;
      hunkEnd = baseOffset;
    } else if (operation === DIFF_INSERT) {
      replacement += text;
    }
  }
  flush();
  return hunks;
}

function hunksAreEqual(left: KnowledgeTextHunk, right: KnowledgeTextHunk): boolean {
  return left.start === right.start
    && left.end === right.end
    && left.newText === right.newText;
}

function hunksOverlap(left: KnowledgeTextHunk, right: KnowledgeTextHunk): boolean {
  const leftInsertion = left.start === left.end;
  const rightInsertion = right.start === right.end;
  if (leftInsertion && rightInsertion) return left.start === right.start;
  if (leftInsertion) return left.start > right.start && left.start < right.end;
  if (rightInsertion) return right.start > left.start && right.start < left.end;
  return Math.max(left.start, right.start) < Math.min(left.end, right.end);
}

function applyHunks(base: string, hunks: KnowledgeTextHunk[]): string {
  const ordered = [...hunks].sort((left, right) =>
    left.start - right.start || left.end - right.end,
  );
  let cursor = 0;
  let result = "";
  for (const hunk of ordered) {
    if (hunk.start < cursor) continue;
    result += base.slice(cursor, hunk.start);
    result += hunk.newText;
    cursor = hunk.end;
  }
  return result + base.slice(cursor);
}

export function rebaseKnowledgeText(
  baseValue: string,
  localValue: string,
  remoteValue: string,
): KnowledgeTextRebaseResult {
  const base = normalizeKnowledgeEditorValue(baseValue);
  const local = normalizeKnowledgeEditorValue(localValue);
  const remote = normalizeKnowledgeEditorValue(remoteValue);
  const localHunks = buildKnowledgeTextHunks(base, local);
  const remoteHunks = buildKnowledgeTextHunks(base, remote);
  if (!localHunks.length) {
    return { text: remote, remotePreferredText: remote, conflicts: [], localHunks, remoteHunks };
  }
  if (!remoteHunks.length) {
    return { text: local, remotePreferredText: local, conflicts: [], localHunks, remoteHunks };
  }

  const satisfiedLocalHunks = new Set<number>();
  const blockedRemoteHunks = new Set<number>();
  const conflicts: KnowledgeTextConflict[] = [];
  for (const [localIndex, localHunk] of localHunks.entries()) {
    for (const [remoteIndex, remoteHunk] of remoteHunks.entries()) {
      if (hunksAreEqual(localHunk, remoteHunk)) {
        satisfiedLocalHunks.add(localIndex);
        continue;
      }
      if (!hunksOverlap(localHunk, remoteHunk)) continue;
      blockedRemoteHunks.add(remoteIndex);
      conflicts.push({ local: localHunk, remote: remoteHunk });
    }
  }

  const mergedHunks = [
    ...localHunks.filter((_, index) => !satisfiedLocalHunks.has(index)),
    ...remoteHunks.filter((_, index) => !blockedRemoteHunks.has(index)),
  ];
  const remotePreferredHunks = [
    ...localHunks.filter((localHunk, index) =>
      !satisfiedLocalHunks.has(index)
      && !remoteHunks.some((remoteHunk) =>
        !hunksAreEqual(localHunk, remoteHunk) && hunksOverlap(localHunk, remoteHunk),
      ),
    ),
    ...remoteHunks,
  ];
  return {
    text: applyHunks(base, mergedHunks),
    remotePreferredText: applyHunks(base, remotePreferredHunks),
    conflicts,
    localHunks,
    remoteHunks,
  };
}

function occursExactlyOnceAt(content: string, target: string, expectedAt: number): boolean {
  if (!target) return false;
  return content.lastIndexOf(target, expectedAt - 1) < 0
    && content.indexOf(target, expectedAt + 1) < 0;
}

function contextualizeHunk(
  base: string,
  hunk: KnowledgeTextHunk,
): { start: number; end: number } {
  let start = hunk.start;
  let end = hunk.end;
  let oldText = base.slice(start, end);
  if (occursExactlyOnceAt(base, oldText, start)) return { start, end };

  // Rechecking the full document after every single-character expansion is
  // quadratic on repetitive long files. Exponential context growth provides
  // the same uniqueness guarantee with O(log n) full-string searches.
  let radius = 8;
  const maxContextRadius = Math.min(base.length, 4_096);
  while ((start > 0 || end < base.length) && radius <= maxContextRadius) {
    start = Math.max(0, hunk.start - radius);
    end = Math.min(base.length, hunk.end + radius);
    oldText = base.slice(start, end);
    if (occursExactlyOnceAt(base, oldText, start)) return { start, end };
    radius *= 2;
  }
  // Highly repetitive text has no compact unique guard. One whole-section
  // operation is safer than an ambiguous replacement and keeps the search
  // budget bounded for megabyte-scale documents.
  return { start: 0, end: base.length };
}

export function buildKnowledgeDocumentEditOperations(
  section: KnowledgeDocumentSection,
  baseValue: string,
  nextValue: string,
): KnowledgeDocumentEditOperation[] {
  const base = normalizeKnowledgeEditorValue(baseValue);
  const next = normalizeKnowledgeEditorValue(nextValue);
  const hunks = buildKnowledgeTextHunks(base, next);
  if (!hunks.length) return [];
  if (!base) {
    return [{
      section,
      oldString: "",
      newString: next,
      expectedEmpty: true,
    }];
  }

  const contextual = hunks.map((hunk) => contextualizeHunk(base, hunk));
  const groups: Array<{
    start: number;
    end: number;
    hunks: KnowledgeTextHunk[];
  }> = [];
  for (const [index, item] of contextual.entries()) {
    const previous = groups[groups.length - 1];
    if (previous && item.start < previous.end) {
      previous.end = Math.max(previous.end, item.end);
      previous.hunks.push(hunks[index]!);
    } else {
      groups.push({
        start: item.start,
        end: item.end,
        hunks: [hunks[index]!],
      });
    }
  }
  return groups.map((group) => {
    let cursor = group.start;
    let newString = "";
    for (const hunk of group.hunks) {
      newString += base.slice(cursor, hunk.start);
      newString += hunk.newText;
      cursor = hunk.end;
    }
    newString += base.slice(cursor, group.end);
    return {
      section,
      oldString: base.slice(group.start, group.end),
      newString,
    };
  });
}
