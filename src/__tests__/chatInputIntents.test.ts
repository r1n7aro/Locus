import { describe, expect, it } from "vitest";
import {
  detectActiveOperator,
  insertInlineMention,
  parseInlineIntentCommands,
  type CommandDef,
} from "../composables/chatInputIntents";

const commands: CommandDef[] = [
  {
    name: "/clear",
    description: "clear",
    commandKind: "action",
    commandType: "clear",
  },
  {
    name: "/plan",
    description: "plan",
    commandKind: "intent",
    commandType: "plan",
    agentOnly: "dev",
  },
  {
    name: "/unity-editor-tooling",
    description: "skill",
    commandKind: "intent",
    commandType: "skill",
    skill: {
      dirName: "unity-editor-tooling",
      source: "project",
      name: "unity-editor-tooling",
    },
  },
];

describe("detectActiveOperator", () => {
  it("detects slash commands in the middle of text", () => {
    const text = "请先 /pla 再继续";
    const operator = detectActiveOperator(text, text.indexOf("/pla") + 4);

    expect(operator).toMatchObject({
      kind: "slash",
      token: "/pla",
    });
  });

  it("keeps slash scope on the typed prefix when there is existing text after the cursor", () => {
    const text = "/plan继续修复这里的逻辑";
    const operator = detectActiveOperator(text, 5);

    expect(operator).toMatchObject({
      kind: "slash",
      start: 0,
      end: 5,
      token: "/plan",
      query: "plan",
    });
  });

  it("stops slash detection once the cursor moves into natural-language text", () => {
    const text = "/plan继续修复这里的逻辑";
    const operator = detectActiveOperator(text, text.length);

    expect(operator).toBeNull();
  });

  it("does not treat slash-prefixed Chinese text as a command query", () => {
    const text = "/创建三步任务";
    const operator = detectActiveOperator(text, text.length);

    expect(operator).toBeNull();
  });

  it("detects mentions anywhere in the composer", () => {
    const text = "先 /plan 再参考 @Assets/UI/Main.prefab";
    const operator = detectActiveOperator(text, text.length);

    expect(operator).toMatchObject({
      kind: "mention",
      token: "@Assets/UI/Main.prefab",
    });
  });

  it("keeps mention scope on the typed prefix when there is existing text after the cursor", () => {
    const text = "@As请继续处理，后面还有正文";
    const operator = detectActiveOperator(text, 3);

    expect(operator).toMatchObject({
      kind: "mention",
      start: 0,
      end: 3,
      token: "@As",
      query: "As",
    });
  });

  it("detects mentions in the middle of existing text without requiring a leading separator", () => {
    const text = "请@As继续处理";
    const operator = detectActiveOperator(text, 4);

    expect(operator).toMatchObject({
      kind: "mention",
      start: 1,
      end: 4,
      token: "@As",
      query: "As",
    });
  });

  it("does not treat URLs as slash commands", () => {
    const text = "see http://example.com/path";
    const operator = detectActiveOperator(text, text.indexOf("example") + 3);
    expect(operator).toBeNull();
  });

  it("does not treat email addresses as mentions", () => {
    const text = "mail foo@bar.com for details";
    const operator = detectActiveOperator(text, text.indexOf("@bar") + 2);
    expect(operator).toBeNull();
  });
});

describe("parseInlineIntentCommands", () => {
  it("extracts plan and skill commands from any position", () => {
    const result = parseInlineIntentCommands(
      "请先 /plan 用 /unity-editor-tooling 修复 Inspector",
      commands,
      "dev",
    );

    expect(result.blockedCommand).toBeNull();
    expect(result.cleanedText).toBe("请先 用 修复 Inspector");
    expect(result.intent.mode).toBe("plan");
    expect(result.intent.skills).toEqual([
      {
        dirName: "unity-editor-tooling",
        source: "project",
        name: "unity-editor-tooling",
      },
    ]);
  });

  it("blocks /plan outside dev sessions", () => {
    const result = parseInlineIntentCommands("请先 /plan 看一下", commands, "designer");

    expect(result.blockedCommand?.name).toBe("/plan");
    expect(result.cleanedText).toBe("请先 /plan 看一下");
  });
});

describe("insertInlineMention", () => {
  it("inserts an inline asset mention before existing text without swallowing it", () => {
    const result = insertInlineMention("@As请继续处理", 0, 3, "Assets/UI/Main.prefab");

    expect(result.text).toBe("`Assets/UI/Main.prefab` 请继续处理");
    expect(result.cursor).toBe("`Assets/UI/Main.prefab` ".length);
  });

  it("adds separators when inserting a mention in the middle of text", () => {
    const text = "请修复@Ma这里的逻辑";
    const start = text.indexOf("@");
    const result = insertInlineMention(text, start, start + 3, "Assets/UI/Main.prefab");

    expect(result.text).toBe("请修复 `Assets/UI/Main.prefab` 这里的逻辑");
    expect(result.cursor).toBe("请修复 `Assets/UI/Main.prefab` ".length);
  });

  it("preserves trailing slash when inserting a folder mention", () => {
    const result = insertInlineMention("@UIE继续处理", 0, 4, "UIElementsSchema/");

    expect(result.text).toBe("`UIElementsSchema/` 继续处理");
    expect(result.cursor).toBe("`UIElementsSchema/` ".length);
  });

  it("wraps scene object mentions so spaces stay inside the path", () => {
    const result = insertInlineMention(
      "@Main继续处理",
      0,
      5,
      "Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)",
    );

    expect(result.text).toBe("`Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)` 继续处理");
    expect(result.cursor).toBe("`Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)` ".length);
  });
});
