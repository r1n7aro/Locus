import { describe, expect, it } from "vitest";
import {
  DEFAULT_PLUGIN_REGISTRY_BRANCH,
  DEFAULT_PLUGIN_REGISTRY_PATH,
  normalizePluginRegistrySources,
  parsePluginRegistryRepoInput,
  pluginRegistrySourceBaseUrl,
  type PluginRegistrySource,
} from "../services/pluginRegistrySources";

describe("plugin registry sources", () => {
  it("parses owner/repo and GitHub URL inputs", () => {
    expect(parsePluginRegistryRepoInput("r1n7aro/locus-plugin-registry")).toMatchObject({
      owner: "r1n7aro",
      repo: "locus-plugin-registry",
      provider: "github",
    });
    expect(parsePluginRegistryRepoInput("https://github.com/r1n7aro/locus-plugin-registry.git")).toMatchObject({
      owner: "r1n7aro",
      repo: "locus-plugin-registry",
      provider: "github",
    });
  });

  it("parses full registry addresses from common Git hosts", () => {
    expect(parsePluginRegistryRepoInput("https://github.com/r1n7aro/locus-plugin-registry/tree/test/public/v1")).toMatchObject({
      owner: "r1n7aro",
      repo: "locus-plugin-registry",
      provider: "github",
      branch: "test",
      path: "public/v1",
    });
    expect(parsePluginRegistryRepoInput("https://gitlab.com/acme/tools/locus-plugin-registry/-/tree/release/registry/v1")).toMatchObject({
      owner: "acme",
      repo: "tools/locus-plugin-registry",
      provider: "gitlab",
      branch: "release",
      path: "registry/v1",
    });
    expect(parsePluginRegistryRepoInput("https://gitee.com/acme/locus-plugin-registry/tree/test/v1")).toMatchObject({
      owner: "acme",
      repo: "locus-plugin-registry",
      provider: "gitee",
      branch: "test",
      path: "v1",
    });
    expect(parsePluginRegistryRepoInput("https://example.com/raw/locus-registry/v1")).toMatchObject({
      owner: "example.com",
      repo: "raw/locus-registry/v1",
      provider: "raw",
    });
    expect(parsePluginRegistryRepoInput("https://git.example.com/acme/locus-plugin-registry/src/branch/main/v1")).toMatchObject({
      owner: "acme",
      repo: "locus-plugin-registry",
      provider: "gitea",
      branch: "main",
      path: "v1",
    });
  });

  it("builds a raw GitHub registry base URL with branch and path defaults", () => {
    const source: PluginRegistrySource = {
      id: "default",
      name: "Locus Registry",
      owner: "r1n7aro",
      repo: "locus-plugin-registry",
      branch: "",
      path: "",
    };

    expect(pluginRegistrySourceBaseUrl(source)).toBe(
      `https://raw.githubusercontent.com/r1n7aro/locus-plugin-registry/${DEFAULT_PLUGIN_REGISTRY_BRANCH}/${DEFAULT_PLUGIN_REGISTRY_PATH}`,
    );
  });

  it("keeps branch selection in the registry URL", () => {
    expect(pluginRegistrySourceBaseUrl({
      id: "test",
      name: "Test Registry",
      owner: "r1n7aro",
      repo: "locus-plugin-registry",
      url: "r1n7aro/locus-plugin-registry",
      branch: "test",
      path: DEFAULT_PLUGIN_REGISTRY_PATH,
    })).toBe("https://raw.githubusercontent.com/r1n7aro/locus-plugin-registry/test/public/v1");
  });

  it("migrates the official registry legacy v1 path to the generated public index path", () => {
    const [source] = normalizePluginRegistrySources([{
      id: "default",
      name: "Locus Registry",
      owner: "r1n7aro",
      repo: "locus-plugin-registry",
      url: "r1n7aro/locus-plugin-registry",
      branch: "test",
      path: "v1",
    }]);

    expect(source.path).toBe(DEFAULT_PLUGIN_REGISTRY_PATH);
    expect(pluginRegistrySourceBaseUrl(source)).toBe(
      "https://raw.githubusercontent.com/r1n7aro/locus-plugin-registry/test/public/v1",
    );
  });

  it("migrates official raw GitHub registry URLs from the legacy v1 path", () => {
    const [source] = normalizePluginRegistrySources([{
      id: "raw-default",
      name: "Locus Registry",
      url: "https://raw.githubusercontent.com/r1n7aro/locus-plugin-registry/test/v1",
    }]);

    expect(source.url).toBe("r1n7aro/locus-plugin-registry");
    expect(source.branch).toBe("test");
    expect(source.path).toBe(DEFAULT_PLUGIN_REGISTRY_PATH);
    expect(pluginRegistrySourceBaseUrl(source)).toBe(
      "https://raw.githubusercontent.com/r1n7aro/locus-plugin-registry/test/public/v1",
    );
  });

  it("keeps custom registry v1 paths unchanged", () => {
    const [source] = normalizePluginRegistrySources([{
      id: "custom",
      name: "Custom Registry",
      owner: "acme",
      repo: "locus-plugin-registry",
      url: "acme/locus-plugin-registry",
      branch: "test",
      path: "v1",
    }]);

    expect(source.path).toBe("v1");
    expect(pluginRegistrySourceBaseUrl(source)).toBe(
      "https://raw.githubusercontent.com/acme/locus-plugin-registry/test/v1",
    );
  });

  it("builds raw registry base URLs for non-GitHub hosts", () => {
    expect(pluginRegistrySourceBaseUrl({
      id: "gitlab",
      name: "GitLab Registry",
      owner: "acme",
      repo: "tools/locus-plugin-registry",
      url: "https://gitlab.com/acme/tools/locus-plugin-registry",
      branch: "test",
      path: "v1",
    })).toBe("https://gitlab.com/acme/tools/locus-plugin-registry/-/raw/test/v1");

    expect(pluginRegistrySourceBaseUrl({
      id: "raw",
      name: "Raw Registry",
      owner: "example.com",
      repo: "registry/v1",
      url: "https://example.com/registry/v1",
      branch: "",
      path: "",
    })).toBe("https://example.com/registry/v1");

    expect(pluginRegistrySourceBaseUrl({
      id: "gitea",
      name: "Gitea Registry",
      owner: "acme",
      repo: "locus-plugin-registry",
      url: "https://git.example.com/acme/locus-plugin-registry/src/branch/main/v1",
      branch: "",
      path: "",
    })).toBe("https://git.example.com/acme/locus-plugin-registry/raw/branch/main/public/v1");
  });
});
