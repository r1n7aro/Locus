import type { EditorState } from "@codemirror/state";
import { DocumentSessionCache } from "../../../document/documentSessionCache";

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
  private readonly entries: DocumentSessionCache<MarkdownEditorSessionSnapshot>;

  constructor(
    limit = DEFAULT_MARKDOWN_EDITOR_SESSION_LIMIT,
  ) {
    this.entries = new DocumentSessionCache({ capacity: limit, canEvict: (entry) => !entry.pinned });
  }

  get size(): number {
    return this.entries.size;
  }

  get(key: string): MarkdownEditorSessionSnapshot | null {
    return this.entries.get(key) ?? null;
  }

  set(key: string, snapshot: MarkdownEditorSessionSnapshot): void {
    this.entries.set(key, snapshot);
  }

  setPinned(key: string, pinned: boolean): void {
    const entry = this.entries.peek(key);
    if (!entry || !!entry.pinned === pinned) return;
    this.entries.replace(key, { ...entry, pinned });
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
    return this.entries.keys();
  }
}
