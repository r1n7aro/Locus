import { isolateHistory } from "@codemirror/commands";
import { syntaxTree } from "@codemirror/language";
import { EditorSelection, type EditorState, Prec, type Range } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, keymap, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import type { SyntaxNode } from "@lezer/common";
import { markdownTableCellRanges, parseMarkdownTable, type MarkdownTableAlignment, type MarkdownTableCellRange } from "./markdownComplexTokens";
import { htmlToEditorMarkdown } from "./markdownRichPaste";

interface TableRow {
  from: number;
  to: number;
  cells: MarkdownTableCellRange[];
}

interface TableLayout {
  from: number;
  to: number;
  rows: TableRow[];
  alignments: MarkdownTableAlignment[];
  separator: { from: number; to: number };
}

export function markdownTableLayout(state: EditorState, node: SyntaxNode): TableLayout | null {
  if (node.to - node.from > 100_000) return null;
  const model = parseMarkdownTable(state.doc.sliceString(node.from, node.to));
  if (!model || model.header.length > 40 || model.rows.length + 1 > 200) return null;
  const firstLine = state.doc.lineAt(node.from);
  // Nested block prefixes need their own layout; leave those as Markdown.
  if (state.doc.sliceString(firstLine.from, node.from).trim()) return null;
  const separator = state.doc.line(firstLine.number + 1);
  const rows: TableRow[] = [];
  for (let number = firstLine.number; number <= state.doc.lineAt(node.to).number; number++) {
    if (number === separator.number) continue;
    const line = state.doc.line(number);
    const cells = markdownTableCellRanges(line.text).map((cell) => ({
      from: line.from + cell.from,
      to: line.from + cell.to,
      contentFrom: line.from + cell.contentFrom,
      contentTo: line.from + cell.contentTo,
    }));
    rows.push({ from: line.from, to: line.to, cells });
  }
  return { from: node.from, to: node.to, rows, separator, alignments: model.alignments };
}

function tableAt(state: EditorState, position: number): TableLayout | null {
  let node: SyntaxNode | null = syntaxTree(state).resolveInner(position, 1);
  while (node && node.name !== "Table") node = node.parent;
  return node ? markdownTableLayout(state, node) : null;
}

function activeCell(state: EditorState) {
  const selection = state.selection.main;
  const table = tableAt(state, selection.head);
  if (!table) return null;
  const rowIndex = table.rows.findIndex((row) => selection.head >= row.from && selection.head <= row.to);
  const row = table.rows[rowIndex];
  if (!row) return null;
  let column = row.cells.findIndex((cell) => selection.head >= cell.from && selection.head <= cell.to);
  if (column < 0) column = selection.head <= row.from ? 0 : row.cells.length - 1;
  const cell = row.cells[column];
  return cell ? { table, rowIndex, column, cell } : null;
}

export { activeCell as activeMarkdownTableCell };

export type MarkdownTableAction = "row-before" | "row-after" | "row-delete" | "column-before" | "column-after" | "column-delete" | "table-delete";

export interface MarkdownTableTarget {
  tableFrom: number;
  rowIndex: number;
  column: number;
}

/** Resolve the clicked cell without changing the text selection or padding empty cells. */
export function markdownTableTarget(view: EditorView, element?: Element): MarkdownTableTarget | null {
  if (element) {
    const cell = element.closest<HTMLElement>(".cm-live-table-cell");
    const row = cell?.closest(".cm-live-table-row");
    if (!cell || !row || !view.contentDOM.contains(row)) return null;
    const rowFrom = view.state.doc.lineAt(view.posAtDOM(row)).from;
    const table = tableAt(view.state, rowFrom);
    const rowIndex = table?.rows.findIndex((candidate) => candidate.from === rowFrom) ?? -1;
    const column = Number(cell.dataset.column);
    if (!table || rowIndex < 0 || !Number.isInteger(column) || column < 0 || column >= table.alignments.length) return null;
    return { tableFrom: table.from, rowIndex, column };
  }
  const active = activeCell(view.state);
  const selection = view.state.selection;
  if (!active || selection.ranges.length !== 1 || selection.main.from < active.table.from || selection.main.to > active.table.to) return null;
  return { tableFrom: active.table.from, rowIndex: active.rowIndex, column: active.column };
}

