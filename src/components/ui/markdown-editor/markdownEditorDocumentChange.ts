import { Text, type ChangeSet } from "@codemirror/state";

/**
 * Immutable CodeMirror document payload used by large-document consumers.
 * Keeping the rope here lets callers defer allocating one contiguous string
 * until an autosave, explicit save, rebase, or session switch boundary.
 */
export interface MarkdownEditorDocumentChange {
  doc: Text;
  changes: ChangeSet;
}

export function markdownEditorTextFromString(value: string): Text {
  return Text.of(value.replace(/\r\n/g, "\n").split("\n"));
}

export function markdownEditorTextHasContent(value: Text): boolean {
  for (let lineNumber = 1; lineNumber <= value.lines; lineNumber += 1) {
    if (value.line(lineNumber).text.trim()) return true;
  }
  return false;
}
