/**
 * Lightweight warmup cache for background preloading.
 * Scoped by workingDir — cache auto-clears on scope change.
 */
let currentScope = "";
let currentGeneration = 0;
const cache = new Map<string, any>();

export function setScope(scope: string) {
  if (scope !== currentScope) {
    cache.clear();
    currentScope = scope;
    currentGeneration += 1;
  }
  return currentGeneration;
}

export function getWarmup<T>(key: string): T | undefined {
  return cache.get(key);
}

export function setWarmup(key: string, value: any, generation = currentGeneration) {
  if (generation !== currentGeneration) return false;
  cache.set(key, value);
  return true;
}

export function clearWarmup() {
  cache.clear();
  currentScope = "";
  currentGeneration += 1;
}