function tableValues(state: EditorState, table: TableLayout): string[][] {
  return table.rows.map((row) => table.alignments.map((_, index) => {
    const cell = row.cells[index];
    return cell ? state.sliceDoc(cell.contentFrom, cell.contentTo) : "";
  }));
}

function replaceTable(view: EditorView, table: TableLayout, values: string[][], alignments: MarkdownTableAlignment[], row: number, column: number): void {
  if (!values.length || !alignments.length) {
    view.dispatch({ changes: { from: table.from, to: table.to, insert: "" }, selection: { anchor: table.from }, userEvent: "delete.table", annotations: isolateHistory.of("full"), scrollIntoView: true });
    return;
  }
  const lines = values.map((cells) => `| ${cells.join(" | ")} |`);
  lines.splice(1, 0, `| ${alignments.map((align) => align === "center" ? ":---:" : align === "right" ? "---:" : align === "left" ? ":---" : "---").join(" | ")} |`);
  const rowIndex = Math.max(0, Math.min(row, values.length - 1));
  const lineIndex = rowIndex === 0 ? 0 : rowIndex + 1;
  const cell = markdownTableCellRanges(lines[lineIndex])[Math.max(0, Math.min(column, alignments.length - 1))];
  const anchor = table.from + lines.slice(0, lineIndex).reduce((offset, line) => offset + line.length + 1, 0) + cell.contentFrom;
  view.dispatch({ changes: { from: table.from, to: table.to, insert: lines.join("\n") }, selection: { anchor }, userEvent: "input.table", annotations: isolateHistory.of("full"), scrollIntoView: true });
}

export function editMarkdownTable(view: EditorView, action: MarkdownTableAction | MarkdownTableAlignment, target = markdownTableTarget(view)): boolean {
  if (!target || view.state.readOnly) return false;
  const table = tableAt(view.state, target.tableFrom);
  const { rowIndex, column } = target;
  if (!table || table.from !== target.tableFrom || !table.rows[rowIndex] || column < 0 || column >= table.alignments.length) return false;
  if (action === "table-delete") {
    replaceTable(view, table, [], [], 0, 0);
    view.focus();
    return true;
  }
  const values = tableValues(view.state, table);
  const alignments = [...table.alignments];
  let row = rowIndex;
  let col = column;
  if (action === "row-before" || action === "row-after") {
    row = rowIndex + (action === "row-after" ? 1 : 0);
    values.splice(row, 0, Array(alignments.length).fill(""));
  } else if (action === "row-delete") values.splice(row, 1);
  else if (action === "column-before" || action === "column-after") {
    col += action === "column-after" ? 1 : 0;
    alignments.splice(col, 0, null);
    values.forEach((cells) => cells.splice(col, 0, ""));
  } else if (action === "column-delete") {
    alignments.splice(col, 1);
    values.forEach((cells) => cells.splice(col, 1));
  } else alignments[col] = action;
  replaceTable(view, table, values, alignments, row, col);
  view.focus();
  return true;
}

export function parseClipboardTable(text: string): string[][] {
  const rows: string[][] = [[]];
  let cell = "";
  let quoted = false;
  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (char === '"' && (quoted || !cell)) {
      if (quoted && text[index + 1] === '"') { cell += '"'; index++; }
      else quoted = !quoted;
    } else if (!quoted && (char === "\t" || char === "\n" || char === "\r")) {
      rows[rows.length - 1].push(cell);
      cell = "";
      if (char !== "\t") {
        if (char === "\r" && text[index + 1] === "\n") index++;
        rows.push([]);
      }
    } else cell += char;
  }
  if (cell || rows[rows.length - 1].length) rows[rows.length - 1].push(cell);
  else if (rows.length > 1) rows.pop();
  return rows;
}

