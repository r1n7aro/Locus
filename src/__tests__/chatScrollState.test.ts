import { describe, expect, it } from "vitest";
import {
  CHAT_SCROLL_BOTTOM_THRESHOLD,
  captureLiveScrollAnchor,
  captureScrollAnchor,
  captureSessionScrollState,
  isNearBottom,
  restoreLiveScrollAnchor,
  resolveSessionScrollTop,
  restoreScrollAnchor,
  type SessionScrollState,
} from "../composables/chatScrollState";

describe("chatScrollState", () => {
  it("treats positions within the bottom threshold as near bottom", () => {
    expect(isNearBottom({
      scrollTop: 452,
      clientHeight: 500,
      scrollHeight: 1000,
    })).toBe(true);

    expect(isNearBottom({
      scrollTop: 451 - CHAT_SCROLL_BOTTOM_THRESHOLD,
      clientHeight: 500,
      scrollHeight: 1000,
    })).toBe(false);
  });

  it("captures bottom and offset modes", () => {
    expect(captureSessionScrollState({
      scrollTop: 700,
      clientHeight: 300,
      scrollHeight: 1000,
    })).toEqual({ mode: "bottom" });

    expect(captureSessionScrollState({
      scrollTop: 240,
      clientHeight: 300,
      scrollHeight: 1000,
    })).toEqual({ mode: "offset", scrollTop: 240 });
  });

  it("captures anchor mode from the first visible anchor", () => {
    const container = {
      scrollTop: 240,
      getBoundingClientRect: () => ({
        top: 100,
        bottom: 400,
      }),
      querySelectorAll: () => ([
        {
          dataset: { scrollAnchorId: "hidden" },
          getBoundingClientRect: () => ({ top: 40, bottom: 90, height: 50 }),
        },
        {
          dataset: { scrollAnchorId: "m2" },
          getBoundingClientRect: () => ({ top: 120, bottom: 220, height: 100 }),
        },
      ]),
    } as any;

    expect(captureSessionScrollState({
      scrollTop: 240,
      clientHeight: 300,
      scrollHeight: 1000,
    }, captureScrollAnchor(container))).toEqual({
      mode: "anchor",
      anchorId: "m2",
      offsetTop: 20,
      fallbackScrollTop: 240,
    });
  });

  it("restores bottom mode to the latest bottom after content grows", () => {
    const state: SessionScrollState = { mode: "bottom" };

    expect(resolveSessionScrollTop({
      scrollTop: 0,
      clientHeight: 300,
      scrollHeight: 1200,
    }, state)).toBe(900);
  });

  it("restores offset mode to the saved position", () => {
    const state: SessionScrollState = { mode: "offset", scrollTop: 240 };

    expect(resolveSessionScrollTop({
      scrollTop: 0,
      clientHeight: 300,
      scrollHeight: 1200,
    }, state)).toBe(240);
  });

  it("restores anchor mode to its fallback scroll top when needed", () => {
    const state: SessionScrollState = {
      mode: "anchor",
      anchorId: "m2",
      offsetTop: 20,
      fallbackScrollTop: 240,
    };

    expect(resolveSessionScrollTop({
      scrollTop: 0,
      clientHeight: 300,
      scrollHeight: 1200,
    }, state)).toBe(240);
  });

  it("clamps offset mode when the new content is shorter", () => {
    const state: SessionScrollState = { mode: "offset", scrollTop: 680 };

    expect(resolveSessionScrollTop({
      scrollTop: 0,
      clientHeight: 300,
      scrollHeight: 700,
    }, state)).toBe(400);
  });

  it("restores an anchor by adjusting scrollTop to keep the same offset", () => {
    const container = {
      scrollTop: 240,
      clientHeight: 300,
      scrollHeight: 1200,
      getBoundingClientRect: () => ({
        top: 100,
        bottom: 400,
      }),
      querySelectorAll: () => ([
        {
          dataset: { scrollAnchorId: "m2" },
          getBoundingClientRect: () => ({ top: 160, bottom: 260, height: 100 }),
        },
      ]),
    } as any;

    expect(restoreScrollAnchor(container, {
      mode: "anchor",
      anchorId: "m2",
      offsetTop: 20,
      fallbackScrollTop: 240,
    })).toBe(true);
    expect(container.scrollTop).toBe(280);
  });

  it("keeps a live element anchored while surrounding content changes", () => {
    const anchor = {
      getBoundingClientRect: () => ({ top: 120, bottom: 150, height: 30 }),
    };
    const container = {
      scrollTop: 240,
      clientHeight: 300,
      scrollHeight: 1200,
      contains: (candidate: unknown) => candidate === anchor,
      getBoundingClientRect: () => ({
        top: 100,
        bottom: 400,
      }),
    } as any;

    const state = captureLiveScrollAnchor(container, anchor as any);
    expect(state).toEqual({
      anchor,
      offsetTop: 20,
      fallbackScrollTop: 240,
    });

    anchor.getBoundingClientRect = () => ({ top: 260, bottom: 290, height: 30 });

    expect(restoreLiveScrollAnchor(container, state)).toBe(true);
    expect(container.scrollTop).toBe(380);
  });
});
