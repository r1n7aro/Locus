import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const TOOL_FILE_PREVIEW_WINDOW_LABEL = "tool-file-preview";
export const TOOL_FILE_PREVIEW_WINDOW_EVENT = "tool-file-preview:payload";
export const TOOL_FILE_PREVIEW_WINDOW_FLAG = "toolFilePreview";

export interface ToolFilePreviewLineRange {
  startLine: number;
  endLine: number;
}

export interface ToolFilePreviewEditHighlight {
  startLine: number;
  textLength: number;
  textFingerprint: string;
  textPrefix: string;
  highlightStartLineOffset: number;
  highlightEndLineOffset: number;
  matchAll: boolean;
}

export type ToolFilePreviewHighlight =
  | { mode: "all" }
  | { mode: "edit"; targets: ToolFilePreviewEditHighlight[] };

export interface ToolFilePreviewWindowPayload {
  filePath: string;
  highlight?: ToolFilePreviewHighlight;
}

function normalizeMatchText(value: string): string {
  return value.replace(/\r\n?/g, "\n").replace(/\n$/, "");
}

function fingerprintText(value: string, start = 0, length = value.length): string {
  let first = 0x811c9dc5;
  let second = 0x9e3779b9;
  const end = Math.min(value.length, start + length);
  for (let index = start; index < end; index += 1) {
    const code = value.charCodeAt(index);
    first = Math.imul(first ^ code, 0x01000193);
    second = Math.imul(second ^ code, 0x85ebca6b);
  }
  return `${(first >>> 0).toString(16)}:${(second >>> 0).toString(16)}`;
}

function changedLineOffsets(oldText: string, newText: string): [number, number] | null {
  const oldLines = normalizeMatchText(oldText).split("\n");
  const newLines = normalizeMatchText(newText).split("\n");
  let prefix = 0;
  while (
    prefix < oldLines.length
    && prefix < newLines.length
    && oldLines[prefix] === newLines[prefix]
  ) {
    prefix += 1;
  }

  let suffix = 0;
  while (
    suffix < oldLines.length - prefix
    && suffix < newLines.length - prefix
    && oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) {
    suffix += 1;
  }

  if (prefix === oldLines.length && prefix === newLines.length) return null;
  const changedEnd = newLines.length - suffix - 1;
  if (prefix <= changedEnd) return [prefix, changedEnd];
  if (normalizeMatchText(newText).length === 0) return null;
  const deletionAnchor = Math.min(prefix, newLines.length - 1);
  return [deletionAnchor, deletionAnchor];
}

export function createToolFilePreviewEditHighlight(input: {
  oldText: string;
  newText: string;
  startLine?: number;
  matchAll?: boolean;
}): ToolFilePreviewEditHighlight | null {
  const text = normalizeMatchText(input.newText);
  const changedOffsets = changedLineOffsets(input.oldText, input.newText);
  if (!text || !changedOffsets) return null;
  return {
    startLine: Math.max(0, Math.floor(input.startLine ?? 0)),
    textLength: text.length,
    textFingerprint: fingerprintText(text),
    textPrefix: text.slice(0, 48),
    highlightStartLineOffset: changedOffsets[0],
    highlightEndLineOffset: changedOffsets[1],
    matchAll: input.matchAll === true,
  };
}

function parseEditHighlights(raw: string | null): ToolFilePreviewEditHighlight[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap((item): ToolFilePreviewEditHighlight[] => {
      if (!item || typeof item !== "object") return [];
      const candidate = item as Partial<ToolFilePreviewEditHighlight>;
      if (
        typeof candidate.startLine !== "number"
        || typeof candidate.textLength !== "number"
        || typeof candidate.textFingerprint !== "string"
        || typeof candidate.textPrefix !== "string"
        || typeof candidate.highlightStartLineOffset !== "number"
        || typeof candidate.highlightEndLineOffset !== "number"
        || typeof candidate.matchAll !== "boolean"
      ) {
        return [];
      }
      return [{
        startLine: Math.max(0, Math.floor(candidate.startLine)),
        textLength: Math.max(0, Math.floor(candidate.textLength)),
        textFingerprint: candidate.textFingerprint,
        textPrefix: candidate.textPrefix,
        highlightStartLineOffset: Math.max(0, Math.floor(candidate.highlightStartLineOffset)),
        highlightEndLineOffset: Math.max(0, Math.floor(candidate.highlightEndLineOffset)),
        matchAll: candidate.matchAll,
      }];
    });
  } catch {
    return [];
  }
}

function sourceLineAtOffset(text: string, offset: number, snippetStartLine: number): number {
  let line = Math.max(1, snippetStartLine);
  for (let index = 0; index < offset; index += 1) {
    if (text.charCodeAt(index) === 10) line += 1;
  }
  return line;
}

