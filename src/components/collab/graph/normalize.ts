import { resolveHistorySelectionKind } from "../historySelection";
import { buildCommitDisplayRefsByHash } from "./refs";
import type { HistoryGraphInput, HistoryGraphScene } from "./types";

function buildPrimaryHistoryState(input: Pick<HistoryGraphInput, "commits" | "stashes">) {
  const stashHashes = new Set(input.stashes.map(stash => stash.hash));
  const stashHelperHashes = new Set(
    input.stashes.flatMap(stash =>
      stash.parentHashes.filter(parentHash => parentHash && parentHash !== stash.baseHash),
    ),
  );
  const primaryCommits = input.commits.filter(commit =>
    !commit.isStash
    && !stashHashes.has(commit.hash)
    && !stashHelperHashes.has(commit.hash),
  );
  const primaryHashes = new Set(primaryCommits.map(commit => commit.hash));

  return {
    primaryCommits,
    primaryHashes,
  };
}

function resolveLoadedHistoryFloorDate(primaryCommits: HistoryGraphScene["primaryCommits"]): number | null {
  if (primaryCommits.length === 0) {
    return null;
  }
  return primaryCommits[primaryCommits.length - 1]?.date ?? null;
}

export function collectUnanchoredStashHashes(
  input: Pick<HistoryGraphInput, "commits" | "stashes">,
): Set<string> {
  const { primaryCommits, primaryHashes } = buildPrimaryHistoryState(input);
  const loadedHistoryFloorDate = resolveLoadedHistoryFloorDate(primaryCommits);
  if (loadedHistoryFloorDate == null) {
    return new Set();
  }
  return new Set(
    input.stashes
      .filter(stash =>
        stash.date >= loadedHistoryFloorDate
        && (!stash.baseHash || !primaryHashes.has(stash.baseHash)),
      )
      .map(stash => stash.hash),
  );
}

export function normalizeHistoryGraph(input: HistoryGraphInput): HistoryGraphScene {
  const { primaryCommits, primaryHashes } = buildPrimaryHistoryState(input);
  const headHash = input.headState.hash && primaryHashes.has(input.headState.hash)
    ? input.headState.hash
    : null;
  const selectedCommitHash = input.selectedHistory?.kind === "commit" || input.selectedHistory?.kind === "stash"
    ? input.selectedHistory.hash
    : null;
  const selectionKind = resolveHistorySelectionKind(
    input.selectedHistory,
    input.commits,
    input.stashes,
    input.workspaceChangeCount > 0,
  );
  const commitRefs = buildCommitDisplayRefsByHash(input.refs, input.headState);

  const auxNodes: HistoryGraphScene["auxNodes"] = [];

  if (input.workspaceChangeCount > 0) {
    auxNodes.push({
      id: "workspace",
      kind: "workspace",
      anchorHash: headHash,
      unanchored: !headHash,
      date: Number.MAX_SAFE_INTEGER,
      hash: null,
      shortHash: "",
      label: "// WIP",
      selected: selectionKind === "workspace",
      refs: [],
      workspaceChangeCount: input.workspaceChangeCount,
    });
  }

  for (const stash of [...input.stashes].sort((a, b) => a.index - b.index)) {
    const anchorHash = stash.baseHash && primaryHashes.has(stash.baseHash) ? stash.baseHash : null;
    if (!anchorHash) {
      continue;
    }
    auxNodes.push({
      id: `stash:${stash.refName}`,
      kind: "stash",
      anchorHash,
      unanchored: false,
      date: stash.date,
      hash: stash.hash,
      shortHash: stash.shortHash,
      label: stash.message,
      selected: selectionKind === "stash" && selectedCommitHash === stash.hash,
      refs: [],
      stash,
    });
  }

  return {
    primaryCommits,
    commitRefs,
    auxNodes,
    selectionKind,
    selectedCommitHash,
    headHash,
  };
}
