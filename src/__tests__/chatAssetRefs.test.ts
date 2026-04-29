import { describe, expect, it } from "vitest";
import { parseChatAssetRefs } from "../composables/chatAssetRefs";

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
});
