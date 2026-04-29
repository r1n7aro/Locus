import { describe, expect, it } from "vitest";
import { displayUserMessageContent } from "../composables/chatUserMessageDisplay";

describe("displayUserMessageContent", () => {
  it("hides system reminder blocks around user text", () => {
    expect(displayUserMessageContent(
      "<system-reminder>\nEnv\n</system-reminder>\n\n创建 test1.cs\n\n<system-reminder>\nPlan\n</system-reminder>",
    )).toBe("创建 test1.cs");
  });

  it("hides Unity editor status change prefixes", () => {
    expect(displayUserMessageContent(
      "[Unity Editor Status Changed] Unity Editor Status: `editing`, Active Scene: Assets/Scenes/Main.unity\n\n在项目根目录下创建文件",
    )).toBe("在项目根目录下创建文件");
  });

  it("hides combined Locus-injected text", () => {
    expect(displayUserMessageContent(
      "<system-reminder>\nEnv\n</system-reminder>\n[Unity Editor Status Changed] Unity Editor Status: `editing`\n\n继续",
    )).toBe("继续");
  });

  it("keeps user-authored bracket prefixes", () => {
    expect(displayUserMessageContent("[BUG] 修复按钮状态")).toBe("[BUG] 修复按钮状态");
  });

  it("returns empty content for injection-only messages", () => {
    expect(displayUserMessageContent("<system-reminder>\nEnv\n</system-reminder>")).toBe("");
  });
});
