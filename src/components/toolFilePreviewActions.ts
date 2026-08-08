import type { ToolCallDisplay } from "../types";
import {
  createToolFilePreviewEditHighlight,
  type ToolFilePreviewEditHighlight,
  type ToolFilePreviewWindowPayload,
} from "../services/toolFilePreviewWindow";

const SINGLE_FILE_PREVIEW_TOOLS = new Set(["read", "edit", "write"]);

function parseToolArguments(argumentsJson: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(argumentsJson);
    return typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function parseEditStartLines(output: string | undefined): number[] {
  const match = output?.match(/\[lines:([0-9,]+)\]/);
  return match ? match[1].split(",").map(Number) : [];
}

function editHighlights(
  args: Record<string, unknown>,
  output: string | undefined,
): ToolFilePreviewEditHighlight[] {
  const startLines = parseEditStartLines(output);
  const rawEdits = Array.isArray(args.edits) ? args.edits : [args];
  return rawEdits.flatMap((rawEdit, index): ToolFilePreviewEditHighlight[] => {
    if (!rawEdit || typeof rawEdit !== "object" || Array.isArray(rawEdit)) return [];
    const edit = rawEdit as Record<string, unknown>;
    const oldText = edit.oldString ?? edit.old_string;
    const newText = edit.newString ?? edit.new_string;
    if (typeof oldText !== "string" || typeof newText !== "string") return [];
    const highlight = createToolFilePreviewEditHighlight({
      oldText,
      newText,
      startLine: startLines[index],
      matchAll: edit.replaceAll === true || edit.replace_all === true,
    });
    return highlight ? [highlight] : [];
  });
}

export function resolveToolFilePreviewPayload(
  toolCall: Pick<ToolCallDisplay, "name" | "arguments" | "status" | "output">,
): ToolFilePreviewWindowPayload | null {
  if (toolCall.status !== "done" || !SINGLE_FILE_PREVIEW_TOOLS.has(toolCall.name)) {
    return null;
  }

  const args = parseToolArguments(toolCall.arguments);
  if (!args) return null;
  const path = args.filePath ?? args.file_path ?? args.path;
  const filePath = typeof path === "string" ? path.trim() : "";
  if (!filePath) return null;
  if (toolCall.name === "write") {
    return { filePath, highlight: { mode: "all" } };
  }
  if (toolCall.name === "edit") {
    return {
      filePath,
      highlight: { mode: "edit", targets: editHighlights(args, toolCall.output) },
    };
  }
  return { filePath };
}

export function resolveToolFilePreviewPath(
  toolCall: Pick<ToolCallDisplay, "name" | "arguments" | "status" | "output">,
): string {
  return resolveToolFilePreviewPayload(toolCall)?.filePath ?? "";
}
