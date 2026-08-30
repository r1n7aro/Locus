import type { Extension, Range } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import { isMarkdownImageSource } from "../../../composables/markdownImages";
import {
  parseUnityPropertyFence,
  unityPropertyFenceUnitySelectionTarget,
} from "../../../composables/unityPropertyFence";
import {
  findMarkdownMathTokens,
  findPlainMarkdownReferences,
  isUnityPropertyFenceLanguage,
  isUnityReferenceFenceLanguage,
  parseInlineMarkdownReference,
  parseMarkdownTable,
  type MarkdownReferenceToken,
} from "./markdownComplexTokens";
import {
  CollapsedSourceWidget,
  MarkdownImageWidget,
  MarkdownMathWidget,
  MarkdownReferenceWidget,
  MarkdownTableRowWidget,
  type MarkdownLivePreviewOptions,
} from "./markdownComplexWidgets";

function selectionTouches(view: EditorView, from: number, to: number): boolean {
  if (!view.hasFocus || view.state.readOnly) return false;
  return view.state.selection.ranges.some((range) => {
    if (range.empty) return range.head >= from && range.head <= to;
    return range.from < to && range.to > from;
  });
}

interface MarkdownSyntaxRange {
  from: number;
  to: number;
  name: string;
  parent: MarkdownSyntaxRange | null;
}

interface MarkdownSyntaxNode extends MarkdownSyntaxRange {
  firstChild: MarkdownSyntaxNode | null;
  nextSibling: MarkdownSyntaxNode | null;
}

function closestSyntaxNode(
  node: MarkdownSyntaxRange,
  name: string,
): MarkdownSyntaxNode | null {
  let current: MarkdownSyntaxRange | null = node;
  while (current) {
    if (current.name === name) return current as MarkdownSyntaxNode;
    current = current.parent;
  }
  return null;
}

function linkTargetActivationRange(
  node: MarkdownSyntaxNode,
): { from: number; to: number } | null {
  const link = closestSyntaxNode(node, "Link");
  if (!link) return null;
  const children = syntaxChildren(link);
  const urlIndex = children.findIndex((child) => child.name === "URL");
  if (urlIndex < 0) return null;

  const url = children[urlIndex];
  const openingMark = children
    .slice(0, urlIndex)
    .reverse()
    .find((child) => child.name === "LinkMark");
  const closingMark = children
    .slice(urlIndex + 1)
    .find((child) => child.name === "LinkMark");
  const target = {
    from: openingMark?.from ?? url.from,
    to: closingMark?.to ?? url.to,
  };
  return node.from >= target.from && node.to <= target.to ? target : null;
}

function linkTitleHiddenFrom(node: MarkdownSyntaxNode): number {
  if (node.name !== "LinkTitle") return node.from;
  const link = closestSyntaxNode(node, "Link");
  if (!link) return node.from;
  const children = syntaxChildren(link);
  const titleIndex = children.findIndex((child) => (
    child.name === node.name && child.from === node.from && child.to === node.to
  ));
  if (titleIndex < 0) return node.from;
  return children[titleIndex - 1]?.to ?? node.from;
}

function fencedCodeOpeningActivationRange(
  node: MarkdownSyntaxNode,
): { from: number; to: number } | null {
  const fencedCode = closestSyntaxNode(node, "FencedCode");
  if (!fencedCode) return null;
  const children = syntaxChildren(fencedCode);
  const openingMark = children.find((child) => child.name === "CodeMark");
  const info = children.find((child) => child.name === "CodeInfo");
  if (!openingMark || !info) return null;
  if (
    (node.name !== "CodeMark" && node.name !== "CodeInfo")
    || node.from < openingMark.from
    || node.to > info.to
  ) {
    return null;
  }
  return { from: openingMark.from, to: info.to };
}

function inlineReferenceWidgetActivationRange(
  view: EditorView,
  node: MarkdownSyntaxNode,
): { from: number; to: number } | null {
  const inlineCode = closestSyntaxNode(node, "InlineCode");
  if (!inlineCode) return null;
  const raw = view.state.doc.sliceString(inlineCode.from, inlineCode.to);
  const reference = parseInlineMarkdownReference(raw, inlineCode.from);
  if (!reference) return null;
  const parent = inlineCode.parent as MarkdownSyntaxNode | null;
  const standaloneView = reference.kind !== "view"
    || (parent?.name === "Paragraph"
      && view.state.doc.sliceString(parent.from, parent.to).trim() === raw.trim());
  return standaloneView ? inlineCode : null;
}

