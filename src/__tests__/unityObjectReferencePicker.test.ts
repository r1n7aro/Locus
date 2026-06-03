import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  filterUnityObjectReferenceSearchResults,
  getUnityObjectReferenceTypeRule,
  isUnityObjectReferenceSearchResult,
  unityObjectReferenceValueForSearchResult,
  unityObjectReferenceSearchQuery,
  UNITY_OBJECT_REFERENCE_SEARCH_ROOTS,
} from "../services/unityObjectReferencePicker";
import type { AssetSearchResult } from "../types";

function asset(path: string, kind: string, typeLabel = "", overrides: Partial<AssetSearchResult> = {}): AssetSearchResult {
  const name = overrides.name ?? path.split("/").pop() ?? path;
  return {
    path,
    name,
    kind,
    typeLabel,
    isSubAsset: false,
    root: path.startsWith("Packages/") ? "packages" : "assets",
    matchScore: 10,
    source: "assetDb",
    ...overrides,
  };
}

describe("unityObjectReferencePicker", () => {
  it("builds AssetDatabase queries from reference types", () => {
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.Audio.AudioMixerGroup",
    })).toBe("t:audiomixergroup");
    expect(unityObjectReferenceSearchQuery("Default", {
      referenceTypeFullName: "UnityEngine.Audio.AudioMixerGroup",
    })).toBe("t:audiomixergroup Default");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.Audio.AudioMixerSnapshot",
    })).toBe("t:audiomixersnapshot");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.Audio.AudioMixer",
    })).toBe("mixer");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.RuntimeAnimatorController",
    })).toBe("t:controller");
    expect(unityObjectReferenceSearchQuery("Plan", {
      referenceTypeFullName: "UnityEngine.RuntimeAnimatorController",
    })).toBe("t:controller Plan");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.Video.VideoClip",
    })).toBe("t:otherYaml");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.RenderTexture",
    })).toBe("t:rendertexture");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.PhysicMaterial",
    })).toBe("t:physicmaterial");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.Camera",
    })).toBe("t:prefab");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.ParticleSystem",
    })).toBe("t:prefab");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.ParticleSystemForceField",
    })).toBe("t:prefab");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.UIElements.VisualTreeAsset",
    })).toBe("t:visualtreeasset");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.UIElements.ThemeStyleSheet",
    })).toBe("t:themestylesheet");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.UIElements.PanelSettings",
    })).toBe("t:PanelSettings");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.TextCore.Text.FontAsset",
    })).toBe("t:FontAsset");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "",
    })).toBe("t:scene|prefab|material|animation|controller|genericAsset|script|texture|audio|shader|model|otherYaml");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEditor.MonoScript",
    })).toBe("t:script");
    expect(unityObjectReferenceSearchQuery("Player", {
      referenceTypeFullName: "Game.PlayerComponent",
    })).toBe("t:prefab component:PlayerComponent Player");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "UnityEngine.Component",
    })).toBe("t:prefab");
    expect(unityObjectReferenceSearchQuery("", {
      referenceTypeFullName: "Game.NPCMovementEventChannelSO",
    })).toBe("t:NPCMovementEventChannelSO");
    expect(unityObjectReferenceSearchQuery("Slime", {
      referenceTypeFullName: "Game.NPCMovementEventChannelSO",
    })).toBe("t:NPCMovementEventChannelSO Slime");
  });

  it("filters common Unity asset reference targets by type", () => {
    const results = [
      asset("Assets/Audio/Main.wav", "audio"),
      asset("Assets/Audio/Main.asset", "genericAsset", "AudioCueSO"),
      asset("Assets/Settings/Audio/DefaultAudioMixer.mixer", "otherYaml"),
      asset("Assets/Settings/Audio/DefaultAudioMixer.mixer", "otherYaml", "AudioMixerGroupController", {
        name: "Music",
        isSubAsset: true,
        fileId: -2919845427630868010,
        objectKey: "mixer:music",
      }),
      asset("Assets/Settings/Audio/DefaultAudioMixer.mixer", "otherYaml", "AudioMixerSnapshotController", {
        name: "Snapshot",
        isSubAsset: true,
        fileId: -3791635409326915812,
        objectKey: "mixer:snapshot",
      }),
      asset("Assets/Prefabs/Player.prefab", "prefab"),
      asset("Assets/Scripts/Player.cs", "script"),
    ];

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.AudioClip",
    }).map((result) => result.path)).toEqual(["Assets/Audio/Main.wav"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.Audio.AudioMixerGroup",
    }).map(unityObjectReferenceValueForSearchResult)).toEqual(["Assets/Settings/Audio/DefaultAudioMixer.mixer/Music"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.Audio.AudioMixerSnapshot",
    }).map(unityObjectReferenceValueForSearchResult)).toEqual(["Assets/Settings/Audio/DefaultAudioMixer.mixer/Snapshot"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.Audio.AudioMixer",
    }).map((result) => result.path)).toEqual(["Assets/Settings/Audio/DefaultAudioMixer.mixer"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEditor.MonoScript",
    }).map((result) => result.path)).toEqual(["Assets/Scripts/Player.cs"]);
  });

  it("keeps animator controller references constrained to controller assets", () => {
    const results = [
      asset("Assets/Art/Characters/PlantCritter/Animation/PlantCritter.controller", "animatorController"),
      asset("Assets/Art/Characters/PlantCritter/Animation/PlantCritterOverride.overrideController", "animatorController"),
      asset("Assets/Art/Characters/PlantCritter/Animation/PlantCritter.fbx", "model"),
      asset("Assets/Art/Characters/PlantCritter/Animation/PlantCritter_Attack.fbx", "model", "AnimationClip", {
        name: "PlantCritter_Attack",
        isSubAsset: true,
      }),
    ];

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.RuntimeAnimatorController",
    }).map((result) => result.path)).toEqual([
      "Assets/Art/Characters/PlantCritter/Animation/PlantCritter.controller",
      "Assets/Art/Characters/PlantCritter/Animation/PlantCritterOverride.overrideController",
    ]);
  });

  it("keeps common built-in asset references constrained by asset type", () => {
    const results = [
      asset("Assets/Video/Intro.mp4", "otherYaml"),
      asset("Assets/Video/Intro.mat", "material"),
      asset("Assets/Render/Main.renderTexture", "otherYaml"),
      asset("Assets/Render/Main.png", "texture"),
      asset("Assets/Physics/Bouncy.physicMaterial", "otherYaml"),
      asset("Assets/Physics/Bouncy.mat", "material"),
      asset("Assets/UI/Hud.uxml", "otherYaml"),
      asset("Assets/UI/Hud.uss", "otherYaml"),
      asset("Assets/UI/DefaultTheme.tss", "otherYaml"),
      asset("Assets/UI/PanelSettings.asset", "genericAsset", "PanelSettings", {
        typeSearch: "panelsettings scriptableobject",
      }),
      asset("Assets/Fonts/MainFont.asset", "genericAsset", "FontAsset", {
        typeSearch: "fontasset textasset scriptableobject",
      }),
      asset("Assets/Data/GameConfig.asset", "genericAsset", "GameConfigSO", {
        typeSearch: "gameconfigso scriptableobject",
      }),
    ];

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.Video.VideoClip",
    }).map((result) => result.path)).toEqual(["Assets/Video/Intro.mp4"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.RenderTexture",
    }).map((result) => result.path)).toEqual(["Assets/Render/Main.renderTexture"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.PhysicMaterial",
    }).map((result) => result.path)).toEqual(["Assets/Physics/Bouncy.physicMaterial"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.UIElements.VisualTreeAsset",
    }).map((result) => result.path)).toEqual(["Assets/UI/Hud.uxml"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.UIElements.ThemeStyleSheet",
    }).map((result) => result.path)).toEqual([
      "Assets/UI/DefaultTheme.tss",
      "Assets/UI/PanelSettings.asset",
      "Assets/Fonts/MainFont.asset",
      "Assets/Data/GameConfig.asset",
    ]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.UIElements.PanelSettings",
    }).map((result) => result.path)).toEqual(["Assets/UI/PanelSettings.asset"]);

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "UnityEngine.TextCore.Text.FontAsset",
    }).map((result) => result.path)).toEqual(["Assets/Fonts/MainFont.asset"]);
  });

  it("keeps importer subassets by object type and formats user-facing references", () => {
    const sprite = asset("Assets/UI/Atlas.png", "texture", "Sprite", {
      name: "PrimaryButton",
      isSubAsset: true,
      fileId: 21300000,
      objectKey: "atlas:21300000",
    });
    const texture = asset("Assets/UI/Atlas.png", "texture");
    const mesh = asset("Assets/Models/Enemy.fbx", "model", "Mesh", {
      name: "EnemyBody",
      isSubAsset: true,
    });
    const clip = asset("Assets/Models/Enemy.fbx", "model", "AnimationClip", {
      name: "Attack",
      isSubAsset: true,
    });

    expect(filterUnityObjectReferenceSearchResults([texture, sprite], {
      referenceTypeFullName: "UnityEngine.Sprite",
    })).toEqual([sprite]);
    expect(filterUnityObjectReferenceSearchResults([mesh, clip], {
      referenceTypeFullName: "UnityEngine.Mesh",
    })).toEqual([mesh]);
    expect(filterUnityObjectReferenceSearchResults([mesh, clip], {
      referenceTypeFullName: "UnityEngine.AnimationClip",
    })).toEqual([clip]);
    expect(unityObjectReferenceValueForSearchResult(sprite)).toBe("Assets/UI/Atlas.png/PrimaryButton");
  });

  it("keeps scriptable object subclasses constrained to matching generic assets", () => {
    const results = [
      asset("Assets/Data/BasicAttack.asset", "genericAsset", "AttackConfigSO"),
      asset("Assets/Data/Music.asset", "genericAsset", "AudioCueSO"),
      asset("Assets/Prefabs/BasicAttack.prefab", "prefab"),
    ];

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "Game.AttackConfigSO",
    }).map((result) => result.path)).toEqual(["Assets/Data/BasicAttack.asset"]);
  });

  it("keeps scriptable object subclasses matched by indexed base type terms", () => {
    const results = [
      asset("Assets/Data/SlimeMove.asset", "genericAsset", "SlimeMoveChannel", {
        typeSearch: "slimemovechannel baseeventchannelso scriptableobject",
      }),
      asset("Assets/Data/Music.asset", "genericAsset", "AudioCueSO", {
        typeSearch: "audiocueso scriptableobject",
      }),
    ];

    expect(filterUnityObjectReferenceSearchResults(results, {
      referenceTypeFullName: "Game.BaseEventChannelSO",
    }).map((result) => result.path)).toEqual(["Assets/Data/SlimeMove.asset"]);
  });

  it("keeps broad UnityEngine.Object references open", () => {
    const prefab = asset("Assets/Prefabs/Player.prefab", "prefab");
    const material = asset("Assets/Materials/Player.mat", "material");

    expect(isUnityObjectReferenceSearchResult(prefab, {
      referenceTypeFullName: "UnityEngine.Object",
    })).toBe(true);
    expect(isUnityObjectReferenceSearchResult(material, {
      referenceTypeFullName: "UnityEngine.Object",
    })).toBe(true);
    expect(getUnityObjectReferenceTypeRule("").broad).toBe(true);
    expect([...UNITY_OBJECT_REFERENCE_SEARCH_ROOTS]).toEqual(["Assets", "Packages", "ProjectSettings"]);
  });

  it("wires the picker into Locus and View object reference editors", () => {
    const field = readFileSync("src/components/unity/UnityObjectReferenceField.vue", "utf8");
    const editor = readFileSync("src/components/unity/UnityPropertyEditor.vue", "utf8");
    const viewRuntime = readFileSync("src/components/view/viewRuntime.ts", "utf8");
    const serializedTable = readFileSync("src-tauri/src/view/templates/serialized_table.rs", "utf8");
    const fieldBlocks = readFileSync("src-tauri/src/view/templates/field_blocks.rs", "utf8");
    const exportScript = readFileSync("scripts/export-view-runtime-sources.mjs", "utf8");

    expect(field).toContain("searchWorkspaceAssets");
    expect(field).toContain("filterUnityObjectReferenceSearchResults");
    expect(field).toContain("referenceTypeFullName?: string");
    expect(field).toContain("const displayText = ref");
    expect(field).toContain("const searchText = ref(\"\")");
    expect(field).toContain("currentValue: displayText.value");
    expect(field).toContain("unityObjectReferenceSearchQuery(searchText.value");
    expect(field).toContain(":value=\"displayText\"");
    expect(field).toContain(":readonly=\"true\"");
    expect(field).toContain("ref=\"searchInputEl\"");
    expect(field).toContain("<Teleport to=\"body\">");
    expect(field).toContain("ref=\"dropdownEl\"");
    expect(field).toContain("document.addEventListener(\"scroll\", scheduleDropdownPositionUpdate, true)");
    expect(field).toContain("@input=\"updateSearchText\"");
    expect(field).toContain("position: fixed;");
    expect(field).toContain("z-index: 1000;");
    expect(field).toContain("border: 1px solid var(--border-strong);");
    expect(field).toContain("background: var(--surface-elevated, var(--panel-bg));");
    expect(field).toContain(":global(:root[data-theme=\"dark\"]) .unity-object-reference-dropdown");
    expect(field).toContain("color-mix(in srgb, var(--border-color) 72%, transparent)");
    expect(field).not.toContain("@input=\"updateText\"");
    expect(field).not.toContain("@change=\"commitText\"");
    expect(editor).toContain(":reference-type-full-name=\"referenceTypeFullName\"");
    expect(viewRuntime).toContain("objectReferencePicker");
    expect(viewRuntime).toContain("...UnityObjectReferencePickerService");
    expect(serializedTable).toContain(":reference-type-full-name=\"cell.referenceTypeFullName\"");
    expect(serializedTable).toContain("referenceTypeFullName = snapshot.referenceTypeFullName");
    expect(fieldBlocks).toContain(":reference-type-full-name=\"propertyString(field, 'referenceTypeFullName')\"");
    expect(exportScript).toContain("src/services/unityObjectReferencePicker.ts");
  });

  it("infers Unity built-in object reference field types for picker filtering", () => {
    const serializedProperties = readFileSync("locus_unity/Editor/LocusBridge.SerializedProperties.cs", "utf8");
    const builtInFields = readFileSync("locus_unity/Editor/LocusBridge.BuiltInSerializedFields.cs", "utf8");
    const fieldCount = (builtInFields.match(/Field\("/g) ?? []).length;

    expect(serializedProperties).toContain("ResolveBuiltInSerializedPropertyFieldType(prop)");
    expect(fieldCount).toBeGreaterThanOrEqual(150);
    expect(builtInFields).toContain("UnityEngine.MeshRenderer");
    expect(builtInFields).toContain("m_AdditionalVertexStreams");
    expect(builtInFields).toContain("UnityEngine.BoxCollider");
    expect(builtInFields).toContain("UnityEngine.Collider\", \"m_Material\"");
    expect(builtInFields).toContain("UnityEngine.Animator\", \"m_Controller\", \"UnityEngine.RuntimeAnimatorController\"");
    expect(builtInFields).toContain("UnityEngine.Video.VideoPlayer\", \"m_VideoClip\", \"UnityEngine.Video.VideoClip\"");
    expect(builtInFields).toContain("UnityEngine.ParticleSystem\", \"ShapeModule.m_Mesh\", \"UnityEngine.Mesh\"");
    expect(builtInFields).toContain("UnityEngine.ParticleSystem\", \"SubModule.subEmitters[].emitter\", \"UnityEngine.ParticleSystem\"");
    expect(builtInFields).toContain("UnityEngine.BillboardRenderer\", \"m_Billboard\", \"UnityEngine.BillboardAsset\"");
    expect(builtInFields).toContain("UnityEngine.UIElements.UIDocument\", \"m_PanelSettings\", \"UnityEngine.UIElements.PanelSettings\"");
    expect(builtInFields).toContain("UnityEngine.UIElements.PanelSettings\", \"themeUss\"");
    expect(builtInFields).toContain("UnityEngine.TextCore.Text.FontAsset\", \"m_SourceFontFile\"");
    expect(builtInFields).toContain("UnityEngine.Tilemaps.Tile\", \"m_InstancedGameObject\", \"UnityEngine.GameObject\"");
    expect(builtInFields).toContain("UnityEngine.TerrainData\", \"m_TreeDatabase.m_TreePrototypes[].m_Prefab\"");
    expect(builtInFields).toContain("UnityEngine.Rigidbody2D\", \"m_Material\", \"UnityEngine.PhysicsMaterial2D\"");
    expect(builtInFields).toContain("UnityEngine.TerrainLayer\", \"m_DiffuseTexture\", \"UnityEngine.Texture2D\"");
    expect(builtInFields).toContain("NormalizeBuiltInSerializedPropertyPath");
  });
});
