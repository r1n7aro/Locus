import TurndownService from "turndown";
import { gfm } from "turndown-plugin-gfm";
import DOMPurify from "dompurify";
import { Prec } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { markdownNodeAt } from "./markdownVisualCommands";

let converter: TurndownService | null = null;
export function htmlToEditorMarkdown(html: string): string {
  const clean = DOMPurify.sanitize(html, { FORBID_TAGS: ["style", "script", "iframe", "form", "button"] });
  if (!converter) {
    converter = new TurndownService({ headingStyle: "atx", codeBlockStyle: "fenced", bulletListMarker: "-", emDelimiter: "*" });
    converter.use(gfm);
    converter.addRule("tableCellBreak", {
      filter: (node) => node.nodeName === "BR" && !!node.closest("td,th"),
      replacement: () => "<br>",
    });
    converter.addRule("editorTableCell", {
      filter: ["th", "td"],
      replacement: (content, node) => {
        const cell = node as HTMLTableCellElement;
        const text = content.trim().replace(/\n+/g, "<br>");
        const escaped = text.replace(/\|/g, (_, offset: number) => (text.slice(0, offset).match(/\\+$/)?.[0].length ?? 0) % 2 === 0 ? "\\|" : "|");
        return `${cell.cellIndex === 0 ? "| " : " "}${escaped} |`;
      },
    });
    converter.addRule("editorTableRow", {
      filter: "tr",
      replacement: (content, node) => {
        const row = node as HTMLTableRowElement;
        const header = row.closest("table")?.rows[0] === row;
        const separator = header ? `\n| ${Array.from(row.cells).map((cell) => {
          const align = cell.getAttribute("align") || cell.style.textAlign;
          return align === "center" ? ":---:" : align === "right" ? "---:" : align === "left" ? ":---" : "---";
        }).join(" | ")} |` : "";
        return `\n${content}${separator}`;
      },
    });
  }
  const document = new DOMParser().parseFromString(clean, "text/html");
  // Clipboard tables from spreadsheets often have no TH row. GFM needs a
  // header, so use the first row without discarding any cell values.
  for (const table of document.querySelectorAll("table")) {
    const first = table.rows[0];
    if (first && !first.querySelector("th")) for (const cell of Array.from(first.cells)) {
      const header = document.createElement("th");
      header.innerHTML = cell.innerHTML;
      if (cell.getAttribute("align")) header.setAttribute("align", cell.getAttribute("align")!);
      header.style.textAlign = cell.style.textAlign;
      cell.replaceWith(header);
    }
  }
  return converter.turndown(document.body).replace(/\r\n?/g, "\n");
}

export function markdownRichPaste() {
  return Prec.high(EditorView.domEventHandlers({
    paste(event, view) {
      if (view.state.readOnly || markdownNodeAt(view.state, view.state.selection.main.head, ["FencedCode", "CodeBlock", "InlineCode", "Table"])) return false;
      const html = event.clipboardData?.getData("text/html") ?? "";
      if (!html || !/<(?:h[1-6]|p|div|ul|ol|li|table|strong|b|em|i|s|del|a|img|pre|blockquote)\b/i.test(html)) return false;
      const insert = htmlToEditorMarkdown(html);
      if (!insert) return false;
      event.preventDefault();
      view.dispatch({ ...view.state.replaceSelection(insert), userEvent: "input.paste" });
      return true;
    },
  }));
}
