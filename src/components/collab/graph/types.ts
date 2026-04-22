import type {
  GitCommitInfo,
  GitGraphRef,
  GitHeadState,
  GitHistorySelection,
  GitStashEntry,
} from "../../../types";
import type { CollabHistorySelectionKind } from "../historySelection";

export interface HistoryGraphInput {
  commits: GitCommitInfo[];
  stashes: GitStashEntry[];
  refs: GitGraphRef[];
  headState: GitHeadState;
  selectedHistory: GitHistorySelection | null;
  workspaceChangeCount: number;
}

export interface HistoryGraphDisplayRef {
  key: string;
  kind: "head" | "branch" | "remote" | "tag" | "stash" | "workspace";
  text: string;
  title?: string;
  sourceMarkers?: Array<{
    key: string;
    kind: "local" | "remote";
    title: string;
    variant?: number;
  }>;
  isCurrent?: boolean;
}

export interface HistoryGraphAuxNode {
  id: string;
  kind: "workspace" | "stash";
  anchorHash: string | null;
  unanchored: boolean;
  date: number;
  hash: string | null;
  shortHash: string;
  label: string;
  selected: boolean;
  refs: HistoryGraphDisplayRef[];
  workspaceChangeCount?: number;
  stash?: GitStashEntry;
}

export interface HistoryGraphScene {
  primaryCommits: GitCommitInfo[];
  commitRefs: Record<string, HistoryGraphDisplayRef[]>;
  auxNodes: HistoryGraphAuxNode[];
  selectionKind: CollabHistorySelectionKind;
  selectedCommitHash: string | null;
  headHash: string | null;
}

export interface HistoryGraphLaneEdge {
  fromLane: number;
  toLane: number;
  color: string;
  kind?: "continuation" | "merge";
}

export interface HistoryGraphCommitLayout {
  id: string;
  kind: "commit";
  commit: GitCommitInfo;
  refs: HistoryGraphDisplayRef[];
  lane: number;
  color: string;
  rowIndex: number;
  x: number;
  y: number;
  top: number;
  height: number;
  isHead: boolean;
  selected: boolean;
  downEdges: HistoryGraphLaneEdge[];
}

export interface HistoryGraphAuxLayout {
  id: string;
  kind: "workspace" | "stash";
  refs: HistoryGraphDisplayRef[];
  date: number;
  lane: number;
  rowIndex: number;
  x: number;
  y: number;
  top: number;
  height: number;
  color: string;
  label: string;
  shortHash: string;
  hash: string | null;
  selected: boolean;
  anchorHash: string | null;
  workspaceChangeCount?: number;
  stash?: GitStashEntry;
  unanchored: boolean;
}

export type HistoryGraphRowLayout = HistoryGraphCommitLayout | HistoryGraphAuxLayout;

export interface HistoryGraphEdgeLayout {
  id: string;
  path: string;
  color: string;
  startRowIndex: number;
  endRowIndex: number;
  dashed?: boolean;
  opacity?: number;
}

export interface HistoryGraphColumnWidthOverrides {
  refsWidth?: number;
  messageWidth?: number;
  metaWidth?: number;
}

export type HistoryGraphResizableColumn = keyof HistoryGraphColumnWidthOverrides;

export interface HistoryGraphRailLayout {
  refsWidth: number;
  graphWidth: number;
  messageWidth: number;
  metaWidth: number;
  rowHeight: number;
  bodyMinWidth: number;
}

export interface HistoryGraphLayoutResult {
  commits: HistoryGraphCommitLayout[];
  auxNodes: HistoryGraphAuxLayout[];
  rows: HistoryGraphRowLayout[];
  edges: HistoryGraphEdgeLayout[];
  contentHeight: number;
  visibleCommitCount: number;
  rails: HistoryGraphRailLayout;
}
