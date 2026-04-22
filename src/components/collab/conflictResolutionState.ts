import type { UnmergedFileEntry } from "../../types";

type ConflictIdentity = Pick<
  UnmergedFileEntry,
  "path" | "conflictCode" | "baseOid" | "leftOid" | "rightOid"
>;

export function buildConflictResolutionKey(file: ConflictIdentity): string {
  return `${file.path}:${file.conflictCode}:${file.baseOid}:${file.leftOid}:${file.rightOid}`;
}

export function prunePendingConflictResolutionKeys(
  pendingKeys: ReadonlySet<string>,
  unmergedFiles: readonly ConflictIdentity[],
): Set<string> {
  if (pendingKeys.size === 0) return new Set();

  const liveKeys = new Set(unmergedFiles.map(buildConflictResolutionKey));
  return new Set(
    [...pendingKeys].filter((key) => liveKeys.has(key)),
  );
}
