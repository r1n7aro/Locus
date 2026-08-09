import { describe, expect, it } from "vitest";
import { TableProperties } from "lucide";
import {
  collectInlineCodePathCandidates,
  walkHtmlText,
  injectAssetRefs,
  injectFileRefs,
  injectUnityObjectFenceRefs,
  injectUnityPropertyFenceRefs,
  injectViewRefs,
  injectWorkspaceMentions,
  isMarkdownUnityObjectFenceLanguage,
  isMarkdownUnityPropertyFenceLanguage,
} from "../composables/markdownInject";
import {
  groupUnityPropertyFenceItems,
  parseUnityPropertyFence,
  unityPropertyFenceDuplicateObjectLabels,
  unityPropertyFenceObjectLabelKey,
  unityPropertyFenceUnitySelectionTarget,
} from "../composables/unityPropertyFence";
import { prepareMarkdownImages } from "../composables/markdownImages";

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
      "@Assets/Tools/extract_psd.py",
      "@Assets/Data/skill.json",
      "@Assets/Docs/SKILL.md",
      "@Assets/Textures/Icon.png",
    ].join(" ");
    const result = injectAssetRefs(html);
    expect(result).toContain('data-asset-kind="scene"');
    expect(result).toContain('data-asset-kind="material"');
    expect(result).toContain('data-asset-kind="csharp"');
    expect(result).toContain('data-asset-kind="python"');
    expect(result).toContain('data-asset-kind="json"');
    expect(result).toContain('data-asset-kind="markdown"');
    expect(result).toContain('data-asset-kind="texture"');
    expect(result).toContain('md-unity-asset-icon--csharp" src="/unity-asset-icons/script.svg"');
    expect(result).toContain('md-unity-asset-icon--json" src="/unity-asset-icons/text.svg"');
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
    expect(result).toContain("md-unity-object-ref");
    expect(result).toContain('data-md-unity-object-preview="true"');
    expect(result).toContain('data-md-unity-level="inline"');
    expect(result).toContain('data-file-path="Assets/Prefabs/Characters/PigChef.prefab"');
  });

  it("converts configured inline-code Unity refs to full-row preview placeholders", () => {
    const html = "<code>asset:row Assets/Prefabs/Player.prefab</code>";
    const result = injectAssetRefs(html);

    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain("md-unity-object-ref--row");
    expect(result).toContain('data-md-unity-object-preview="true"');
    expect(result).toContain('data-md-unity-level="row"');
    expect(result).toContain('data-md-unity-ref-kind="asset"');
    expect(result).toContain('data-asset-path="Assets/Prefabs/Player.prefab"');
    expect(result).not.toContain("<code>");
  });

  it("converts preview aliases to thumbnail Unity object placeholders", () => {
    const html = "<code>Assets/Models/Hero.fbx | preview</code>";
    const result = injectAssetRefs(html);

    expect(result).toContain("md-unity-object-ref--thumbnail");
    expect(result).toContain('data-md-unity-level="thumbnail"');
    expect(result).toContain('data-asset-path="Assets/Models/Hero.fbx"');
  });

  it("forces editable inline-code Unity refs into row placeholders", () => {
    const html = "<code>asset:editable Assets/Data/Enemy.asset</code>";
    const result = injectAssetRefs(html);

    expect(result).toContain("md-unity-object-ref--row");
    expect(result).toContain('data-md-unity-level="row"');
    expect(result).toContain('data-md-unity-editable="true"');
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
    expect(result).toContain("md-unity-object-ref");
    expect(result).toContain('data-md-unity-ref-kind="sceneObject"');
    expect(result).toContain('data-scene-path="Assets/Scenes/Main.unity"');
    expect(result).toContain('data-scene-object-path="UI/HUD"');
  });

  it("converts configured scene/object references to row placeholders", () => {
    const html = "<code>unity:row Assets/Scenes/Main.unity/UI/HUD</code>";
    const result = injectAssetRefs(html);

    expect(result).toContain("md-unity-scene-object-ref");
    expect(result).toContain("md-unity-object-ref--row");
    expect(result).toContain('data-md-unity-level="row"');
    expect(result).toContain('data-md-unity-ref-kind="sceneObject"');
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

  it("renders existing inline-code workspace roots from path status", () => {
    const html = "<code>Assets</code> <code>Assets/</code> <code>Packages/</code> <code>ProjectSettings/</code>";
    const result = injectAssetRefs(html, {
      requireInlinePathStatus: true,
      inlinePathStatus(path) {
        if (path === "Assets" || path === "Packages" || path === "ProjectSettings") {
          return { path, exists: true, entryKind: "folder" };
        }
        return { path, exists: false };
      },
    });

    expect(result).toContain("md-workspace-ref");
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-workspace-path="Assets"');
    expect(result).toContain('data-workspace-path="Packages"');
    expect(result).toContain('data-workspace-path="ProjectSettings"');
    expect(result).not.toContain('data-asset-path="Assets"');
    expect(result).not.toContain('data-file-path="Assets"');
    expect(result).not.toContain("<code>Assets</code>");
  });

  it("classifies exported assistant inline roots as folders", () => {
    const html = "<li><code>list</code>：看到项目根目录有 <code>.git/</code>, <code>Assets/</code>, <code>Locus/</code>, <code>Packages/</code>, <code>ProjectSettings/</code></li>";
    const result = injectAssetRefs(html, {
      requireInlinePathStatus: true,
      inlinePathStatus(path) {
        if ([".git", "Assets", "Locus", "Packages", "ProjectSettings"].includes(path)) {
          return { path, exists: true, entryKind: "folder" };
        }
        return { path, exists: false };
      },
    });

    for (const path of [".git", "Assets", "Locus", "Packages", "ProjectSettings"]) {
      expect(result).toContain(`data-workspace-path="${path}"`);
    }
    expect(result).toContain('data-entry-kind="folder"');
    expect(result).not.toContain('data-asset-path="Assets"');
    expect(result).not.toContain('data-file-path="Assets"');
    expect(result).not.toContain('data-asset-path="Packages"');
    expect(result).not.toContain('data-file-path="Packages"');
  });

  it("keeps missing inline-code paths as code when status is required", () => {
    const html = "<code>Assets/Missing.prefab</code>";
    const result = injectAssetRefs(html, {
      requireInlinePathStatus: true,
      inlinePathStatus: (path) => ({ path, exists: false }),
    });

    expect(result).toBe(html);
    expect(result).not.toContain("md-unity-asset-ref");
  });

  it("uses path status to classify extensionless inline-code asset files", () => {
    const html = "<code>Assets/LICENSE</code>";
    const result = injectAssetRefs(html, {
      requireInlinePathStatus: true,
      inlinePathStatus: (path) => ({ path, exists: true, entryKind: "file" }),
    });

    expect(result).toContain("md-unity-asset-ref");
    expect(result).toContain('data-file-path="Assets/LICENSE"');
    expect(result).toContain('data-entry-kind="file"');
    expect(result).not.toContain('data-asset-kind="folder"');
  });

  it("renders inline-code Unity folders as folder refs without asset previews", () => {
    const html = "<code>Assets/DevTools/AnimationPipeline</code>";
    const result = injectAssetRefs(html, {
      requireInlinePathStatus: true,
      inlinePathStatus: (path) => ({ path, exists: true, entryKind: "folder" }),
    });

    expect(result).toContain("md-file-ref");
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-file-path="Assets/DevTools/AnimationPipeline"');
    expect(result).toContain('data-entry-kind="folder"');
    expect(result).not.toContain("md-unity-asset-ref");
    expect(result).not.toContain('data-asset-path=');
    expect(result).not.toContain('data-md-unity-object-preview="true"');
  });

  it("collects inline-code path candidates for workspace stat", () => {
    const html = [
      "<code>Assets</code>",
      "<code>Packages/</code>",
      "<code>list</code>",
      "<code>Assets/Scenes/Main.unity/Root/Player</code>",
      "<pre><code>Packages</code></pre>",
    ].join(" ");

    expect(collectInlineCodePathCandidates(html)).toEqual([
      "Assets",
      "Packages",
      "Assets/Scenes/Main.unity",
    ]);
  });

  it("converts absolute local file paths inside inline code", () => {
    const html = "<code>C:\\Users\\ExampleUser\\AppData\\Roaming\\Locus\\temp\\locus-temp-test.txt</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-file-path="C:/Users/ExampleUser/AppData/Roaming/Locus/temp/locus-temp-test.txt"');
    expect(result).toContain('data-entry-kind="file"');
    expect(result).toContain("locus-temp-test.txt");
    expect(result).not.toContain("<code>");
  });

  it("converts absolute local folder paths inside inline code", () => {
    const html = "<code>C:\\Users\\ExampleUser\\AppData\\Roaming\\Locus\\temp\\</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-file-path="C:/Users/ExampleUser/AppData/Roaming/Locus/temp"');
    expect(result).toContain('data-entry-kind="folder"');
    expect(result).toContain("temp");
  });

  it("converts POSIX absolute paths inside inline code", () => {
    const html = "<code>/tmp/locus-temp-test.txt</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-file-path="/tmp/locus-temp-test.txt"');
    expect(result).toContain('data-entry-kind="file"');
    expect(result).toContain("locus-temp-test.txt");
  });

  it("keeps slash commands inside inline code out of file refs", () => {
    const html = "<code>/psd-to-ugui &lt;psd-path&gt; [output-folder]</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-command-ref");
    expect(result).toContain('data-command-trigger="/psd-to-ugui"');
    expect(result).toContain("/psd-to-ugui &lt;psd-path&gt; [output-folder]");
    expect(result).not.toContain("md-file-ref");
    expect(result).not.toContain("data-file-path");
  });

  it("converts knowledge paths inside inline code to knowledge refs", () => {
    const html = "<code>skill/com.locus.psd-to-ugui/SKILL.md</code>";
    const result = injectAssetRefs(html);
    expect(result).toContain("md-knowledge-ref");
    expect(result).toContain('data-knowledge-type="skill"');
    expect(result).toContain('data-knowledge-path="skill/com.locus.psd-to-ugui/SKILL.md"');
    expect(result).toContain("SKILL.md");
    expect(result).not.toContain("<code>");
  });

  it("does not convert generic workspace mentions", () => {
    const html = "See @UIElementsSchema/UnityEditor.Overlays.xsd";
    const result = injectAssetRefs(html);
    expect(result).not.toContain("md-unity-asset-ref");
  });
});

