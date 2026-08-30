export class KnowledgeEditorSessionCache<T> {
  private readonly entries = new Map<string, T>();

  constructor(
    private readonly capacity = 24,
    private readonly canEvict: (value: T) => boolean = () => true,
  ) {}

  get(key: string): T | undefined {
    const value = this.entries.get(key);
    if (value === undefined) return undefined;
    this.entries.delete(key);
    this.entries.set(key, value);
    return value;
  }

  set(key: string, value: T): void {
    this.entries.delete(key);
    this.entries.set(key, value);
    while (this.entries.size > this.capacity) {
      const oldestEvictable = Array.from(this.entries)
        .find(([candidateKey, candidate]) =>
          candidateKey !== key && this.canEvict(candidate)
        );
      if (!oldestEvictable) break;
      this.entries.delete(oldestEvictable[0]);
    }
  }

  clear(): void {
    this.entries.clear();
  }

  get size(): number {
    return this.entries.size;
  }
}
