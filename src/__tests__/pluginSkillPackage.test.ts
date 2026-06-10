import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

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
    const publish = read("skills/plugin/publish.md");

    expect(manifest.id).toBe("plugin");
    expect(manifest.injectMode).toBe("excerpt");
    expect(manifest.description).toContain("Locus app extension");
    expect(manifest.command.enabled).toBe(true);
    expect(manifest.command.trigger).toBe("/plugin");
    expect(manifest.command.argumentHint).toBe("<plugin request>");
    expect(skill).toContain("plugin_search");
    expect(skill).toContain("## L1");
    expect(skill).toContain("Locus app extension");
    expect(skill).toContain("plugin_install");
    expect(skill).toContain("plugin_set_enabled");
    expect(skill).toContain("plugin_uninstall");
    expect(skill).toContain("plugin_export");
    expect(skill).toContain("Use progressive disclosure for specialized editing.");
    expect(skill).toContain('path: "skill/view"');
    expect(skill).toContain('path: "skill/builtin/create-skill.md"');
    expect(skill).toContain('path: "skill/plugin/publish.md"');
    expect(skill).toContain("ask_user_question");
    expect(skill).toContain("- sheet");
    expect(skill).toContain("Confirm the export metadata with the `sheet` tool");
    expect(skill).toContain("Never call `plugin_export` while the latest sheet outcome is a change request.");
    expect(skill).toContain("record the confirmed sheet values, including user edits, in `userApproval`");
    expect(publish).toContain("confirm the registry metadata with the `sheet` tool");
    expect(skill).toContain("Optional plugin Rules live under the plugin root");
    expect(skill).toContain("They are enabled by default while the plugin is enabled.");
    expect(skill).toContain("Disabling a plugin keeps it installed and listed");
    expect(skill).toContain("components.rules");
    expect(skill).toContain("rules/<rule-name>.md");
    expect(skill).toContain("Always ask before creating, exporting, or publishing a new plugin id.");
    expect(skill).toContain("Prefer concise package-manager style ids");
    expect(skill).toContain("asset-browser-tools");
    expect(skill).toContain("Use reverse-DNS or owner-prefixed ids only when the user chooses that naming scheme");
    expect(skill).toContain("After the user chooses an id, check for duplicates before continuing.");
    expect(skill).toContain("installAfterExport: true");
    expect(skill).toContain("transferOwnership: true");
    expect(publish).toContain("gh auth login -h github.com -s repo,read:org");
    expect(publish).toContain("The official Locus plugin registry is always `r1n7aro/locus-plugin-registry`.");
    expect(publish).toContain("Never infer a different registry from plugin id");
    expect(publish).toContain("installable plugin source tree");
    expect(publish).toContain("Do not create a repository that only contains README/LICENSE and a release asset");
    expect(publish).toContain("do not hide the plugin under `release/`, `dist/`, or another nested folder");
    expect(publish).toContain("Verify the final release archive");
    expect(publish).toContain("A new plugin adds exactly one file");
    expect(publish).toContain("Include the user-facing metadata");
    expect(publish).toContain("legacy or npm-style fields");
    expect(publish).toContain("Include standard stats definitions for GitHub-hosted plugins");
    expect(publish).toContain("Store `downloadSource`, never generated `download`, `latestVersion`, or release SHA fields");
    expect(publish).toContain("Use `downloadSource.type: \"latestRelease\"`");
    expect(publish).toContain("assetPattern");
    expect(publish).toContain("Updating an existing entry must first read the file on the target branch and include its current blob `sha`");
    expect(publish).toContain("`public/v1/**` left unchanged");
    expect(publish).not.toContain("Write exactly the registry files needed for one plugin: `v1/manifest.json`");
    expect(publish).toContain("all writable branches for registry changes belong to the user's fork");
    expect(publish).toContain("gh repo view r1n7aro/locus-plugin-registry --json nameWithOwner,defaultBranchRef");
    expect(publish).toContain("gh repo fork r1n7aro/locus-plugin-registry --clone=false --remote=false");
    expect(publish).toContain("--head <viewer>:<fork-branch>");
    expect(publish).toContain("--body-file <body.md>");
    expect(publish).toContain("Write literal Markdown");
    expect(publish).toContain("## What This Plugin Does");
    expect(publish).toContain("## How To Use");
    expect(publish).toContain("GitHub raw branch URLs can lag briefly after a generated commit");
    expect(publish).toContain("Download the public entry archive URL");
    expect(publish).toContain("Do not create or update `entries/v1`");
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