export function pasteMarkdownTable(view: EditorView, rows: string[][], markdown = false): boolean {
  const active = activeCell(view.state);
  if (!active || view.state.readOnly || !rows.length) return false;
  const { table, rowIndex, column } = active;
  const values = tableValues(view.state, table);
  const width = Math.max(table.alignments.length, column + Math.max(...rows.map((row) => row.length)));
  const alignments = [...table.alignments];
  while (alignments.length < width) alignments.push(null);
  while (values.length < rowIndex + rows.length) values.push([]);
  values.forEach((row) => { while (row.length < width) row.push(""); });
  rows.forEach((row, y) => row.forEach((cell, x) => {
    const normalized = (markdown ? cell : cell.replace(/\\/g, "\\\\")).replace(/\r\n?|\n/g, "<br>");
    values[rowIndex + y][column + x] = normalized.replace(/\|/g, (_, offset: number) => {
      const slashes = normalized.slice(0, offset).match(/\\+$/)?.[0].length ?? 0;
      return slashes % 2 === 0 ? "\\|" : "|";
    });
  }));
  replaceTable(view, table, values, alignments, rowIndex, column);
  return true;
}

function selectCell(view: EditorView, table: TableLayout, rowIndex: number, column: number, end = false): void {
  const row = table.rows[rowIndex];
  if (!row || view.state.readOnly) return;
  const cell = row.cells[column];
  if (!cell) {
    // GFM permits short body rows. Materialize only when editing a missing cell.
    const values = row.cells.map((range) => view.state.doc.sliceString(range.contentFrom, range.contentTo));
    while (values.length < table.alignments.length) values.push("");
    const insert = `| ${values.join(" | ")} |`;
    const target = markdownTableCellRanges(insert)[column];
    view.dispatch({ changes: { from: row.from, to: row.to, insert }, selection: { anchor: row.from + target.contentFrom }, userEvent: "input" });
  } else if (cell.from === cell.to) {
    // A compact empty cell (||) has no editable DOM position. Add padding when
    // entering it so native typing/IME can place a caret in real document text.
    view.dispatch({ changes: { from: cell.from, insert: "  " }, selection: { anchor: cell.from + 1 }, userEvent: "input" });
  } else {
    const anchor = end ? cell.contentTo : cell.contentFrom;
    view.dispatch({ selection: EditorSelection.cursor(anchor, end ? -1 : 1), effects: EditorView.scrollIntoView(anchor, { y: "nearest" }) });
  }
  view.focus();
}

class EmptyTableCellWidget extends WidgetType {
  constructor(readonly tableFrom: number, readonly row: number, readonly column: number, readonly alignment: string | null) { super(); }
  eq(other: EmptyTableCellWidget): boolean {
    return this.tableFrom === other.tableFrom && this.row === other.row && this.column === other.column && this.alignment === other.alignment;
  }
  toDOM(view: EditorView): HTMLElement {
    const cell = document.createElement("span");
    cell.className = "cm-live-table-cell cm-live-table-empty-cell";
    cell.dataset.column = String(this.column);
    cell.dataset.align = this.alignment ?? "left";
    cell.setAttribute("role", this.row === 0 ? "columnheader" : "cell");
    cell.setAttribute("aria-label", "空单元格");
    cell.addEventListener("mousedown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      const table = tableAt(view.state, this.tableFrom);
      if (table) selectCell(view, table, this.row, this.column);
    });
    return cell;
  }
}

class TableBreakWidget extends WidgetType {
  eq(): boolean { return true; }
  toDOM(): HTMLElement { return document.createElement("br"); }
  get lineBreaks(): number { return 1; }
}

