export interface DocumentSessionCacheOptions<T> {
  capacity?: number;
  /** Dirty/conflicted sessions may exceed the soft capacity. */
  canEvict?: (value: T) => boolean;
  /** Knowledge drafts keep the just-stored session even if all older ones are pinned. */
  protectLatest?: boolean;
}

/** Renderer-independent LRU shared by document drafts and editor view snapshots. */
export class DocumentSessionCache<T> {
  private readonly entries = new Map<string, T>();
  private readonly capacity: number;

  constructor(private readonly options: DocumentSessionCacheOptions<T> = {}) {
    this.capacity = Math.max(1, Math.floor(options.capacity ?? 24));
  }

  get size(): number { return this.entries.size; }

  peek(key: string): T | undefined { return this.entries.get(key); }

  get(key: string): T | undefined {
    if (!this.entries.has(key)) return undefined;
    const value = this.entries.get(key)!;
    this.entries.delete(key);
    this.entries.set(key, value);
    return value;
  }

  set(key: string, value: T): void {
    this.entries.delete(key);
    this.entries.set(key, value);
    this.evict(this.options.protectLatest ? key : undefined);
  }

  /** Update pinning without making an inactive editor the most recently used. */
  replace(key: string, value: T): void {
    if (!this.entries.has(key)) return;
    this.entries.set(key, value);
    this.evict();
  }

  has(key: string): boolean { return this.entries.has(key); }
  delete(key: string): void { this.entries.delete(key); }
  clear(): void { this.entries.clear(); }
  keys(): string[] { return [...this.entries.keys()]; }

  private evict(protectedKey?: string): void {
    while (this.entries.size > this.capacity) {
      let candidate: string | undefined;
      for (const [key, value] of this.entries) {
        if (key !== protectedKey && (this.options.canEvict?.(value) ?? true)) {
          candidate = key;
          break;
        }
      }
      if (candidate === undefined) break;
      this.entries.delete(candidate);
    }
  }
}
