import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("plugin download sources", () => {
  it("supports fixed downloads and dynamic repository sources", () => {
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");

    expect(service).toContain("download?: PluginRegistryDownload | null");
    expect(service).toContain("downloadSource?: PluginDownloadSource | null");
    expect(service).toContain("export interface PluginRegistryInstallRequest");
    expect(service).toContain("download?: PluginRegistryDownload;");
    expect(service).toContain("downloadSource?: PluginDownloadSource;");
    expect(service).toContain("assetPattern?: string | null");
    expect(service).toContain("latestVersion: string");
    expect(backend).toContain("download_source: PluginDownloadSource");
    expect(backend).toContain("asset_pattern: String");
    expect(backend).toContain("ensure_secure_plugin_url");
    expect(backend).toContain("plugin_download_error_blocks_registry_fallback");
    expect(backend).toContain("release_asset_pattern_matches");
    expect(backend).toContain("registry_download_is_resolved");
    expect(backend).toContain("resolve_github_release_source");
    expect(backend).toContain("github_release_asset_download_url");
    expect(backend).toContain('segments.push("latest")');
    expect(backend).toContain("github_archive_zip_url");
    expect(backend).toContain("clone_plugin_git_source");
    expect(backend).toContain("plugin_github_auth_token");
    expect(backend).toContain("plugin_github_oauth_start");
    expect(backend).toContain("plugin_github_oauth_poll");
    expect(backend).toContain("resolve_github_cli");
    expect(backend).toContain("run_github_cli_login");
    expect(backend).not.toContain("https://github.com/login/device/code");
    expect(backend).not.toContain("https://github.com/login/oauth/access_token");
    expect(backend).toContain("fetch_github_api_bytes");
    expect(backend).toContain("KEY_PLUGIN_GITHUB_TOKEN");
    expect(backend).toContain("fallback registry download failed");
    expect(backend).toContain("fallback download source failed");
    expect(backend).toContain("latestrelease");
    expect(backend).toContain('"branch" | "tag" | "commit"');

    const tool = read("tools/plugin_install.json");
    expect(tool).toContain("assetPattern");
    expect(tool).toContain("versioned release assets");
  });
});
