import { indentLess, indentMore } from "@codemirror/commands";
import { syntaxTree } from "@codemirror/language";
import { deleteMarkupBackward, insertNewlineContinueMarkup } from "@codemirror/lang-markdown";
import { EditorSelection, EditorState, Prec, type Transaction } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import type { SyntaxNode } from "@lezer/common";

export type MarkdownFormat = "bold" | "italic" | "strike" | "code";
const formatNodes = { bold: "StrongEmphasis", italic: "Emphasis", strike: "Strikethrough", code: "InlineCode" };
const formatMarkers = { bold: "**", italic: "*", strike: "~~", code: "`" };

export function markdownNodeAt(state: EditorState, position: number, names: readonly string[]): SyntaxNode | null {
  for (const bias of [1, -1] as const) {
    let node: SyntaxNode | null = syntaxTree(state).resolve(position, bias);
    while (node) {
      if (names.includes(node.name)) return node;
      node = node.parent;
    }
  }
  return null;
}

export function inlineContentRange(node: SyntaxNode): { from: number; to: number } {
  if (node.name === "Link") {
    const marks = node.getChildren("LinkMark");
    return { from: marks[0]?.to ?? node.from, to: marks[1]?.from ?? node.to };
  }
  return { from: node.firstChild?.to ?? node.from, to: node.lastChild?.from ?? node.to };
}

export function toggleMarkdownFormat(view: EditorView, format: MarkdownFormat): boolean {
  if (view.state.readOnly) return false;
  const selection = view.state.selection.main;
  const node = markdownNodeAt(view.state, selection.from, [formatNodes[format]]);
  if (node && selection.to <= node.to) {
    const content = inlineContentRange(node);
    view.dispatch({ changes: [
      { from: node.from, to: content.from, insert: "" },
      { from: content.to, to: node.to, insert: "" },
    ], userEvent: "input.format" });
  } else {
    const marker = formatMarkers[format];
    const range = selection.empty ? view.state.wordAt(selection.head) ?? selection : selection;
    const text = view.state.sliceDoc(range.from, range.to) || "文字";
    if (text.includes("\n")) return false;
    view.dispatch({
      changes: { from: range.from, to: range.to, insert: `${marker}${text}${marker}` },
      selection: EditorSelection.range(range.from + marker.length, range.from + marker.length + text.length),
      userEvent: "input.format",
    });
  }
  view.focus();
  return true;
}

