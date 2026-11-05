/**
 * Inline file references in rendered markdown show a short label (usually a
 * file name) for a longer reference text (the path). They join text
 * selections that pass over them, while a press on one starts a reference drag
 * rather than a selection. Copying a selection that covers references writes
 * each one's full reference text instead of its label, using the browser's own
 * selection serialization for everything else.
 */

const REFERENCE_SELECTOR = ".md-file-ref, .md-workspace-ref, .md-asset-chip";
const MARKDOWN_REFERENCE_SELECTOR = ".markdown-body .md-file-ref, .markdown-body .md-workspace-ref, .markdown-body .md-asset-chip";
/** Parts of a reference that are not its label text; they copy as nothing. */
const REFERENCE_DECORATION_SELECTOR = "svg, .md-workspace-ref-prefix";
/** Copied HTML drops controls and icons the selection never shows as text. */
const COPY_HTML_DROP_SELECTOR = "button, svg, img.md-ref-icon, .ui-select-none, .streaming-cursor";

function elementOf(target: EventTarget | Node | null | undefined): Element | null {
  if (!target || typeof (target as Node).nodeType !== "number") return null;
  const node = target as Node;
  return node.nodeType === 1 ? node as Element : node.parentElement;
}

/**
 * The inline markdown reference containing `target`, if any. Block-level Unity
 * previews (rows, inspectors, editors) are not inline references.
 */
export function selectableMarkdownReference(target: EventTarget | Node | null | undefined): HTMLElement | null {
  const reference = elementOf(target)?.closest<HTMLElement>(REFERENCE_SELECTOR);
  if (!reference?.closest(".markdown-body")) return null;
  const level = reference.dataset.mdUnityLevel;
  if (level && level !== "inline") return null;
  return reference;
}

/** Full text a reference stands for: its path (with line or folder slash). */
export function markdownReferenceCopyText(reference: Element): string {
  return reference.getAttribute("title")?.trim() || reference.textContent?.trim() || "";
}

/** Text nodes of a reference with the text each holds while it is copied. */
function referenceCopyTextNodes(reference: HTMLElement): Array<{ node: Text; text: string }> {
  const doc = reference.ownerDocument;
  const walker = doc.createTreeWalker(reference, 4 /* NodeFilter.SHOW_TEXT */);
  const nodes: Array<{ node: Text; text: string }> = [];
  let labelWritten = false;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const decoration = !!node.parentElement?.closest(REFERENCE_DECORATION_SELECTOR);
    const text = decoration || labelWritten ? "" : markdownReferenceCopyText(reference);
    if (!decoration) labelWritten = true;
    nodes.push({ node: node as Text, text });
  }
  return labelWritten ? nodes : [];
}

interface SelectionBoundary {
  anchorNode: Node;
  anchorOffset: number;
  focusNode: Node;
  focusOffset: number;
}

function readBoundary(selection: Selection): SelectionBoundary | null {
  const { anchorNode, focusNode } = selection;
  if (!anchorNode || !focusNode) return null;
  return {
    anchorNode,
    anchorOffset: selection.anchorOffset,
    focusNode,
    focusOffset: selection.focusOffset,
  };
}

/** A selection that starts or ends inside a reference covers all of it. */
function expandOverReferences(range: Range): Range {
  const expanded = range.cloneRange();
  const startReference = selectableMarkdownReference(range.startContainer);
  if (startReference) expanded.setStartBefore(startReference);
  const endReference = selectableMarkdownReference(range.endContainer);
  if (endReference) expanded.setEndAfter(endReference);
  return expanded;
}

function selectionHtml(doc: Document, range: Range): string {
  const container = doc.createElement("div");
  container.append(range.cloneContents());
  for (const element of container.querySelectorAll(COPY_HTML_DROP_SELECTOR)) element.remove();
  for (const reference of container.querySelectorAll(REFERENCE_SELECTOR)) {
    reference.replaceWith(doc.createTextNode(markdownReferenceCopyText(reference)));
  }
  return container.innerHTML;
}

export interface MarkdownReferenceClipboard {
  text: string;
  html: string;
}

/**
 * Serializes the current selection with references written as their full
 * text, or returns null when it covers no markdown reference (the browser's
 * default copy then applies). The document and selection are restored before
 * returning.
 */
export function serializeMarkdownReferenceSelection(doc: Document): MarkdownReferenceClipboard | null {
  const selection = doc.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount !== 1) return null;
  const references = [...doc.querySelectorAll<HTMLElement>(MARKDOWN_REFERENCE_SELECTOR)]
    .filter((reference) => selectableMarkdownReference(reference) === reference && selection.containsNode(reference, true));
  if (references.length === 0) return null;

  const boundary = readBoundary(selection);
  if (!boundary) return null;
  const swapped: Array<{ node: Text; data: string }> = [];
  try {
    const range = expandOverReferences(selection.getRangeAt(0));
    selection.removeAllRanges();
    selection.addRange(range);
    // Swap labels only after the range sits outside every reference: editing
    // a text node that holds a range boundary would move that boundary.
    for (const reference of references) {
      for (const { node, text } of referenceCopyTextNodes(reference)) {
        swapped.push({ node, data: node.data });
        node.data = text;
      }
    }
    return { text: selection.toString(), html: selectionHtml(doc, range) };
  } finally {
    for (const { node, data } of swapped) node.data = data;
    try {
      selection.setBaseAndExtent(boundary.anchorNode, boundary.anchorOffset, boundary.focusNode, boundary.focusOffset);
    } catch {
      // The selected nodes are unchanged; a failed restore only drops the selection.
    }
  }
}

export function handleMarkdownReferenceCopy(event: ClipboardEvent): void {
  if (event.defaultPrevented || !event.clipboardData) return;
  const doc = elementOf(event.target)?.ownerDocument
    ?? ((event.target as Document | null)?.nodeType === 9 ? event.target as Document : null);
  if (!doc) return;
  const clipboard = serializeMarkdownReferenceSelection(doc);
  if (!clipboard) return;
  event.clipboardData.setData("text/plain", clipboard.text);
  event.clipboardData.setData("text/html", clipboard.html);
  event.preventDefault();
}

/** A press on a reference starts a reference drag (or a click), not a text selection. */
export function handleMarkdownReferenceSelectStart(event: Event): void {
  if (selectableMarkdownReference(event.target)) event.preventDefault();
}

const installedDocuments = new WeakSet<Document>();

/** Installs the copy handler once per document. */
export function installMarkdownReferenceCopy(doc: Document): void {
  if (installedDocuments.has(doc)) return;
  installedDocuments.add(doc);
  doc.addEventListener("copy", handleMarkdownReferenceCopy);
}