function specializedFenceWidgetActivationRange(
  view: EditorView,
  node: MarkdownSyntaxNode,
): { from: number; to: number } | null {
  const fencedCode = closestSyntaxNode(node, "FencedCode");
  if (!fencedCode) return null;
  const info = syntaxChild(fencedCode, "CodeInfo");
  const language = info
    ? view.state.doc.sliceString(info.from, info.to).trim()
    : "";
  return isUnityReferenceFenceLanguage(language) || isUnityPropertyFenceLanguage(language)
    ? fencedCode
    : null;
}

function syntaxTokenActivationRange(
  view: EditorView,
  node: MarkdownSyntaxNode,
): { from: number; to: number } {
  return inlineReferenceWidgetActivationRange(view, node)
    ?? specializedFenceWidgetActivationRange(view, node)
    ?? linkTargetActivationRange(node)
    ?? fencedCodeOpeningActivationRange(node)
    ?? { from: node.from, to: node.to };
}

function syntaxTokenIsActive(view: EditorView, node: MarkdownSyntaxNode): boolean {
  const range = syntaxTokenActivationRange(view, node);
  return selectionTouches(view, range.from, range.to);
}

class ListMarkerWidget extends WidgetType {
  constructor(private readonly marker: string) {
    super();
  }

  eq(other: ListMarkerWidget): boolean {
    return other.marker === this.marker;
  }

  toDOM(): HTMLElement {
    const marker = document.createElement("span");
    marker.className = "cm-live-list-marker";
    marker.textContent = /^\d/.test(this.marker) ? this.marker.trim() : "•";
    marker.setAttribute("aria-hidden", "true");
    return marker;
  }
}

class TaskCheckboxWidget extends WidgetType {
  constructor(
    private readonly from: number,
    private readonly checked: boolean,
    private readonly readOnly: boolean,
  ) {
    super();
  }

  eq(other: TaskCheckboxWidget): boolean {
    return other.from === this.from
      && other.checked === this.checked
      && other.readOnly === this.readOnly;
  }

  toDOM(view: EditorView): HTMLElement {
    const checkbox = document.createElement("input");
    checkbox.className = "cm-live-task-checkbox";
    checkbox.type = "checkbox";
    checkbox.checked = this.checked;
    checkbox.disabled = this.readOnly;
    checkbox.setAttribute("aria-label", this.checked ? "标记为未完成" : "标记为已完成");
    checkbox.addEventListener("change", () => {
      if (view.state.readOnly) return;
      view.dispatch({
        changes: {
          from: this.from + 1,
          to: this.from + 2,
          insert: this.checked ? " " : "x",
        },
      });
      view.focus();
    });
    return checkbox;
  }
}

class HorizontalRuleWidget extends WidgetType {
  toDOM(): HTMLElement {
    const rule = document.createElement("span");
    rule.className = "cm-live-horizontal-rule";
    rule.setAttribute("aria-hidden", "true");
    return rule;
  }
}

const MAX_TABLE_SOURCE_CHARS = 100_000;
const MAX_TABLE_ROWS = 200;
const MAX_TABLE_COLUMNS = 40;
const MAX_FENCE_SOURCE_CHARS = 100_000;
const MAX_FENCE_LINES = 240;
const MAX_MATH_BLOCK_LINES = 80;
const COMPLEX_SCAN_OVERSCAN_CHARS = 4096;
const COMPLEX_SCAN_PROTECTED_NAMES = new Set([
  "Table",
  "Image",
  "Link",
  "Autolink",
  "InlineCode",
  "FencedCode",
  "CodeBlock",
  "HTMLBlock",
  "HTMLTag",
]);

function syntaxChildren(node: MarkdownSyntaxNode): MarkdownSyntaxNode[] {
  const children: MarkdownSyntaxNode[] = [];
  let child = node.firstChild;
  while (child) {
    children.push(child);
    child = child.nextSibling;
  }
  return children;
}

function syntaxChild(node: MarkdownSyntaxNode, name: string): MarkdownSyntaxNode | null {
  return syntaxChildren(node).find((child) => child.name === name) ?? null;
}

