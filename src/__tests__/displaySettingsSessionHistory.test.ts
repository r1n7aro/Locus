import { describe, expect, it } from "vitest";
import {
  DEFAULT_MOUNTED_SESSION_EDITORS_PER_GROUP,
  DEFAULT_SESSION_MESSAGE_PAGE_SIZE,
  MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS,
  normalizeMountedSessionEditorsPerGroup,
  normalizeSessionMessagePageSize,
  SESSION_MESSAGE_PAGE_SIZE_OPTIONS,
} from "../composables/useDisplaySettings";

describe("session history display settings", () => {
  it("uses 120 messages by default and accepts each supported page size", () => {
    expect(DEFAULT_SESSION_MESSAGE_PAGE_SIZE).toBe(120);
    expect(SESSION_MESSAGE_PAGE_SIZE_OPTIONS).toEqual([80, 120, 160, 240, 400]);
    for (const value of SESSION_MESSAGE_PAGE_SIZE_OPTIONS) {
      expect(normalizeSessionMessagePageSize(value)).toBe(value);
      expect(normalizeSessionMessagePageSize(String(value))).toBe(value);
    }
  });

  it("falls back to the default for unsupported or corrupt persisted values", () => {
    for (const value of [undefined, null, "", 0, 121, 1_000, Number.NaN]) {
      expect(normalizeSessionMessagePageSize(value)).toBe(DEFAULT_SESSION_MESSAGE_PAGE_SIZE);
    }
  });
});

describe("mounted session tabs per editor group", () => {
  it("keeps six session tabs mounted by default and accepts each offered limit", () => {
    expect(DEFAULT_MOUNTED_SESSION_EDITORS_PER_GROUP).toBe(6);
    expect(MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS).toContain(DEFAULT_MOUNTED_SESSION_EDITORS_PER_GROUP);
    for (const value of MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS) {
      expect(normalizeMountedSessionEditorsPerGroup(value)).toBe(value);
      expect(normalizeMountedSessionEditorsPerGroup(String(value))).toBe(value);
    }
    for (const value of [undefined, null, "", 4, -1, Number.NaN]) {
      expect(normalizeMountedSessionEditorsPerGroup(value)).toBe(DEFAULT_MOUNTED_SESSION_EDITORS_PER_GROUP);
    }
  });
});
