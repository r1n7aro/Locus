import type { KnowledgeDocumentSection } from "../../types";
import type { KnowledgeDocumentSource } from "../../services/knowledgeSelection";
import type { MarkdownEditorSelection } from "../ui/markdown-editor/markdownEditorSelection";

const normalize = (text: string) => text.replace(/\r\n?/g, "\n");
const lineAt = (text: string, offset: number) => text.slice(0, offset).split("\n").length;

/** Locate the whole section, not just the selected words (which may repeat). */
export function knowledgeSelectionReference(
  source: KnowledgeDocumentSource,
  section: KnowledgeDocumentSection,
  savedText: string,
  selection: MarkdownEditorSelection,
  labels: { draft: string; unavailable: string },
): string {
  const raw = normalize(source.content);
  const current = selection.doc.toString();
  const frontmatter = raw.match(/^\s*---[^\S\n]*\n[\s\S]*?\n---[^\S\n]*(?:\n|$)/);
  const bodyOffset = frontmatter?.[0].length ?? 0;
  const searchStart = section === "body" ? bodyOffset : 0;
  const searchEnd = section === "body" ? raw.length : bodyOffset || raw.length;
  const searchable = raw.slice(searchStart, searchEnd);

  const findSection = (value: string): number => {
    const text = normalize(value).trim();
    if (!text) return -1;
    const index = searchable.indexOf(text);
    if (index < 0 || searchable.indexOf(text, index + 1) >= 0) return -1;
    return index + searchStart;
  };
  let offset = findSection(current);
  let draft = false;
  if (offset < 0) {
    offset = findSection(savedText);
    draft = current.trim() !== normalize(savedText).trim();
  }
  if (offset < 0 && section === "body" && !savedText.trim()) {
    offset = bodyOffset;
    draft = true;
  }

  // YAML scalars can escape/fold text. In that case cite the source field's
  // complete line range instead of inventing a one-to-one mapping.
  let fieldRange: { start: number; end: number } | null = null;
  if (section !== "body" && frontmatter) {
    const lines = raw.slice(0, bodyOffset).split("\n");
    const key = section === "summary" ? "summary" : "maintenanceRules";
    const index = lines.findIndex((line) => new RegExp(`^${key}:`).test(line));
    if (index >= 0) {
      let end = index + 1;
      while (end < lines.length && (/^\s+\S/.test(lines[end]!) || !lines[end]!.trim())) end += 1;
      while (end > index + 1 && !lines[end - 1]!.trim()) end -= 1;
      fieldRange = { start: index + 1, end };
    }
  }
  if (offset < 0 && !fieldRange) throw new Error(labels.unavailable);
  const leading = current.length - current.trimStart().length;
  const sectionLine = offset >= 0 ? lineAt(raw, offset) : 1;
  const leadingLines = lineAt(current, leading) - 1;

  return selection.ranges.map((range) => {
    const startLine = fieldRange ? fieldRange.start : Math.max(sectionLine, sectionLine + range.startLine - 1 - leadingLines);
    const endLine = fieldRange ? fieldRange.end : Math.max(startLine, sectionLine + range.endLine - 1 - leadingLines);
    const location = `${source.path.replace(/\\/g, "/")}:${startLine}${endLine === startLine ? "" : `-${endLine}`}`;
    let fenceLength = 3;
    for (const match of range.text.matchAll(/`+/g)) fenceLength = Math.max(fenceLength, match[0].length + 1);
    const fence = "`".repeat(fenceLength);
    return `${location}${draft ? ` (${labels.draft})` : ""}\n${fence}markdown\n${range.text}\n${fence}`;
  }).join("\n\n");
}