function syntaxRangeIsProtected(
  tree: ReturnType<typeof syntaxTree>,
  from: number,
  to: number,
): boolean {
  const position = Math.max(from, Math.min(Math.max(from, to - 1), from + 1));
  let node = tree.resolveInner(position, 1) as unknown as MarkdownSyntaxNode | null;
  while (node) {
    if (COMPLEX_SCAN_PROTECTED_NAMES.has(node.name)) return true;
    node = node.parent as MarkdownSyntaxNode | null;
  }
  return false;
}

function parseImageSyntax(
  view: EditorView,
  node: MarkdownSyntaxNode,
): { source: string; alt: string } | null {
  const children = syntaxChildren(node);
  const url = children.find((child) => child.name === "URL");
  const closingAlt = children.find((child) => (
    child.name === "LinkMark"
    && view.state.doc.sliceString(child.from, child.to) === "]"
  ));
  if (!url || !closingAlt || closingAlt.from < node.from + 2) return null;
  const source = view.state.doc.sliceString(url.from, url.to).trim();
  if (!source || /^(?:javascript|vbscript):/i.test(source) || /^data:(?!image\/)/i.test(source)) {
    return null;
  }
  return {
    source,
    alt: view.state.doc.sliceString(node.from + 2, closingAlt.from),
  };
}

function linkImageSyntax(
  view: EditorView,
  node: MarkdownSyntaxNode,
): { source: string; alt: string } | null {
  const children = syntaxChildren(node);
  const url = children.find((child) => child.name === "URL");
  if (!url) return null;
  const source = view.state.doc.sliceString(url.from, url.to).trim();
  if (!isMarkdownImageSource(source)) return null;

  if (node.name === "Autolink") return { source, alt: "" };
  const marks = children.filter((child) => child.name === "LinkMark");
  if (marks.length < 2) return null;
  const label = view.state.doc.sliceString(marks[0].to, marks[1].from).trim();
  return label === source ? { source, alt: "" } : null;
}

function referenceForFenceLine(
  source: string,
  from: number,
): MarkdownReferenceToken | null {
  const leading = source.length - source.trimStart().length;
  const trimmed = source.trim();
  if (!trimmed) return null;
  const absoluteFrom = from + leading;
  const direct = parseInlineMarkdownReference(trimmed, absoluteFrom);
  if (direct) return direct;
  const explicit = parseInlineMarkdownReference(`\`${trimmed}\``, absoluteFrom);
  return explicit ? {
    ...explicit,
    from: absoluteFrom,
    to: absoluteFrom + trimmed.length,
    raw: trimmed,
  } : null;
}

