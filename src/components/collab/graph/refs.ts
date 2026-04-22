import type { GitGraphRef, GitHeadState } from "../../../types";
import type { HistoryGraphDisplayRef } from "./types";

interface BranchDisplayGroup {
  text: string;
  localRefs: GitGraphRef[];
  remoteRefs: GitGraphRef[];
  isCurrent: boolean;
}

export interface CollapsedDisplayRefs {
  visibleRefs: HistoryGraphDisplayRef[];
  hiddenRefs: HistoryGraphDisplayRef[];
  hiddenCount: number;
  isCollapsed: boolean;
}

const DISPLAY_REF_GAP = 6;
const DEFAULT_REFS_RAIL_FLOOR = 72;
const DEFAULT_REFS_RAIL_CEILING = 280;

function isHiddenGraphRef(ref: GitGraphRef): boolean {
  if (ref.kind !== "remoteBranch") return false;
  const branchName = ref.branchName ?? ref.shortName;
  return branchName === "HEAD" || branchName.endsWith("/HEAD");
}

function graphRefOrder(ref: GitGraphRef): number {
  switch (ref.kind) {
    case "localBranch":
      return ref.isCurrent ? 0 : 1;
    case "remoteBranch":
      return 2;
    case "tag":
      return 3;
    default:
      return 4;
  }
}

function compareGraphRefs(left: GitGraphRef, right: GitGraphRef): number {
  const orderDelta = graphRefOrder(left) - graphRefOrder(right);
  if (orderDelta !== 0) return orderDelta;
  return left.shortName.localeCompare(right.shortName);
}

function displayBranchText(ref: GitGraphRef): string {
  return ref.branchName ?? ref.shortName;
}

function displayRefTitle(ref: GitGraphRef): string {
  switch (ref.kind) {
    case "localBranch":
      return ref.branchName ?? ref.shortName;
    case "remoteBranch":
      return ref.shortName;
    case "tag":
      return `tag: ${ref.shortName}`;
  }
}

function buildRemoteVariantMap(refs: GitGraphRef[]): Map<string, number> {
  const remoteNames = [...new Set(
    refs
      .filter(ref => ref.kind === "remoteBranch")
      .filter(ref => !isHiddenGraphRef(ref))
      .map(ref => ref.remoteName ?? ref.shortName)
      .filter((name): name is string => !!name && !!name.trim()),
  )].sort((left, right) => left.localeCompare(right));

  return new Map(remoteNames.map((name, index) => [name, index]));
}

function buildLocalSourceMarker() {
  return {
    key: "local",
    kind: "local" as const,
    title: "local",
  };
}

function buildRemoteSourceMarker(ref: GitGraphRef, remoteVariants: Map<string, number>) {
  const remoteName = ref.remoteName ?? ref.shortName;
  return {
    key: `remote:${remoteName}`,
    kind: "remote" as const,
    title: remoteName || "remote",
    variant: remoteVariants.get(remoteName || "") ?? 0,
  };
}

function toDisplayRef(ref: GitGraphRef, remoteVariants: Map<string, number>): HistoryGraphDisplayRef {
  switch (ref.kind) {
    case "localBranch":
      return {
        key: ref.fullName,
        kind: "branch",
        text: displayBranchText(ref),
        title: displayRefTitle(ref),
        sourceMarkers: [buildLocalSourceMarker()],
        isCurrent: ref.isCurrent,
      };
    case "remoteBranch":
      return {
        key: ref.fullName,
        kind: "remote",
        text: displayBranchText(ref),
        title: displayRefTitle(ref),
        sourceMarkers: [buildRemoteSourceMarker(ref, remoteVariants)],
      };
    case "tag":
      return {
        key: ref.fullName,
        kind: "tag",
        text: ref.shortName,
        title: displayRefTitle(ref),
      };
  }
}

function toGroupedBranchDisplayRef(
  group: BranchDisplayGroup,
  remoteVariants: Map<string, number>,
): HistoryGraphDisplayRef {
  const refs = [...group.localRefs, ...group.remoteRefs];
  const titles = refs.map(displayRefTitle);
  const sourceMarkers: NonNullable<HistoryGraphDisplayRef["sourceMarkers"]> = [];
  if (group.localRefs.length > 0) {
    sourceMarkers.push(buildLocalSourceMarker());
  }
  const seenRemoteMarkers = new Set<string>();
  for (const ref of group.remoteRefs) {
    const marker = buildRemoteSourceMarker(ref, remoteVariants);
    if (seenRemoteMarkers.has(marker.key)) continue;
    seenRemoteMarkers.add(marker.key);
    sourceMarkers.push(marker);
  }
  return {
    key: refs.map(ref => ref.fullName).sort((left, right) => left.localeCompare(right)).join("|"),
    kind: group.localRefs.length > 0 ? "branch" : "remote",
    text: group.text,
    title: titles.join(", "),
    sourceMarkers,
    isCurrent: group.isCurrent,
  };
}

