import type { HistoryGraphRowLayout } from "./types";

export const HISTORY_GRAPH_DOT_RADIUS = 5.5;
export const HISTORY_GRAPH_AUX_RADIUS = 4.5;
export const HISTORY_GRAPH_HEAD_RING_RADIUS = HISTORY_GRAPH_DOT_RADIUS + 2.1;
export const HISTORY_GRAPH_SELECTED_HEAD_RING_RADIUS = HISTORY_GRAPH_DOT_RADIUS + 4.2;
export const HISTORY_GRAPH_REF_CONNECTOR_GAP = 2;

const HISTORY_GRAPH_RING_STROKE_WIDTH = 1.75;
const HISTORY_GRAPH_SELECTED_STROKE_WIDTH = 1.5;

export function historyGraphNodeOuterRadius(row: HistoryGraphRowLayout): number {
  if (row.kind !== "commit") {
    const selectedStroke = row.selected ? HISTORY_GRAPH_SELECTED_STROKE_WIDTH / 2 : 0;
    return HISTORY_GRAPH_AUX_RADIUS + selectedStroke;
  }

  const selectedRadius = row.selected
    ? (row.isHead ? HISTORY_GRAPH_SELECTED_HEAD_RING_RADIUS : HISTORY_GRAPH_DOT_RADIUS)
      + HISTORY_GRAPH_SELECTED_STROKE_WIDTH / 2
    : 0;
  const headRadius = row.isHead
    ? HISTORY_GRAPH_HEAD_RING_RADIUS + HISTORY_GRAPH_RING_STROKE_WIDTH / 2
    : 0;
  return Math.max(HISTORY_GRAPH_DOT_RADIUS, selectedRadius, headRadius);
}

export function historyGraphRefConnectorRunout(row: HistoryGraphRowLayout): number {
  return Math.max(
    0,
    row.x - historyGraphNodeOuterRadius(row) - HISTORY_GRAPH_REF_CONNECTOR_GAP,
  );
}
