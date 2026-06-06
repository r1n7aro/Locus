import { describe, expect, it } from "vitest";
import {
  parseUnityPropertyPath,
  unityPropertyObjectTarget,
  unityPropertyTargetWithPath,
} from "../services/unityPropertyPath";

describe("unityPropertyPath", () => {
  it("parses selection property paths", () => {
    expect(parseUnityPropertyPath("selection/property/m_Name")).toEqual({
      kind: "selection",
      propertyPath: "m_Name",
    });
  });

  it("parses asset property paths with slash separated property segments", () => {
    expect(parseUnityPropertyPath("asset/Assets/Data/Config.asset/property/stats/Array/data[0]/value"))
      .toEqual({
        kind: "asset",
        path: "Assets/Data/Config.asset",
        propertyPath: "stats.Array.data[0].value",
      });
  });

  it("parses guid asset property paths", () => {
    expect(parseUnityPropertyPath("guid/0123456789abcdef0123456789abcdef/property/m_Name"))
      .toEqual({
        kind: "asset",
        guid: "0123456789abcdef0123456789abcdef",
        propertyPath: "m_Name",
      });
  });

  it("parses scene object component property paths", () => {
    expect(parseUnityPropertyPath(
      "scene/Assets/Scenes/Main.unity/object/Player/Camera/component/UnityEngine.Transform/0/property/m_LocalPosition",
    )).toEqual({
      kind: "component",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Player/Camera",
      componentType: "UnityEngine.Transform",
      componentIndex: 0,
      propertyPath: "m_LocalPosition",
    });
  });

  it("parses prefab child object properties and can retarget a property path", () => {
    const target = parseUnityPropertyPath(
      "prefab/Assets/Prefabs/Hero.prefab/object/Hero/Weapon/property/m_IsActive",
    );

    expect(unityPropertyObjectTarget(target)).toEqual({
      kind: "gameObject",
      path: "Assets/Prefabs/Hero.prefab",
      objectPath: "Hero/Weapon",
    });
    expect(unityPropertyTargetWithPath(target, "m_Name")).toEqual({
      kind: "gameObject",
      path: "Assets/Prefabs/Hero.prefab",
      objectPath: "Hero/Weapon",
      propertyPath: "m_Name",
    });
  });

  it("parses guid object component property paths", () => {
    const target = parseUnityPropertyPath(
      "guid/0123456789abcdef0123456789abcdef/object/Hero/Weapon/component/Game.Inventory/1/property/items/Array/data[0]",
    );

    expect(target).toEqual({
      kind: "component",
      guid: "0123456789abcdef0123456789abcdef",
      objectPath: "Hero/Weapon",
      componentType: "Game.Inventory",
      componentIndex: 1,
      propertyPath: "items.Array.data[0]",
    });
    expect(unityPropertyObjectTarget(target)).toEqual({
      kind: "component",
      guid: "0123456789abcdef0123456789abcdef",
      objectPath: "Hero/Weapon",
      componentType: "Game.Inventory",
      componentIndex: 1,
    });
  });
});
