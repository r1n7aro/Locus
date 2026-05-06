import { estimateDisplayRefsRailWidth } from "./refs";
import type {
  HistoryGraphAuxLayout,
  HistoryGraphColumnWidthOverrides,
  HistoryGraphCommitLayout,
  HistoryGraphDisplayRef,
  HistoryGraphEdgeLayout,
  HistoryGraphLaneEdge,
  HistoryGraphLayoutResult,
  HistoryGraphResizableColumn,
  HistoryGraphRowLayout,
  HistoryGraphScene,
} from "./types";

const TRACK_COLORS = [
  "#29b7d6",
  "#d05cff",
  "#f28e2c",
  "#76b7b2",
  "#59a14f",
  "#edc949",
  "#e15759",
  "#af7aa1",
  "#9c755f",
  "#bab0ab",
];

const ROW_HEIGHT = 34;
const TOP_PAD = 8;
const BOTTOM_PAD = 14;
const GRAPH_PAD_X = 14;
const LANE_W = 22;
const AUX_LANE = 0;
const COMMIT_LANE_OFFSET = 1;
const DEFAULT_MESSAGE_W = 360;
const DEFAULT_META_W = 264;
const DEFAULT_REFS_W_PADDING = 12;

const HISTORY_GRAPH_COLUMN_LIMITS: Record<HistoryGraphResizableColumn, { min: number; max: number }> = {
  refsWidth: { min: 88, max: 560 },
  messageWidth: { min: 240, max: 1200 },
  metaWidth: { min: 180, max: 520 },
};

export function clampHistoryGraphColumnWidth(column: HistoryGraphResizableColumn, width: number): number {
  const limits = HISTORY_GRAPH_COLUMN_LIMITS[column];
  if (!Number.isFinite(width)) return limits.min;
  return Math.min(limits.max, Math.max(limits.min, Math.round(width)));
}

interface LaneRow {
  commitHash: string;
  lane: number;
  color: string;
  downEdges: HistoryGraphLaneEdge[];
}

interface DisplayOwner {
  key: string;
  color: string;
  priority: number;
  claimCount: number;
  oldestRowIndex: number;
  isPrimaryBranch: boolean;
}

interface ActiveLane {
  hash: string;
  track: number;
  owner?: DisplayOwner;
}

interface PendingLane extends ActiveLane {
  sourceLane?: number;
  sourceColor?: string;
}

interface RenderLane extends ActiveLane {
  /**
   * For real tracks, realTrack === track.
   * For preview placeholders, realTrack === null.
   */
  realTrack: number | null;
}

interface RenderSnapshot {
  laneByTrack: Map<number, number>;
}

interface MergeTarget {
  fromLane: number;
  track: number;
  color: string;
}

type SparseLane<T extends ActiveLane> = T | null;

function trackColor(track: number): string {
  return TRACK_COLORS[track % TRACK_COLORS.length];
}

function graphLaneX(lane: number): number {
  return GRAPH_PAD_X + lane * LANE_W + LANE_W / 2;
}

function curvePath(x1: number, y1: number, x2: number, y2: number): string {
  if (x1 === x2) {
    return `M${x1},${y1}L${x2},${y2}`;
  }
  const midY = (y1 + y2) / 2;
  return `M${x1},${y1}C${x1},${midY} ${x2},${midY} ${x2},${y2}`;
}

function fallbackOwner(key: string, color: string): DisplayOwner {
  return {
    key,
    color,
    priority: Number.MAX_SAFE_INTEGER,
    claimCount: 0,
    oldestRowIndex: -1,
    isPrimaryBranch: false,
  };
}

function resolveCommitOwner(
  hash: string,
  parentHashes: string[],
  laneOwners: Map<string, DisplayOwner>,
  fallbackColor: string,
): DisplayOwner {
  const directOwner = laneOwners.get(hash);
  if (directOwner) return directOwner;

  const firstParentOwner = parentHashes[0] ? laneOwners.get(parentHashes[0]) : undefined;
  if (firstParentOwner) return firstParentOwner;

  return fallbackOwner(`commit:${hash}`, fallbackColor);
}

function makeRenderPlaceholderTrack(nextTrackId: number): number {
  return Number.MAX_SAFE_INTEGER - nextTrackId;
}

