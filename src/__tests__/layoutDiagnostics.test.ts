import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  TranscriptElementSample,
  TranscriptLayoutSnapshot,
  ToolBlockLayoutSample,
  ToolBlockLayoutSnapshot,
  ToolLayoutChildSample,
  ToolLayoutStyle,
} from "../services/layoutDiagnostics";
import {
  compareToolBlockLayoutSnapshots,
  compareTranscriptLayoutSnapshots,
  isLayoutDiagnosticsEnabled,
} from "../services/layoutDiagnostics";

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

function style(overrides: Partial<ToolLayoutStyle> = {}): ToolLayoutStyle {
  return {
    display: "flex",
    visibility: "visible",
    opacity: "1",
    position: "static",
    overflow: "visible",
    height: "120px",
    maxHeight: "none",
    marginTop: "0px",
    marginBottom: "0px",
    paddingTop: "0px",
    paddingBottom: "0px",
    gap: "6px",
    rowGap: "6px",
    transform: "none",
    ...overrides,
  };
}

function missingChild(): ToolLayoutChildSample {
  return {
    exists: false,
    rect: null,
    className: "",
    style: null,
  };
}

function child(overrides: Partial<ToolLayoutChildSample> = {}): ToolLayoutChildSample {
  return {
    exists: true,
    rect: { top: 100, left: 20, width: 400, height: 30 },
    className: "child",
    style: style({ height: "30px" }),
    ...overrides,
  };
}

function sample(overrides: Partial<ToolBlockLayoutSample>): ToolBlockLayoutSample {
  return {
    key: "collection:tool-1",
    kind: "collection",
    scope: "transient",
    rawKey: "tool-1",
    toolCallIds: ["tool-1"],
    statuses: "tool-1:running",
    state: {
      allowCollapse: "true",
      animateCollapseOnMount: "false",
      canCollapse: "true",
      collapseEnabled: "false",
      expanded: "true",
      keepExpanded: "",
      panelLeaving: "false",
      panelVisible: "true",
      showWaiting: "true",
    },
    rect: { top: 100, left: 20, width: 400, height: 120 },
    offsetTop: 80,
    offsetLeft: 20,
    contentTop: 300,
    className: "tool-call-collection is-expanded",
    ancestorClassChain: ["chat-transcript-tool-calls-group"],
    style: style(),
    children: {
      summary: child({ className: "tool-call-batch-summary" }),
      panel: child({ className: "tool-call-collection-panel", rect: { top: 132, left: 20, width: 400, height: 84 } }),
      list: child({ className: "tool-call-collection-list", rect: { top: 140, left: 28, width: 384, height: 68 } }),
      waitingRow: missingChild(),
    },
    ...overrides,
  };
}

function snapshot(
  item: ToolBlockLayoutSample,
  overrides: Partial<ToolBlockLayoutSnapshot> = {},
): ToolBlockLayoutSnapshot {
  return {
    scope: "chat-transcript:session",
    reason: "test",
    at: 0,
    scrollTop: 220,
    scrollHeight: 1200,
    clientHeight: 600,
    contentHeight: 1200,
    samples: [item],
    ...overrides,
  };
}

function transcriptSample(overrides: Partial<TranscriptElementSample>): TranscriptElementSample {
  return {
    key: "render-part:transient:text:transient:text-1",
    kind: "render-part:text",
    scrollAnchorId: "__transient__",
    messageAnchorId: "__transient__",
    renderPartKind: "text",
    renderPartKey: "transient:text-1",
    renderPartScope: "transient",
    textLength: 21,
    textPreview: "body text after tools",
    textSignature: "21:body text after tools:body text after tools",
    firstChildTag: "p",
    lastChildTag: "p",
    className: "markdown-body ui-select-text",
    rect: { top: 420, left: 20, width: 480, height: 48 },
    offsetTop: 360,
    contentTop: 700,
    style: style({ display: "block", height: "48px" }),
    ...overrides,
  };
}

function transcriptSnapshot(
  renderParts: TranscriptElementSample[],
  overrides: Partial<TranscriptLayoutSnapshot> = {},
): TranscriptLayoutSnapshot {
  return {
    scope: "chat-transcript:session",
    reason: "test",
    at: 0,
    scrollTop: 340,
    scrollHeight: 1300,
    clientHeight: 600,
    contentHeight: 1300,
    content: null,
    currentAnchor: null,
    messages: [],
    anchors: [],
    renderParts,
    ...overrides,
  };
}

