/**
 * Incremental splitter that turns a growing markdown stream into frozen
 * prefix blocks plus an active tail, so streaming re-renders cost O(tail)
 * per frame instead of O(document).
 *
 * Markdown blocks separated by blank lines render independently, with a few
 * cross-block couplings the boundary rules below refuse to cut through:
 * fenced code (blank lines inside a fence), list continuations (a split
 * would retighten a loose list and restart ordered numbering), blockquote
 * runs (normalizeLooseBlockquotes joins them across blank lines), and
 * indented-code runs (a blank line belongs to the code block). Boundaries
 * additionally trail one completed block behind the cut so structures whose
 * meaning depends on the following line never freeze prematurely.
 *
 * Known streaming-phase divergence (corrected by the one-shot full render
 * once the round lands in history): link reference definitions that appear
 * in a later block than their usage render as literal text while streaming.
 */

export interface StreamingMarkdownBlock {
  /** Stable identity for keyed v-for; never reused across resets. */
  id: string;
  text: string;
}

export interface StreamingMarkdownSplit {
  /** Frozen prefix blocks. Append-only between resets. */
  blocks: StreamingMarkdownBlock[];
  /** Unfrozen suffix: the trailing completed block plus the growing block. */
  tail: string;
}

const LIST_MARKER_RE = /^ {0,3}(?:[-*+]|\d{1,9}[.)])(?:\s|$)/;
const BLOCKQUOTE_RE = /^ {0,3}>/;
const FENCE_OPEN_RE = /^( {0,3})(`{3,}|~{3,})(.*)$/;
const INDENT_RE = /^[ \t]*/;

interface LineInfo {
  blank: boolean;
  listMarker: boolean;
  blockquote: boolean;
  indent: number;
}

function classifyLine(line: string): LineInfo {
  const trimmed = line.trim();
  const indent = INDENT_RE.exec(line)![0].replace(/\t/g, "    ").length;
  return {
    blank: trimmed.length === 0,
    listMarker: LIST_MARKER_RE.test(line),
    blockquote: BLOCKQUOTE_RE.test(line),
    indent,
  };
}

export class StreamingMarkdownSplitter {
  private source = "";
  /** Offset of the first byte not yet consumed as a complete line. */
  private scanned = 0;
  /** Offset up to which blocks are frozen. */
  private frozenEnd = 0;
  /** Cut points (block-start offsets) after frozenEnd, oldest first. */
  private cuts: number[] = [];
  private blocks: StreamingMarkdownBlock[] = [];
  /** True once append() has run: `source` then holds only the unfrozen
   * suffix, so update()'s full-text prefix checks no longer apply. */
  private appendMode = false;

  private inFence = false;
  private fenceChar = "";
  private fenceLen = 0;
  /** A blank line was seen since the last non-blank line (outside fences). */
  private sawBlank = false;
  /** Classification of the previous non-blank line, for continuation rules. */
  private prevListy = false;
  private prevBlockquote = false;
  private prevIndentedCode = false;
  private hasNonBlank = false;

  private generation = 0;
  private seq = 0;

  /**
   * Feed the current full text. Append-only growth is scanned incrementally;
   * anything else (new round, replacement, shrink) resets the splitter.
   */
  update(next: string): StreamingMarkdownSplit {
    if (this.appendMode) {
      // append() rebases `source` to the unfrozen suffix; full-text prefix
      // comparisons against it would be meaningless (and a suffix that
      // happens to also be a prefix would corrupt the scan state).
      this.reset();
    }
    if (next === this.source) {
      return this.snapshot();
    }
    if (next.length < this.source.length || !next.startsWith(this.source)) {
      this.reset();
    }
    this.source = next;
    this.scan();
    return this.snapshot();
  }

  /**
   * Feed a delta known to extend the current text. Skips the prefix check of
   * `update`, and rebasing after each scan keeps `source` at O(tail), so
   * growth costs O(delta + tail) regardless of document size.
   */
  append(delta: string): StreamingMarkdownSplit {
    if (!delta) {
      return this.snapshot();
    }
    this.appendMode = true;
    this.source += delta;
    this.scan();
    this.rebase();
    return this.snapshot();
  }

  /**
   * Drop the frozen prefix from `source` and shift all offsets. Keeps every
   * subsequent flatten (scan's indexOf) and snapshot slice proportional to
   * the tail instead of the whole document, and lets newly frozen blocks pin
   * only tail-sized parent strings.
   */
  private rebase(): void {
    if (this.frozenEnd === 0) return;
    this.source = this.source.slice(this.frozenEnd);
    this.scanned -= this.frozenEnd;
    for (let i = 0; i < this.cuts.length; i += 1) {
      this.cuts[i]! -= this.frozenEnd;
    }
    this.frozenEnd = 0;
  }

  reset(): void {
    this.source = "";
    this.scanned = 0;
    this.frozenEnd = 0;
    this.cuts = [];
    this.blocks = [];
    this.appendMode = false;
    this.inFence = false;
    this.fenceChar = "";
    this.fenceLen = 0;
    this.sawBlank = false;
    this.prevListy = false;
    this.prevBlockquote = false;
    this.prevIndentedCode = false;
    this.hasNonBlank = false;
    this.generation += 1;
    this.seq = 0;
  }

  private snapshot(): StreamingMarkdownSplit {
    return {
      blocks: this.blocks,
      tail: this.source.slice(this.frozenEnd),
    };
  }

  private scan(): void {
    // Only complete lines participate; the trailing partial line waits.
    let lineStart = this.scanned;
    while (true) {
      const newline = this.source.indexOf("\n", lineStart);
      if (newline < 0) break;
      this.consumeLine(this.source.slice(lineStart, newline), lineStart);
      lineStart = newline + 1;
    }
    this.scanned = lineStart;
  }

  private consumeLine(rawLine: string, lineStart: number): void {
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;

    if (this.inFence) {
      if (this.isFenceClose(line)) {
        this.inFence = false;
      }
      // Fence interiors never produce boundaries or continuation context.
      return;
    }

    const info = classifyLine(line);
    if (info.blank) {
      if (this.hasNonBlank) {
        this.sawBlank = true;
      }
      return;
    }

    if (this.sawBlank && !this.isContinuation(info)) {
      this.cutAt(lineStart);
    }
    this.sawBlank = false;
    this.hasNonBlank = true;

    const fence = FENCE_OPEN_RE.exec(line);
    if (fence) {
      this.inFence = true;
      this.fenceChar = fence[2][0];
      this.fenceLen = fence[2].length;
      // A fence opener starts a code block: not a list/quote continuation
      // context for whatever follows the fence.
      this.prevListy = false;
      this.prevBlockquote = false;
      this.prevIndentedCode = false;
      return;
    }

    const indentedCode = !this.prevListy && info.indent >= 4;
    this.prevBlockquote = info.blockquote;
    this.prevIndentedCode = indentedCode;
    this.prevListy = info.listMarker
      || (this.prevListy && info.indent >= 2)
      || (this.prevListy && info.blockquote);
  }

  /**
   * True when the first non-blank line after a blank must stay in the same
   * block as what precedes it, because splitting would change rendering.
   */
  private isContinuation(info: LineInfo): boolean {
    // Loose-list items / lazy indented content of a list.
    if (this.prevListy && (info.listMarker || info.indent >= 2)) return true;
    // Blockquote runs are joined across blank lines by
    // normalizeLooseBlockquotes, so both sides must render together.
    if (this.prevBlockquote && info.blockquote) return true;
    // A blank line inside an indented code block belongs to the code.
    if (this.prevIndentedCode && info.indent >= 4) return true;
    return false;
  }

  private isFenceClose(line: string): boolean {
    const match = /^( {0,3})(`{3,}|~{3,})\s*$/.exec(line);
    if (!match) return false;
    return match[2][0] === this.fenceChar && match[2].length >= this.fenceLen;
  }

  private cutAt(offset: number): void {
    if (offset <= this.frozenEnd) return;
    if (this.cuts.length > 0 && offset <= this.cuts[this.cuts.length - 1]!) return;
    this.cuts.push(offset);
    // Trail one completed block: freeze only while two cuts are pending, so
    // the newest completed block stays in the tail until its successor is
    // itself complete.
    while (this.cuts.length >= 2) {
      const end = this.cuts.shift()!;
      this.blocks = [
        ...this.blocks,
        {
          id: `g${this.generation}-b${this.seq}`,
          text: this.source.slice(this.frozenEnd, end),
        },
      ];
      this.seq += 1;
      this.frozenEnd = end;
    }
  }
}