describe("injectUnityObjectFenceRefs", () => {
  it("converts asset preview fenced blocks to Unity object placeholders", () => {
    const html = '<pre><code class="hljs language-asset:preview">Assets/Models/Hero.fbx</code></pre>';
    const result = injectUnityObjectFenceRefs(html);

    expect(result).toContain("md-unity-object-ref--thumbnail");
    expect(result).toContain('data-md-unity-level="thumbnail"');
    expect(result).toContain('data-asset-path="Assets/Models/Hero.fbx"');
    expect(result).not.toContain("<pre>");
  });

  it("converts row fenced blocks with multiple Unity asset paths", () => {
    const html = [
      '<pre><code class="hljs language-asset-row">',
      "Assets/Prefabs/Player.prefab\n",
      "Assets/Prefabs/Enemy.prefab",
      "</code></pre>",
    ].join("");
    const result = injectUnityObjectFenceRefs(html);

    expect(result.match(/data-md-unity-level="row"/g)).toHaveLength(2);
    expect(result).toContain('data-asset-path="Assets/Prefabs/Player.prefab"');
    expect(result).toContain('data-asset-path="Assets/Prefabs/Enemy.prefab"');
  });

  it("converts prefixed plain fenced blocks", () => {
    const html = "<pre><code>asset:preview Assets/Prefabs/Player.prefab</code></pre>";
    const result = injectUnityObjectFenceRefs(html);

    expect(result).toContain("md-unity-object-ref--thumbnail");
    expect(result).toContain('data-asset-path="Assets/Prefabs/Player.prefab"');
  });

  it("keeps plain Unity path fenced blocks as code", () => {
    const html = "<pre><code>Assets/Prefabs/Player.prefab</code></pre>";

    expect(injectUnityObjectFenceRefs(html)).toBe(html);
  });

  it("recognizes Unity object fence languages", () => {
    expect(isMarkdownUnityObjectFenceLanguage("asset:preview")).toBe(true);
    expect(isMarkdownUnityObjectFenceLanguage("unity-row")).toBe(true);
    expect(isMarkdownUnityObjectFenceLanguage("json")).toBe(false);
  });
});

