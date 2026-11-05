import { remoteClientTransport, type RemoteFileWatch } from "../services/remoteClient";

// Paths the phone shows, so the desktop forwards their file changes (and no
// others: a build can change thousands of files). Screens retain a watch
// while mounted; the merged set is sent once per burst of changes.

const FLUSH_DELAY_MS = 50;
const retained = new Map<string, { watch: RemoteFileWatch; count: number }>();
let flushTimer: ReturnType<typeof setTimeout> | null = null;
let lastSent = "";

function key(watch: RemoteFileWatch): string {
  return `${watch.checkoutId}\n${watch.path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "").toLowerCase()}`;
}

function scheduleFlush() {
  if (flushTimer) return;
  flushTimer = setTimeout(() => {
    flushTimer = null;
    const watches = [...retained.values()].map((entry) => entry.watch);
    const payload = JSON.stringify(watches);
    if (payload === lastSent) return;
    lastSent = payload;
    // The app replays the last set after a reconnect; a failure here only
    // costs live updates, which the screens' own refreshes cover.
    void remoteClientTransport()?.setFileWatches?.(watches).catch(() => {
      lastSent = "";
    });
  }, FLUSH_DELAY_MS);
}

/** Forwards file changes of `path` (a file, or a folder's direct entries)
 * while retained. Returns the release. */
export function retainFileWatch(checkoutId: string, path: string): () => void {
  const watch = { checkoutId, path };
  const id = key(watch);
  const entry = retained.get(id);
  if (entry) entry.count += 1;
  else retained.set(id, { watch, count: 1 });
  scheduleFlush();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    const current = retained.get(id);
    if (!current) return;
    current.count -= 1;
    if (current.count <= 0) retained.delete(id);
    scheduleFlush();
  };
}