function tableDecorations(view: EditorView) {
  const decorations: Range<Decoration>[] = [];
  const cells: Range<Decoration>[] = [];
  const atomic: Range<Decoration>[] = [];
  const seen = new Set<number>();
  const hide = (from: number, to: number) => {
    if (from < to) decorations.push(Decoration.replace({}).range(from, to));
  };
  for (const visible of view.visibleRanges) {
    syntaxTree(view.state).iterate({ from: visible.from, to: visible.to, enter(ref) {
      if (ref.name !== "Table") return;
      if (seen.has(ref.from)) return false;
      seen.add(ref.from);
      const table = markdownTableLayout(view.state, ref.node);
      if (!table) return false;
      decorations.push(Decoration.line({ class: "cm-live-collapsed-line" }).range(table.separator.from));
      hide(table.separator.from, table.separator.to);
      table.rows.forEach((row, rowIndex) => {
        decorations.push(Decoration.line({
          class: `cm-live-table-line cm-live-table-row${rowIndex === 0 ? " cm-live-table-header" : ""}${rowIndex === table.rows.length - 1 ? " cm-live-table-last-row" : ""}`,
          attributes: { style: `grid-template-columns: repeat(${table.alignments.length}, minmax(0, 1fr))`, role: "row" },
        }).range(row.from));
        let previous = row.from;
        for (let column = 0; column < table.alignments.length; column++) {
          const cell = row.cells[column];
          if (!cell) {
            decorations.push(Decoration.widget({ widget: new EmptyTableCellWidget(table.from, rowIndex, column, table.alignments[column]), side: column + 1 }).range(row.to));
            continue;
          }
          hide(previous, cell.from);
          previous = cell.to;
          if (cell.from === cell.to) {
            const widget = new EmptyTableCellWidget(table.from, rowIndex, column, table.alignments[column]);
            decorations.push(Decoration.widget({ widget, side: column + 1 }).range(cell.from));
          } else {
            const empty = cell.contentFrom === cell.contentTo;
            cells.push(Decoration.mark({ class: `cm-live-table-cell${empty ? " cm-live-table-empty-cell" : ""}`, attributes: {
              "data-column": String(column), "data-align": table.alignments[column] ?? "left", role: rowIndex === 0 ? "columnheader" : "cell",
            } }).range(cell.from, cell.to));
            // Keep blank padding editable; a replacement widget cannot receive
            // the browser's native text or composition input.
            if (empty) continue;
            hide(cell.from, cell.contentFrom);
            hide(cell.contentTo, cell.to);
            const source = view.state.doc.sliceString(cell.contentFrom, cell.contentTo);
            for (const match of source.matchAll(/<br\s*\/?\s*>/gi)) {
              const from = cell.contentFrom + match.index!;
              if (syntaxTree(view.state).resolve(from, 1).name !== "HTMLTag") continue;
              const to = from + match[0].length;
              decorations.push(Decoration.replace({ widget: new TableBreakWidget() }).range(from, to));
              atomic.push(Decoration.mark({}).range(from, to));
            }
            for (const match of source.matchAll(/\\\|/g)) {
              const from = cell.contentFrom + match.index!;
              hide(from, from + 1);
              atomic.push(Decoration.mark({}).range(from, from + 2));
            }
          }
        }
        hide(previous, row.to);
      });
      return false;
    } });
  }
  return { decorations: Decoration.set(decorations, true), cells: Decoration.set(cells, true), atomic: Decoration.set(atomic, true) };
}

function moveToCellEdge(view: EditorView, end: boolean): boolean {
  const active = activeCell(view.state);
  if (!active || view.state.readOnly) return false;
  selectCell(view, active.table, active.rowIndex, active.column, end);
  return true;
}

function leaveTable(view: EditorView, table: TableLayout, direction: number): void {
  const boundary = direction < 0 ? table.from : table.to;
  if (direction < 0 ? boundary === 0 : boundary === view.state.doc.length) {
    view.dispatch({ changes: { from: boundary, insert: "\n\n" }, selection: { anchor: direction < 0 ? 0 : boundary + 2 }, userEvent: "input" });
  } else {
    view.dispatch({ selection: { anchor: boundary + direction }, scrollIntoView: true });
  }
}

