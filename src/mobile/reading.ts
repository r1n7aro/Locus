import { readonly, ref } from "vue";
import type { DocumentSource } from "./routes";

// Reading state of the document viewer, kept on this phone: one text size
// for every document, and where each document was left off.

const FONT_SCALE_KEY = "locus-remote.documentFontScale";
const POSITIONS_KEY = "locus-remote.readingPositions";

export const FONT_SCALE_DEFAULT = 1;
export const FONT_SCALE_MIN = 0.8;
export const FONT_SCALE_MAX = 2;
/** A pinch snaps to this grid, so the text reflows a bounded number of times
 * per gesture. */
const PINCH_STEP = 0.05;
/** Positions of the least recently read documents are dropped beyond this. */
export const POSITION_LIMIT = 200;

function storageGet(key: string): string | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** `null` removes the key. Without storage the state lasts for this page. */
function storageSet(key: string, value: string | null): void {
  try {
    if (typeof localStorage === "undefined") return;
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Storage full or unavailable.
  }
}

function round(value: number, digits = 4): number {
  const factor = 10 ** digits;
  return Math.round(value * factor) / factor;
}

// ----- text size

export function normalizeFontScale(value: unknown, step = 0.01): number {
  const parsed = typeof value === "number" ? value : typeof value === "string" && value.trim() ? Number(value) : Number.NaN;
  if (!Number.isFinite(parsed)) return FONT_SCALE_DEFAULT;
  const clamped = Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, parsed));
  return round(Math.round(clamped / step) * step, 2);
}

/** Next tenth up or down; a scale between tenths (after a pinch) goes to the
 * neighbouring tenth. */
export function stepFontScale(current: number, direction: 1 | -1): number {
  const tenths = current * 10;
  const base = direction > 0 ? Math.floor(tenths + 1e-6) : Math.ceil(tenths - 1e-6);
  return normalizeFontScale((base + direction) / 10);
}

/** Scale while pinching: the start scale times the change of finger distance. */
export function pinchFontScale(start: number, ratio: number): number {
  return normalizeFontScale(start * ratio, PINCH_STEP);
}

const fontScale = ref(normalizeFontScale(storageGet(FONT_SCALE_KEY)));

/** Text size of the document viewer, as a factor of the default. */
export const documentFontScale = readonly(fontScale);

/** `persist: false` while a pinch is still changing it. */
export function setDocumentFontScale(value: number, options: { persist?: boolean } = {}): number {
  const next = normalizeFontScale(value);
  fontScale.value = next;
  if (options.persist !== false) storageSet(FONT_SCALE_KEY, next === FONT_SCALE_DEFAULT ? null : String(next));
  return next;
}

// ----- reading positions

/** Where a document was left off, independent of the text size and screen
 * width: the block at the top of the viewport and how far into it. */
export interface ReadingPosition {
  /** Index of the block among the document's top-level blocks. */
  block: number;
  /** Fraction of that block above the viewport top, 0..1. */
  offset: number;
  /** Number of blocks when saved; the block is trusted only while it matches. */
  blocks: number;
  /** Fraction of the scrollable height, used once the document has changed. */
  ratio: number;
}

interface StoredPosition extends ReadingPosition {
  /** Saved at (ms), for dropping the oldest entries. */
  at: number;
}

function normalizeKeyPath(path: string): string {
  return path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "").toLowerCase();
}

/** Storage key of a document. `workspacePath`: the file's workspace-relative
 * path when known, so a file opened by absolute path shares its entry. */
export function readingKey(source: DocumentSource, workspacePath: string | null): string {
  const checkout = source.workspaceRef.checkoutId;
  if (source.kind === "knowledge") {
    const path = normalizeKeyPath(source.path);
    const prefix = `${source.docType}/`;
    return `${checkout}|knowledge|${source.docType}|${path.startsWith(prefix) ? path.slice(prefix.length) : path}`;
  }
  return `${checkout}|file|${normalizeKeyPath(workspacePath ?? source.path)}`;
}

let positionsCache: Record<string, StoredPosition> | null = null;

function positions(): Record<string, StoredPosition> {
  if (positionsCache) return positionsCache;
  let parsed: unknown = null;
  try {
    parsed = JSON.parse(storageGet(POSITIONS_KEY) ?? "null");
  } catch {
    parsed = null;
  }
  positionsCache = parsed && typeof parsed === "object" && !Array.isArray(parsed)
    ? parsed as Record<string, StoredPosition>
    : {};
  return positionsCache;
}

export function loadReadingPosition(key: string): ReadingPosition | null {
  const entry = positions()[key];
  if (!entry || typeof entry !== "object") return null;
  const { block, offset, blocks, ratio } = entry;
  if (![block, offset, blocks, ratio].every((value) => typeof value === "number" && Number.isFinite(value))) return null;
  return { block, offset, blocks, ratio };
}

/** `null` forgets the document (read from the top). */
export function saveReadingPosition(key: string, position: ReadingPosition | null): void {
  const all = positions();
  if (position) {
    all[key] = { ...position, at: Date.now() };
  } else if (key in all) {
    delete all[key];
  } else {
    return;
  }
  const keys = Object.keys(all);
  if (keys.length > POSITION_LIMIT) {
    keys
      .sort((left, right) => (all[right]?.at ?? 0) - (all[left]?.at ?? 0))
      .slice(POSITION_LIMIT)
      .forEach((stale) => delete all[stale]);
  }
  storageSet(POSITIONS_KEY, Object.keys(all).length ? JSON.stringify(all) : null);
}

/** Drops the in-memory copy (tests). */
export function resetReadingCache(): void {
  positionsCache = null;
  fontScale.value = normalizeFontScale(storageGet(FONT_SCALE_KEY));
}

/** Position of the line at `y` (client coordinates) among `blocks`, which are
 * in document order. */
export function capturePosition(blocks: readonly Element[], y: number, ratio: number): ReadingPosition | null {
  if (!blocks.length) return null;
  // First block that ends below the line.
  let low = 0;
  let high = blocks.length - 1;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (blocks[middle]!.getBoundingClientRect().bottom > y) high = middle;
    else low = middle + 1;
  }
  const rect = blocks[low]!.getBoundingClientRect();
  const offset = rect.height > 0 ? Math.min(1, Math.max(0, (y - rect.top) / rect.height)) : 0;
  return { block: low, offset: round(offset), blocks: blocks.length, ratio: round(Math.min(1, Math.max(0, ratio))) };
}

/** How far to scroll down so the saved line is at `y` again; `null` when the
 * blocks no longer match the saved ones. */
export function positionDelta(blocks: readonly Element[], position: ReadingPosition, y: number): number | null {
  if (position.blocks !== blocks.length) return null;
  const block = blocks[position.block];
  if (!block) return null;
  const rect = block.getBoundingClientRect();
  return rect.top + position.offset * rect.height - y;
}
