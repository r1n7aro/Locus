import type { VcsUndoEntry, VcsCheckpoint, ChangedFile } from "../types";

// ── Types ──

export interface ChatChangeRound {
  assistantMessageId: string;
  runId: string | null;
  checkpoint: VcsCheckpoint;
  files: ChangedFile[];
}

export interface ChatMergedFileItem {
  /** Stable synthetic key (identity ID) — unique even when finalPath collides */
  id: number;
  /** Final path (after rename if any) */
  finalPath: string;
  /** Old path before rename (undefined if the file was introduced in the merged range) */
  baseOldPath?: string;
  /** assistantMessageId of the first round that touched this file */
  baseAssistantMessageId: string;
  /** Net status: A/M/D/R */
  status: string;
  /** Number of rounds that touched this file */
  roundCount: number;
}

// ── Pure functions ──

/**
 * Build per-round file change lists from undo entries.
 * Entries are sorted by checkpoint.createdAt ascending.
 */
export function buildRounds(entries: VcsUndoEntry[]): ChatChangeRound[] {
  return [...entries]
    .sort((a, b) => a.checkpoint.createdAt - b.checkpoint.createdAt)
    .map((e) => ({
      assistantMessageId: e.assistantMessageId,
      runId: e.runId ?? null,
      checkpoint: e.checkpoint,
      files: e.changedFiles,
    }));
}

/**
 * Internal identity tracking for file merge across rounds.
 *
 * Tracks a file's lifecycle through renames, modifications, and deletions
 * using semantic state bits rather than first/last status inference.
 */
interface FileIdentity {
  /** Current path (updated on rename) */
  finalPath: string;
  /** Original path before any rename in the merged range (undefined if introduced) */
  originPath?: string;
  /** assistantMessageId of the first round that touched this file */
  baseAssistantMessageId: string;
  /** True if the file was first introduced within the merged rounds (Add) */
  introduced: boolean;
  /** True if the file was renamed at least once */
  sawRename: boolean;
  /** True if the file currently exists (false after Delete) */
  existsNow: boolean;
  /** Number of rounds that touched this file */
  roundCount: number;
}

/**
 * Derive net status from identity state bits.
 *
 * Rules:
 * - introduced && !existsNow → no-op (filter out)
 * - introduced && existsNow  → "A" (net new file)
 * - !existsNow               → "D" (deleted)
 * - sawRename                → "R" (renamed)
 * - else                     → "M" (modified)
 */
function deriveNetStatus(id: FileIdentity): string | null {
  if (id.introduced && !id.existsNow) return null; // no-op
  if (id.introduced) return "A";
  if (!id.existsNow) return "D";
  if (id.sawRename) return "R";
  return "M";
}

/**
 * Merge a set of rounds into a deduplicated net-change file list.
 *
 * Shared by both changes-panel tabs: "all changes" merges every round of the
 * conversation, "current turn" merges only the latest run's rounds. Statuses
 * are net relative to the first merged round's checkpoint, matching the
 * chatCheckpoint diff anchor.
 *
 * Algorithm:
 * 1. Rounds sorted by checkpoint.createdAt ascending.
 * 2. Each file is tracked by an identity ID, mapped from its current path.
 * 3. Within each round, renames are processed first to avoid ordering issues.
 * 4. Rename chains are followed: a→b then b→c merges into a→c.
 * 5. Net status is derived from (introduced, sawRename, existsNow).
 * 6. Files added then deleted within the merged range are excluded.
 */
export function mergeRoundFiles(rounds: ChatChangeRound[]): ChatMergedFileItem[] {
  const sorted = [...rounds].sort(
    (a, b) => a.checkpoint.createdAt - b.checkpoint.createdAt,
  );

  let nextId = 0;
  const identities = new Map<number, FileIdentity>();
  // Reverse index: current file path → identity ID
  const pathToId = new Map<string, number>();

  for (const round of sorted) {
    // Process renames first within each round to avoid ordering issues
    const renames: ChangedFile[] = [];
    const others: ChangedFile[] = [];
    for (const f of round.files) {
      if (f.oldPath) {
        renames.push(f);
      } else {
        others.push(f);
      }
    }

    // --- Renames ---
    for (const f of renames) {
      const existingId = pathToId.get(f.oldPath!);
      if (existingId !== undefined) {
        // Known file being renamed again — follow the chain
        const identity = identities.get(existingId)!;
        // Record originPath on first rename (only for pre-existing files)
        if (!identity.introduced && identity.originPath === undefined) {
          identity.originPath = f.oldPath;
        }
        identity.finalPath = f.path;
        identity.sawRename = true;
        identity.existsNow = true;
        identity.roundCount++;
        pathToId.delete(f.oldPath!);
        pathToId.set(f.path, existingId);
      } else {
        // First appearance is a rename (pre-existing file)
        const id = nextId++;
        identities.set(id, {
          finalPath: f.path,
          originPath: f.oldPath,
          baseAssistantMessageId: round.assistantMessageId,
          introduced: false,
          sawRename: true,
          existsNow: true,
          roundCount: 1,
        });
        pathToId.set(f.path, id);
      }
    }

    // --- Non-renames (A, M, D) ---
    for (const f of others) {
      const existingId = pathToId.get(f.path);
      if (existingId !== undefined) {
        const identity = identities.get(existingId)!;
        if (f.status === "D") {
          identity.existsNow = false;
        } else if (f.status === "A") {
          // Re-creation of a previously deleted file at the same path
          identity.existsNow = true;
        } else {
          identity.existsNow = true;
        }
        identity.roundCount++;
      } else {
        const id = nextId++;
        identities.set(id, {
          finalPath: f.path,
          originPath: undefined,
          baseAssistantMessageId: round.assistantMessageId,
          introduced: f.status === "A",
          sawRename: false,
          existsNow: f.status !== "D",
          roundCount: 1,
        });
        pathToId.set(f.path, id);
      }
    }
  }

  // Build output
  const result: ChatMergedFileItem[] = [];
  for (const [id, identity] of identities) {
    const netStatus = deriveNetStatus(identity);
    if (netStatus === null) continue; // no-op

    result.push({
      id,
      finalPath: identity.finalPath,
      // Introduced files have no "old path" since they didn't exist before
      baseOldPath: identity.introduced ? undefined : identity.originPath,
      baseAssistantMessageId: identity.baseAssistantMessageId,
      status: netStatus,
      roundCount: identity.roundCount,
    });
  }

  return result;
}

/**
 * Merge all rounds into a deduplicated file list for the "all changes" view.
 */
export function buildMergedFiles(entries: VcsUndoEntry[]): ChatMergedFileItem[] {
  return mergeRoundFiles(buildRounds(entries));
}
