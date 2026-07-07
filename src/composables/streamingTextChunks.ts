import { markRaw, shallowRef, type ShallowRef } from "vue";

/**
 * Append-only text buffer for streaming deltas with a stable identity.
 *
 * High-frequency consumers (transcript renderer, thinking panel) subscribe to
 * `version` and read the coalesced parts incrementally, so a delta costs
 * O(delta) instead of re-materializing the whole accumulated string. Whole-text
 * reads (`full()`) are reserved for per-round events (finalize, snapshots) and
 * cache their join until the next append.
 *
 * Passing the buffer itself as a prop never re-renders the component tree:
 * the object identity is stable and growth is only observable through
 * `version`, which consumers watch explicitly.
 */

/** Frozen-part target size: large enough to keep DOM/vnode counts tiny, small
 * enough that re-rendering the active tail part stays cheap. */
const CHUNK_TARGET = 4096;

export interface StreamingTextSource {
  /** Bumped on every append and reset; watch this to consume growth. */
  readonly version: ShallowRef<number>;
  /** Bumped on reset; consumers must discard cursors from older generations. */
  readonly generation: number;
  readonly length: number;
  /** True once any non-whitespace character has been appended. */
  readonly hasNonWhitespace: boolean;
  /** Completed ~4KB parts. Append-only between resets; indexes are stable. */
  readonly frozenParts: readonly string[];
  /** The growing tail part (always shorter than CHUNK_TARGET). */
  readonly activePart: string;
  full(): string;
  /** Text from logical offset to the end. O(result length). */
  readFrom(offset: number): string;
}

export class StreamingTextChunks implements StreamingTextSource {
  constructor() {
    // Growth is observed through `version` alone; a reactive proxy (e.g. from
    // a pinia store or reactive state object) would deep-unwrap that ref and
    // add per-read proxy overhead on the hot append path.
    markRaw(this);
  }

  readonly version = shallowRef(0);
  private _generation = 0;
  private _length = 0;
  private _hasNonWhitespace = false;
  private _frozen: string[] = [];
  /** Logical start offset of each frozen part, parallel to `_frozen`. */
  private _frozenOffsets: number[] = [];
  private _active = "";
  private _flat: string | null = "";

  get generation(): number {
    return this._generation;
  }

  get length(): number {
    return this._length;
  }

  get hasNonWhitespace(): boolean {
    return this._hasNonWhitespace;
  }

  get frozenParts(): readonly string[] {
    return this._frozen;
  }

  get activePart(): string {
    return this._active;
  }

  append(text: string): void {
    if (!text) return;
    if (!this._hasNonWhitespace && /\S/.test(text)) {
      this._hasNonWhitespace = true;
    }
    this._active += text;
    this._length += text.length;
    if (this._active.length >= CHUNK_TARGET) {
      this._frozenOffsets.push(this._length - this._active.length);
      this._frozen.push(this._active);
      this._active = "";
    }
    this._flat = null;
    this.version.value += 1;
  }

  reset(): void {
    // Nothing appended since the last reset (or ever): consumers have no
    // state to invalidate, so skip the generation/version bump entirely.
    if (this._length === 0) return;
    this._generation += 1;
    this._length = 0;
    this._hasNonWhitespace = false;
    this._frozen = [];
    this._frozenOffsets = [];
    this._active = "";
    this._flat = "";
    this.version.value += 1;
  }

  full(): string {
    if (this._flat === null) {
      this._flat = this._frozen.length === 0
        ? this._active
        : this._frozen.join("") + this._active;
    }
    return this._flat;
  }

  readFrom(offset: number): string {
    if (offset <= 0) return this.full();
    if (offset >= this._length) return "";
    const activeStart = this._length - this._active.length;
    if (offset >= activeStart) {
      return this._active.slice(offset - activeStart);
    }
    let index = this._frozen.length - 1;
    while (index > 0 && this._frozenOffsets[index]! > offset) {
      index -= 1;
    }
    const parts: string[] = [
      this._frozen[index]!.slice(offset - this._frozenOffsets[index]!),
    ];
    for (let i = index + 1; i < this._frozen.length; i += 1) {
      parts.push(this._frozen[i]!);
    }
    if (this._active) parts.push(this._active);
    return parts.join("");
  }

  /** Text in [start, end). O(end - start) regardless of buffer size. */
  readRange(start: number, end: number): string {
    const from = Math.max(0, start);
    const to = Math.min(this._length, end);
    if (from >= to) return "";
    if (from === 0 && to === this._length) return this.full();
    const activeStart = this._length - this._active.length;
    if (from >= activeStart) {
      return this._active.slice(from - activeStart, to - activeStart);
    }
    let index = this._frozen.length - 1;
    while (index > 0 && this._frozenOffsets[index]! > from) {
      index -= 1;
    }
    const parts: string[] = [];
    let cursor = from;
    for (let i = index; i < this._frozen.length && cursor < to; i += 1) {
      const partStart = this._frozenOffsets[i]!;
      const part = this._frozen[i]!;
      const sliceStart = cursor - partStart;
      const sliceEnd = Math.min(part.length, to - partStart);
      if (sliceStart < sliceEnd) {
        parts.push(part.slice(sliceStart, sliceEnd));
        cursor = partStart + sliceEnd;
      }
    }
    if (cursor < to) {
      parts.push(this._active.slice(cursor - activeStart, to - activeStart));
    }
    return parts.length === 1 ? parts[0]! : parts.join("");
  }
}
