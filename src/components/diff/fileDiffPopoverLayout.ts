import type { FileDiffPayload, InspectorField } from "../../types";

export const DIFF_POPOVER_WIDTH_PX = 760;
export const DIFF_POPOVER_MIN_WIDTH_PX = 420;
export const DIFF_POPOVER_MAX_WIDTH_PX = DIFF_POPOVER_WIDTH_PX;
export const DIFF_POPOVER_MIN_HEIGHT_PX = 180;
export const DIFF_POPOVER_MAX_HEIGHT_PX = 520;

const DIFF_POPOVER_CHROME_HEIGHT_PX = 92;
const DIFF_POPOVER_ROW_HEIGHT_PX = 18;
const DIFF_POPOVER_MIN_ROWS = 4;
const DIFF_POPOVER_MAX_VISIBLE_ROWS = 24;
const DIFF_POPOVER_TEXT_BASE_WIDTH_PX = 128;
const DIFF_POPOVER_TEXT_CHAR_WIDTH_PX = 7;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function visibleTextLength(value: string | undefined): number {
  if (!value) return 0;
  return value
    .replace(/<\/?span\b[^>]*>/g, "")
    .replace(/&(?:nbsp|amp|lt|gt|quot|#39);/g, " ")
    .length;
}

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

function longestInspectorFieldLabel(fields: readonly InspectorField[] | undefined): number {
  if (!fields?.length) return 0;
  return fields.reduce((longest, field) => {
    return Math.max(
      longest,
      visibleTextLength(field.label),
      visibleTextLength(field.before),
      visibleTextLength(field.after),
      longestInspectorFieldLabel(field.children),
    );
  }, 0);
}

function longestSemanticLabel(payload: FileDiffPayload): number {
  const semantic = payload.semantic;
  if (!semantic) return 0;
  const targetLabels = semantic.targets?.reduce((longest, target) => {
    return Math.max(
      longest,
      visibleTextLength(target.label),
      visibleTextLength(target.subtitle),
      visibleTextLength(target.path),
    );
  }, 0) ?? 0;
  const treeLabels = semantic.tree?.reduce((longest, node) => {
    return Math.max(longest, visibleTextLength(node.label), visibleTextLength(node.path));
  }, 0) ?? 0;
  const inspectorLabels = semantic.inspector?.panels.reduce((longest, panel) => {
    return Math.max(
      longest,
      visibleTextLength(panel.title),
      longestInspectorFieldLabel(panel.fields),
    );
  }, 0) ?? 0;
  return Math.max(
    visibleTextLength(semantic.scriptClassName),
    targetLabels,
    treeLabels,
    inspectorLabels,
  );
}

function longestTextLine(payload: FileDiffPayload): number {
  const hunkLines = payload.text?.hunks.reduce((longest, hunk) => {
    const lineLongest = hunk.lines.reduce((lineMax, line) => {
      return Math.max(lineMax, visibleTextLength(line.content));
    }, 0);
    return Math.max(longest, visibleTextLength(hunk.header), lineLongest);
  }, 0) ?? 0;
  const summaryLongest = payload.previewSummary.reduce((longest, line) => {
    return Math.max(longest, visibleTextLength(line));
  }, 0);
  return Math.max(hunkLines, summaryLongest, visibleTextLength(payload.filePath));
}

export function estimateDiffPopoverWidth(payload: FileDiffPayload): number {
  const contentWidth =
    DIFF_POPOVER_TEXT_BASE_WIDTH_PX
    + Math.max(longestTextLine(payload), longestSemanticLabel(payload)) * DIFF_POPOVER_TEXT_CHAR_WIDTH_PX;
  const semanticLayoutWidth = payload.semantic
    ? payload.semantic.layout === "sceneHierarchyInspector" ? 680 : 600
    : 0;
  const binaryWidth = payload.isBinary ? 520 : 0;

  return clamp(
    Math.max(contentWidth, semanticLayoutWidth, binaryWidth),
    DIFF_POPOVER_MIN_WIDTH_PX,
    DIFF_POPOVER_MAX_WIDTH_PX,
  );
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
  const visibleRows = Math.min(rows, DIFF_POPOVER_MAX_VISIBLE_ROWS);
  const height = DIFF_POPOVER_CHROME_HEIGHT_PX + visibleRows * DIFF_POPOVER_ROW_HEIGHT_PX;

  return clamp(height, DIFF_POPOVER_MIN_HEIGHT_PX, DIFF_POPOVER_MAX_HEIGHT_PX);
}
