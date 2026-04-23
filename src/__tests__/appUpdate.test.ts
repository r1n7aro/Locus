import { describe, expect, it, vi } from "vitest";
import type { AppUpdateManifest } from "../types";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import {
  compareReleaseVersions,
  resolveAppUpdateInfo,
  resolveUpdateUrl,
} from "../services/appUpdate";

const manifest: AppUpdateManifest = {
  version: "0.2.0",
  releasedAt: "2026-04-23",
  channel: "stable",
  locales: {
    zh: {
      title: "发现新版本",
      summary: "Locus 0.2.0 已发布。",
      changelogUrl: "/overview/latest-version",
      changes: [
        {
          title: "新增",
          items: [
            "新增更新弹窗",
          ],
        },
      ],
    },
    en: {
      title: "Update available",
      summary: "Locus 0.2.0 is available.",
      changelogUrl: "/en/overview/latest-version",
      changes: [
        {
          title: "Added",
          items: [
            "Added the update modal",
          ],
        },
      ],
    },
  },
};

describe("compareReleaseVersions", () => {
  it("compares numeric versions in order", () => {
    expect(compareReleaseVersions("0.1.9", "0.2.0")).toBeLessThan(0);
    expect(compareReleaseVersions("0.2.0", "0.2.0")).toBe(0);
    expect(compareReleaseVersions("0.3.0", "0.2.0")).toBeGreaterThan(0);
  });

  it("detects updates across many skipped versions", () => {
    expect(compareReleaseVersions("0.1.0", "2.8.3")).toBeLessThan(0);
    expect(compareReleaseVersions("0.1.0", "12.0.0")).toBeLessThan(0);
  });

  it("treats prerelease as older than the stable release", () => {
    expect(compareReleaseVersions("0.2.0-beta.1", "0.2.0")).toBeLessThan(0);
    expect(compareReleaseVersions("0.2.0", "0.2.0-beta.1")).toBeGreaterThan(0);
  });

  it("ignores build metadata during comparison", () => {
    expect(compareReleaseVersions("1.0.0", "1.0.0+build.7")).toBe(0);
    expect(compareReleaseVersions("1.0.0+build.2", "1.0.0+build.10")).toBe(0);
  });

  it("keeps hyphenated prerelease identifiers ordered", () => {
    expect(compareReleaseVersions("1.0.0-rc-1", "1.0.0-rc-2")).toBeLessThan(0);
    expect(compareReleaseVersions("1.0.0-rc-2", "1.0.0-rc-1")).toBeGreaterThan(0);
  });
});

describe("resolveAppUpdateInfo", () => {
  it("returns localized update info when the remote version is newer", () => {
    const info = resolveAppUpdateInfo(manifest, "0.1.0", "zh");

    expect(info).toEqual(
      expect.objectContaining({
        currentVersion: "0.1.0",
        latestVersion: "0.2.0",
        title: "发现新版本",
        changelogUrl: "https://unity.farlocus.com/overview/latest-version",
        sourceKind: "remote",
        sourceBaseUrl: "https://unity.farlocus.com",
      }),
    );
  });

  it("returns the requested locale when it exists", () => {
    const info = resolveAppUpdateInfo(manifest, "0.1.0", "en");

    expect(info?.title).toBe("Update available");
    expect(info?.changes[0]?.title).toBe("Added");
  });

  it("skips the dialog when the local build is already current", () => {
    expect(resolveAppUpdateInfo(manifest, "0.2.0", "zh")).toBeNull();
    expect(resolveAppUpdateInfo(manifest, "0.3.0", "zh")).toBeNull();
  });

  it("treats build metadata as the same released version", () => {
    expect(resolveAppUpdateInfo(manifest, "0.2.0+build.9", "zh")).toBeNull();
  });
});

describe("resolveUpdateUrl", () => {
  it("resolves relative docs paths against the public docs host", () => {
    expect(resolveUpdateUrl("/overview/latest-version")).toBe(
      "https://unity.farlocus.com/overview/latest-version",
    );
    expect(resolveUpdateUrl("https://unity.farlocus.com/en/overview/latest-version")).toBe(
      "https://unity.farlocus.com/en/overview/latest-version",
    );
    expect(resolveUpdateUrl("/overview/latest-version", "http://localhost:3002")).toBe(
      "http://localhost:3002/overview/latest-version",
    );
  });
});
