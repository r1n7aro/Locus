import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isToolCollapseTraceEnabled } from "../services/toolCollapseTrace";

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

describe("tool collapse trace enablement", () => {
  beforeEach(() => {
    vi.stubGlobal("window", { location: { search: "" } });
    vi.stubGlobal("localStorage", createStorageMock());
    vi.stubGlobal("sessionStorage", createStorageMock());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("stays disabled by default even when the legacy persistent flags remain", () => {
    localStorage.setItem("locus.toolCollapseTraceEnabled", "true");
    localStorage.setItem("locus.toolCollapseTrace", "all");

    expect(isToolCollapseTraceEnabled("waitingLayoutStateChanged")).toBe(false);
  });

  it("can be enabled for the current browser session", () => {
    sessionStorage.setItem("locus.toolCollapseTraceEnabled", "true");
    sessionStorage.setItem("locus.toolCollapseTrace", "waiting");

    expect(isToolCollapseTraceEnabled("waitingLayoutStateChanged")).toBe(true);
    expect(isToolCollapseTraceEnabled("transcriptBlockOrderChanged")).toBe(false);
  });

  it("can be enabled from the window query", () => {
    vi.stubGlobal("window", { location: { search: "?toolCollapseTrace=all" } });

    expect(isToolCollapseTraceEnabled("transcriptBlockOrderChanged")).toBe(true);
  });
});
