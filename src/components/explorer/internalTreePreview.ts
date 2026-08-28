export interface InternalTreeEntryMeta {
  nodeKey?: string;
  depth: number;
}

export interface InternalTreePreviewIntent {
  sourceKey: string;
  targetKey: string | null;
  position: "before" | "after" | "inside" | "root";
  rootDepth?: number;
}

export function withInternalTreePreview<T>(
  entries: readonly T[],
  intent: InternalTreePreviewIntent,
  describe: (entry: T) => InternalTreeEntryMeta,
  createPreview: (depth: number) => T,
): T[] {
  const sourceIndex = entries.findIndex((entry) => describe(entry).nodeKey === intent.sourceKey);
  if (sourceIndex < 0) return [...entries];
  const sourceDepth = describe(entries[sourceIndex]!).depth;
  let sourceEnd = sourceIndex + 1;
  while (sourceEnd < entries.length && describe(entries[sourceEnd]!).depth > sourceDepth) {
    sourceEnd += 1;
  }
  const remaining = [
    ...entries.slice(0, sourceIndex),
    ...entries.slice(sourceEnd),
  ];

  if (intent.position === "root" || !intent.targetKey) {
    remaining.push(createPreview(intent.rootDepth ?? 0));
    return remaining;
  }

  const targetIndex = remaining.findIndex((entry) => describe(entry).nodeKey === intent.targetKey);
  if (targetIndex < 0) return [...entries];
  const targetDepth = describe(remaining[targetIndex]!).depth;
  let insertIndex = targetIndex;
  let previewDepth = targetDepth;
  if (intent.position === "after" || intent.position === "inside") {
    insertIndex = targetIndex + 1;
    while (insertIndex < remaining.length && describe(remaining[insertIndex]!).depth > targetDepth) {
      insertIndex += 1;
    }
    if (intent.position === "inside") previewDepth = targetDepth + 1;
  }
  remaining.splice(insertIndex, 0, createPreview(previewDepth));
  return remaining;
}
