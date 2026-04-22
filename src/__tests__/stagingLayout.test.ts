import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  persistStagingLayout,
  persistStagingViewMode,
  readStoredStagingLayout,
  readStoredStagingViewMode,
  STAGING_LAYOUT_STORAGE_KEY,
  STAGING_VIEW_MODE_STORAGE_KEY,
} from "../components/collab/stagingLayout";

function createStorageMock() {
  const store = new Map<string, string>();
  return {
    getItem(key: string) {
      return store.has(key) ? store.get(key)! : null;
    },
    setItem(key: string, value: string) {
      store.set(key, value);
    },
    removeItem(key: string) {
      store.delete(key);
    },
    clear() {
      store.clear();
    },
  };
}

describe("staging layout persistence", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", createStorageMock());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("defaults to vertical layout when no stored value exists", () => {
    expect(readStoredStagingLayout()).toBe(false);
  });

  it("restores the stored layout direction", () => {
    localStorage.setItem(STAGING_LAYOUT_STORAGE_KEY, "true");
    expect(readStoredStagingLayout()).toBe(true);

    localStorage.setItem(STAGING_LAYOUT_STORAGE_KEY, "false");
    expect(readStoredStagingLayout()).toBe(false);
  });

  it("persists layout direction changes", () => {
    persistStagingLayout(true);
    expect(localStorage.getItem(STAGING_LAYOUT_STORAGE_KEY)).toBe("true");

    persistStagingLayout(false);
    expect(localStorage.getItem(STAGING_LAYOUT_STORAGE_KEY)).toBe("false");
  });

  it("defaults to list view when no stored view mode exists", () => {
    expect(readStoredStagingViewMode()).toBe("list");
  });

  it("restores the stored view mode", () => {
    localStorage.setItem(STAGING_VIEW_MODE_STORAGE_KEY, "tree");
    expect(readStoredStagingViewMode()).toBe("tree");

    localStorage.setItem(STAGING_VIEW_MODE_STORAGE_KEY, "list");
    expect(readStoredStagingViewMode()).toBe("list");
  });

  it("persists tree/list view mode changes", () => {
    persistStagingViewMode("tree");
    expect(localStorage.getItem(STAGING_VIEW_MODE_STORAGE_KEY)).toBe("tree");

    persistStagingViewMode("list");
    expect(localStorage.getItem(STAGING_VIEW_MODE_STORAGE_KEY)).toBe("list");
  });
});
