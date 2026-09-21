import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppUpdateManifest } from "../types";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import {
  compareReleaseVersions,
  currentAppUpdateTarget,
  resolveAppUpdateInfo,
  resolveUpdateUrl,
} from "../services/appUpdate";

const manifest: AppUpdateManifest = {
  version: "0.2.0",
  releasedAt: "2026-04-23",
  channel: "stable",
  installers: [
    {
      id: "windows-x64",
      label: "Windows x64",
      url: "https://github.com/r1n7aro/Locus/releases/download/v0.2.0/locus_0.2.0_x64-setup.exe",
      platform: "windows",
      arch: "x64",
      includesManagedPython: true,
      includesManagedGit: true,
      requiresSystemPython: false,
      requiresSystemGit: false,
    },
    {
      id: "windows-x64-without-embed-python-git",
      label: "Windows x64 - system Python/Git",
      url: "https://github.com/r1n7aro/Locus/releases/download/v0.2.0/locus_0.2.0_x64-without_embed_python_git-setup.exe",
      platform: "windows",
      arch: "x64",
      includesManagedPython: false,
      includesManagedGit: false,
      requiresSystemPython: true,
      requiresSystemGit: true,
    },
  ],
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
        releaseUrl: "https://github.com/r1n7aro/Locus/releases/tag/v0.2.0",
        downloadUrl: "https://github.com/r1n7aro/Locus/releases/download/v0.2.0/locus_0.2.0_x64-setup.exe",
        downloadLabel: "Windows x64",
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

  it("carries explicit release channels for current and latest versions", () => {
    const info = resolveAppUpdateInfo(
      {
        ...manifest,
        channel: "experimental",
      },
      "0.1.0",
      "zh",
      undefined,
      "remote",
      "experimental",
    );

    expect(info?.currentChannel).toBe("experimental");
    expect(info?.latestChannel).toBe("experimental");
    expect(info?.currentIsExperimental).toBe(true);
    expect(info?.latestIsExperimental).toBe(true);
  });

  it("resolves the browser update target to the GitHub release page", () => {
    const info = resolveAppUpdateInfo({
      ...manifest,
      installers: [],
      locales: {
        zh: {
          ...manifest.locales.zh,
          downloadChannels: [
            {
              label: "Windows x64",
              url: "https://github.com/r1n7aro/Locus/releases/download/v0.2.0/locus_0.2.0_x64-setup.exe",
            },
          ],
        },
      },
    }, "0.1.0", "zh");

    expect(info?.releaseUrl).toBe("https://github.com/r1n7aro/Locus/releases/tag/v0.2.0");
    expect(info?.downloadUrl).toBe("https://unity.farlocus.com/overview/latest-version");
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

describe("macOS installer selection", () => {
  const macInstaller = (arch: string) => ({
    ...manifest.installers![0]!,
    id: `macos-${arch}`, label: `macOS ${arch}`, platform: "macos", arch,
    url: `https://example.com/locus-${arch}.dmg`,
    includesManagedPython: false, includesManagedGit: false,
    requiresSystemPython: true, requiresSystemGit: true,
  });

  it.each(["arm64", "x64"] as const)("selects %s without downloading the other architecture", (arch) => {
    const mixed = { ...manifest, installers: [...manifest.installers!, macInstaller("arm64"), macInstaller("x64")] };
    expect(resolveAppUpdateInfo(mixed, "0.1.0", "zh", undefined, "remote", "stable", { platform: "macos", arch })?.installer?.id)
      .toBe(`macos-${arch}`);
  });

  it("falls back only to a universal Mac package", () => {
    const mixed = { ...manifest, installers: [...manifest.installers!, macInstaller("universal")] };
    expect(resolveAppUpdateInfo(mixed, "0.1.0", "zh", undefined, "remote", "stable", { platform: "macos", arch: "unknown" })?.installer?.id)
      .toBe("macos-universal");
  });

  it("does not offer Windows packages or guess an unknown Mac architecture", () => {
    for (const installers of [manifest.installers!, [macInstaller("x64")], [{ ...macInstaller("arm64"), url: "https://example.com/windows.exe" }]]) {
      const info = resolveAppUpdateInfo({ ...manifest, installers }, "0.1.0", "zh", undefined, "remote", "stable", { platform: "macos", arch: "arm64" });
      expect(info?.installer).toBeNull();
      expect(info?.downloadUrl).toBe(info?.changelogUrl);
    }
    expect(resolveAppUpdateInfo({ ...manifest, installers: [macInstaller("arm64")] }, "0.1.0", "zh", undefined, "remote", "stable", { platform: "macos", arch: "unknown" })?.installer).toBeNull();
  });

  it("preserves Windows selection when Mac installers are added", () => {
    const info = resolveAppUpdateInfo({ ...manifest, installers: [macInstaller("arm64"), ...manifest.installers!] }, "0.1.0", "zh", undefined, "remote", "stable", { platform: "windows", arch: "unknown" });
    expect(info?.installer?.id).toBe("windows-x64");
  });
});

describe("update build target", () => {
  afterEach(() => { vi.unstubAllEnvs(); vi.unstubAllGlobals(); });

  it("uses the build target independently of the browser host", () => {
    vi.stubGlobal("navigator", { platform: "Win32" });
    vi.stubEnv("VITE_LOCUS_TARGET_OS", "macos");
    vi.stubEnv("VITE_LOCUS_TARGET_ARCH", "arm64");
    expect(currentAppUpdateTarget()).toEqual({ platform: "macos", arch: "arm64" });
  });

  it("does not infer CPU architecture from MacIntel browser identification", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    vi.stubEnv("VITE_LOCUS_TARGET_OS", "");
    vi.stubEnv("VITE_LOCUS_TARGET_ARCH", "");
    expect(currentAppUpdateTarget()).toEqual({ platform: "macos", arch: "unknown" });
  });

  it("keeps an explicit Windows target on a Mac browser host", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    vi.stubEnv("VITE_LOCUS_TARGET_OS", "windows");
    expect(currentAppUpdateTarget().platform).toBe("windows");
  });
});