describe("layoutDiagnostics", () => {
  describe("enablement", () => {
    beforeEach(() => {
      vi.stubGlobal("window", { location: { search: "" } });
      vi.stubGlobal("localStorage", createStorageMock());
      vi.stubGlobal("sessionStorage", createStorageMock());
    });

    afterEach(() => {
      vi.unstubAllGlobals();
    });

    it("stays disabled by default even when the legacy persistent flag remains", () => {
      localStorage.setItem("locus:layoutDiagnostics", "1");

      expect(isLayoutDiagnosticsEnabled()).toBe(false);
    });

    it("can be enabled for the current browser session", () => {
      sessionStorage.setItem("locus:layoutDiagnostics", "1");

      expect(isLayoutDiagnosticsEnabled()).toBe(true);
    });

    it("can be enabled from the window query", () => {
      vi.stubGlobal("window", { location: { search: "?layoutDiagnostics=1" } });

      expect(isLayoutDiagnosticsEnabled()).toBe(true);
    });
  });

  it("classifies transient to history movement as handoff reposition", () => {
    const before = sample({});
    const after = sample({
      scope: "history",
      statuses: "tool-1:done",
      rect: { top: 72, left: 20, width: 400, height: 32 },
      offsetTop: 52,
      contentTop: 260,
      state: {
        ...before.state,
        collapseEnabled: "true",
        expanded: "false",
        panelVisible: "false",
        showWaiting: "false",
      },
      className: "tool-call-collection is-collapsible",
    });

    const shifts = compareToolBlockLayoutSnapshots(
      snapshot(before),
      snapshot(after, { scrollTop: 208, scrollHeight: 1112, contentHeight: 1112 }),
    );

    expect(shifts).toHaveLength(1);
    expect(shifts[0]?.primaryCause).toBe("tool-handoff-reposition");
    expect(shifts[0]?.causes).toContain("render-scope-changed");
    expect(shifts[0]?.causes).toContain("tool-status-changed");
    expect(shifts[0]?.causes).toContain("element-height-changed");
  });

  it("classifies expansion state movement as collapse transition", () => {
    const before = sample({});
    const after = sample({
      rect: { top: 100, left: 20, width: 400, height: 32 },
      offsetTop: 80,
      contentTop: 300,
      state: {
        ...before.state,
        expanded: "false",
        panelVisible: "false",
        panelLeaving: "true",
      },
    });

    const shifts = compareToolBlockLayoutSnapshots(snapshot(before), snapshot(after));

    expect(shifts).toHaveLength(1);
    expect(shifts[0]?.primaryCause).toBe("tool-collapse-transition");
    expect(shifts[0]?.causes).toContain("collection-expanded-state-changed");
    expect(shifts[0]?.causes).toContain("collapse-animation-state-changed");
  });

  it("keeps state-only scope swaps separate from geometry shifts", () => {
    const before = sample({
      rect: { top: 100, left: 20, width: 400, height: 30 },
      state: {
        allowCollapse: "true",
        animateCollapseOnMount: "true",
        canCollapse: "",
        collapseEnabled: "true",
        expanded: "",
        keepExpanded: "",
        panelLeaving: "",
        panelVisible: "",
        showWaiting: "false",
      },
    });
    const after = sample({
      scope: "history",
      rect: before.rect,
      state: {
        allowCollapse: "",
        animateCollapseOnMount: "",
        canCollapse: "",
        collapseEnabled: "",
        expanded: "",
        keepExpanded: "false",
        panelLeaving: "",
        panelVisible: "",
        showWaiting: "",
      },
      ancestorClassChain: ["chat-transcript-item-stack is-session"],
    });

    const shifts = compareToolBlockLayoutSnapshots(snapshot(before), snapshot(after));

    expect(shifts).toHaveLength(1);
    expect(shifts[0]?.primaryCause).toBe("visual-state-only");
    expect(shifts[0]?.visualShiftOnly).toBe(true);
    expect(shifts[0]?.delta.height).toBe(0);
  });

  it("reports waiting row geometry as the primary cause", () => {
    const before = sample({
      children: {
        ...sample({}).children,
        waitingRow: missingChild(),
      },
    });
    const after = sample({
      rect: { top: 100, left: 20, width: 400, height: 148.8 },
      state: {
        ...before.state,
        showWaiting: "true",
      },
      children: {
        ...before.children,
        waitingRow: child({
          className: "chat-transcript-tool-waiting-row",
          rect: { top: 218, left: 20, width: 160, height: 28.8 },
          style: style({ display: "inline-flex", height: "22px" }),
        }),
      },
    });

    const shifts = compareToolBlockLayoutSnapshots(snapshot(before), snapshot(after));

    expect(shifts).toHaveLength(1);
    expect(shifts[0]?.primaryCause).toBe("waiting-row-layout");
    expect(shifts[0]?.childDeltas.waitingRow?.existsChanged).toBe(true);
    expect(shifts[0]?.childDeltas.waitingRow?.delta.height).toBe(28.8);
  });

  it("reports text render part movement when transient body lands in history", () => {
    const beforeText = transcriptSample({});
    const afterText = transcriptSample({
      key: "render-part:history:text:message-1:text-1",
      scrollAnchorId: "message-1",
      messageAnchorId: "message-1",
      renderPartKey: "message-1:text-1",
      renderPartScope: "history",
      rect: { top: 390, left: 20, width: 480, height: 44 },
      offsetTop: 330,
      contentTop: 670,
      style: style({ display: "block", height: "44px" }),
    });

    const comparison = compareTranscriptLayoutSnapshots(
      transcriptSnapshot([beforeText]),
      transcriptSnapshot([afterText], { scrollHeight: 1296, contentHeight: 1296 }),
    );

    expect(comparison.addedRenderParts).toHaveLength(1);
    expect(comparison.removedRenderParts).toHaveLength(1);
    expect(comparison.textPartMoves).toHaveLength(1);
    expect(comparison.textPartMoves[0]?.from.renderPartScope).toBe("transient");
    expect(comparison.textPartMoves[0]?.to.renderPartScope).toBe("history");
    expect(comparison.textPartMoves[0]?.delta.contentTop).toBe(-30);
    expect(comparison.textPartMoves[0]?.delta.height).toBe(-4);
  });
});
