import { reactive } from "vue";

const storageKey = "locus:explorerFullPaths";
function read(): Record<string, true> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(storageKey) ?? "{}");
    return value && typeof value === "object" && !Array.isArray(value)
      ? Object.fromEntries(Object.entries(value).filter(([, enabled]) => enabled === true)) : {};
  } catch { return {}; }
}
const fullPaths = reactive(read());
if (typeof window !== "undefined") window.addEventListener("storage", (event) => {
  if (event.key !== storageKey && event.key !== null) return;
  for (const key of Object.keys(fullPaths)) delete fullPaths[key];
  Object.assign(fullPaths, read());
});

export function explorerFilePath(root: string, path: string): string {
  const normalized = path.replace(/\\/g, "/");
  return /^(?:[a-z]:\/|\/)/i.test(normalized)
    ? normalized : `${root.replace(/\\/g, "/").replace(/\/$/, "")}/${normalized.replace(/^\.\//, "")}`;
}

export function explorerFileKey(path: string): string {
  const normalized = path.replace(/\\/g, "/").replace(/\/$/, "");
  return `file:${/^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : normalized}`;
}

export function explorerKnowledgeKey(root: string, documentId: string): string {
  return `${explorerFileKey(root)}:knowledge:${documentId}`;
}

export function useExplorerPathDisplay() {
  const showsFullPath = (key: string) => fullPaths[key] === true;
  function persist() {
    try { localStorage.setItem(storageKey, JSON.stringify(fullPaths)); }
    catch (error) { console.warn("[explorer] path display preference could not be saved", error); }
  }
  function togglePath(key: string) {
    if (fullPaths[key]) delete fullPaths[key];
    else fullPaths[key] = true;
    persist();
  }
  function movePathPreference(oldPath: string, newPath: string) {
    const oldKey = explorerFileKey(oldPath);
    if (!fullPaths[oldKey]) return;
    delete fullPaths[oldKey];
    fullPaths[explorerFileKey(newPath)] = true;
    persist();
  }
  return { showsFullPath, togglePath, movePathPreference };
}