export function setMarkdownHeading(view: EditorView, level: number): boolean {
  if (view.state.readOnly || markdownNodeAt(view.state, view.state.selection.main.head, ["Table", "FencedCode", "CodeBlock"])) return false;
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  const prefix = line.text.match(/^\s{0,3}#{1,6}\s+/)?.[0] ?? "";
  const insert = level > 0 ? `${"#".repeat(Math.min(6, level))} ` : "";
  const setext = markdownNodeAt(view.state, view.state.selection.main.head, ["SetextHeading1", "SetextHeading2"]);
  const underline = setext?.getChild("HeaderMark");
  const changes = [{ from: line.from, to: line.from + prefix.length, insert }];
  if (underline) changes.push({ from: view.state.doc.lineAt(underline.from).from - 1, to: underline.to, insert: "" });
  view.dispatch({ changes, userEvent: "input.format" });
  view.focus();
  return true;
}

function deleteVisualBoundary(view: EditorView, direction: -1 | 1): boolean {
  if (view.state.readOnly) return false;
  const selection = view.state.selection.main;
  const node = markdownNodeAt(view.state, selection.from, [...Object.values(formatNodes), "Link"]);
  if (node && selection.to <= node.to) {
    const content = inlineContentRange(node);
    const text = view.state.sliceDoc(content.from, content.to);
    if (selection.empty && Array.from(text).length === 1 && (direction < 0 ? selection.head >= content.to : selection.head <= content.from)) {
      view.dispatch({ changes: { from: node.from, to: node.to, insert: "" }, selection: { anchor: node.from }, userEvent: "delete" });
      return true;
    }
    if (selection.empty && (direction < 0 && selection.head > content.to || direction > 0 && selection.head < content.from)) {
      const char = direction < 0 ? Array.from(text).slice(-1)[0] : Array.from(text)[0];
      if (char) {
        const from = direction < 0 ? content.to - char.length : content.from;
        view.dispatch({ changes: { from, to: from + char.length, insert: "" }, selection: { anchor: from }, userEvent: "delete" });
        return true;
      }
    }
    if (!selection.empty && selection.from <= content.from && selection.to >= content.to) {
      view.dispatch({ changes: { from: node.from, to: node.to, insert: "" }, selection: { anchor: node.from }, userEvent: "delete" });
      return true;
    }
    if (selection.empty && (direction < 0 ? selection.head <= content.from : selection.head >= content.to)) {
      view.dispatch({ changes: [
        { from: node.from, to: content.from, insert: "" },
        { from: content.to, to: node.to, insert: "" },
      ], userEvent: "delete.format" });
      return true;
    }
  }
  if (direction < 0 && selection.empty) {
    const line = view.state.doc.lineAt(selection.head);
    const heading = line.text.match(/^\s{0,3}#{1,6}\s+/);
    if (heading && selection.head <= line.from + heading[0].length) {
      view.dispatch({ changes: { from: line.from, to: line.from + heading[0].length, insert: "" }, userEvent: "delete.format" });
      return true;
    }
    return deleteMarkupBackward(view);
  }
  return false;
}

function indentList(view: EditorView, outdent: boolean): boolean {
  if (view.state.readOnly || !markdownNodeAt(view.state, view.state.selection.main.head, ["ListItem", "Blockquote"])) return false;
  if (!markdownNodeAt(view.state, view.state.selection.main.head, ["ListItem"])) {
    const line = view.state.doc.lineAt(view.state.selection.main.head);
    const prefix = line.text.match(/^\s{0,3}> ?/);
    if (!prefix) return false;
    view.dispatch({ changes: { from: line.from, to: outdent ? line.from + prefix[0].length : line.from, insert: outdent ? "" : "> " }, userEvent: "input.indent" });
    return true;
  }
  return outdent ? indentLess(view) : indentMore(view);
}

function visualEnter(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  const selection = view.state.selection.main;
  const line = view.state.doc.lineAt(selection.head);
  if (selection.empty && /^\s*(?:[-*+]|\d+[.)])\s*(?:\[[ xX]\]\s*)?$/.test(line.text)) {
    const indent = line.text.match(/^\s*/)?.[0].length ?? 0;
    view.dispatch({ changes: { from: line.from, to: indent >= 2 ? line.from + 2 : line.to, insert: "" }, selection: { anchor: indent >= 2 ? line.to - 2 : line.from }, userEvent: "input" });
    return true;
  }
  return insertNewlineContinueMarkup(view);
}

function typedAtxHeadingSpacePosition(transaction: Transaction): number | null {
  if (!transaction.docChanged || !transaction.isUserEvent("input.type")) return null;

  let spacePosition: number | null = null;
  let changeCount = 0;
  transaction.changes.iterChanges((fromA, toA, fromB, _toB, inserted) => {
    changeCount += 1;
    if (changeCount > 1 || fromA !== toA) return;
    const text = inserted.toString();
    if (!text || /^\s|^#/.test(text)) return;

    const line = transaction.startState.doc.lineAt(fromA);
    if (fromA !== line.to || !/^ {0,3}#{1,6}$/.test(line.text)) return;
    spacePosition = fromB;
  });
  return changeCount === 1 ? spacePosition : null;
}

export function markdownVisualCommands() {
  return [EditorState.transactionFilter.of((transaction) => {
    const headingSpacePosition = typedAtxHeadingSpacePosition(transaction);
    if (headingSpacePosition !== null) {
      return [
        transaction,
        {
          changes: { from: headingSpacePosition, insert: " " },
          sequential: true,
        },
      ];
    }
    if (!transaction.selection) return transaction;
    const state = transaction.state;
    const ranges = state.selection.ranges.map((range) => {
      if (!range.empty) return range;
      const position = range.head;
      const node = syntaxTree(state).resolve(position, 1);
      let anchor = position;
      if (node.name === "HeaderMark" && node.parent?.name.startsWith("SetextHeading")) {
        anchor = node.parent.from;
      } else if (node.name === "HeaderMark" || node.name === "QuoteMark" || node.name === "ListMark" || node.name === "TaskMarker") {
        anchor = node.to;
        if (state.sliceDoc(anchor, anchor + 1) === " ") anchor++;
      } else if (node.name === "EmphasisMark" || node.name === "StrikethroughMark" || node.name === "CodeMark" && node.parent?.name === "InlineCode") {
        anchor = node.from === node.parent?.from ? node.to : node.from;
      } else if (["LinkMark", "URL", "LinkTitle"].includes(node.name) && node.parent?.name === "Link") {
        const content = inlineContentRange(node.parent);
        anchor = position < content.from ? content.from : content.to;
      }
      return anchor === position ? range : EditorSelection.cursor(anchor, range.assoc);
    });
    const selection = EditorSelection.create(ranges, state.selection.mainIndex);
    return selection.eq(state.selection) ? transaction : [transaction, { selection, sequential: true }];
  }), Prec.high(keymap.of([
    { key: "Mod-b", run: (view) => toggleMarkdownFormat(view, "bold") },
    { key: "Mod-i", run: (view) => toggleMarkdownFormat(view, "italic") },
    { key: "Mod-Shift-x", run: (view) => toggleMarkdownFormat(view, "strike") },
    { key: "Mod-e", run: (view) => toggleMarkdownFormat(view, "code") },
    ...Array.from({ length: 7 }, (_, level) => ({ key: `Mod-Alt-${level}`, run: (view: EditorView) => setMarkdownHeading(view, level) })),
    { key: "Tab", run: (view) => indentList(view, false) },
    { key: "Shift-Tab", run: (view) => indentList(view, true) },
    { key: "Backspace", run: (view) => deleteVisualBoundary(view, -1) },
    { key: "Delete", run: (view) => deleteVisualBoundary(view, 1) },
    { key: "Enter", run: visualEnter },
  ]))];
}
