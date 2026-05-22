import hljs, { langFromPath } from "../../hljs";
import type { DiffHunk, DiffLine } from "../../types";

export function escapeDiffHtml(source: string): string {
  return source
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function escapeHunkLines(hunk: DiffHunk): DiffLine[] {
  return hunk.lines.map((line) => ({ ...line, content: escapeDiffHtml(line.content) }));
}

export function highlightDiffHunk(hunk: DiffHunk, filePath: string, skipHighlight = false): DiffLine[] {
  if (skipHighlight) return escapeHunkLines(hunk);

  const lang = langFromPath(filePath);
  if (!lang || !hljs.getLanguage(lang)) return escapeHunkLines(hunk);

  const oldLines = hunk.lines
    .filter((line) => line.kind === "context" || line.kind === "delete")
    .map((line) => line.content);
  const newLines = hunk.lines
    .filter((line) => line.kind === "context" || line.kind === "add")
    .map((line) => line.content);

  let oldHighlighted: string[] = [];
  let newHighlighted: string[] = [];
  try {
    oldHighlighted = hljs
      .highlight(oldLines.join(""), { language: lang })
      .value.split("\n");
    newHighlighted = hljs
      .highlight(newLines.join(""), { language: lang })
      .value.split("\n");
  } catch {
    return escapeHunkLines(hunk);
  }

  let oldIndex = 0;
  let newIndex = 0;
  return hunk.lines.map((line) => {
    let content = escapeDiffHtml(line.content);
    if (line.kind === "delete" && oldIndex < oldHighlighted.length) {
      content = oldHighlighted[oldIndex++];
    } else if (line.kind === "add" && newIndex < newHighlighted.length) {
      content = newHighlighted[newIndex++];
    } else if (line.kind === "context") {
      if (oldIndex < oldHighlighted.length) {
        content = oldHighlighted[oldIndex++];
      }
      newIndex++;
    }
    return { ...line, content };
  });
}