function moveVerticallyInTable(view: EditorView, direction: number): boolean {
  const active = activeCell(view.state);
  if (!active || view.state.readOnly) return false;
  const { table, rowIndex, column } = active;
  const rowDOM = Array.from(view.contentDOM.querySelectorAll<HTMLElement>(".cm-live-table-row"))
    .find((element) => view.state.doc.lineAt(view.posAtDOM(element)).from === table.rows[rowIndex].from);
  const cellDOM = rowDOM?.querySelector<HTMLElement>(`[data-column="${column}"]`);
  const caret = view.coordsAtPos(view.state.selection.main.head);
  const cellRect = cellDOM?.getBoundingClientRect();
  const x = caret && cellRect ? Math.max(cellRect.left + 10, Math.min(cellRect.right - 10, caret.left)) : 0;
  // CodeMirror's vertical movement assumes one continuous line. A grid row has
  // independent wrapped lines; hit-test inside the current/adjacent cell instead.
  const positionAt = (y: number, cell: MarkdownTableCellRange) => {
    const doc = view.dom.ownerDocument;
    const range = doc.caretRangeFromPoint?.(x, y);
    if (!range || !view.contentDOM.contains(range.startContainer)) return null;
    const position = view.posAtDOM(range.startContainer, range.startOffset);
    return Math.max(cell.contentFrom, Math.min(cell.contentTo, position));
  };
  if (caret && cellRect) {
    const y = (caret.top + caret.bottom) / 2 + direction * view.defaultLineHeight;
    if (y > cellRect.top + 7 && y < cellRect.bottom - 7) {
      const position = positionAt(y, active.cell);
      const nextCaret = position === null ? null : view.coordsAtPos(position);
      if (position !== null && nextCaret && Math.abs(nextCaret.top - caret.top) > view.defaultLineHeight / 2) {
        view.dispatch({ selection: EditorSelection.cursor(position, direction), scrollIntoView: true });
        return true;
      }
    }
  }
  const nextRow = table.rows[rowIndex + direction];
  if (!nextRow) {
    leaveTable(view, table, direction);
    return true;
  }
  const nextDOM = direction < 0 ? rowDOM?.previousElementSibling : rowDOM?.nextElementSibling;
  const targetDOM = nextDOM?.classList.contains("cm-live-collapsed-line")
    ? (direction < 0 ? nextDOM.previousElementSibling : nextDOM.nextElementSibling)
    : nextDOM;
  const nextCell = nextRow.cells[column];
  const nextRect = targetDOM?.querySelector(`[data-column="${column}"]`)?.getBoundingClientRect();
  const position = nextCell && nextRect && caret
    ? positionAt(direction < 0 ? nextRect.bottom - 8 : nextRect.top + 8, nextCell)
    : null;
  if (position === null) selectCell(view, table, rowIndex + direction, column, direction < 0);
  else view.dispatch({ selection: EditorSelection.cursor(position, direction), scrollIntoView: true });
  return true;
}

function moveCell(view: EditorView, direction: number, vertical = false): boolean {
  const active = activeCell(view.state);
  if (!active || view.state.readOnly) return false;
  const { table, rowIndex, column } = active;
  const count = table.alignments.length;
  const index = rowIndex * count + column + direction * (vertical ? count : 1);
  if (index < 0) {
    leaveTable(view, table, -1);
  } else if (index >= table.rows.length * count) {
    const insert = `\n| ${Array(count).fill("").join(" | ")} |`;
    const targetColumn = vertical ? column : 0;
    const cell = markdownTableCellRanges(insert.slice(1))[targetColumn];
    view.dispatch({ changes: { from: table.to, insert }, selection: { anchor: table.to + 1 + cell.contentFrom }, userEvent: "input" });
  } else selectCell(view, table, Math.floor(index / count), index % count);
  return true;
}

function protectBoundary(view: EditorView, direction: number, move: boolean): boolean {
  const active = activeCell(view.state);
  if (!active || !view.state.selection.main.empty || view.state.readOnly) return false;
  const { head } = view.state.selection.main;
  if (direction < 0 ? head > active.cell.contentFrom : head < active.cell.contentTo) return false;
  if (move) {
    const count = active.table.alignments.length;
    const next = active.rowIndex * count + active.column + direction;
    if (next >= 0 && next < count * active.table.rows.length) {
      selectCell(view, active.table, Math.floor(next / count), next % count, direction < 0);
    } else {
      leaveTable(view, active.table, direction);
    }
  }
  return true;
}