function displayRefOrder(ref: HistoryGraphDisplayRef): number {
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

function compareDisplayRefs(left: HistoryGraphDisplayRef, right: HistoryGraphDisplayRef): number {
  const orderDelta = displayRefOrder(left) - displayRefOrder(right);
  if (orderDelta !== 0) return orderDelta;
  return left.text.localeCompare(right.text);
}

export function buildCommitDisplayRefsByHash(
  refs: GitGraphRef[],
  headState: GitHeadState,
): Record<string, HistoryGraphDisplayRef[]> {
  const remoteVariants = buildRemoteVariantMap(refs);
  const grouped = new Map<string, GitGraphRef[]>();
  for (const ref of refs) {
    if (isHiddenGraphRef(ref)) continue;
    const list = grouped.get(ref.targetHash) ?? [];
    list.push(ref);
    grouped.set(ref.targetHash, list);
  }

  if (headState.hash && !grouped.has(headState.hash)) {
    grouped.set(headState.hash, []);
  }

  const result: Record<string, HistoryGraphDisplayRef[]> = {};
  for (const [hash, list] of grouped.entries()) {
    const branchGroups = new Map<string, BranchDisplayGroup>();
    const displayRefs: HistoryGraphDisplayRef[] = [];
    for (const ref of [...list].sort(compareGraphRefs)) {
      if (ref.kind === "localBranch" || ref.kind === "remoteBranch") {
        const text = displayBranchText(ref);
        const group = branchGroups.get(text) ?? {
          text,
          localRefs: [],
          remoteRefs: [],
          isCurrent: false,
        };
        if (ref.kind === "localBranch") {
          group.localRefs.push(ref);
          group.isCurrent = group.isCurrent || ref.isCurrent;
        } else {
          group.remoteRefs.push(ref);
        }
        branchGroups.set(text, group);
        continue;
      }
      displayRefs.push(toDisplayRef(ref, remoteVariants));
    }
    for (const group of branchGroups.values()) {
      displayRefs.push(toGroupedBranchDisplayRef(group, remoteVariants));
    }
    result[hash] = displayRefs.sort(compareDisplayRefs);
  }

  return result;
}

export function extractLocalBranchNamesForHash(hash: string, refs: GitGraphRef[]): string[] {
  const names = new Set<string>();
  for (const ref of refs) {
    if (ref.kind !== "localBranch" || ref.targetHash !== hash) continue;
    const name = ref.branchName ?? ref.shortName;
    if (name) names.add(name);
  }
  return [...names].sort((left, right) => left.localeCompare(right));
}

export function estimateDisplayRefWidth(ref: HistoryGraphDisplayRef): number {
  const base = 22;
  const perChar = ref.kind === "head" ? 5.8 : 6.2;
  const markerWidth = (ref.sourceMarkers ?? []).reduce((total, marker) => total + (marker.kind === "local" ? 12 : 13), 0);
  const markerGap = Math.max(0, (ref.sourceMarkers?.length ?? 0) - 1) * 3;
  return Math.min(160, Math.ceil(base + markerWidth + markerGap + ref.text.length * perChar));
}

export function estimateDisplayRefOverflowWidth(hiddenCount: number): number {
  const text = `+${hiddenCount}`;
  return Math.max(28, Math.ceil(20 + text.length * 6.2));
}

function estimateCollapsedSummaryWidth(refs: HistoryGraphDisplayRef[]): number {
  if (refs.length === 0) {
    return 0;
  }

  const firstRefWidth = estimateDisplayRefWidth(refs[0]!);
  if (refs.length === 1) {
    return firstRefWidth;
  }

  return firstRefWidth + DISPLAY_REF_GAP + estimateDisplayRefOverflowWidth(refs.length - 1);
}

export function collapseDisplayRefsForRail(
  refs: HistoryGraphDisplayRef[],
  availableWidth: number,
): CollapsedDisplayRefs {
  if (refs.length <= 1) {
    return {
      visibleRefs: refs,
      hiddenRefs: [],
      hiddenCount: 0,
      isCollapsed: false,
    };
  }

  const widths = refs.map(estimateDisplayRefWidth);
  const totalWidth = widths.reduce((sum, width) => sum + width, 0)
    + Math.max(0, refs.length - 1) * DISPLAY_REF_GAP;
  if (totalWidth <= availableWidth) {
    return {
      visibleRefs: refs,
      hiddenRefs: [],
      hiddenCount: 0,
      isCollapsed: false,
    };
  }

  let usedWidth = 0;
  let visibleCount = 0;
  for (let index = 0; index < refs.length; index++) {
    const nextWidth = widths[index]!;
    const candidateWidth = visibleCount === 0
      ? nextWidth
      : usedWidth + DISPLAY_REF_GAP + nextWidth;
    const remainingCount = refs.length - index - 1;
    const overflowWidth = remainingCount > 0
      ? (visibleCount === 0 ? 0 : DISPLAY_REF_GAP) + estimateDisplayRefOverflowWidth(remainingCount)
      : 0;
    if (candidateWidth + overflowWidth > availableWidth) {
      break;
    }
    usedWidth = candidateWidth;
    visibleCount = index + 1;
  }

  if (visibleCount === 0) {
    visibleCount = 1;
  }

  const visibleRefs = refs.slice(0, visibleCount);
  const hiddenRefs = refs.slice(visibleCount);
  return {
    visibleRefs,
    hiddenRefs,
    hiddenCount: hiddenRefs.length,
    isCollapsed: hiddenRefs.length > 0,
  };
}

export function estimateDisplayRefsRailWidth(refGroups: HistoryGraphDisplayRef[][]): number {
  let maxWidth = DEFAULT_REFS_RAIL_FLOOR;
  for (const refs of refGroups) {
    if (refs.length === 0) continue;
    const width = estimateCollapsedSummaryWidth(refs);
    maxWidth = Math.max(maxWidth, width);
  }
  return Math.min(DEFAULT_REFS_RAIL_CEILING, Math.max(DEFAULT_REFS_RAIL_FLOOR, maxWidth));
}