describe("injectUnityPropertyFenceRefs", () => {
  it("converts unity_property fenced blocks to property editor hosts", () => {
    const html = [
      '<pre><code class="hljs language-unity_property">',
      "Assets/Data/Config.asset#m_Name",
      "</code></pre>",
    ].join("");
    const result = injectUnityPropertyFenceRefs(html);

    expect(result).toContain('data-md-unity-property-fence="true"');
    expect(result).toContain('data-md-unity-property-source="true"');
    expect(result).toContain("Assets/Data/Config.asset#m_Name");
    expect(result).not.toContain("<pre><code class");
  });

  it("recognizes Unity property fence languages", () => {
    expect(isMarkdownUnityPropertyFenceLanguage("unity_property")).toBe(true);
    expect(isMarkdownUnityPropertyFenceLanguage("unity-property")).toBe(true);
    expect(isMarkdownUnityPropertyFenceLanguage("unity:property")).toBe(true);
    expect(isMarkdownUnityPropertyFenceLanguage("json")).toBe(false);
  });

  it("parses compact asset and component property targets", () => {
    const result = parseUnityPropertyFence([
      "Assets/Data/Config.asset#m_Name",
      "Assets/Scenes/Main.unity/Player#UnityEngine.Transform:m_LocalPosition",
      "Assets/Prefabs/Enemy.prefab/Body#Renderer[1]:m_Enabled",
    ].join("\n"));

    expect(result.issues).toHaveLength(0);
    expect(result.entries[0]?.objectLabel).toBe("Config");
    expect(result.entries[0]?.objectTitle).toBe("Assets/Data/Config.asset");
    expect(result.entries[0]?.target).toEqual({
      kind: "asset",
      path: "Assets/Data/Config.asset",
      propertyPath: "m_Name",
    });
    expect(result.entries[1]?.objectLabel).toBe("Player");
    expect(result.entries[1]?.objectTitle).toBe("Assets/Scenes/Main.unity/Player");
    expect(result.entries[1]?.target).toEqual({
      kind: "component",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Player",
      componentType: "UnityEngine.Transform",
      componentIndex: 0,
      propertyPath: "m_LocalPosition",
    });
    expect(result.entries[2]?.objectLabel).toBe("Body");
    expect(result.entries[2]?.objectTitle).toBe("Assets/Prefabs/Enemy.prefab/Body");
    expect(result.entries[2]?.target).toEqual({
      kind: "component",
      path: "Assets/Prefabs/Enemy.prefab",
      objectPath: "Body",
      componentType: "Renderer",
      componentIndex: 1,
      propertyPath: "m_Enabled",
    });
  });

  it("preserves Unity YAML ordinal paths for scene object property targets", () => {
    const result = parseUnityPropertyFence(
      "Assets/Scenes/Main.unity/Root/Enemy[2]#UnityEngine.Transform:m_LocalPosition",
    );

    expect(result.issues).toHaveLength(0);
    expect(result.entries[0]?.target).toEqual({
      kind: "component",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Root/Enemy[2]",
      componentType: "UnityEngine.Transform",
      componentIndex: 0,
      propertyPath: "m_LocalPosition",
    });
  });

  it("parses JSON property targets", () => {
    const result = parseUnityPropertyFence(JSON.stringify([
      {
        target: {
          kind: "gameObject",
          scenePath: "Assets/Scenes/Main.unity",
          objectPath: "Player",
          objectFileId: 100200300,
          targetFileId: 100200300,
          propertyPath: "m_Name",
        },
      },
    ]));

    expect(result.issues).toHaveLength(0);
    expect(result.entries[0]?.objectLabel).toBe("Player");
    expect(result.entries[0]?.objectTitle).toBe("Assets/Scenes/Main.unity/Player");
    expect(result.entries[0]?.target).toEqual({
      kind: "gameObject",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Player",
      objectFileId: 100200300,
      targetFileId: 100200300,
      propertyPath: "m_Name",
    });
  });

  it("parses explicit component selector targets", () => {
    const result = parseUnityPropertyFence(
      "Assets/Scenes/Main.unity/Player#component:UnityEngine.Transform:m_LocalScale",
    );

    expect(result.issues).toHaveLength(0);
    expect(result.entries[0]?.target).toEqual({
      kind: "component",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Player",
      componentType: "UnityEngine.Transform",
      componentIndex: 0,
      propertyPath: "m_LocalScale",
    });
  });

  it("groups consecutive properties by the same object or component target", () => {
    const result = parseUnityPropertyFence([
      "Assets/Scenes/Main.unity/Player#m_Name",
      "Assets/Scenes/Main.unity/Player#m_IsActive",
      "Assets/Scenes/Main.unity/Player#UnityEngine.Transform:m_LocalPosition",
      "Assets/Scenes/Main.unity/Player#UnityEngine.Transform:m_LocalRotation",
      "Assets/Scenes/Main.unity/Player#UnityEngine.Animator:m_Enabled",
      "Assets/Scenes/Main.unity/Player#UnityEngine.Transform:m_LocalScale",
    ].join("\n"));

    const groups = groupUnityPropertyFenceItems(result.entries, (entry) => entry);

    expect(result.issues).toHaveLength(0);
    expect(groups.map((group) =>
      group.items.map((entry) => entry.target.propertyPath),
    )).toEqual([
      ["m_Name", "m_IsActive"],
      ["m_LocalPosition", "m_LocalRotation"],
      ["m_Enabled"],
      ["m_LocalScale"],
    ]);
  });

  it("detects same object labels that point at different Unity paths", () => {
    const result = parseUnityPropertyFence([
      "Assets/Prefabs/Player.prefab/Player#PlayerPlatformerController:maxMoveSpeed",
      "Assets/Scenes/Main.unity/Player#PlayerPlatformerController:maxMoveSpeed",
      "Assets/Scenes/Main.unity/Enemy#m_Name",
      "Assets/Scenes/Main.unity/Enemy#m_IsActive",
    ].join("\n"));

    const groups = groupUnityPropertyFenceItems(result.entries, (entry) => entry);
    const duplicates = unityPropertyFenceDuplicateObjectLabels(groups.map((group) => group.entry));

    expect(result.issues).toHaveLength(0);
    expect(duplicates.has(unityPropertyFenceObjectLabelKey("Player"))).toBe(true);
    expect(duplicates.has(unityPropertyFenceObjectLabelKey("Enemy"))).toBe(false);
  });

  it("resolves row targets for Unity selection actions", () => {
    expect(unityPropertyFenceUnitySelectionTarget({
      kind: "component",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Player",
      componentType: "UnityEngine.Transform",
      componentIndex: 0,
      propertyPath: "m_LocalPosition",
    })).toEqual({
      kind: "sceneObject",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Player",
    });

    expect(unityPropertyFenceUnitySelectionTarget({
      kind: "asset",
      path: "Assets/Data/Config.asset",
      propertyPath: "m_Name",
    })).toEqual({
      kind: "asset",
      path: "Assets/Data/Config.asset",
    });

    expect(unityPropertyFenceUnitySelectionTarget({
      kind: "component",
      path: "Assets/Prefabs/Enemy.prefab",
      objectPath: "Body",
      componentType: "Renderer",
      componentIndex: 0,
      propertyPath: "m_Enabled",
    })).toEqual({
      kind: "asset",
      path: "Assets/Prefabs/Enemy.prefab",
    });

    expect(unityPropertyFenceUnitySelectionTarget({
      kind: "asset",
      guid: "0123456789abcdef0123456789abcdef",
      propertyPath: "m_Name",
    })).toBeNull();
  });
});

