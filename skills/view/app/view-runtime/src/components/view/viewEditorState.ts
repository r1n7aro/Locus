interface Entry { state: Map<string, unknown>; owners: number; cleanup?: ReturnType<typeof setTimeout>; }
const states = new Map<string, Entry>();
/** Vue may recreate a subtree while moving an editor between panes. Its state
 * belongs to the editor, and survives that same-tick ownership handover. */
export function acquireViewEditorState(key: string) {
  let entry = states.get(key);
  if (!entry) { entry = { state: new Map(), owners: 0 }; states.set(key, entry); }
  if (entry.cleanup) clearTimeout(entry.cleanup);
  entry.cleanup = undefined; entry.owners += 1;
  const owned = entry;
  let released = false;
  return { state: entry.state, release() {
    if (released) return; released = true; owned.owners -= 1;
    if (owned.owners === 0) owned.cleanup = setTimeout(() => { if (owned.owners === 0 && states.get(key) === owned) states.delete(key); }, 0);
  } };
}
