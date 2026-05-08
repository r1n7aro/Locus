import { describe, expect, it } from "vitest";
import { extractChatAssetRefs, parseChatAssetRefs } from "../composables/chatAssetRefs";

describe("parseChatAssetRefs", () => {
  it("keeps Unity asset paths with spaces intact", () => {
    const segments = parseChatAssetRefs(
      "Audio: @Assets/Space Shooter/GameRes/Audio/sound_weapon_player.wav",
    );

    expect(segments).toEqual([
      { type: "text", value: "Audio: " },
      { type: "asset", value: "Assets/Space Shooter/GameRes/Audio/sound_weapon_player.wav" },
    ]);
  });

  it("keeps font file names with spaces intact", () => {
    const segments = parseChatAssetRefs(
      "Font: @Assets/Font Awesome 6 Free-Solid-900.otf",
    );

    expect(segments).toEqual([
      { type: "text", value: "Font: " },
      { type: "asset", value: "Assets/Font Awesome 6 Free-Solid-900.otf" },
    ]);
  });

  it("keeps braced asset refs with spaces intact", () => {
    const segments = parseChatAssetRefs(
      "Audio: {@Assets/Space Shooter/GameRes/Audio/sound weapon player.wav} 继续处理",
    );

    expect(segments).toEqual([
      { type: "text", value: "Audio: " },
      { type: "asset", value: "Assets/Space Shooter/GameRes/Audio/sound weapon player.wav" },
      { type: "text", value: " 继续处理" },
    ]);
  });

  it("keeps backticked asset refs with spaces intact", () => {
    const segments = parseChatAssetRefs(
      "Audio: `Assets/Space Shooter/GameRes/Audio/sound weapon player.wav` 继续处理",
    );

    expect(segments).toEqual([
      { type: "text", value: "Audio: " },
      { type: "asset", value: "Assets/Space Shooter/GameRes/Audio/sound weapon player.wav" },
      { type: "text", value: " 继续处理" },
    ]);
  });

  it("keeps braced ProjectSettings refs intact", () => {
    const segments = parseChatAssetRefs(
      "Settings: {@ProjectSettings/Tag Manager.asset}",
    );

    expect(segments).toEqual([
      { type: "text", value: "Settings: " },
      { type: "asset", value: "ProjectSettings/Tag Manager.asset" },
    ]);
  });

  it("keeps backticked ProjectSettings refs intact", () => {
    const segments = parseChatAssetRefs(
      "Settings: `ProjectSettings/Tag Manager.asset`",
    );

    expect(segments).toEqual([
      { type: "text", value: "Settings: " },
      { type: "asset", value: "ProjectSettings/Tag Manager.asset" },
    ]);
  });

  it("keeps scene object refs with spaces intact", () => {
    const segments = parseChatAssetRefs(
      "Object: @Assets/Scenes/Main Menu.unity/Canvas Root/Start Button",
    );

    expect(segments).toEqual([
      { type: "text", value: "Object: " },
      { type: "asset", value: "Assets/Scenes/Main Menu.unity/Canvas Root/Start Button" },
    ]);
  });

  it("keeps braced scene object refs with spaces intact", () => {
    const segments = parseChatAssetRefs(
      "Object: {@Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)} 继续处理",
    );

    expect(segments).toEqual([
      { type: "text", value: "Object: " },
      { type: "asset", value: "Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)" },
      { type: "text", value: " 继续处理" },
    ]);
  });

  it("keeps backticked scene object refs with spaces intact", () => {
    const segments = parseChatAssetRefs(
      "Object: `Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)` 继续处理",
    );

    expect(segments).toEqual([
      { type: "text", value: "Object: " },
      { type: "asset", value: "Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)" },
      { type: "text", value: " 继续处理" },
    ]);
  });

  it("falls back to simple extensionless asset mentions", () => {
    const segments = parseChatAssetRefs("Folder: @Assets/AmplifyShaderEditor/");

    expect(segments).toEqual([
      { type: "text", value: "Folder: " },
      { type: "asset", value: "Assets/AmplifyShaderEditor" },
    ]);
  });

  it("parses project knowledge refs", () => {
    const segments = parseChatAssetRefs(
      "Knowledge: @design/combat/core-loop.md and `reference/unity/api.md`",
    );

    expect(segments).toEqual([
      { type: "text", value: "Knowledge: " },
      { type: "knowledge", value: "design/combat/core-loop.md" },
      { type: "text", value: " and " },
      { type: "knowledge", value: "reference/unity/api.md" },
    ]);
  });
});

describe("extractChatAssetRefs", () => {
  it("removes inline Unity refs from text and returns extracted refs", () => {
    expect(extractChatAssetRefs("检查 `Assets/Prefabs/Player.prefab` 和 @Packages/com.game/tool.asmdef")).toEqual({
      text: "检查  和 ",
      refs: [
        "Assets/Prefabs/Player.prefab",
        "Packages/com.game/tool.asmdef",
      ],
    });
  });

  it("extracts folder refs selected from mention browse", () => {
    expect(extractChatAssetRefs("参考 `Assets/AmplifyShaderEditor/` 处理")).toEqual({
      text: "参考  处理",
      refs: ["Assets/AmplifyShaderEditor"],
    });
  });

  it("extracts project knowledge refs", () => {
    expect(extractChatAssetRefs("参考 {@memory/project/background.md} 和 @design/core-loop.md")).toEqual({
      text: "参考  和 ",
      refs: [
        "memory/project/background.md",
        "design/core-loop.md",
      ],
    });
  });
});