function hasVisibleHistory(owner: DisplayOwner): boolean {
  return owner.claimCount > 0;
}

function compareOwnerPriority(left: DisplayOwner | undefined, right: DisplayOwner | undefined): number {
  if (!left && !right) return 0;
  if (!left) return 1;
  if (!right) return -1;

  const visibleDelta = Number(hasVisibleHistory(right)) - Number(hasVisibleHistory(left));
  if (visibleDelta !== 0) return visibleDelta;

  const primaryDelta = Number(right.isPrimaryBranch) - Number(left.isPrimaryBranch);
  if (primaryDelta !== 0) return primaryDelta;

  if (left.claimCount !== right.claimCount) return right.claimCount - left.claimCount;
  if (left.oldestRowIndex !== right.oldestRowIndex) return right.oldestRowIndex - left.oldestRowIndex;
  if (left.priority !== right.priority) return left.priority - right.priority;
  return left.key.localeCompare(right.key);
}

function ownerWins(candidate: DisplayOwner | undefined, current: DisplayOwner | undefined): boolean {
  if (!candidate) return false;
  if (!current) return true;
  return compareOwnerPriority(candidate, current) < 0;
}

function colorOwnerWins(candidate: DisplayOwner | undefined, current: DisplayOwner | undefined): boolean {
  if (!candidate) return false;
  if (!current) return true;
  if (candidate.priority !== current.priority) return candidate.priority < current.priority;
  if (candidate.claimCount !== current.claimCount) return candidate.claimCount > current.claimCount;
  return candidate.key.localeCompare(current.key) < 0;
}

function compareActiveLanes(left: ActiveLane, right: ActiveLane): number {
  const ownerDelta = compareOwnerPriority(left.owner, right.owner);
  if (ownerDelta !== 0) return ownerDelta;
  if (left.owner?.key && left.owner.key === right.owner?.key) {
    return right.track - left.track;
  }
  return left.track - right.track;
}

function findLaneIndexByHash<T extends ActiveLane>(lanes: SparseLane<T>[], hash: string): number {
  return lanes.findIndex(activeLane => activeLane?.hash === hash);
}

function trimTrailingEmptyLanes<T extends ActiveLane>(lanes: SparseLane<T>[]) {
  while (lanes.length > 0 && !lanes[lanes.length - 1]) {
    lanes.pop();
  }
}

function insertLaneByPriority<T extends ActiveLane>(lanes: SparseLane<T>[], lane: T): number {
  trimTrailingEmptyLanes(lanes);

  const activeIndexes: number[] = [];
  for (let index = 0; index < lanes.length; index++) {
    if (lanes[index]) {
      activeIndexes.push(index);
    }
  }

  let insertRank = activeIndexes.findIndex(index => compareActiveLanes(lane, lanes[index]!) < 0);
  if (insertRank === -1) {
    insertRank = activeIndexes.length;
  }

  const previousActiveIndex = insertRank === 0 ? -1 : activeIndexes[insertRank - 1]!;
  const nextActiveIndex = insertRank === activeIndexes.length ? lanes.length : activeIndexes[insertRank]!;

  for (let index = previousActiveIndex + 1; index < nextActiveIndex; index++) {
    if (!lanes[index]) {
      lanes[index] = lane;
      return index;
    }
  }

  lanes.splice(nextActiveIndex, 0, lane);
  return nextActiveIndex;
}

function shouldCurrentLaneClaimParent(
  candidate: DisplayOwner | undefined,
  current: DisplayOwner | undefined,
  candidateLane: number,
  currentLane: number,
): boolean {
  const ownerDelta = compareOwnerPriority(candidate, current);
  if (ownerDelta !== 0) {
    return ownerDelta < 0;
  }
  return candidateLane < currentLane;
}

function resolveUpcomingPreviewCommit(
  scene: HistoryGraphScene,
  commitIndex: number,
): HistoryGraphScene["primaryCommits"][number] | null {
  const commit = scene.primaryCommits[commitIndex];
  const upcomingCommit = scene.primaryCommits[commitIndex + 1];
  if (!upcomingCommit) return null;

  if (upcomingCommit.hash === commit.parents[0]) {
    return null;
  }

  return upcomingCommit;
}

