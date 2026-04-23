import { describe, expect, it } from "vitest";
import { ref } from "vue";
import { rankSearchResults, splitSearchTerms } from "../composables/searchMatcher";
import { useCommandRegistry } from "../composables/useCommandRegistry";
import type { SkillManifest } from "../types";

describe("splitSearchTerms", () => {
  it("splits camel case and separators into searchable terms", () => {
    expect(splitSearchTerms("/PlayerInput_Controller")).toEqual([
      "player",
      "input",
      "controller",
    ]);
  });
});

describe("rankSearchResults", () => {
  it("matches multi-term queries against camel case asset names", () => {
    const ranked = rankSearchResults(
      [
        { name: "InputRouter" },
        { name: "PlayerInputController" },
      ],
      "player input",
      (item) => [{ text: item.name, weight: 100 }],
    );

    expect(ranked[0]?.name).toBe("PlayerInputController");
  });
});

describe("useCommandRegistry", () => {
  it("finds skill commands even when the query only matches a later segment", () => {
    const skills = ref<SkillManifest[]>([
      {
        name: "create-skill",
        description: "Create a new reusable skill",
        argumentHint: "Describe the skill",
        dirName: "create-skill",
        source: "project",
        relPath: ".skills/create-skill",
        updatedAt: Date.now(),
        skillEnabled: true,
        skillSurface: "both",
        skillDescription: "Create or update skills",
        commandTrigger: "/custom-skill",
      },
    ]);
    const agentId = ref("dev");
    const { filteredCommands } = useCommandRegistry(skills, agentId);

    const results = filteredCommands("/skill");

    expect(results[0]?.name).toBe("/custom-skill");
  });

  it("keeps bare slash as the command palette query and ignores natural-language slash text", () => {
    const skills = ref<SkillManifest[]>([
      {
        name: "create-skill",
        description: "Create a new reusable skill",
        argumentHint: "Describe the skill",
        dirName: "create-skill",
        source: "project",
        relPath: ".skills/create-skill",
        updatedAt: Date.now(),
        skillEnabled: true,
        skillSurface: "both",
        skillDescription: "Create or update skills",
        commandTrigger: "/custom-skill",
      },
    ]);
    const agentId = ref("dev");
    const { filteredCommands } = useCommandRegistry(skills, agentId);

    expect(filteredCommands("/").map((command) => command.name)).toContain("/custom-skill");
    expect(filteredCommands("/创建三步任务")).toEqual([]);
  });
});
