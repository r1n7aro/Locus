import { describe, expect, it } from "vitest";
import {
  walkHtmlText,
  injectAssetRefs,
  injectFileRefs,
  injectWorkspaceMentions,
} from "../composables/markdownInject";

describe("walkHtmlText", () => {
  it("transforms plain text", () => {
    expect(walkHtmlText("hello world", (t) => t.toUpperCase())).toBe("HELLO WORLD");
  });

  it("skips text inside <code> tags", () => {
    const html = "before <code>inside</code> after";
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      "BEFORE <code>inside</code> AFTER",
    );
  });

  it("skips text inside <pre> tags", () => {
    const html = "before <pre>inside code</pre> after";
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      "BEFORE <pre>inside code</pre> AFTER",
    );
  });

  it("skips text inside <a> tags", () => {
    const html = 'click <a href="#">link text</a> here';
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      'CLICK <a href="#">link text</a> HERE',
    );
  });

  it("handles nested code inside pre", () => {
    const html = "text <pre><code>code</code></pre> more";
    expect(walkHtmlText(html, (t) => t.toUpperCase())).toBe(
      "TEXT <pre><code>code</code></pre> MORE",
    );
  });
});

describe("injectAssetRefs", () => {
  it("converts @Assets/... references to unity asset refs", () => {
    const html = "See @Assets/Prefabs/Player.prefab for details";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain("ui-select-text");
    expect(result).toContain('data-file-path="Assets/Prefabs/Player.prefab"');
    expect(result).toContain('data-asset-path="Assets/Prefabs/Player.prefab"');
    expect(result).toContain('data-asset-kind="prefab"');
    expect(result).toContain('title="Assets/Prefabs/Player.prefab"');
    expect(result).toContain("md-unity-asset-icon--prefab");
    expect(result).toContain('src="/unity-asset-icons/prefab.svg"');
    expect(result).toContain("Player.prefab");
  });

  it("converts quoted asset paths without keeping wrapper quotes", () => {
    const html = "Check 'Assets/WIP/Materials/RedCube_Mat.mat '";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/WIP/Materials/RedCube_Mat.mat"');
    expect(result).toContain('data-asset-kind="material"');
    expect(result).toContain("md-unity-asset-icon--material");
    expect(result).toContain('src="/unity-asset-icons/material.svg"');
    expect(result).not.toContain("'Assets/WIP");
  });

  it("converts quoted asset paths with spaces", () => {
    const html = "Config: 'Assets/Data/Enemy Configs/Elite Guard.asset'";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Data/Enemy Configs/Elite Guard.asset"');
    expect(result).toContain("Elite Guard.asset");
  });

  it("converts braced Unity object refs and strips fileID suffixes", () => {
    const html = "  Enemy Config: {Assets/Data/Enemy Configs/Elite Guard.asset#fileID:11400000}";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Data/Enemy Configs/Elite Guard.asset"');
    expect(result).not.toContain('data-file-path="Assets/Data/Enemy Configs/Elite Guard.asset#fileID');
    expect(result).toContain("Elite Guard.asset");
  });

  it("converts braced Unity asset refs with spaces", () => {
    const html = "音频 {@Assets/Space Shooter/GameRes/Audio/sound weapon player.wav} 需要检查";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Space Shooter/GameRes/Audio/sound weapon player.wav"');
    expect(result).toContain("sound weapon player.wav");
    expect(result).not.toContain("{@Assets/Space Shooter");
    expect(result).toContain(" 需要检查");
  });

  it("converts parenthesized Unity asset refs from object labels", () => {
    const html = "Enemy Config (Assets/Data/Enemy Configs/Elite Guard.asset)";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Data/Enemy Configs/Elite Guard.asset"');
  });

  it("assigns Unity-style asset icon kinds by extension", () => {
    const html = [
      "@Assets/Scenes/Main.unity",
      "@Assets/Materials/Ground.mat",
      "@Assets/Scripts/Player.cs",
      "@Assets/Textures/Icon.png",
    ].join(" ");
    const result = injectAssetRefs(html);
    expect(result).toContain('data-asset-kind="scene"');
    expect(result).toContain('data-asset-kind="material"');
    expect(result).toContain('data-asset-kind="script"');
    expect(result).toContain('data-asset-kind="texture"');
  });

  it("converts @scene/object references to Unity scene object refs", () => {
    const html = "Select @Assets/Scenes/Main.unity/Environment/SpawnPoint";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain('data-file-path="Assets/Scenes/Main.unity/Environment/SpawnPoint"');
    expect(result).toContain('data-scene-path="Assets/Scenes/Main.unity"');
    expect(result).toContain('data-scene-object-path="Environment/SpawnPoint"');
    expect(result).toContain('title="Assets/Scenes/Main.unity/Environment/SpawnPoint"');
    expect(result).toContain('src="/unity-asset-icons/gameobject.svg"');
    expect(result).toContain("SpawnPoint");
  });

  it("converts quoted scene/object references with spaces", () => {
    const html = "'Assets/Scenes/Main Menu.unity/Canvas Root/Start Button'";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain('data-scene-path="Assets/Scenes/Main Menu.unity"');
    expect(result).toContain('data-scene-object-path="Canvas Root/Start Button"');
    expect(result).toContain("Start Button");
  });

  it("converts braced scene/object references with spaces", () => {
    const html = "调整 {@Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)} 的阴影";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain('data-scene-path="Assets/Scenes/Main Menu.unity"');
    expect(result).toContain('data-scene-object-path="Canvas Root/Spot Light (2)"');
    expect(result).toContain("Spot Light (2)");
    expect(result).not.toContain("{@Assets/Scenes");
    expect(result).toContain(" 的阴影");
  });

  it("keeps unquoted scene object names with spaces and separators intact", () => {
    const html = "最高的是 @Assets/Scenes/World.unity/Trees/Tree(Polybrush | Clone)，位置约为 (47.79, 8.20, 6.84)。";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain('data-scene-path="Assets/Scenes/World.unity"');
    expect(result).toContain('data-scene-object-path="Trees/Tree(Polybrush | Clone)"');
    expect(result).toContain("Tree(Polybrush | Clone)");
    expect(result).toContain("，位置约为");
  });

  it("treats extensionless asset paths as folder refs", () => {
    const html = "<code>Assets/Prefabs/Characters</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Prefabs/Characters"');
    expect(result).toContain('data-asset-kind="folder"');
    expect(result).toContain('src="/unity-asset-icons/folder.svg"');
    expect(result).toContain("Characters");
  });

  it("trims trailing slash when rendering folder asset refs", () => {
    const html = "'Assets/Prefabs/Characters/'";
    const result = injectAssetRefs(html);
    expect(result).toContain('data-file-path="Assets/Prefabs/Characters"');
    expect(result).toContain('data-asset-kind="folder"');
    expect(result).toContain(">Characters</span>");
  });

  it("converts asset paths inside inline code", () => {
    const html = "<code>@Assets/Prefabs/Player.prefab</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Prefabs/Player.prefab"');
    expect(result).not.toContain("<code>");
  });

  it("converts the assistant inline-code asset path form", () => {
    const html = "找到了：主角预制件是 <code>Assets/Prefabs/Characters/PigChef.prefab</code>。";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Prefabs/Characters/PigChef.prefab"');
  });

  it("converts legacy braced asset paths inside inline code", () => {
    const html = "当前场景 <code>{@Assets/Assets/Scenes/EventScene/E0002/E0002.unity}</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Assets/Scenes/EventScene/E0002/E0002.unity"');
    expect(result).not.toContain("<code>");
    expect(result).not.toContain("{@Assets/Assets/Scenes");
  });

  it("converts scene/object references inside inline code", () => {
    const html = "<code>Assets/Scenes/Main.unity/UI/HUD</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain('data-scene-path="Assets/Scenes/Main.unity"');
    expect(result).toContain('data-scene-object-path="UI/HUD"');
  });

  it("converts scene/object references with spaces inside inline code", () => {
    const html = "<code>Assets/Scenes/Main Menu.unity/Canvas Root/Spot Light (2)</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain('data-scene-path="Assets/Scenes/Main Menu.unity"');
    expect(result).toContain('data-scene-object-path="Canvas Root/Spot Light (2)"');
  });

  it("converts ProjectSettings paths inside inline code", () => {
    const html = "<code>ProjectSettings/Tag Manager.asset</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-workspace-ref");
    expect(result).toContain('data-workspace-path="ProjectSettings/Tag Manager.asset"');
    expect(result).not.toContain("<code>");
  });

  it("does not convert asset paths inside fenced code blocks", () => {
    const html = "<pre><code>@Assets/Prefabs/Player.prefab</code></pre>";
    const result = injectAssetRefs(html);
    expect(result).not.toContain("md-unity-asset-ref");
    expect(result).toContain("<pre><code>@Assets/Prefabs/Player.prefab</code></pre>");
  });

  it("converts workspace file paths inside inline code", () => {
    const html = "<code>src/main.ts</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-workspace-path="src/main.ts"');
    expect(result).not.toContain("<code>");
  });

  it("does not convert generic workspace mentions", () => {
    const html = "See @UIElementsSchema/UnityEditor.Overlays.xsd";
    const result = injectAssetRefs(html);
    expect(result).not.toContain("md-unity-asset-ref");
  });
});

