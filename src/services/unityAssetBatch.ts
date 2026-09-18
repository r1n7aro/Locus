import type { AssetBatchEntry, AssetBatchResult, AssetOperation, AssetApplyOptions } from "./unityAssets";

/** A caller-owned batch. No timer, filesystem write or Editor call until flush. */
export interface UnityAssetBatch {
  readonly operationCount: number;
  readonly assetCount: number;
  readonly state: "collecting" | "flushing" | "failed";
  enqueue(path: string, operations: AssetOperation[], options: AssetApplyOptions): void;
  flush(): Promise<AssetBatchResult | null>;
  clear(): void;
}

export function createUnityAssetBatch(
  encode: (entry: AssetBatchEntry) => AssetBatchEntry,
  apply: (entries: AssetBatchEntry[]) => Promise<AssetBatchResult>,
  assertActive: () => void,
): UnityAssetBatch {
  const entries = new Map<string, AssetBatchEntry>();
  let count = 0;
  let state: UnityAssetBatch["state"] = "collecting";
  let pending: Promise<AssetBatchResult | null> | null = null;
  function assertCollecting() {
    assertActive();
    if (state !== "collecting") throw new Error(`Asset batch is ${state}; ${state === "failed" ? "inspect the transaction outcome, then clear and rebuild from fresh snapshots" : "wait for flush"}.`);
  }
  return {
    get operationCount() { return count; },
    get assetCount() { return entries.size; },
    get state() { return state; },
    enqueue(path, operations, options) {
      assertCollecting();
      // Encode now so later mutations of caller-owned values cannot alter edits.
      const entry = encode({ path, operations, expected_revision: options?.expected_revision });
      const previous = entries.get(path);
      if (previous && previous.expected_revision !== entry.expected_revision) throw new Error("A batch requires one expected_revision per asset.");
      if (count + operations.length > 10_000 || (!previous && entries.size === 256)) throw new Error("A batch supports at most 256 assets and 10000 operations; flush before adding more.");
      // Do not collapse sets across array edits or parent/child paths: order matters.
      if (previous) previous.operations.push(...entry.operations);
      else entries.set(path, entry);
      count += operations.length;
    },
    flush() {
      if (pending) return pending;
      try { assertCollecting(); } catch (error) { return Promise.reject(error); }
      if (!entries.size) return Promise.resolve(null);
      state = "flushing";
      pending = Promise.resolve().then(() => apply([...entries.values()])).then((result) => {
        if (!result.applied || !result.persisted || result.results.length !== entries.size) throw new Error("Asset batch did not confirm persistence; inspect the transaction outcome before retrying.");
        entries.clear(); count = 0; state = "collecting";
        return result;
      }).catch((error) => {
        // A transport failure may occur after disk commit. Never replay automatically.
        state = "failed";
        throw error;
      }).finally(() => { pending = null; });
      return pending;
    },
    clear() {
      if (state === "flushing") throw new Error("Cannot clear an in-flight asset batch.");
      entries.clear(); count = 0; state = "collecting";
    },
  };
}