function insertCellText(view: EditorView, from: number, to: number, text: string): boolean {
  const active = activeCell(view.state);
  if (!active || view.state.readOnly) return false;
  const redirect = from === to && (from < active.cell.contentFrom || from > active.cell.contentTo);
  if (redirect) from = to = Math.max(active.cell.contentFrom, Math.min(active.cell.contentTo, from));
  if (from < active.cell.from || to > active.cell.to) return false;
  const normalized = text.replace(/\r\n?|\n/g, "<br>");
  const prefix = view.state.doc.sliceString(active.cell.from, from);
  const insert = normalized.replace(/\|/g, (_, offset: number) => {
    const before = prefix + normalized.slice(0, offset);
    const slashes = before.match(/\\+$/)?.[0].length ?? 0;
    return `${"\\".repeat(slashes % 2 === 0 ? 1 : 2)}|`;
  });
  if (text === insert && !redirect) return false;
  view.dispatch({ changes: { from, to, insert }, selection: { anchor: from + insert.length }, userEvent: "input" });
  return true;
}

export function markdownTableEditing() {
  const plugin = ViewPlugin.fromClass(class {
    decorations: DecorationSet;
    cells: DecorationSet;
    atomic: DecorationSet;
    constructor(view: EditorView) {
      const result = tableDecorations(view);
      this.decorations = result.decorations;
      this.cells = result.cells;
      this.atomic = result.atomic;
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged || syntaxTree(update.state) !== syntaxTree(update.startState)) {
        const result = tableDecorations(update.view);
        this.decorations = result.decorations;
        this.cells = result.cells;
        this.atomic = result.atomic;
      }
    }
  }, {
    decorations: (value) => value.decorations,
    provide: (plugin) => [
      // Keep a single cell wrapper even when syntax highlighting splits text.
      EditorView.outerDecorations.of((view) => view.plugin(plugin)?.cells ?? Decoration.none),
      EditorView.atomicRanges.of((view) => view.plugin(plugin)?.atomic ?? Decoration.none),
    ],
  });
  return [plugin, Prec.high(keymap.of([
    { key: "Tab", run: (view) => moveCell(view, 1) },
    { key: "Shift-Tab", run: (view) => moveCell(view, -1) },
    { key: "Enter", run: (view) => moveCell(view, 1, true) },
    { key: "Shift-Enter", run: (view) => insertCellText(view, view.state.selection.main.from, view.state.selection.main.to, "\n") },
    { key: "Backspace", run: (view) => protectBoundary(view, -1, false) },
    { key: "Delete", run: (view) => protectBoundary(view, 1, false) },
    { key: "ArrowLeft", run: (view) => protectBoundary(view, -1, true) },
    { key: "ArrowRight", run: (view) => protectBoundary(view, 1, true) },
    { key: "ArrowUp", run: (view) => moveVerticallyInTable(view, -1) },
    { key: "ArrowDown", run: (view) => moveVerticallyInTable(view, 1) },
    { key: "Home", run: (view) => moveToCellEdge(view, false) },
    { key: "End", run: (view) => moveToCellEdge(view, true) },
  ])), Prec.high(EditorView.inputHandler.of(insertCellText)), Prec.high(EditorView.domEventHandlers({
    paste(event, view) {
      if (view.state.readOnly || !activeCell(view.state)) return false;
      const html = event.clipboardData?.getData("text/html") ?? "";
      if (/<table\b/i.test(html)) {
        const table = new DOMParser().parseFromString(html, "text/html").querySelector("table");
        if (table && pasteMarkdownTable(view, Array.from(table.rows).map((row) => Array.from(row.cells).map((cell) => htmlToEditorMarkdown(cell.innerHTML))), true)) {
          event.preventDefault();
          return true;
        }
      }
      const text = event.clipboardData?.getData("text/plain");
      if (!text) return false;
      if (text.includes("\t") && pasteMarkdownTable(view, parseClipboardTable(text))) {
        event.preventDefault();
        return true;
      }
      const { from, to } = view.state.selection.main;
      if (!insertCellText(view, from, to, text)) return false;
      event.preventDefault();
      return true;
    },
  }))];
}