describe("injectWorkspaceMentions", () => {
  it("converts generic workspace file mentions", () => {
    const html = "Inspect @UIElementsSchema/UnityEditor.Overlays.xsd";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-workspace-ref");
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-workspace-path="UIElementsSchema/UnityEditor.Overlays.xsd"');
    expect(result).toContain('data-entry-kind="file"');
    expect(result).toContain('title="UIElementsSchema/UnityEditor.Overlays.xsd"');
    expect(result).toContain("@</span>UnityEditor.Overlays.xsd");
  });

  it("converts folder mentions with a trailing slash", () => {
    const html = "Inspect @UIElementsSchema/";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-workspace-path="UIElementsSchema"');
    expect(result).toContain('data-entry-kind="folder"');
    expect(result).toContain('src="/unity-asset-icons/folder.svg"');
    expect(result).toContain("@</span>UIElementsSchema/");
  });

  it("does not override asset-root mentions", () => {
    const html = "Inspect @Assets/Prefabs/Player.prefab";
    const assetRefs = injectAssetRefs(html);
    const result = injectWorkspaceMentions(assetRefs);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).not.toContain("md-workspace-ref");
  });

  it("keeps asset-root folder mentions interactive", () => {
    const html = "Inspect @Assets/Scripts/";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-workspace-path="Assets/Scripts"');
  });

  it("converts braced workspace mentions with spaces", () => {
    const html = "Inspect {@UI Elements Schema/Unity Editor Overlays.xsd} now";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-workspace-ref");
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-workspace-path="UI Elements Schema/Unity Editor Overlays.xsd"');
    expect(result).toContain("@</span>Unity Editor Overlays.xsd");
    expect(result).not.toContain("{@UI Elements Schema");
    expect(result).toContain(" now");
  });

  it("converts braced ProjectSettings mentions with spaces", () => {
    const html = "Inspect {@ProjectSettings/Tag Manager.asset}";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-workspace-ref");
    expect(result).toContain('data-workspace-path="ProjectSettings/Tag Manager.asset"');
    expect(result).toContain("@</span>Tag Manager.asset");
  });
});

