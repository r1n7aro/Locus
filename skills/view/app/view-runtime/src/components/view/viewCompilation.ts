import { version } from "../../../package.json";
import type { ViewPackageDetail } from "../../services/view";
import { VIEW_COMPILATION_VERSION, viewSourceHash, type CompiledViewPackage, type ViewCompileReply } from "./viewCompilationTypes";

const LIMIT = 24;
const cache = new Map<string, CompiledViewPackage>();
const inFlight = new Map<string, Promise<CompiledViewPackage>>();
let worker: Worker | null = null;
let idleTimer: ReturnType<typeof setTimeout> | null = null;
let sequence = 0;
let database: Promise<IDBDatabase | null> | null = null;
const pending = new Map<number, { resolve(result: CompiledViewPackage): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> }>();

function openCache(): Promise<IDBDatabase | null> {
  if (typeof indexedDB === "undefined") return Promise.resolve(null);
  return database ??= new Promise((resolve) => {
    const request = indexedDB.open("locus-view-compiled", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("artifacts", { keyPath: "key" });
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => resolve(null);
    request.onblocked = () => resolve(null);
  });
}

async function persisted(key: string): Promise<CompiledViewPackage | null> {
  const db = await openCache();
  if (!db) return null;
  return new Promise((resolve) => {
    try {
      const request = db.transaction("artifacts").objectStore("artifacts").get(key);
      request.onsuccess = () => resolve(request.result?.artifact ?? null);
      request.onerror = () => resolve(null);
    } catch { resolve(null); }
  });
}

async function persist(artifact: CompiledViewPackage): Promise<void> {
  const db = await openCache();
  if (!db) return;
  try {
    const store = db.transaction("artifacts", "readwrite").objectStore("artifacts");
    store.put({ key: artifact.key, artifact, time: Date.now() });
    const request = store.getAll();
    request.onsuccess = () => {
      for (const item of request.result.sort((a, b) => b.time - a.time).slice(LIMIT)) store.delete(item.key);
    };
  } catch { /* A full cache must not prevent opening a View. */ }
}

function failWorker(error: Error): void {
  if (idleTimer) clearTimeout(idleTimer); idleTimer = null;
  worker?.terminate(); worker = null;
  for (const task of pending.values()) { clearTimeout(task.timer); task.reject(error); }
  pending.clear();
}

function getWorker(): Worker {
  if (idleTimer) clearTimeout(idleTimer); idleTimer = null;
  if (worker) return worker;
  worker = new Worker(new URL("./viewCompilation.worker.ts", import.meta.url), { type: "module", name: "locus-view-compiler" });
  worker.onmessage = ({ data }: MessageEvent<ViewCompileReply>) => {
    const task = pending.get(data.id);
    if (!task) return;
    pending.delete(data.id); clearTimeout(task.timer);
    if ("error" in data) task.reject(new Error(data.error)); else task.resolve(data.result);
    if (pending.size === 0) idleTimer = setTimeout(() => { worker?.terminate(); worker = null; idleTimer = null; }, 30_000);
  };
  worker.onerror = (event) => failWorker(new Error(event.message || "View compiler worker failed."));
  worker.onmessageerror = () => failWorker(new Error("Invalid View compiler worker response."));
  return worker;
}

export async function compileViewPackage(detail: ViewPackageDetail): Promise<CompiledViewPackage> {
  const source = JSON.stringify([version, VIEW_COMPILATION_VERSION, detail.summary.packageRoot, detail.manifest, detail.files.map(({ relPath, content, truncated }) => [relPath, content, truncated])]);
  const bytes = new TextEncoder().encode(source);
  const key = typeof crypto !== "undefined" && crypto.subtle
    ? Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (byte) => byte.toString(16).padStart(2, "0")).join("")
    : viewSourceHash(source);
  const existing = cache.get(key);
  if (existing) { cache.delete(key); cache.set(key, existing); return existing; }
  const current = inFlight.get(key);
  if (current) return current;
  const task = (async () => {
    const stored = await persisted(key);
    const scopeId = `view-${viewSourceHash(detail.summary.packageRoot)}`;
    const request = { id: ++sequence, key, scopeId, detail };
    const artifact = stored ?? (typeof Worker === "undefined"
      ? (await import("./viewCompilationCore")).compileViewPackageSource(request)
      : await new Promise<CompiledViewPackage>((resolve, reject) => {
          try {
            const compiler = getWorker();
            const timer = setTimeout(() => failWorker(new Error("View compilation timed out.")), 30_000);
            pending.set(request.id, { resolve, reject, timer }); compiler.postMessage(request);
          } catch (error) { failWorker(error instanceof Error ? error : new Error(String(error))); reject(error); }
        }));
    cache.set(key, artifact);
    while (cache.size > LIMIT) cache.delete(cache.keys().next().value!);
    if (!stored) void persist(artifact);
    return artifact;
  })();
  inFlight.set(key, task);
  try { return await task; } finally { inFlight.delete(key); }
}
