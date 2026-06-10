import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();
const pluginWord = "plugin";
const legacyCreatePluginCommand = `/create-${pluginWord}`;
const legacyCreatePluginPath = `knowledge/skill/builtin/create-${pluginWord}.md`;

function read(relPath: string): string {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function readJson<T>(relPath: string): T {
  return JSON.parse(read(relPath)) as T;
}

describe("plugin Skill package", () => {
  it("exposes /plugin as the single command entry", () => {
    const manifest = readJson<{
      id: string;
      injectMode: string;
      description: string;
      command: { enabled: boolean; trigger: string; argumentHint: string };
    }>("skills/plugin/skill.json");
    const skill = read("skills/plugin/SKILL.md");

    expect(manifest.id).toBe("plugin");
    expect(manifest.injectMode).toBe("excerpt");
    expect(manifest.description).toContain("mentions plugin or 插件");
    expect(manifest.command.enabled).toBe(true);
    expect(manifest.command.trigger).toBe("/plugin");
    expect(manifest.command.argumentHint).toBe("<plugin request>");
    expect(skill).toContain("plugin_search");
    expect(skill).toContain("## L1");
    expect(skill).toContain("targets Locus app extensions");
    expect(skill).toContain("plugin_install");
    expect(skill).toContain("plugin_set_enabled");
    expect(skill).toContain("plugin_uninstall");
    expect(skill).toContain("plugin_export");
    expect(skill).toContain("Use progressive disclosure for specialized editing.");
    expect(skill).toContain('path: "skill/view"');
    expect(skill).toContain('path: "skill/builtin/create-skill.md"');
    expect(skill).toContain("ask_user_question");
    expect(skill).toContain("gh auth login -h github.com -s repo,read:org");
    expect(skill).toContain("A public plugin GitHub repository must contain an installable plugin source tree at the repository root.");
    expect(skill).toContain("Optional plugin Rules live under the plugin root");
    expect(skill).toContain("They are enabled by default while the plugin is enabled.");
    expect(skill).toContain("Disabling a plugin keeps it installed and listed");
    expect(skill).toContain("components.rules");
    expect(skill).toContain("`rules/`");
    expect(skill).toContain("Do not create a repository that only contains README/LICENSE and a release asset.");
    expect(skill).toContain("Build the GitHub repository source tree from the same exported plugin root used for the release asset.");
    expect(skill).toContain("Do not hide the installable plugin under `release/`, `dist/`, or another nested folder");
    expect(skill).toContain("validate repository-source installation separately from release-asset installation");
    expect(skill).toContain("The official Locus plugin registry is always `r1n7aro/locus-plugin-registry`.");
    expect(skill).toContain("use `r1n7aro/locus-plugin-registry` directly. Do not ask the user to choose a registry repository.");
    expect(skill).toContain("Do not infer the registry repository from plugin id, plugin repository name, GitHub login, package namespace, organization name, or common names");
    expect(skill).toContain("Always use `ask_user_question` before creating, exporting, or publishing a new plugin id.");
    expect(skill).toContain("Prefer concise package-manager style ids");
    expect(skill).toContain("asset-browser-tools");
    expect(skill).toContain("Use reverse-DNS or owner-prefixed ids only when the user chooses that naming scheme");
    expect(skill).toContain("After the user chooses an id, check for duplicates before continuing.");
    expect(skill).toContain("read `entries/v1/plugins/<bucket>/<plugin-id>.json` on the target base branch");
    expect(skill).toContain("Write exactly one registry source file for a new plugin");
    expect(skill).toContain("Every registry source entry must include user-facing metadata");
    expect(skill).toContain("Do not use legacy or npm-style fields such as `schema`, `version`, `homepage`, `repository`, or `components`");
    expect(skill).toContain("Include standard stats definitions for GitHub-hosted plugins");
    expect(skill).toContain("Registry CI refreshes these stat values when it generates `public/v1`.");
    expect(skill).toContain("Updating an existing entry must first read the file on the target branch and include its current blob `sha`");
    expect(skill).toContain("Do not modify `public/v1/**` in the registration PR.");
    expect(skill).toContain("`public/v1/**` left unchanged");
    expect(skill).not.toContain("Write exactly the registry files needed for one plugin: `v1/manifest.json`");
    expect(skill).toContain("The registry repository owner does not determine plugin id.");
    expect(skill).toContain("All writable branches for registry changes belong to the user's fork.");
    expect(skill).toContain("Set `<registry-owner>/<registry-repo>` to `r1n7aro/locus-plugin-registry` for official registry publishing.");
    expect(skill).toContain("skip registry-repository discovery and registry-repository confirmation");
    expect(skill).toContain("gh repo view r1n7aro/locus-plugin-registry --json nameWithOwner,defaultBranchRef");
    expect(skill).toContain("gh repo fork r1n7aro/locus-plugin-registry --clone=false --remote=false");
    expect(skill).toContain("For version-only updates of an already registered plugin");
    expect(skill).toContain("update and commit the plugin repository root source tree, publish the GitHub release asset from that committed source");
    expect(skill).toContain("Do not open a registry PR unless metadata, compatibility, dependency metadata, description, icon, tags, repo, license, or download source rules change.");
    expect(skill).toContain("the source entry should store `downloadSource`, not generated `download`, `latestVersion`, or release SHA fields");
    expect(skill).toContain("Registry CI resolves `latestVersion`, `download.url`, `download.sha256`, `download.sizeBytes`, `updatedAt`, and `downloadSource.version` into `public/v1`.");
    expect(skill).toContain("use `assetPattern`, such as `locus-workspace-*.zip`");
    expect(skill).toContain("Do not set `asset` to a versioned filename");
    expect(skill).toContain("After publishing or replacing a GitHub release asset");
    expect(skill).toContain("If `downloadSource.assetPattern` is used, it must match exactly one asset.");
    expect(skill).toContain("fix the release assets or selector before opening a registry PR and before declaring a version-only release complete.");
    expect(skill).toContain("Pushes, scheduled runs, and manual dispatches rebuild `public/v1` from `entries/v1` and GitHub release assets");
    expect(skill).toContain("If a latest GitHub release contains multiple zip assets");
    expect(skill).toContain("--head <viewer>:<fork-branch>");
    expect(skill).toContain("--body-file <body.md>");
    expect(skill).toContain("Write PR body files as literal Markdown.");
    expect(skill).toContain("## What This Plugin Does");
    expect(skill).toContain("## How To Use");
    expect(skill).toContain("plugin repo, author, stats definitions, download source rule");
    expect(skill).toContain("plugin repository source installability, release archive inspection");
    expect(skill).toContain("Verify changed files before merge");
    expect(skill).toContain("GitHub raw branch URLs can lag briefly after a generated commit");
    expect(skill).toContain("Download the public entry archive URL after merge and verify SHA-256");
    expect(skill).toContain("Download the plugin repository source archive or install from the GitHub repo after publishing.");
    expect(skill).toContain("Do not create or update `entries/v1` for that release unless registry metadata changed.");
    expect(skill).toContain("installAfterExport: true");
    expect(skill).toContain("transferOwnership: true");
    expect(existsSync(resolve(cwd, legacyCreatePluginPath))).toBe(false);
  });

  it("keeps plugin tools aligned with the natural-language workflow", () => {
    const exportTool = read("tools/plugin_export.json");
    const exportToolJson = readJson<{
      parameters: {
        properties: {
          ruleFiles: { type: string };
          installAfterExport: { type: string | string[] };
          transferOwnership: { type: string | string[] };
        };
      };
    }>("tools/plugin_export.json");
    const searchTool = readJson<{ parameters: { required: string[] } }>("tools/plugin_search.json");
    const installTool = read("tools/plugin_install.json");
    const setEnabledTool = readJson<{ parameters: { required: string[] } }>("tools/plugin_set_enabled.json");
    const setEnabledToolText = read("tools/plugin_set_enabled.json");
    const uninstallTool = readJson<{ parameters: { required: string[] } }>("tools/plugin_uninstall.json");
    const skillReloadTool = read("tools/skill_reload.json");
    const viewListTool = read("tools/view_list.json");

    expect(exportTool).toContain("/plugin workflow");
    expect(exportTool).not.toContain(legacyCreatePluginCommand);
    expect(exportTool).toContain("asset-tools");
    expect(exportTool).toContain("Prefer concise package-manager-style ids");
    expect(exportTool).toContain("installAfterExport");
    expect(exportTool).toContain("transferOwnership");
    expect(exportTool).toContain("components.rules");
    expect(exportTool).toContain("ruleFiles");
    expect(exportToolJson.parameters.properties.ruleFiles.type).toBe("array");
    expect(exportToolJson.parameters.properties.installAfterExport.type).toBe("boolean");
    expect(exportToolJson.parameters.properties.transferOwnership.type).toBe("boolean");
    expect(searchTool.parameters.required).toEqual(["query"]);
    expect(installTool).toContain("pluginId");
    expect(installTool).toContain("repo");
    expect(setEnabledTool.parameters.required).toEqual(["pluginId", "enabled"]);
    expect(setEnabledToolText).toContain("remain installed and listed");
    expect(uninstallTool.parameters.required).toEqual(["pluginId"]);
    expect(skillReloadTool).toContain("pluginApp");
    expect(skillReloadTool).toContain("pluginProject");
    expect(viewListTool).toContain("installed plugin Views");
  });
});
