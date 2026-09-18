import { EditorSelection, type EditorState, type Text } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

export interface MarkdownEditorSelection {
  doc: Text;
  ranges: Array<{ from: number; to: number; text: string; startLine: number; endLine: number }>;
}

export function captureMarkdownEditorSelection(state: EditorState): MarkdownEditorSelection {
  return {
    doc: state.doc,
    ranges: state.selection.ranges.filter((range) => !range.empty).map(({ from, to }) => ({
      from,
      to,
      text: state.doc.sliceString(from, to),
      startLine: state.doc.lineAt(from).number,
      // A selection ending at the next line's start does not include that line.
      endLine: state.doc.lineAt(Math.max(from, to - 1)).number,
    })),
  };
}

/** Read-only CodeMirror content uses the browser selection instead of an editable DOM. */
export function syncMarkdownDOMSelection(view: EditorView): void {
  if (!view.state.readOnly) return;
  const selection = view.dom.ownerDocument.getSelection();
  if (!selection || selection.isCollapsed || !selection.anchorNode || !selection.focusNode) return;
  if (!view.contentDOM.contains(selection.anchorNode) || !view.contentDOM.contains(selection.focusNode)) return;
  try {
    view.dispatch({ selection: EditorSelection.single(
      view.posAtDOM(selection.anchorNode, selection.anchorOffset),
      view.posAtDOM(selection.focusNode, selection.focusOffset),
    ) });
  } catch {
    // A replaced widget may not expose a source position; keep the editor selection.
  }
}
