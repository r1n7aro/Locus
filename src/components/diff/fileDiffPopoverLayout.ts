import type { FileDiffPayload, InspectorField } from "../../types";

export const DIFF_POPOVER_WIDTH_PX = 760;
export const DIFF_POPOVER_MIN_HEIGHT_PX = 360;
export const DIFF_POPOVER_MAX_HEIGHT_PX = 720;

const DIFF_POPOVER_CHROME_HEIGHT_PX = 116;
const DIFF_POPOVER_ROW_HEIGHT_PX = 24;
const DIFF_POPOVER_MIN_ROWS = 10;

function countInspectorFieldRows(fields: readonly InspectorField[] | undefined): number {
  if (!fields?.length) return 0;
  return fields.reduce((total, field) => {
    return total + 1 + countInspectorFieldRows(field.children);
  }, 0);
}

function countInspectorRows(payload: FileDiffPayload): number {
  const panels = payload.semantic?.inspector?.panels ?? [];
  return panels.reduce((total, panel) => {
    return total + 1 + countInspectorFieldRows(panel.fields);
  }, 0);
}

function countTextRows(payload: FileDiffPayload): number {
  return payload.text?.hunks.reduce((total, hunk) => total + hunk.lines.length, 0) ?? 0;
}

export function estimateDiffPopoverHeight(payload: FileDiffPayload): number {
  const semanticRows = Math.max(
    payload.semantic?.summary.changedFields ?? 0,
    countInspectorRows(payload),
  );
  const rows = Math.max(
    DIFF_POPOVER_MIN_ROWS,
    semanticRows,
    countTextRows(payload),
    payload.previewSummary.length,
  );
  const height = DIFF_POPOVER_CHROME_HEIGHT_PX + rows * DIFF_POPOVER_ROW_HEIGHT_PX;

  return Math.min(
    DIFF_POPOVER_MAX_HEIGHT_PX,
    Math.max(DIFF_POPOVER_MIN_HEIGHT_PX, height),
  );
}