describe("injectFileRefs", () => {
  it("converts src/ relative paths to file refs", () => {
    const html = "Modified src/components/ChatView.vue to fix the bug";
    const result = injectFileRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain("ui-select-text");
    expect(result).toContain('data-file-path="src/components/ChatView.vue"');
    expect(result).toContain('title="src/components/ChatView.vue"');
    expect(result).toContain("ChatView.vue");
  });

  it("converts Assets/ paths to file refs", () => {
    const html = "Check Assets/Scripts/Player.cs for logic";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="Assets/Scripts/Player.cs"');
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain("Player.cs");
  });

  it("converts bare scene/object paths to scene object refs", () => {
    const html = "Select Assets/Scenes/Main.unity/Environment/SpawnPoint";
    const result = injectFileRefs(html);
    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain('data-scene-path="Assets/Scenes/Main.unity"');
    expect(result).toContain('data-scene-object-path="Environment/SpawnPoint"');
  });

  it("converts src-tauri/ paths", () => {
    const html = "See src-tauri/src/commands/workspace.rs";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="src-tauri/src/commands/workspace.rs"');
  });

  it("converts generic dir/file.ext paths", () => {
    const html = "Update utils/helpers.ts";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="utils/helpers.ts"');
  });

  it("converts bare Unity asset file refs with spaces", () => {
    const html = "Uses Assets/Data/Enemy Configs/Elite Guard.asset in the scene";
    const result = injectFileRefs(html);
    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/Data/Enemy Configs/Elite Guard.asset"');
  });

  it("handles :line suffix", () => {
    const html = "Error at src/main.ts:42";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="src/main.ts"');
    expect(result).toContain('data-file-line="42"');
    expect(result).toContain("main.ts:42");
  });

  it("keeps line suffixes on Unity asset file refs", () => {
    const html = "Error at Assets/Scripts/Player.cs:42";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="Assets/Scripts/Player.cs"');
    expect(result).toContain('data-file-line="42"');
    expect(result).toContain("Player.cs:42");
  });

  it("handles #Lline suffix", () => {
    const html = "See src/main.ts#L120";
    const result = injectFileRefs(html);
    expect(result).toContain('data-file-path="src/main.ts"');
    expect(result).toContain('data-file-line="120"');
    expect(result).toContain("main.ts:120");
  });

  it("does not match inside code blocks", () => {
    const html = "<pre><code>src/main.ts</code></pre>";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("does not match inside inline code", () => {
    const html = "<code>src/main.ts</code>";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("does not match inside anchor tags", () => {
    const html = '<a href="#">src/main.ts</a>';
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("does not double-process @Assets/ paths", () => {
    // After injectAssetRefs runs first, the @Assets path becomes a unity asset ref.
    // injectFileRefs should not double-process it.
    const assetRefs = injectAssetRefs("See @Assets/Prefabs/Player.prefab");
    const result = injectFileRefs(assetRefs);
    const matches = result.match(/md-file-ref/g);
    expect(result).toContain("md-unity-asset-ref");
    expect(matches).toHaveLength(1);
  });

  it("does not double-process workspace mentions", () => {
    const mentioned = injectWorkspaceMentions("See @UIElementsSchema/UnityEditor.Overlays.xsd");
    const result = injectFileRefs(mentioned);
    const matches = result.match(/md-file-ref/g);
    expect(matches).toHaveLength(1);
  });

  it("does not match URLs", () => {
    const html = "Visit https://example.com/path/to/file.html for docs";
    const result = injectFileRefs(html);
    // The URL should not produce a file ref for path/to/file.html
    expect(result).not.toContain("md-file-ref");
  });

  it("does not match paths without slashes", () => {
    const html = "Run main.ts to start";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
  });

  it("handles multiple file refs in one text", () => {
    const html = "Changed src/a.ts and src/b.ts";
    const result = injectFileRefs(html);
    const matches = result.match(/md-file-ref/g);
    expect(matches).toHaveLength(2);
  });
});