function exactMatchOffsets(
  snippet: string,
  snippetStartLine: number,
  target: ToolFilePreviewEditHighlight,
): number[] {
  if (!target.textPrefix || target.textLength < target.textPrefix.length) return [];
  const matches: number[] = [];
  let searchFrom = 0;
  while (searchFrom <= snippet.length - target.textPrefix.length) {
    const offset = snippet.indexOf(target.textPrefix, searchFrom);
    if (offset < 0) break;
    if (
      offset + target.textLength <= snippet.length
      && fingerprintText(snippet, offset, target.textLength) === target.textFingerprint
    ) {
      matches.push(offset);
    }
    searchFrom = offset + Math.max(1, target.textPrefix.length);
  }

  if (target.matchAll || matches.length <= 1) return matches;
  const atOriginalLine = matches.filter(
    (offset) => sourceLineAtOffset(snippet, offset, snippetStartLine) === target.startLine,
  );
  return atOriginalLine.length === 1 ? atOriginalLine : [];
}

function mergeLineRanges(ranges: ToolFilePreviewLineRange[]): ToolFilePreviewLineRange[] {
  const sorted = ranges
    .filter((range) => range.startLine > 0 && range.endLine >= range.startLine)
    .sort((left, right) => left.startLine - right.startLine || left.endLine - right.endLine);
  const merged: ToolFilePreviewLineRange[] = [];
  for (const range of sorted) {
    const previous = merged[merged.length - 1];
    if (previous && range.startLine <= previous.endLine + 1) {
      previous.endLine = Math.max(previous.endLine, range.endLine);
    } else {
      merged.push({ ...range });
    }
  }
  return merged;
}

export function resolveToolFilePreviewHighlightRanges(
  snippet: string,
  snippetStartLine: number,
  highlight?: ToolFilePreviewHighlight,
): ToolFilePreviewLineRange[] {
  if (!highlight) return [];
  const firstLine = Math.max(1, snippetStartLine);
  const lastLine = firstLine + snippet.split("\n").length - 1;
  if (highlight.mode === "all") {
    return [{ startLine: firstLine, endLine: lastLine }];
  }

  const ranges = highlight.targets.flatMap((target) => (
    exactMatchOffsets(snippet, firstLine, target).map((offset) => {
      const matchStartLine = sourceLineAtOffset(snippet, offset, firstLine);
      return {
        startLine: matchStartLine + target.highlightStartLineOffset,
        endLine: Math.min(lastLine, matchStartLine + target.highlightEndLineOffset),
      };
    })
  ));
  return mergeLineRanges(ranges);
}

function fileTitle(filePath: string): string {
  return filePath.trim().replace(/\\/g, "/").split("/").pop() || "File";
}

export function isToolFilePreviewWindowLocation(
  locationLike: Pick<Location, "search"> = window.location,
): boolean {
  return new URLSearchParams(locationLike.search).get(TOOL_FILE_PREVIEW_WINDOW_FLAG) === "1";
}

export function getToolFilePreviewWindowPayload(
  search = window.location.search,
): ToolFilePreviewWindowPayload | null {
  const params = new URLSearchParams(search);
  const filePath = params.get("filePath")?.trim() ?? "";
  if (!filePath) return null;
  const mode = params.get("highlight");
  if (mode === "all") return { filePath, highlight: { mode: "all" } };
  if (mode === "edit") {
    return {
      filePath,
      highlight: { mode: "edit", targets: parseEditHighlights(params.get("editHighlights")) },
    };
  }
  return { filePath };
}

export function buildToolFilePreviewWindowQuery(
  payload: ToolFilePreviewWindowPayload,
): string {
  const params = new URLSearchParams({
    [TOOL_FILE_PREVIEW_WINDOW_FLAG]: "1",
    filePath: payload.filePath.trim(),
  });
  if (payload.highlight?.mode === "all") {
    params.set("highlight", "all");
  } else if (payload.highlight?.mode === "edit") {
    params.set("highlight", "edit");
    params.set("editHighlights", JSON.stringify(payload.highlight.targets));
  }
  return params.toString();
}

export function buildToolFilePreviewWindowUrl(
  payload: ToolFilePreviewWindowPayload,
): string {
  return buildSubWindowUrl(buildToolFilePreviewWindowQuery(payload));
}

export async function openToolFilePreviewWindow(
  payload: ToolFilePreviewWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;
  const filePath = payload.filePath.trim();
  if (!filePath) return false;

  const result = await openSubWindow({
    kind: TOOL_FILE_PREVIEW_WINDOW_LABEL,
    title: `Locus - ${fileTitle(filePath)}`,
    width: 980,
    height: 720,
    minWidth: 600,
    minHeight: 420,
    resizable: true,
    maximizable: true,
    minimizable: true,
  }, buildToolFilePreviewWindowQuery({ ...payload, filePath }));
  if (result.existing) {
    await result.window?.emit(TOOL_FILE_PREVIEW_WINDOW_EVENT, { ...payload, filePath });
  }
  return true;
}