function buildLivePreviewDecorations(
  view: EditorView,
  options: MarkdownLivePreviewOptions = {},
): DecorationSet {
  const decorations: Array<Range<Decoration>> = [];
  const seen = new Set<string>();
  const occupiedRanges: Array<{ from: number; to: number }> = [];

  const add = (from: number, to: number, decoration: Decoration, key: string) => {
    const identity = `${key}:${from}:${to}`;
    if (seen.has(identity)) return;
    seen.add(identity);
    decorations.push(decoration.range(from, to));
  };

  const addLine = (position: number, className: string) => {
    const lineFrom = view.state.doc.lineAt(position).from;
    add(lineFrom, lineFrom, Decoration.line({ class: className }), `line:${className}`);
  };

  const overlapsOccupied = (from: number, to: number) => occupiedRanges.some(
    (range) => from < range.to && to > range.from,
  );

  const occupy = (from: number, to: number) => {
    occupiedRanges.push({ from, to });
  };

  const collapseLine = (lineFrom: number, lineTo: number, key: string) => {
    addLine(lineFrom, "cm-live-collapsed-line");
    if (lineTo > lineFrom) {
      add(
        lineFrom,
        lineTo,
        Decoration.replace({ widget: new CollapsedSourceWidget() }),
        `collapsed:${key}`,
      );
    }
  };

  const tree = syntaxTree(view.state);
  for (const visibleRange of view.visibleRanges) {
    tree.iterate({
      from: visibleRange.from,
      to: visibleRange.to,
      enter(nodeRef) {
        const node = nodeRef.node;
        const { name, from, to } = nodeRef;

        if (name === "Table") {
          if (to - from > MAX_TABLE_SOURCE_CHARS) return false;
          const source = view.state.doc.sliceString(from, to);
          const model = parseMarkdownTable(source);
          if (
            !model
            || model.header.length > MAX_TABLE_COLUMNS
            || model.rows.length + 1 > MAX_TABLE_ROWS
            || selectionTouches(view, from, to)
          ) return false;

          const lines: Array<{ from: number; to: number }> = [];
          let line = view.state.doc.lineAt(from);
          while (line.from <= to) {
            lines.push({ from: line.from, to: line.to });
            if (line.to >= to || line.number >= view.state.doc.lines) break;
            line = view.state.doc.line(line.number + 1);
          }
          const tableRows = [model.header, ...model.rows];
          if (lines.length !== tableRows.length + 1) return false;

          occupy(from, to);
          for (let index = 0; index < lines.length; index += 1) {
            const currentLine = lines[index];
            if (index === 1) {
              collapseLine(currentLine.from, currentLine.to, `table-separator:${from}`);
              continue;
            }
            const rowIndex = index === 0 ? 0 : index - 1;
            addLine(currentLine.from, "cm-live-table-line");
            add(
              currentLine.from,
              currentLine.to,
              Decoration.replace({
                widget: new MarkdownTableRowWidget(
                  tableRows[rowIndex] ?? [],
                  model.alignments,
                  rowIndex === 0,
                  rowIndex,
                  tableRows.length,
                  currentLine.from,
                  currentLine.to,
                  from,
                  to,
                ),
              }),
              `table-row:${rowIndex}`,
            );
          }
          return false;
        }

        if (name === "Paragraph") {
          const raw = view.state.doc.sliceString(from, to);
          const trimmed = raw.trim();
          if (trimmed && isMarkdownImageSource(trimmed) && !selectionTouches(view, from, to)) {
            const leading = raw.length - raw.trimStart().length;
            const source = trimmed.replace(/^(["'])([\s\S]+)\1$/, "$2");
            occupy(from, to);
            add(
              from + leading,
              from + leading + trimmed.length,
              Decoration.replace({
                widget: new MarkdownImageWidget(source, "", from, to, options),
              }),
              "standalone-image",
            );
            return false;
          }
        }

        if (name === "Image") {
          const image = parseImageSyntax(view, node as unknown as MarkdownSyntaxNode);
          if (!image || selectionTouches(view, from, to)) return false;
          occupy(from, to);
          add(
            from,
            to,
            Decoration.replace({
              widget: new MarkdownImageWidget(image.source, image.alt, from, to, options),
            }),
            "image",
          );
          return false;
        }

        if (name === "Link" || name === "Autolink") {
          const image = linkImageSyntax(view, node as unknown as MarkdownSyntaxNode);
          if (image && !selectionTouches(view, from, to)) {
            occupy(from, to);
            add(
              from,
              to,
              Decoration.replace({
                widget: new MarkdownImageWidget(image.source, image.alt, from, to, options),
              }),
              "linked-image",
            );
            return false;
          }
        }

        if (name === "URL" && node.parent?.name === "Paragraph") {
          const source = view.state.doc.sliceString(from, to);
          if (isMarkdownImageSource(source) && !selectionTouches(view, from, to)) {
            occupy(from, to);
            add(
              from,
              to,
              Decoration.replace({
                widget: new MarkdownImageWidget(source, "", from, to, options),
              }),
              "bare-url-image",
            );
            return false;
          }
        }

        if (name === "InlineCode") {
          const raw = view.state.doc.sliceString(from, to);
          const reference = parseInlineMarkdownReference(raw, from);
          const parent = node.parent as MarkdownSyntaxNode | null;
          const standaloneView = reference?.kind !== "view"
            || (parent?.name === "Paragraph"
              && view.state.doc.sliceString(parent.from, parent.to).trim() === raw.trim());
          if (reference && standaloneView && !selectionTouches(view, from, to)) {
            occupy(from, to);
            add(
              from,
              to,
              Decoration.replace({
                widget: new MarkdownReferenceWidget(reference, from, to, options),
              }),
              `inline-reference:${reference.kind}`,
            );
            return false;
          }
        }

        if (/^ATXHeading[1-6]$/.test(name)) {
          const level = name.slice(-1);
          addLine(from, `cm-live-heading cm-live-heading-${level}`);
          return;
        }

        if (name === "StrongEmphasis") {
          add(from, to, Decoration.mark({ class: "cm-live-strong" }), "strong");
          return;
        }
        if (name === "Emphasis") {
          add(from, to, Decoration.mark({ class: "cm-live-emphasis" }), "emphasis");
          return;
        }
        if (name === "Strikethrough") {
          add(from, to, Decoration.mark({ class: "cm-live-strikethrough" }), "strike");
          return;
        }
        if (name === "InlineCode") {
          add(from, to, Decoration.mark({ class: "cm-live-inline-code" }), "inline-code");
          return;
        }
        if (name === "Link") {
          add(from, to, Decoration.mark({ class: "cm-live-link" }), "link");
          return;
        }

        if (name === "HeaderMark") {
          if (!syntaxTokenIsActive(view, node as unknown as MarkdownSyntaxNode)) {
            add(from, to, Decoration.replace({}), "hide-header-mark");
          }
          return;
        }

        if (name === "EmphasisMark" || name === "StrikethroughMark") {
          if (!syntaxTokenIsActive(view, node as unknown as MarkdownSyntaxNode)) {
            add(from, to, Decoration.replace({}), `hide-${name}`);
          }
          return;
        }

        if (
          name === "LinkMark"
          || ((name === "URL" || name === "LinkTitle") && node.parent?.name === "Link")
        ) {
          const syntaxNode = node as unknown as MarkdownSyntaxNode;
          if (!syntaxTokenIsActive(view, syntaxNode)) {
            add(linkTitleHiddenFrom(syntaxNode), to, Decoration.replace({}), `hide-link-${name}`);
          }
          return;
        }

        if (name === "CodeMark" || name === "CodeInfo") {
          if (!syntaxTokenIsActive(view, node as unknown as MarkdownSyntaxNode)) {
            add(from, to, Decoration.replace({}), `hide-code-${name}`);
          }
          return;
        }

        if (name === "ListMark") {
          if (!syntaxTokenIsActive(view, node as unknown as MarkdownSyntaxNode)) {
            const marker = view.state.doc.sliceString(from, to);
            add(
              from,
              to,
              Decoration.replace({ widget: new ListMarkerWidget(marker) }),
              "list-marker",
            );
          }
          return;
        }

        if (name === "TaskMarker") {
          if (!syntaxTokenIsActive(view, node as unknown as MarkdownSyntaxNode)) {
            const source = view.state.doc.sliceString(from, to);
            add(
              from,
              to,
              Decoration.replace({
                widget: new TaskCheckboxWidget(
                  from,
                  /^\[[xX]\]$/.test(source),
                  view.state.readOnly,
                ),
              }),
              "task-marker",
            );
          }
          return;
        }

        if (name === "Blockquote") {
          let line = view.state.doc.lineAt(from);
          while (line.from <= to) {
            addLine(line.from, "cm-live-blockquote");
            if (line.to >= to || line.number >= view.state.doc.lines) break;
            line = view.state.doc.line(line.number + 1);
          }
          return;
        }

        if (name === "QuoteMark") {
          if (!syntaxTokenIsActive(view, node as unknown as MarkdownSyntaxNode)) {
            add(from, to, Decoration.replace({}), "hide-quote-mark");
          }
          return;
        }

        if (name === "HorizontalRule") {
          if (!selectionTouches(view, from, to)) {
            add(
              from,
              to,
              Decoration.replace({ widget: new HorizontalRuleWidget() }),
              "horizontal-rule",
            );
          }
          return;
        }

        if (name === "FencedCode") {
          const fencedNode = node as unknown as MarkdownSyntaxNode;
          const infoNode = syntaxChild(fencedNode, "CodeInfo");
          const textNode = syntaxChild(fencedNode, "CodeText");
          const language = infoNode
            ? view.state.doc.sliceString(infoNode.from, infoNode.to).trim()
            : "";
          const specializedLanguage = isUnityReferenceFenceLanguage(language)
            || isUnityPropertyFenceLanguage(language);
          const firstFenceLine = view.state.doc.lineAt(from);
          const lastFenceLine = view.state.doc.lineAt(Math.max(from, to - 1));
          const fenceLineCount = lastFenceLine.number - firstFenceLine.number + 1;

          if (to - from > MAX_FENCE_SOURCE_CHARS || fenceLineCount > MAX_FENCE_LINES) {
            return false;
          }

          if (
            textNode
            && !selectionTouches(view, from, to)
            && specializedLanguage
          ) {
            const blockLines: Array<{ from: number; to: number; number: number }> = [];
            let blockLine = view.state.doc.lineAt(from);
            while (blockLine.from <= to) {
              blockLines.push({ from: blockLine.from, to: blockLine.to, number: blockLine.number });
              if (blockLine.to >= to || blockLine.number >= view.state.doc.lines) break;
              blockLine = view.state.doc.line(blockLine.number + 1);
            }

            if (isUnityReferenceFenceLanguage(language)) {
              const references = new Map<number, MarkdownReferenceToken>();
              let sourceLine = view.state.doc.lineAt(textNode.from);
              let valid = true;
              while (sourceLine.from <= textNode.to) {
                const source = view.state.doc.sliceString(
                  Math.max(sourceLine.from, textNode.from),
                  Math.min(sourceLine.to, textNode.to),
                );
                if (source.trim()) {
                  const reference = referenceForFenceLine(source, Math.max(sourceLine.from, textNode.from));
                  if (!reference || !reference.kind.startsWith("unity-")) {
                    valid = false;
                    break;
                  }
                  const mode = language.match(/^(?:asset|unity|ref)(?::|-)?(.+)?$/i)?.[1];
                  if (!reference.level && mode) reference.level = mode.toLowerCase();
                  references.set(sourceLine.number, reference);
                }
                if (sourceLine.to >= textNode.to || sourceLine.number >= view.state.doc.lines) break;
                sourceLine = view.state.doc.line(sourceLine.number + 1);
              }

              if (valid && references.size) {
                occupy(from, to);
                for (const currentLine of blockLines) {
                  const reference = references.get(currentLine.number);
                  if (!reference) {
                    collapseLine(currentLine.from, currentLine.to, `unity-fence:${from}:${currentLine.number}`);
                    continue;
                  }
                  addLine(currentLine.from, "cm-live-reference-line");
                  add(
                    currentLine.from,
                    currentLine.to,
                    Decoration.replace({
                      widget: new MarkdownReferenceWidget(reference, from, to, options),
                    }),
                    `unity-fence-ref:${currentLine.number}`,
                  );
                }
                return false;
              }
            } else {
              const propertySource = view.state.doc.sliceString(textNode.from, textNode.to).trim();
              const parsedProperty = propertySource ? parseUnityPropertyFence(propertySource) : null;
              if (parsedProperty?.entries.length && !parsedProperty.issues.length) {
                const firstSourceLine = view.state.doc.lineAt(textNode.from);
                const firstEntry = parsedProperty.entries[0];
                const firstLabel = `${firstEntry.objectLabel} · ${firstEntry.propertyLabel}`.slice(0, 96);
                const selectionTarget = unityPropertyFenceUnitySelectionTarget(firstEntry.target);
                const selectionPath = selectionTarget?.kind === "sceneObject"
                  ? `${selectionTarget.scenePath}/${selectionTarget.objectPath}`
                  : selectionTarget?.kind === "asset"
                    ? selectionTarget.path
                    : firstLabel;
                const reference: MarkdownReferenceToken = {
                  from: textNode.from,
                  to: textNode.to,
                  raw: propertySource,
                  path: selectionPath,
                  label: firstLabel,
                  kind: "unity-property",
                };
                occupy(from, to);
                let rendered = false;
                for (const currentLine of blockLines) {
                  if (!rendered && currentLine.number === firstSourceLine.number) {
                    addLine(currentLine.from, "cm-live-reference-line");
                    add(
                      currentLine.from,
                      currentLine.to,
                      Decoration.replace({
                        widget: new MarkdownReferenceWidget(reference, from, to, options),
                      }),
                      `unity-property:${from}`,
                    );
                    rendered = true;
                  } else {
                    collapseLine(currentLine.from, currentLine.to, `unity-property:${from}:${currentLine.number}`);
                  }
                }
                return false;
              }
            }
            // A recognized specialized fence is all-or-nothing. Keeping the
            // syntax subtree untouched makes malformed blocks editable.
            return false;
          }

          if (specializedLanguage && !selectionTouches(view, from, to)) return false;

          let line = view.state.doc.lineAt(from);
          let index = 0;
          while (line.from <= to) {
            const edgeClass = index === 0
              ? " cm-live-fenced-code-start"
              : line.to >= to ? " cm-live-fenced-code-end" : "";
            addLine(line.from, `cm-live-fenced-code${edgeClass}`);
            if (line.to >= to || line.number >= view.state.doc.lines) break;
            line = view.state.doc.line(line.number + 1);
            index += 1;
          }
        }
      },
    });
  }

  const scannedChunks = new Set<string>();
  for (const visibleRange of view.visibleRanges) {
    const scanFrom = Math.max(0, visibleRange.from - COMPLEX_SCAN_OVERSCAN_CHARS);
    const scanTo = Math.min(view.state.doc.length, visibleRange.to + COMPLEX_SCAN_OVERSCAN_CHARS);
    const chunkKey = `${scanFrom}:${scanTo}`;
    if (scannedChunks.has(chunkKey)) continue;
    scannedChunks.add(chunkKey);
    const source = view.state.doc.sliceString(scanFrom, scanTo);

    for (const math of findMarkdownMathTokens(source, scanFrom)) {
      if (math.to < visibleRange.from || math.from > visibleRange.to) continue;
      if (
        overlapsOccupied(math.from, math.to)
        || syntaxRangeIsProtected(tree, math.from, math.to)
        || selectionTouches(view, math.from, math.to)
      ) {
        continue;
      }

      const firstLine = view.state.doc.lineAt(math.from);
      const lastLine = view.state.doc.lineAt(Math.max(math.from, math.to - 1));
      if (firstLine.number !== lastLine.number && !math.block) continue;
      if (lastLine.number - firstLine.number + 1 > MAX_MATH_BLOCK_LINES) continue;
      occupy(math.from, math.to);

      const widget = new MarkdownMathWidget(
        math.latex,
        math.display,
        math.from,
        math.to,
        math.from + math.openingLength,
      );
      if (firstLine.number === lastLine.number) {
        add(
          math.from,
          math.to,
          Decoration.replace({ widget }),
          `math:${math.display ? "display" : "inline"}`,
        );
        continue;
      }

      addLine(firstLine.from, "cm-live-math-line");
      add(
        firstLine.from,
        firstLine.to,
        Decoration.replace({ widget }),
        `math-block:${math.from}`,
      );
      let collapsed = view.state.doc.line(firstLine.number + 1);
      while (collapsed.number <= lastLine.number) {
        collapseLine(collapsed.from, collapsed.to, `math-block:${math.from}:${collapsed.number}`);
        if (collapsed.number >= lastLine.number) break;
        collapsed = view.state.doc.line(collapsed.number + 1);
      }
    }

    for (const reference of findPlainMarkdownReferences(source, scanFrom)) {
      if (reference.to < visibleRange.from || reference.from > visibleRange.to) continue;
      if (
        overlapsOccupied(reference.from, reference.to)
        || syntaxRangeIsProtected(tree, reference.from, reference.to)
        || selectionTouches(view, reference.from, reference.to)
      ) {
        continue;
      }
      occupy(reference.from, reference.to);
      add(
        reference.from,
        reference.to,
        Decoration.replace({
          widget: new MarkdownReferenceWidget(
            reference,
            reference.from,
            reference.to,
            options,
          ),
        }),
        `reference:${reference.kind}`,
      );
    }
  }

  return Decoration.set(decorations, true);
}

function createMarkdownLivePreviewPlugin(options: MarkdownLivePreviewOptions) {
  return ViewPlugin.fromClass(class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = buildLivePreviewDecorations(view, options);
    }

    update(update: ViewUpdate): void {
      if (
        update.docChanged
        || update.selectionSet
        || update.focusChanged
        || update.viewportChanged
        || update.geometryChanged
        || update.startState.readOnly !== update.state.readOnly
        || syntaxTree(update.startState) !== syntaxTree(update.state)
      ) {
        this.decorations = buildLivePreviewDecorations(update.view, options);
      }
    }
  }, {
    decorations: (plugin) => plugin.decorations,
  });
}

const markdownComplexWidgetTheme = EditorView.theme({
  ".cm-live-collapsed-line": {
    height: "0",
    minHeight: "0",
    lineHeight: "0",
    overflow: "hidden",
  },
  ".cm-live-collapsed-source": {
    display: "none",
  },
  ".cm-live-table-line": {
    padding: "0",
  },
  ".cm-live-table-row": {
    display: "inline-grid",
    boxSizing: "border-box",
    width: "100%",
    minWidth: "100%",
    borderRight: "1px solid color-mix(in srgb, var(--border-color) 86%, transparent)",
    borderBottom: "1px solid color-mix(in srgb, var(--border-color) 86%, transparent)",
    borderLeft: "1px solid color-mix(in srgb, var(--border-color) 86%, transparent)",
    background: "color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%)",
    cursor: "text",
  },
  ".cm-live-table-header": {
    borderTop: "1px solid color-mix(in srgb, var(--border-color) 86%, transparent)",
    borderRadius: "8px 8px 0 0",
    background: "color-mix(in srgb, var(--sidebar-bg) 68%, var(--panel-bg) 32%)",
    color: "var(--text-secondary)",
    fontWeight: "600",
  },
  ".cm-live-table-last-row": {
    borderRadius: "0 0 8px 8px",
  },
  ".cm-live-table-cell": {
    boxSizing: "border-box",
    minWidth: "120px",
    padding: "7px 10px",
    borderRight: "1px solid color-mix(in srgb, var(--border-color) 86%, transparent)",
    whiteSpace: "normal",
    overflowWrap: "anywhere",
  },
  ".cm-live-table-cell:last-child": {
    borderRight: "none",
  },
  ".cm-live-table-cell[data-align='center']": {
    textAlign: "center",
  },
  ".cm-live-table-cell[data-align='right']": {
    textAlign: "right",
  },
  ".cm-live-image-frame": {
    display: "inline-flex",
    boxSizing: "border-box",
    maxWidth: "100%",
    minHeight: "36px",
    alignItems: "center",
    justifyContent: "center",
    margin: "4px 0",
    border: "1px solid color-mix(in srgb, var(--border-color) 82%, transparent)",
    borderRadius: "8px",
    background: "color-mix(in srgb, var(--sidebar-bg) 38%, transparent)",
    overflow: "hidden",
    cursor: "text",
  },
  ".cm-live-image": {
    display: "none",
    maxWidth: "min(100%, 720px)",
    maxHeight: "480px",
    objectFit: "contain",
  },
  ".cm-live-image-frame[data-state='ready'] .cm-live-image": {
    display: "block",
  },
  ".cm-live-image-frame[data-state='ready'] .cm-live-image-fallback": {
    display: "none",
  },
  ".cm-live-image-fallback": {
    padding: "8px 12px",
    color: "var(--text-secondary)",
    fontFamily: "var(--font-mono-inline)",
    fontSize: "12px",
  },
  ".cm-live-image-frame[data-state='error'] .cm-live-image-fallback": {
    color: "var(--status-error-fg, var(--text-secondary))",
  },
  ".cm-live-math": {
    display: "inline-flex",
    alignItems: "center",
    minHeight: "1.45em",
    padding: "0 2px",
    color: "var(--text-color)",
    cursor: "text",
  },
  ".cm-live-math-display": {
    boxSizing: "border-box",
    width: "100%",
    justifyContent: "center",
    padding: "8px 12px",
    overflowX: "auto",
  },
  ".cm-live-math[data-state='loading']": {
    color: "var(--text-secondary)",
    fontFamily: "var(--font-mono-inline)",
  },
  ".cm-live-reference-line": {
    minHeight: "1.75em",
  },
  ".cm-live-reference": {
    display: "inline-flex",
    maxWidth: "100%",
    alignItems: "center",
    gap: "6px",
    padding: "1px 6px",
    border: "1px solid color-mix(in srgb, var(--border-color) 76%, transparent)",
    borderRadius: "4px",
    background: "color-mix(in srgb, var(--sidebar-bg) 46%, transparent)",
    color: "var(--text-color)",
    fontFamily: "var(--font-mono-inline)",
    fontSize: "0.92em",
    cursor: "text",
    verticalAlign: "baseline",
  },
  ".cm-live-reference-kind": {
    color: "var(--text-secondary)",
    fontFamily: "var(--font-ui, inherit)",
    fontSize: "10px",
    fontWeight: "600",
    letterSpacing: "0.02em",
    textTransform: "uppercase",
  },
  ".cm-live-reference-label": {
    minWidth: "0",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  ".cm-live-reference--knowledge": {
    borderColor: "color-mix(in srgb, var(--accent-color) 32%, var(--border-color))",
  },
  ".cm-live-reference--unity-asset, .cm-live-reference--unity-scene-object, .cm-live-reference--unity-property": {
    borderColor: "color-mix(in srgb, var(--status-warn-fg, var(--text-secondary)) 28%, var(--border-color))",
  },
});

export function markdownLivePreview(options: MarkdownLivePreviewOptions = {}): Extension {
  return [
    EditorView.editorAttributes.of({ class: "cm-live-preview" }),
    markdownComplexWidgetTheme,
    createMarkdownLivePreviewPlugin(options),
  ];
}

export { buildLivePreviewDecorations };
export type { MarkdownLivePreviewOptions } from "./markdownComplexWidgets";
