const STAGING_LAYOUT_STORAGE_KEY = "locus.collab.stagingLayout.horizontal";
const STAGING_VIEW_MODE_STORAGE_KEY = "locus.collab.stagingViewMode";

export type StagingViewMode = "list" | "tree";

export function readStoredStagingLayout() {
  try {
    const raw = localStorage.getItem(STAGING_LAYOUT_STORAGE_KEY);
    if (raw === "true") return true;
    if (raw === "false") return false;
  } catch {
    // Ignore storage read failures and fall back to the default layout.
  }
  return false;
}

export function persistStagingLayout(horizontal: boolean) {
  try {
    localStorage.setItem(STAGING_LAYOUT_STORAGE_KEY, String(horizontal));
  } catch {
    // Ignore storage write failures; layout still works for the current session.
  }
}

export function readStoredStagingViewMode(): StagingViewMode {
  try {
    const raw = localStorage.getItem(STAGING_VIEW_MODE_STORAGE_KEY);
    if (raw === "tree") return "tree";
    if (raw === "list") return "list";
  } catch {
    // Ignore storage read failures and fall back to the default tree view.
  }
  return "tree";
}

export function persistStagingViewMode(mode: StagingViewMode) {
  try {
    localStorage.setItem(STAGING_VIEW_MODE_STORAGE_KEY, mode);
  } catch {
    // Ignore storage write failures; the view mode still works for the current session.
  }
}

export {
  STAGING_LAYOUT_STORAGE_KEY,
  STAGING_VIEW_MODE_STORAGE_KEY,
};
