import type { EditorState } from "@codemirror/state";

export const DEFAULT_MARKDOWN_EDITOR_SESSION_LIMIT = 12;

export interface MarkdownEditorSessionSnapshot {
  state: EditorState;
  scrollTop: number;
  scrollLeft: number;
  /** Last external model observed for this state. */
  modelValue?: string;
  /** Dirty/conflicted states stay resident even when the soft limit is full. */
  pinned?: boolean;
}

/** Minimal ownership-neutral contract consumed by BaseMarkdownEditor. */
export interface MarkdownEditorSessionStore {
  get(key: string): MarkdownEditorSessionSnapshot | null;
  set(key: string, snapshot: MarkdownEditorSessionSnapshot): void;
  setPinned(key: string, pinned: boolean): void;
}

/**
 * Per-pane bounded cache. EditorState already contains the selection and undo
 * history; scroll lives on EditorView and is stored alongside it.
 */
export class MarkdownEditorSessionCache implements MarkdownEditorSessionStore {
  private readonly entries = new Map<string, MarkdownEditorSessionSnapshot>();

  constructor(
    private readonly limit = DEFAULT_MARKDOWN_EDITOR_SESSION_LIMIT,
  ) {}

  get size(): number {
    return this.entries.size;
  }

  get(key: string): MarkdownEditorSessionSnapshot | null {
    const entry = this.entries.get(key);
    if (!entry) return null;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry;
  }

  set(key: string, snapshot: MarkdownEditorSessionSnapshot): void {
    this.entries.delete(key);
    this.entries.set(key, snapshot);
    this.evictToLimit();
  }

  setPinned(key: string, pinned: boolean): void {
    const entry = this.entries.get(key);
    if (!entry || !!entry.pinned === pinned) return;
    this.entries.set(key, { ...entry, pinned });
    this.evictToLimit();
  }

  has(key: string): boolean {
    return this.entries.has(key);
  }

  delete(key: string): void {
    this.entries.delete(key);
  }

  clear(): void {
    this.entries.clear();
  }

  keys(): string[] {
    return [...this.entries.keys()];
  }

  private evictToLimit(): void {
    const limit = Math.max(1, this.limit);
    while (this.entries.size > limit) {
      let evictableKey: string | undefined;
      for (const [key, entry] of this.entries) {
        if (!entry.pinned) {
          evictableKey = key;
          break;
        }
      }
      // The capacity is a soft bound. Preserving undo for every dirty or
      // conflicted document takes precedence until one becomes clean.
      if (evictableKey === undefined) break;
      this.entries.delete(evictableKey);
    }
  }
}
