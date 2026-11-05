import { langFromPath } from "../hljs";
import { diffStrings } from "../services/diff";
import type { DiffHunk, FileDiffPayload } from "../types";

export interface EditDiffSource {
  filePath: string;
  oldStr: string;
  newStr: string;
  /** Position of the edit inside its tool call; becomes the payload key. */
  index: number;
}

interface EditDiffPayloadEntry {
  payload: FileDiffPayload | null;
  promise: Promise<FileDiffPayload | null>;
}

// Edit tool blocks are rebuilt whenever their transcript group re-renders or
// moves from the live stream into history. Sharing payloads by content makes
// such a rebuild synchronous and keeps the payload object identical, so the
// diff viewer does not fall back, reload or collapse while the session runs.
const EDIT_DIFF_PAYLOAD_CACHE_LIMIT = 128;
const editDiffPayloadCache = new Map<string, EditDiffPayloadEntry>();

function editDiffPayloadCacheKey(source: EditDiffSource) {
  return [source.index, source.filePath, source.oldStr.length, source.oldStr, source.newStr].join("\u0000");
}

function touch(key: string, entry: EditDiffPayloadEntry) {
  editDiffPayloadCache.delete(key);
  editDiffPayloadCache.set(key, entry);
}

function evictOverflow() {
  while (editDiffPayloadCache.size > EDIT_DIFF_PAYLOAD_CACHE_LIMIT) {
    const oldest = editDiffPayloadCache.keys().next().value;
    if (oldest === undefined) return;
    editDiffPayloadCache.delete(oldest);
  }
}

function buildEditDiffPayload(source: EditDiffSource, hunks: DiffHunk[]): FileDiffPayload {
  let additions = 0;
  let deletions = 0;
  for (const hunk of hunks) {
    for (const line of hunk.lines) {
      if (line.kind === "add") additions += 1;
      else if (line.kind === "delete") deletions += 1;
    }
  }
  return {
    key: `edit-${source.index}`,
    filePath: source.filePath,
    status: "M",
    language: langFromPath(source.filePath) || undefined,
    isBinary: false,
    isLarge: false,
    contentState: { type: "normal" },
    stats: { additions, deletions, changedHunks: hunks.length },
    previewSummary: [`+${additions} -${deletions}`],
    text: { hunks },
  };
}

/** Returns the computed payload for this edit when it is already available. */
export function peekEditDiffPayload(source: EditDiffSource): FileDiffPayload | null {
  const key = editDiffPayloadCacheKey(source);
  const entry = editDiffPayloadCache.get(key);
  if (!entry?.payload) return null;
  touch(key, entry);
  return entry.payload;
}

/**
 * Computes the diff payload for one edit, sharing in-flight and completed
 * results by content. Resolves to null when the backend diff fails so callers
 * keep their plain old/new rendering; failures are not cached.
 */
export function loadEditDiffPayload(source: EditDiffSource): Promise<FileDiffPayload | null> {
  const key = editDiffPayloadCacheKey(source);
  const existing = editDiffPayloadCache.get(key);
  if (existing) {
    touch(key, existing);
    return existing.promise;
  }

  const entry: EditDiffPayloadEntry = {
    payload: null,
    promise: Promise.resolve(null),
  };
  entry.promise = diffStrings(source.oldStr, source.newStr, 3).then(
    (hunks) => {
      const payload = buildEditDiffPayload(source, hunks);
      entry.payload = payload;
      return payload;
    },
    () => {
      if (editDiffPayloadCache.get(key) === entry) editDiffPayloadCache.delete(key);
      return null;
    },
  );
  editDiffPayloadCache.set(key, entry);
  evictOverflow();
  return entry.promise;
}

/** Edits are equal for diff purposes when every edited text matches. */
export function sameEditDiffSources(a: readonly EditDiffSource[], b: readonly EditDiffSource[]) {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let index = 0; index < a.length; index += 1) {
    const left = a[index]!;
    const right = b[index]!;
    if (
      left.index !== right.index
      || left.filePath !== right.filePath
      || left.oldStr !== right.oldStr
      || left.newStr !== right.newStr
    ) return false;
  }
  return true;
}

export function resetEditDiffPayloadCacheForTest() {
  editDiffPayloadCache.clear();
}
