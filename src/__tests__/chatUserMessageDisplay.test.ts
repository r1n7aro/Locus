import { describe, expect, it } from "vitest";
import {
  displayUserMessageContent,
  userMessageConsoleEntries,
  userMessageLocalFileEntries,
} from "../composables/chatUserMessageDisplay";

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

  it("hides structured Unity asset reference blocks", () => {
    expect(displayUserMessageContent(
      "检查这个预制体\n\n<unity-asset-refs>\n- asset: {@Assets/Prefabs/Player.prefab}\n- scene object: {@Assets/Scenes/Main.unity/Root/Player}\n</unity-asset-refs>",
    )).toBe("检查这个预制体");
  });

  it("hides structured Console blocks from user text", () => {
    expect(displayUserMessageContent(
      "分析原因\n\n<locus-console>\nUse these Unity Console entries as diagnostic context.\n\n## Entry 1: [Error] InvalidCastException\nSource: unity-console\nChars: 18\n\n[Error] InvalidCastException\n</locus-console>",
    )).toBe("分析原因");
  });

  it("hides structured local file blocks from user text", () => {
    expect(displayUserMessageContent(
      "分析这个 PSD\n\n<locus-local-files>\nThese are local paths supplied by drag and drop. Read contents only when needed, using `read` for files and `list` for folders.\n- file: `E:/cache/Mobile Game GUI.psd`; type: psd\n</locus-local-files>",
    )).toBe("分析这个 PSD");
  });

  it("extracts structured Console entries for attachment display", () => {
    const entries = userMessageConsoleEntries(
      "<locus-console>\nUse these Unity Console entries as diagnostic context.\n\n## Entry 1: [Error] InvalidCastException\nSource: unity-console\nChars: 18\n\n[Error] InvalidCastException\n\n---\n\n## Entry 2: [Warning] Slow call\nSource: unity-console\nChars: 14\n\n[Warning] Slow call\n</locus-console>",
    );

    expect(entries).toEqual([
      {
        title: "[Error] InvalidCastException",
        level: "Error",
        source: "unity-console",
        chars: 18,
        text: "[Error] InvalidCastException",
      },
      {
        title: "[Warning] Slow call",
        level: "Warning",
        source: "unity-console",
        chars: 14,
        text: "[Warning] Slow call",
      },
    ]);
  });

  it("extracts structured local file entries for attachment display", () => {
    const entries = userMessageLocalFileEntries(
      "<locus-local-files>\nThese are local paths supplied by drag and drop. Read contents only when needed, using `read` for files and `list` for folders.\n- file: `E:/cache/Mobile Game GUI.psd`; type: psd\n- folder: `E:/cache/exports`\n</locus-local-files>",
    );

    expect(entries).toEqual([
      {
        kind: "file",
        path: "E:/cache/Mobile Game GUI.psd",
        typeLabel: "psd",
      },
      {
        kind: "folder",
        path: "E:/cache/exports",
        typeLabel: "",
      },
    ]);
  });

  it("keeps user-authored bracket prefixes", () => {
    expect(displayUserMessageContent("[BUG] 修复按钮状态")).toBe("[BUG] 修复按钮状态");
  });

  it("returns empty content for injection-only messages", () => {
    expect(displayUserMessageContent("<system-reminder>\nEnv\n</system-reminder>")).toBe("");
  });
});