function buildRenderSnapshot<T extends ActiveLane>(
  lanes: SparseLane<T>[],
  previewCommit: HistoryGraphScene["primaryCommits"][number] | null,
  laneOwners: Map<string, DisplayOwner>,
  nextTrackId: number,
): RenderSnapshot {
  const renderLanes: SparseLane<RenderLane>[] = lanes.map(activeLane => activeLane
    ? {
      hash: activeLane.hash,
      track: activeLane.track,
      owner: activeLane.owner,
      realTrack: activeLane.track,
    }
    : null);

  if (previewCommit && findLaneIndexByHash(renderLanes, previewCommit.hash) < 0) {
    insertLaneByPriority(renderLanes, {
      hash: previewCommit.hash,
      track: makeRenderPlaceholderTrack(nextTrackId),
      owner: resolveCommitOwner(
        previewCommit.hash,
        previewCommit.parents,
        laneOwners,
        trackColor(nextTrackId),
      ),
      realTrack: null,
    });
  }

  const laneByTrack = new Map<number, number>();
  for (let index = 0; index < renderLanes.length; index++) {
    const activeLane = renderLanes[index];
    if (!activeLane || activeLane.realTrack === null) continue;
    laneByTrack.set(activeLane.realTrack, index);
  }

  return { laneByTrack };
}

function buildLaneRows(
  scene: HistoryGraphScene,
  laneOwners: Map<string, DisplayOwner>,
  commitColors: Map<string, string>,
): LaneRow[] {
  const rows: LaneRow[] = [];
  let lanes: SparseLane<ActiveLane>[] = [];
  let nextTrackId = 0;

  for (let commitIndex = 0; commitIndex < scene.primaryCommits.length; commitIndex++) {
    const commit = scene.primaryCommits[commitIndex];
    let lane = findLaneIndexByHash(lanes, commit.hash);
    if (lane === -1) {
      const track = nextTrackId++;
      lane = insertLaneByPriority(lanes, {
        hash: commit.hash,
        track,
        owner: resolveCommitOwner(commit.hash, commit.parents, laneOwners, trackColor(track)),
      });
    }

    const currentLane = lanes[lane]!;
    const currentOwner = laneOwners.get(commit.hash)
      ?? currentLane.owner
      ?? resolveCommitOwner(commit.hash, commit.parents, laneOwners, trackColor(currentLane.track));
    const previewCommit = resolveUpcomingPreviewCommit(scene, commitIndex);
    const renderBefore = buildRenderSnapshot(
      lanes,
      previewCommit,
      laneOwners,
      nextTrackId,
    );
    const renderLane = renderBefore.laneByTrack.get(currentLane.track) ?? lane;
    const rowColor = commitColors.get(commit.hash) ?? currentOwner.color;
    const nextLanes: SparseLane<PendingLane>[] = lanes.map((activeLane, index) => activeLane
      ? {
        ...activeLane,
        sourceLane: renderBefore.laneByTrack.get(activeLane.track) ?? index,
        sourceColor: activeLane.owner?.color
          ?? trackColor(activeLane.track),
      }
      : null);
    nextLanes[lane] = null;
    const mergeTargetTracks: MergeTarget[] = [];

    for (let index = 0; index < commit.parents.length; index++) {
      const parentHash = commit.parents[index];
      const existingLane = findLaneIndexByHash(nextLanes, parentHash);
      if (existingLane >= 0) {
        const existingPendingLane = nextLanes[existingLane]!;
        if (index === 0 && shouldCurrentLaneClaimParent(currentOwner, existingPendingLane.owner, lane, existingLane)) {
          nextLanes[lane] = {
            hash: parentHash,
            track: currentLane.track,
            owner: currentOwner,
            sourceLane: renderLane,
            sourceColor: rowColor,
          };
          nextLanes[existingLane] = null;
          if (existingPendingLane.sourceLane !== undefined) {
            mergeTargetTracks.push({
              fromLane: existingPendingLane.sourceLane,
              track: currentLane.track,
              color: existingPendingLane.owner?.color
                ?? commitColors.get(existingPendingLane.hash)
                ?? trackColor(existingPendingLane.track),
            });
          }
          continue;
        }

        mergeTargetTracks.push({
          fromLane: renderLane,
          track: existingPendingLane.track,
          color: rowColor,
        });
        continue;
      }

      if (index === 0) {
        nextLanes[lane] = {
          hash: parentHash,
          track: currentLane.track,
          owner: currentOwner,
          sourceLane: renderLane,
          sourceColor: rowColor,
        };
        continue;
      }

      const track = nextTrackId++;
      insertLaneByPriority(nextLanes, {
        hash: parentHash,
        track,
        owner: laneOwners.get(parentHash) ?? fallbackOwner(`commit:${parentHash}`, trackColor(track)),
        sourceLane: renderLane,
        sourceColor: rowColor,
      });
    }

    trimTrailingEmptyLanes(nextLanes);
    const renderAfter = buildRenderSnapshot(
      nextLanes,
      previewCommit,
      laneOwners,
      nextTrackId,
    );
    const downEdges: HistoryGraphLaneEdge[] = [];
    for (let index = 0; index < nextLanes.length; index++) {
      const activeLane = nextLanes[index];
      if (!activeLane || activeLane.sourceLane === undefined) continue;
      downEdges.push({
        fromLane: activeLane.sourceLane + COMMIT_LANE_OFFSET,
        toLane: (renderAfter.laneByTrack.get(activeLane.track) ?? index) + COMMIT_LANE_OFFSET,
        color: activeLane.sourceColor
          ?? commitColors.get(activeLane.hash)
          ?? activeLane.owner?.color
          ?? trackColor(activeLane.track),
        kind: "continuation",
      });
    }

    for (const mergeTarget of mergeTargetTracks) {
      const mergeLane = renderAfter.laneByTrack.get(mergeTarget.track);
      if (mergeLane === undefined) continue;
      downEdges.push({
        fromLane: mergeTarget.fromLane + COMMIT_LANE_OFFSET,
        toLane: mergeLane + COMMIT_LANE_OFFSET,
        color: mergeTarget.color,
        kind: "merge",
      });
    }

    rows.push({
      commitHash: commit.hash,
      lane: renderLane + COMMIT_LANE_OFFSET,
      color: rowColor,
      downEdges,
    });

    lanes = nextLanes.map(activeLane => activeLane
      ? {
        hash: activeLane.hash,
        track: activeLane.track,
        owner: activeLane.owner,
      }
      : null);
  }

  return rows;
}