describe("injectWorkspaceMentions", () => {
  it("converts generic workspace file mentions", () => {
    const html = "Inspect @UIElementsSchema/UnityEditor.Overlays.xsd";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-workspace-ref");
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-workspace-path="UIElementsSchema/UnityEditor.Overlays.xsd"');
    expect(result).toContain('draggable="true"');
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

  it("converts knowledge mentions to knowledge refs", () => {
    const html = "Inspect @skill/com.locus.psd-to-ugui/SKILL.md";
    const result = injectWorkspaceMentions(html);
    expect(result).toContain("md-knowledge-ref");
    expect(result).toContain('data-knowledge-type="skill"');
    expect(result).toContain('data-knowledge-path="skill/com.locus.psd-to-ugui/SKILL.md"');
    expect(result).not.toContain("md-workspace-ref");
  });
});

describe("injectViewRefs", () => {
  it("converts standalone view refs to openable View blocks", () => {
    const result = injectViewRefs("<p>view:attack-config-table</p>", {
      openLabel: "Open View",
    });

    expect(result).toContain("md-view-ref-block");
    expect(result).toContain('data-view-id="attack-config-table"');
    expect(result).toContain("md-view-open-button");
    expect(result).toContain("Open View");
    expect(result).not.toContain("<p>view:attack-config-table</p>");
  });

  it("converts standalone inline-code view refs", () => {
    const result = injectViewRefs("<p><code>view:attack-config-table</code></p>");

    expect(result).toContain("md-view-ref-block");
    expect(result).toContain('data-view-id="attack-config-table"');
  });

  it("uses resolved View metadata for the View block icon and run id", () => {
    const result = injectViewRefs("<p>view:{Gameplay/Attack Config Table}</p>", {
      resolveViewRef: () => ({
        id: "attack-config-table",
        icon: TableProperties,
        iconName: "TableProperties",
      }),
    });

    expect(result).toContain('data-view-id="attack-config-table"');
    expect(result).toContain('data-view-ref="Gameplay/Attack Config Table"');
    expect(result).toContain('data-view-icon="TableProperties"');
    expect(result).toContain("md-view-ref-kind-icon");
  });

  it("keeps non-standalone view refs as text", () => {
    const html = "<p>Created view:attack-config-table for editing.</p>";

    expect(injectViewRefs(html)).toBe(html);
  });

  it("does not convert fenced code block content", () => {
    const html = "<pre><code>view:attack-config-table</code></pre>";

    expect(injectViewRefs(html)).toBe(html);
  });
});

describe("injectFileRefs", () => {
  it("converts src/ relative paths to file refs", () => {
    const html = "Modified src/components/ChatView.vue to fix the bug";
    const result = injectFileRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain("ui-select-text");
    expect(result).toContain('data-file-path="src/components/ChatView.vue"');
    expect(result).toContain('draggable="true"');
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

  it("converts bare knowledge document paths to knowledge refs", () => {
    const html = "Created skill/com.locus.psd-to-ugui/SKILL.md for the package";
    const result = injectFileRefs(html);
    expect(result).toContain("md-knowledge-ref");
    expect(result).toContain('data-knowledge-type="skill"');
    expect(result).toContain('data-knowledge-path="skill/com.locus.psd-to-ugui/SKILL.md"');
    expect(result).not.toContain('data-file-path="skill/com.locus.psd-to-ugui/SKILL.md"');
  });

  it("converts bare absolute local file paths", () => {
    const html = "Wrote C:/Users/ExampleUser/AppData/Roaming/Locus/temp/locus-temp-test.txt.";
    const result = injectFileRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-file-path="C:/Users/ExampleUser/AppData/Roaming/Locus/temp/locus-temp-test.txt"');
    expect(result).toContain('data-entry-kind="file"');
    expect(result).toContain("locus-temp-test.txt");
    expect(result).toContain("</span>.");
  });

  it("converts quoted absolute local paths with spaces", () => {
    const html = "Saved 'C:/Users/ExampleUser/AppData/Roaming/Locus/temp/My File.txt'";
    const result = injectFileRefs(html);
    expect(result).toContain("md-file-ref");
    expect(result).toContain('data-file-path="C:/Users/ExampleUser/AppData/Roaming/Locus/temp/My File.txt"');
    expect(result).toContain("My File.txt");
    expect(result).not.toContain("'C:/Users");
  });

  it("converts bare absolute local folder paths", () => {
    const html = "Open C:/Users/ExampleUser/AppData/Roaming/Locus/temp/ when needed";
    const result = injectFileRefs(html);
    expect(result).toContain("md-folder-ref");
    expect(result).toContain('data-file-path="C:/Users/ExampleUser/AppData/Roaming/Locus/temp"');
    expect(result).toContain('data-entry-kind="folder"');
    expect(result).toContain("temp");
  });

  it("does not convert bare POSIX absolute paths", () => {
    const html = "Open /tmp/locus-temp-test.txt when needed";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
    expect(result).toBe(html);
  });

  it("does not convert Chinese slash phrases as POSIX refs", () => {
    const html = "左侧有一个小猪角色和一个绿色怪物/植物，摄像机当前构图比较靠近岩石主体。";
    const result = injectFileRefs(html);
    expect(result).not.toContain("md-file-ref");
    expect(result).toBe(html);
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

describe("prepareMarkdownImages", () => {
  it("converts a standalone local image path to a resolvable image preview", () => {
    const result = prepareMarkdownImages("<p>C:/Users/ExampleUser/AppData/Roaming/Locus/temp/result.png</p>");
    expect(result).toContain("md-image-frame");
    expect(result).toContain("md-image-preview");
    expect(result).toContain('data-md-image-source="C:/Users/ExampleUser/AppData/Roaming/Locus/temp/result.png"');
    expect(result).toContain('data-md-image-state="pending"');
    expect(result).not.toContain("md-file-ref");
  });

  it("converts a bare autolinked network image URL to an image preview", () => {
    const result = prepareMarkdownImages(
      '<p><a href="https://example.com/output/result.webp">https://example.com/output/result.webp</a></p>',
    );
    expect(result).toContain("md-image-preview");
    expect(result).toContain('src="https://example.com/output/result.webp"');
    expect(result).toContain('data-md-image-state="ready"');
    expect(result).not.toContain("<a ");
  });

  it("wraps explicit markdown image tags and keeps local paths for backend resolution", () => {
    const result = prepareMarkdownImages('<p><img src="Assets/Textures/Hero.png" alt="Hero"></p>');
    expect(result).toContain("md-image-frame");
    expect(result).toContain('data-md-image-source="Assets/Textures/Hero.png"');
    expect(result).toContain('alt="Hero"');
    expect(result).not.toContain('src="Assets/Textures/Hero.png"');
  });

  it("keeps non-image paths as text", () => {
    const html = "<p>src/components/ChatView.vue</p>";
    expect(prepareMarkdownImages(html)).toBe(html);
  });

  it("does not convert image paths inside code", () => {
    const html = "<pre><code>C:/temp/result.png</code></pre>";
    expect(prepareMarkdownImages(html)).toBe(html);
  });
});
