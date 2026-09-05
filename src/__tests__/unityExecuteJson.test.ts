import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function read(path: string): string {
  return readFileSync(path, "utf8");
}

describe("unity_execute printJson", () => {
  it("keeps Unity object serialization and uses general JSON for plain objects", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.Types.cs");

    expect(bridge).toContain("EditorJsonUtility.ToJson(uObj, false)");
    expect(bridge).toContain("Locus.Json.LocusJson.Serialize(obj)");
    expect(bridge).not.toContain("JsonUtility.ToJson(obj");
  });

  it("plans one BFS-owned definition for every reference identity", () => {
    const serializer = read("locus_json/LocusJson.cs");

    expect(serializer).toContain("Queue<PendingLocation>");
    expect(serializer).toContain("ReferenceIdentityComparer");
    expect(serializer).toContain("RuntimeHelpers.GetHashCode(value)");
    expect(serializer).toContain("existing.RequiresId = true");
    expect(serializer).toContain('writer.WritePropertyName("$id")');
    expect(serializer).toContain('writer.WritePropertyName("$ref")');
    expect(serializer).not.toContain("MaxDepth");
    expect(serializer).not.toContain("MaxNodes");
  });

  it("reads stored fields without invoking target getters or lazy enumerators", () => {
    const serializer = read("locus_json/LocusJson.cs");
    const bridge = read("locus_unity/Editor/LocusBridge.Types.cs");

    expect(serializer).toContain("Field.GetValue(owner)");
    expect(serializer).toContain("AutoPropertyBackingField(property)");
    expect(serializer).toContain('"<" + property.Name + ">i__Field"');
    expect(serializer).toContain("NodeKind.DeferredEnumerable");
    expect(serializer).toContain('WriteDescriptor(writer, "$deferredEnumerable"');
    expect(serializer).not.toContain("property.GetValue(");
    expect(serializer).not.toContain("JsonConvert.SerializeObject");
    expect(bridge).not.toContain(".AppendLine(obj.ToString())");
  });

  it("documents and exercises anonymous, cyclic, and nested Unity values", () => {
    const definition = JSON.parse(read("tools/unity_execute.json"));
    const driver = read("src-tauri/src/cli_driver.rs");

    expect(definition.description).toContain("serializes stored data");
    expect(definition.description).toContain("without invoking property getters");
    expect(definition.description).toContain("$id/$ref");
    expect(definition.description).toContain("nested Unity references");
    expect(driver).toContain("E3J anonymous JSON");
    expect(driver).toContain("E3J reference loop");
    expect(driver).toContain("E3J BFS ownership");
    expect(driver).toContain("E3J deferred enumerable");
    expect(driver).toContain("E3J nested Unity object");
  });
});
