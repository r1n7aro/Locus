import type { ChangeSpec } from "@codemirror/state";
import { buildDocumentTextHunks } from "../../../document/documentText";

/** Map shared, disjoint text edits into CodeMirror so selection and undo survive external changes. */
export function createMinimalTextChange(
  currentText: string,
  nextText: string,
): ChangeSpec | null {
  const changes = buildDocumentTextHunks(currentText, nextText)
    .map(({ start, end, newText }) => ({ from: start, to: end, insert: newText }));
  if (!changes.length) return null;
  return changes.length === 1 ? changes[0]! : changes;
}
