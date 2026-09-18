import type { CellComponent, Editor } from "tabulator-tables";
import { normalizeTextDocumentLineEndings } from "../../document/textDocumentFormat";
import { applyCsvNormalizedTextEdit } from "../../document/csv/csvSourceEditing";

export interface CsvCellEditPoint { x: number; y: number }
export interface CsvCellEditorParams {
  newline: string;
  point?: CsvCellEditPoint;
  navigate: (cell: CellComponent, x: number, y: number) => void;
}

export const csvCellEditor: Editor = (cell, onRendered, success, cancel, editorParams) => {
  const params = editorParams as unknown as CsvCellEditorParams;
  const text = cell.getElement().querySelector<HTMLElement>(".csv-cell-text") ?? document.createElement("span");
  const input = editCsvCellText(text, String(cell.getValue() ?? ""), params.newline, success, () => cancel(undefined),
    (x, y) => params.navigate(cell, x, y), () => cell.getElement().focus());
  onRendered(() => focusCsvCellInput(input, params.point));
  return input;
};

export function csvCellInputValue(input: HTMLElement): string {
  // innerText preserves hard breaks introduced by native paste/editing without
  // treating visual line wrapping as CSV newlines. textContent supports jsdom.
  let value = input.innerText ?? input.textContent ?? "";
  const last = input.lastChild;
  // Chromium keeps the final caret line alive with a separate newline or BR.
  // It is an editing placeholder, not an additional newline in the CSV value.
  if (last?.nodeName === "BR" && value.endsWith("\n")
    || last?.nodeType === Node.TEXT_NODE && last.nodeValue === "\n" && value.endsWith("\n\n")) value = value.slice(0, -1);
  return normalizeTextDocumentLineEndings(value);
}

/** Edit the formatter's existing text node, with native selection and IME. */
export function editCsvCellText(input: HTMLElement, original: string, newline: string, success: (value: string) => void,
  cancel: () => void, navigate: (x: number, y: number) => void, focus: () => void): HTMLElement {
  const initial = normalizeTextDocumentLineEndings(original);
  input.classList.add("csv-cell-text", "csv-cell-input");
  input.setAttribute("contenteditable", "plaintext-only");
  input.tabIndex = 0;
  input.setAttribute("role", "textbox");
  input.setAttribute("aria-multiline", "true");
  input.spellcheck = false;
  if (input.textContent !== initial) input.textContent = initial;
  if (initial.endsWith("\n")) input.appendChild(input.ownerDocument.createTextNode("\n"));
  // Text selection belongs to the editor. Tabulator's range mousedown handler
  // otherwise focuses the grid, blurs this node and ends the edit on a click.
  // Keep native defaults so clicks, word selection and dragging move the caret.
  for (const type of ["mousedown", "mousemove", "mouseup", "click", "dblclick"]) {
    input.addEventListener(type, (event) => event.stopPropagation());
  }
  let finished = false;
  let composing = false;
  let blurredDuringComposition = false;
  const commit = () => {
    if (finished || composing) return;
    const value = csvCellInputValue(input);
    finished = true;
    if (value === initial) cancel();
    else success(applyCsvNormalizedTextEdit(original, value, newline));
  };
  input.addEventListener("compositionstart", () => { composing = true; });
  input.addEventListener("compositionend", () => {
    composing = false;
    if (blurredDuringComposition) commit();
  });
  input.addEventListener("blur", () => {
    blurredDuringComposition = composing;
    commit();
  });
  input.addEventListener("keydown", (event) => {
    // Merged editors live outside Tabulator's editing state too; never let its
    // range shortcuts consume their caret movement, deletion or IME keys.
    event.stopPropagation();
    if (event.isComposing || composing || event.keyCode === 229) return;
    if (event.key === "Escape") {
      event.preventDefault(); event.stopImmediatePropagation();
      finished = true; cancel(); queueMicrotask(focus);
    } else if (event.key === "Enter" && event.altKey) {
      event.preventDefault(); event.stopImmediatePropagation();
      // Native insertion keeps the browser's undo history and caret behavior.
      input.ownerDocument.execCommand("insertText", false, "\n");
    } else if (event.key === "Enter" || event.key === "Tab") {
      event.preventDefault(); event.stopImmediatePropagation();
      commit();
      navigate(event.key === "Tab" ? (event.shiftKey ? -1 : 1) : 0,
        event.key === "Enter" ? (event.shiftKey ? -1 : 1) : 0);
    }
  });
  return input;
}

export function focusCsvCellInput(input: HTMLElement, point?: CsvCellEditPoint): void {
  input.focus({ preventScroll: true });
  const document = input.ownerDocument;
  const hit = point ? document.caretRangeFromPoint?.(point.x, point.y) : null;
  const range = hit && input.contains(hit.startContainer) ? hit : document.createRange();
  if (range !== hit) { range.selectNodeContents(input); range.collapse(false); }
  const selection = document.getSelection();
  selection?.removeAllRanges(); selection?.addRange(range);
}

export function seedCsvCellInput(input: HTMLElement, value: string): void {
  input.textContent = value;
  focusCsvCellInput(input);
}
