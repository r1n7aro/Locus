import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("plugin registry cache", () => {
  it("uses a persistent cache and shared HTTP clients for registry metadata", () => {
    const source = read("src-tauri/src/commands/plugin.rs");

    expect(source).toContain("static REGISTRY_HTTP_CLIENT");
    expect(source).toContain("static REGISTRY_DOWNLOAD_HTTP_CLIENT");
    expect(source).toContain("PLUGIN_REGISTRY_INDEX_CACHE_TTL");
    expect(source).toContain("PLUGIN_REGISTRY_DESCRIPTION_CACHE_TTL");
    expect(source).toContain("pub enum PluginRegistryCacheMode");
    expect(source).toContain("CachePreferred");
    expect(source).toContain("NetworkPreferred");
    expect(source).toContain("prefer_any_cache");
    expect(source).toContain("skip_fresh_cache");
    expect(source).toContain("plugin-registry-cache");
    expect(source).toContain("read_plugin_registry_cache");
    expect(source).toContain("write_plugin_registry_cache");
    expect(source).toContain("fetch_registry_cached_bytes");
    expect(source).toContain("fetch_registry_cached_bytes_optional");
    expect(source).toContain("read_plugin_registry_cache(url, extension, None)");
    expect(source).toContain("super::persistent_config_dir()");
    expect(source).toContain("serde_json::from_slice");
  });
});