interface DisplayColorCandidate {
  hash: string;
  key: string;
  text: string;
  priority: number;
}

interface DisplayColorState {
  laneOwners: Map<string, DisplayOwner>;
  commitColors: Map<string, string>;
}

function displayRefPriority(ref: HistoryGraphDisplayRef): number {
  switch (ref.kind) {
    case "branch":
      return ref.isCurrent ? 0 : 1;
    case "remote":
      return 2;
    case "tag":
      return 3;
    default:
      return 4;
  }
}

function stableColorSeed(key: string): number {
  let hash = 2166136261;
  for (let index = 0; index < key.length; index++) {
    hash ^= key.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function isPrimaryBranchName(name: string): boolean {
  const normalized = name.trim().toLowerCase();
  return normalized === "main" || normalized === "master";
}

function measureClaimedCommits(
  startHash: string,
  commitByHash: Map<string, HistoryGraphScene["primaryCommits"][number]>,
  rowIndexByHash: Map<string, number>,
): { claimCount: number; oldestRowIndex: number } {
  let count = 0;
  let oldestRowIndex = -1;
  let hash: string | undefined = startHash;
  while (hash) {
    const commit = commitByHash.get(hash);
    if (!commit) break;
    count += 1;
    oldestRowIndex = Math.max(oldestRowIndex, rowIndexByHash.get(hash) ?? -1);
    hash = commit.parents[0];
  }
  return { claimCount: count, oldestRowIndex };
}

function buildCommitDisplayColors(scene: HistoryGraphScene): DisplayColorState {
  const commitByHash = new Map(scene.primaryCommits.map(commit => [commit.hash, commit]));
  const rowIndexByHash = new Map(scene.primaryCommits.map((commit, index) => [commit.hash, index]));
  const colorOwnerByHash = new Map<string, DisplayOwner>();
  const laneOwnerByHash = new Map<string, DisplayOwner>();
  const colorByHash = new Map<string, string>();
  const candidates: DisplayColorCandidate[] = [];
  const seenRefKeys = new Set<string>();

  for (const [hash, refs] of Object.entries(scene.commitRefs)) {
    for (const ref of refs) {
      if (ref.kind !== "branch" && ref.kind !== "remote") continue;
      if (seenRefKeys.has(ref.key)) continue;
      seenRefKeys.add(ref.key);
      candidates.push({
        hash,
        key: ref.key,
        text: ref.text,
        priority: displayRefPriority(ref),
      });
    }
  }

  const ownerByKey = new Map<string, DisplayOwner>();
  const candidateStats = new Map<string, { claimCount: number; oldestRowIndex: number }>();
  for (const candidate of candidates) {
    candidateStats.set(candidate.key, measureClaimedCommits(candidate.hash, commitByHash, rowIndexByHash));
  }

  const colorCandidates = [...candidates].sort((left, right) => {
    const priorityDelta = left.priority - right.priority;
    if (priorityDelta !== 0) return priorityDelta;
    return left.key.localeCompare(right.key);
  });

  for (let index = 0; index < colorCandidates.length; index++) {
    const candidate = colorCandidates[index];
    const stats = candidateStats.get(candidate.key)!;
    ownerByKey.set(candidate.key, {
      key: candidate.key,
      color: trackColor(index),
      priority: candidate.priority,
      claimCount: stats.claimCount,
      oldestRowIndex: stats.oldestRowIndex,
      isPrimaryBranch: isPrimaryBranchName(candidate.text),
    });
  }

  for (const candidate of candidates) {
    const owner = ownerByKey.get(candidate.key)!;
    let hash: string | undefined = candidate.hash;
    while (hash) {
      const currentColorOwner = colorOwnerByHash.get(hash);
      if (!colorOwnerWins(owner, currentColorOwner)) break;
      colorOwnerByHash.set(hash, owner);
      hash = commitByHash.get(hash)?.parents[0];
    }
  }

  for (const candidate of candidates) {
    const owner = ownerByKey.get(candidate.key)!;
    let hash: string | undefined = candidate.hash;
    while (hash) {
      const currentLaneOwner = laneOwnerByHash.get(hash);
      if (!ownerWins(owner, currentLaneOwner)) break;
      laneOwnerByHash.set(hash, owner);
      hash = commitByHash.get(hash)?.parents[0];
    }
  }

  for (const [hash, owner] of colorOwnerByHash.entries()) {
    colorByHash.set(hash, owner.color);
  }

  return {
    laneOwners: laneOwnerByHash,
    commitColors: colorByHash,
  };
}

function pickDistinctColor(seedKey: string, forbidden: Set<string>): string {
  const startTrack = stableColorSeed(seedKey) % TRACK_COLORS.length;
  for (let offset = 0; offset < TRACK_COLORS.length; offset++) {
    const track = startTrack + offset;
    const color = trackColor(track);
    if (!forbidden.has(color)) {
      return color;
    }
  }
  return trackColor(startTrack);
}

function buildAuxDisplayColors(
  scene: HistoryGraphScene,
  commitColors: Map<string, string>,
): Map<string, string> {
  const auxColors = new Map<string, string>();

  for (const auxNode of sortAuxNodes(scene.auxNodes)) {
    if (auxNode.kind !== "stash") continue;
    const forbidden = new Set<string>();
    if (auxNode.anchorHash) {
      const anchorColor = commitColors.get(auxNode.anchorHash);
      if (anchorColor) forbidden.add(anchorColor);
    }
    auxColors.set(auxNode.id, pickDistinctColor(auxNode.id, forbidden));
  }

  return auxColors;
}

function occupyLaneRange(occupied: Set<number>, fromLane: number, toLane: number) {
  const start = Math.min(fromLane, toLane);
  const end = Math.max(fromLane, toLane);
  for (let lane = start; lane <= end; lane++) {
    occupied.add(lane);
  }
}

function buildAnchoredAuxLaneReservations(
  scene: HistoryGraphScene,
  laneRows: LaneRow[],
): Map<string, Set<number>> {
  const reservations = new Map<string, Set<number>>();

  for (let index = 0; index < scene.primaryCommits.length; index++) {
    const commit = scene.primaryCommits[index];
    const occupied = new Set<number>([laneRows[index]?.lane ?? COMMIT_LANE_OFFSET]);
    const previousLaneRow = index > 0 ? laneRows[index - 1] : null;

    if (previousLaneRow) {
      for (const edge of previousLaneRow.downEdges) {
        occupyLaneRange(occupied, edge.fromLane, edge.toLane);
      }
    }

    reservations.set(commit.hash, occupied);
  }

  return reservations;
}

function pickNearestOpenLane(anchorLane: number, occupied: Set<number>): number {
  for (let distance = 1; distance <= occupied.size + anchorLane + 2; distance++) {
    const leftLane = anchorLane - distance;
    if (leftLane >= AUX_LANE && !occupied.has(leftLane)) {
      return leftLane;
    }

    const rightLane = anchorLane + distance;
    if (!occupied.has(rightLane)) {
      return rightLane;
    }
  }

  return anchorLane + occupied.size + 1;
}

function auxSortWeight(kind: HistoryGraphAuxLayout["kind"]): number {
  return kind === "workspace" ? 0 : 1;
}

function sortAuxNodes(nodes: HistoryGraphScene["auxNodes"]): HistoryGraphScene["auxNodes"] {
  return [...nodes].sort((left, right) => {
    const kindDelta = auxSortWeight(left.kind) - auxSortWeight(right.kind);
    if (kindDelta !== 0) return kindDelta;
    if (left.kind === "stash" && right.kind === "stash") {
      return (left.stash?.index ?? 0) - (right.stash?.index ?? 0);
    }
    return left.id.localeCompare(right.id);
  });
}

export function layoutHistoryGraph(
  scene: HistoryGraphScene,
  overrides: HistoryGraphColumnWidthOverrides = {},
): HistoryGraphLayoutResult {
  const displayState = buildCommitDisplayColors(scene);
  const laneRows = buildLaneRows(scene, displayState.laneOwners, displayState.commitColors);
  const commitLaneByHash = new Map(laneRows.map(row => [row.commitHash, row]));
  const auxLaneReservations = buildAnchoredAuxLaneReservations(scene, laneRows);
  const auxDisplayColors = buildAuxDisplayColors(scene, displayState.commitColors);
  const anchoredAuxByHash = new Map<string, HistoryGraphScene["auxNodes"]>();
  const topAuxNodes = sortAuxNodes(
    scene.auxNodes.filter(node => node.kind === "workspace" || node.unanchored),
  );

  for (const auxNode of scene.auxNodes) {
    if (auxNode.kind === "workspace") continue;
    if (!auxNode.anchorHash) continue;
    const list = anchoredAuxByHash.get(auxNode.anchorHash) ?? [];
    list.push(auxNode);
    anchoredAuxByHash.set(auxNode.anchorHash, sortAuxNodes(list));
  }

  const commits: HistoryGraphCommitLayout[] = [];
  const auxNodes: HistoryGraphAuxLayout[] = [];
  const rows: HistoryGraphRowLayout[] = [];

  const pushAux = (auxNode: HistoryGraphScene["auxNodes"][number]) => {
    const anchorLaneRow = auxNode.anchorHash ? commitLaneByHash.get(auxNode.anchorHash) : undefined;
    const anchorLane = anchorLaneRow?.lane;
    const reservedLanes = auxNode.anchorHash ? auxLaneReservations.get(auxNode.anchorHash) : undefined;
    const rowIndex = rows.length;
    const top = TOP_PAD + rowIndex * ROW_HEIGHT;
    const y = top + ROW_HEIGHT / 2;
    const lane = auxNode.kind === "workspace" && anchorLane !== undefined
      ? anchorLane
      : auxNode.kind === "stash" && anchorLane !== undefined
        ? pickNearestOpenLane(anchorLane, reservedLanes ?? new Set([anchorLane]))
        : AUX_LANE;
    reservedLanes?.add(lane);
    const color = auxDisplayColors.get(auxNode.id)
      ?? (auxNode.anchorHash ? displayState.commitColors.get(auxNode.anchorHash) : undefined)
      ?? anchorLaneRow?.color
      ?? trackColor(0);
    const layout: HistoryGraphAuxLayout = {
      ...auxNode,
      refs: auxNode.refs,
      lane,
      rowIndex,
      x: graphLaneX(lane),
      y,
      top,
      height: ROW_HEIGHT,
      color,
      unanchored: auxNode.unanchored,
    };
    auxNodes.push(layout);
    rows.push(layout);
  };

  const pushCommit = (commit: HistoryGraphScene["primaryCommits"][number], laneRow: LaneRow) => {
    const rowIndex = rows.length;
    const top = TOP_PAD + rowIndex * ROW_HEIGHT;
    const y = top + ROW_HEIGHT / 2;
    const color = displayState.commitColors.get(commit.hash) ?? laneRow.color;
    const layout: HistoryGraphCommitLayout = {
      id: `commit:${commit.hash}`,
      kind: "commit",
      commit,
      refs: scene.commitRefs[commit.hash] ?? [],
      lane: laneRow.lane,
      color,
      rowIndex,
      x: graphLaneX(laneRow.lane),
      y,
      top,
      height: ROW_HEIGHT,
      isHead: scene.headHash === commit.hash,
      selected: scene.selectionKind === "commit" && scene.selectedCommitHash === commit.hash,
      downEdges: laneRow.downEdges,
    };
    commits.push(layout);
    rows.push(layout);
  };

  topAuxNodes.forEach(pushAux);

  for (let index = 0; index < laneRows.length; index++) {
    const laneRow = laneRows[index];
    const commit = scene.primaryCommits[index];
    const attachments = anchoredAuxByHash.get(commit.hash) ?? [];

    attachments.forEach(pushAux);
    pushCommit(commit, laneRow);
  }

  const edges: HistoryGraphEdgeLayout[] = [];
  for (let index = 0; index < commits.length - 1; index++) {
    const current = commits[index];
    const next = commits[index + 1];
    for (const edge of current.downEdges) {
      edges.push({
        id: `lane:${current.commit.hash}:${edge.fromLane}:${edge.toLane}`,
        path: curvePath(graphLaneX(edge.fromLane), current.y, graphLaneX(edge.toLane), next.y),
        color: edge.color,
        startRowIndex: current.rowIndex,
        endRowIndex: next.rowIndex,
      });
    }
  }

  const commitByHash = new Map(commits.map(commit => [commit.commit.hash, commit]));
  for (const auxNode of auxNodes) {
    const anchor = auxNode.anchorHash ? commitByHash.get(auxNode.anchorHash) : null;
    if (!anchor) continue;
    edges.push({
      id: `aux:${auxNode.id}:${anchor.commit.hash}`,
      path: curvePath(auxNode.x, auxNode.y, anchor.x, anchor.y),
      color: auxNode.color,
      startRowIndex: Math.min(auxNode.rowIndex, anchor.rowIndex),
      endRowIndex: Math.max(auxNode.rowIndex, anchor.rowIndex),
      dashed: auxNode.kind === "workspace" ? true : undefined,
      opacity: auxNode.kind === "workspace" ? 0.78 : 0.58,
    });
  }

  const laneCount = Math.max(
    1,
    ...laneRows.flatMap(row => [
      row.lane,
      ...row.downEdges.map(edge => Math.max(edge.fromLane, edge.toLane)),
    ]),
    ...auxNodes.map(node => node.lane),
  );
  const refsWidth = clampHistoryGraphColumnWidth(
    "refsWidth",
    overrides.refsWidth ?? (estimateDisplayRefsRailWidth(rows.map(row => row.refs)) + DEFAULT_REFS_W_PADDING),
  );
  const graphWidth = GRAPH_PAD_X * 2 + (laneCount + 1) * LANE_W;
  const messageWidth = clampHistoryGraphColumnWidth("messageWidth", overrides.messageWidth ?? DEFAULT_MESSAGE_W);
  const metaWidth = clampHistoryGraphColumnWidth("metaWidth", overrides.metaWidth ?? DEFAULT_META_W);
  const contentHeight = Math.max(72, TOP_PAD + rows.length * ROW_HEIGHT + BOTTOM_PAD);

  return {
    commits,
    auxNodes,
    rows,
    edges,
    contentHeight,
    visibleCommitCount: commits.length,
    rails: {
      refsWidth,
      graphWidth,
      messageWidth,
      metaWidth,
      rowHeight: ROW_HEIGHT,
      bodyMinWidth: refsWidth + graphWidth + messageWidth + metaWidth,
    },
  };
}
