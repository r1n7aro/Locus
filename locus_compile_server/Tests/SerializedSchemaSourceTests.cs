using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Xunit;

namespace Locus.CompileServer.Tests;

public class SerializedSchemaSourceTests : IDisposable
{
    private readonly string _tempDir;

    public SerializedSchemaSourceTests()
    {
        _tempDir = Path.Combine(Path.GetTempPath(), "locus-schema-tests-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(_tempDir);
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(_tempDir, recursive: true);
        }
        catch
        {
        }
    }

    private string CompileToDisk(string assemblyName, string source)
    {
        var compilation = CSharpCompilation.Create(
            assemblyName,
            new[] { CSharpSyntaxTree.ParseText(source) },
            ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
                .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
                .Where(File.Exists)
                .Select(p => (MetadataReference)MetadataReference.CreateFromFile(p)),
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
        string path = Path.Combine(_tempDir, assemblyName + ".dll");
        var emit = compilation.Emit(path);
        Assert.True(emit.Success, string.Join("\n", emit.Diagnostics));
        return path;
    }

    private static JsonObject BuildFromPaths(params string[] paths)
    {
        var cache = new ReferenceCache();
        var references = paths
            .Concat(((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
                .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
                .Where(File.Exists))
            .Select(p => cache.GetOrCreate(p))
            .Where(r => r != null)
            .Select(r => r!);
        return SerializedSchemaSource.Build(references);
    }

    [Fact]
    public void Builds_serialized_field_schema_from_metadata()
    {
        string path = CompileToDisk("SchemaSample", @"
using System;
using System.Collections.Generic;

namespace UnityEngine
{
    public class Object { }
    public sealed class SerializeField : Attribute { }
    public sealed class SerializeReference : Attribute { }
    public sealed class TooltipAttribute : Attribute
    {
        public string tooltip;
        public TooltipAttribute(string tooltip) { this.tooltip = tooltip; }
    }
    public sealed class RangeAttribute : Attribute
    {
        public float min;
        public float max;
        public RangeAttribute(float min, float max) { this.min = min; this.max = max; }
    }
    public sealed class TextAreaAttribute : Attribute
    {
        public int minLines;
        public int maxLines;
        public TextAreaAttribute(int minLines, int maxLines) { this.minLines = minLines; this.maxLines = maxLines; }
    }
}

namespace Game
{
    [Serializable]
    public class Inventory
    {
        [UnityEngine.Tooltip(""Shown in UI"")]
        [UnityEngine.Range(1, 9)]
        public int count;

        [UnityEngine.SerializeReference]
        public Node node;

        [UnityEngine.SerializeField]
        private int serializedPrivate;

        private int hidden;

        public List<Item> items;
        public Kind kind;
    }

    [Serializable]
    public class Item
    {
        [UnityEngine.TextArea(2, 5)]
        public string label;
    }

    [Serializable]
    public abstract class Node { }

    [Serializable]
    public class Leaf : Node { }

    public class UnityAsset : UnityEngine.Object { }

    [Flags]
    public enum Kind { A = 1, B = 2 }
}
");

        JsonObject result = BuildFromPaths(path);

        JsonArray types = result["types"]!.AsArray();
        JsonNode inventory = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Inventory")!;
        JsonArray fields = inventory["fields"]!.AsArray();

        JsonNode count = fields.Single(f => f!["name"]!.GetValue<string>() == "count")!;
        Assert.Equal("System.Int32", count["fieldTypeFullName"]!.GetValue<string>());
        Assert.Equal("Shown in UI", count["tooltip"]!.GetValue<string>());
        Assert.True(count["hasRange"]!.GetValue<bool>());
        Assert.Equal(1d, count["rangeMin"]!.GetValue<double>());
        Assert.Equal(9d, count["rangeMax"]!.GetValue<double>());

        JsonNode node = fields.Single(f => f!["name"]!.GetValue<string>() == "node")!;
        Assert.True(node["hasSerializeReference"]!.GetValue<bool>());
        Assert.Equal("Game.Node", node["fieldTypeFullName"]!.GetValue<string>());

        Assert.Contains(fields, field =>
            field!["name"]!.GetValue<string>() == "serializedPrivate" &&
            field["fieldTypeFullName"]!.GetValue<string>() == "System.Int32");
        Assert.DoesNotContain(fields, field =>
            field!["name"]!.GetValue<string>() == "hidden");

        JsonNode items = fields.Single(f => f!["name"]!.GetValue<string>() == "items")!;
        Assert.True(items["isList"]!.GetValue<bool>());
        Assert.StartsWith(
            "System.Collections.Generic.List`1[[Game.Item, SchemaSample,",
            items["fieldTypeFullName"]!.GetValue<string>());
        Assert.Equal("Game.Item", items["elementTypeFullName"]!.GetValue<string>());

        JsonNode kind = fields.Single(f => f!["name"]!.GetValue<string>() == "kind")!;
        Assert.True(kind["isFlagsEnum"]!.GetValue<bool>());
        Assert.Contains(kind["enumOptions"]!.AsArray(), option =>
            option!["name"]!.GetValue<string>() == "B" &&
            option["numericValue"]!.GetValue<long>() == 2);

        JsonNode leaf = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Leaf")!;
        Assert.True(leaf["isSerializable"]!.GetValue<bool>());
        Assert.Equal("Game.Node", leaf["baseTypeFullName"]!.GetValue<string>());

        JsonNode asset = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.UnityAsset")!;
        Assert.True(asset["isUnityObject"]!.GetValue<bool>());
    }

    [Fact]
    public void Includes_serialized_field_type_closure_for_nested_collection_fields()
    {
        string path = CompileToDisk("SchemaNestedFieldClosure", @"
using System.Collections.Generic;

namespace UnityEngine
{
    public class Object { }
}

namespace Game
{
    public class Holder : UnityEngine.Object
    {
        public List<Sprite> sprites;
    }

    public class Sprite
    {
        public Vector2 pivot;
    }

    public struct Vector2
    {
        public float x;
        public float y;
    }
}
");

        JsonObject result = BuildFromPaths(path);
        JsonArray types = result["types"]!.AsArray();

        JsonNode sprite = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Sprite")!;
        JsonNode pivot = sprite["fields"]!.AsArray()
            .Single(f => f!["name"]!.GetValue<string>() == "pivot")!;
        Assert.Equal("Game.Vector2", pivot["fieldTypeFullName"]!.GetValue<string>());

        JsonNode vector = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Vector2")!;
        JsonArray vectorFields = vector["fields"]!.AsArray();
        Assert.Contains(vectorFields, field =>
            field!["name"]!.GetValue<string>() == "x" &&
            field["fieldTypeFullName"]!.GetValue<string>() == "System.Single");
        Assert.Contains(vectorFields, field =>
            field!["name"]!.GetValue<string>() == "y" &&
            field["fieldTypeFullName"]!.GetValue<string>() == "System.Single");
    }

    [Fact]
    public void Handles_enum_and_nicify_edges_without_failing_schema()
    {
        string path = CompileToDisk("SchemaEnumEdges", @"
using System;

namespace UnityEngine
{
    public class Object { }
}

namespace Game
{
    [Serializable]
    public class Holder
    {
        public Direction direction;
        public Sparse sparse;
        public HugeFlags flags;
    }

    public enum Direction
    {
        north_east,
        _value,
        m_speed,
        HTTPStatus
    }

    public enum Sparse
    {
        Ten = 10,
        Zero = 0,
        Two = 2
    }

    [Flags]
    public enum HugeFlags : ulong
    {
        None = 0,
        All = 0xFFFFFFFFFFFFFFFF
    }
}
");

        JsonObject result = BuildFromPaths(path);
        JsonArray types = result["types"]!.AsArray();
        JsonNode holder = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Holder")!;
        JsonArray fields = holder["fields"]!.AsArray();
        JsonNode direction = fields.Single(f => f!["name"]!.GetValue<string>() == "direction")!;
        JsonArray directionOptions = direction["enumOptions"]!.AsArray();

        Assert.Contains(directionOptions, option =>
            option!["name"]!.GetValue<string>() == "north_east" &&
            option["label"]!.GetValue<string>() == "North East");
        Assert.Contains(directionOptions, option =>
            option!["name"]!.GetValue<string>() == "_value" &&
            option["label"]!.GetValue<string>() == "Value");
        Assert.Contains(directionOptions, option =>
            option!["name"]!.GetValue<string>() == "m_speed" &&
            option["label"]!.GetValue<string>() == "Speed");

        JsonNode sparse = fields.Single(f => f!["name"]!.GetValue<string>() == "sparse")!;
        JsonArray sparseOptions = sparse["enumOptions"]!.AsArray();
        Assert.Equal(new[] { "Zero", "Two", "Ten" }, sparseOptions
            .Select(option => option!["name"]!.GetValue<string>())
            .ToArray());

        JsonNode flags = fields.Single(f => f!["name"]!.GetValue<string>() == "flags")!;
        Assert.Contains(flags["enumOptions"]!.AsArray(), option =>
            option!["name"]!.GetValue<string>() == "All" &&
            option["numericValue"]!.GetValue<long>() == 1);
    }

    [Fact]
    public void Includes_serialized_auto_property_backing_fields()
    {
        string path = CompileToDisk("SchemaBackingFields", @"
using System;

namespace UnityEngine
{
    public class Object { }
    public sealed class SerializeField : Attribute { }
}

namespace Game
{
    [Serializable]
    public class Holder
    {
        [field: UnityEngine.SerializeField]
        public int Level { get; private set; }
    }
}
");

        JsonObject result = BuildFromPaths(path);
        JsonArray types = result["types"]!.AsArray();
        JsonNode holder = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Holder")!;
        JsonArray fields = holder["fields"]!.AsArray();

        Assert.Contains(fields, field =>
            field!["name"]!.GetValue<string>() == "<Level>k__BackingField" &&
            field["fieldTypeFullName"]!.GetValue<string>() == "System.Int32");
    }

    [Fact]
    public void Skips_non_serialized_plain_classes()
    {
        string path = CompileToDisk("SchemaRelevance", @"
using System;

namespace UnityEngine
{
    public class Object { }
}

namespace Game
{
    public class Utility
    {
        public int value;
    }

    [Serializable]
    public class Data
    {
        public int value;
    }
}
");

        JsonObject result = BuildFromPaths(path);
        JsonArray types = result["types"]!.AsArray();

        Assert.DoesNotContain(types, type => type!["fullName"]!.GetValue<string>() == "Game.Utility");
        Assert.Contains(types, type => type!["fullName"]!.GetValue<string>() == "Game.Data");
    }

    [Fact]
    public void Builds_schema_for_generic_base_with_array_type_argument()
    {
        // Regression: a constructed generic reached through the base-type closure
        // whose type argument is an array (Base<int[]>) used to throw a
        // NullReferenceException, because array/pointer symbols have no
        // ContainingAssembly of their own. The whole index/schema request failed.
        string path = CompileToDisk("SchemaArrayTypeArg", @"
using System;

namespace UnityEngine
{
    public class Object { }
}

namespace Game
{
    public class Base<T> { }

    [Serializable]
    public class Derived : Base<int[]> { }
}
");

        JsonObject result = BuildFromPaths(path);
        JsonArray types = result["types"]!.AsArray();

        // Must be present (not silently skipped) and carry a well-formed base name
        // with the array argument resolved to its element type's assembly.
        JsonNode derived = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Derived")!;
        Assert.StartsWith(
            "Game.Base`1[[System.Int32[], ",
            derived["baseTypeFullName"]!.GetValue<string>());
    }

    [Fact]
    public void Emits_type_parameters_and_positional_placeholders_for_generics()
    {
        string path = CompileToDisk("SchemaGenerics", @"
using System;
using System.Collections.Generic;

namespace UnityEngine
{
    public class Object { }
    public sealed class SerializeField : Attribute { }
    public sealed class SerializeReference : Attribute { }
}

namespace Game
{
    [Serializable]
    public struct Toggle<T>
    {
        public bool enable;
        public T data;
        public List<T> history;
    }

    [Serializable]
    public class Owner
    {
        public Toggle<int> toggle;
    }

    public abstract class Wrapper<TValue> : UnityEngine.Object, IWrapper<TValue>
    {
        [UnityEngine.SerializeReference]
        public TValue payload;
    }

    public interface IThing { }
    public interface IWrapper<TValue> { }

    public class ConcreteWrapper : Wrapper<IThing> { }
}
");

        JsonObject result = BuildFromPaths(path);
        Assert.Equal(2, result["schemaVersion"]!.GetValue<int>());
        JsonArray types = result["types"]!.AsArray();

        // Generic definition records its type-parameter names.
        JsonNode toggle = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Toggle`1")!;
        Assert.Equal(
            new[] { "T" },
            toggle["typeParameters"]!.AsArray().Select(p => p!.GetValue<string>()).ToArray());

        JsonArray toggleFields = toggle["fields"]!.AsArray();
        // Bare type parameter -> positional placeholder.
        JsonNode data = toggleFields.Single(f => f!["name"]!.GetValue<string>() == "data")!;
        Assert.Equal("!0", data["fieldTypeFullName"]!.GetValue<string>());
        // Concrete member keeps its own type.
        JsonNode enable = toggleFields.Single(f => f!["name"]!.GetValue<string>() == "enable")!;
        Assert.Equal("System.Boolean", enable["fieldTypeFullName"]!.GetValue<string>());
        // Nested generic keeps the placeholder inside the argument list.
        JsonNode history = toggleFields.Single(f => f!["name"]!.GetValue<string>() == "history")!;
        Assert.Equal(
            "System.Collections.Generic.List`1[[!0]]",
            history["fieldTypeFullName"]!.GetValue<string>());
        Assert.Equal("!0", history["elementTypeFullName"]!.GetValue<string>());

        // A closed instantiation used as a field keeps the assembly-qualified
        // constructed name (byte-identical to the previous renderer).
        JsonNode owner = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Owner")!;
        JsonNode toggleField = owner["fields"]!.AsArray()
            .Single(f => f!["name"]!.GetValue<string>() == "toggle")!;
        Assert.StartsWith(
            "Game.Toggle`1[[System.Int32, ",
            toggleField["fieldTypeFullName"]!.GetValue<string>());

        // The open generic base is now registered (previously dropped because the
        // base closure keyed it by its constructed name), and the derived type
        // points at the constructed base.
        JsonNode wrapperDef = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.Wrapper`1")!;
        Assert.Equal(
            new[] { "TValue" },
            wrapperDef["typeParameters"]!.AsArray().Select(p => p!.GetValue<string>()).ToArray());
        JsonNode payload = wrapperDef["fields"]!.AsArray()
            .Single(f => f!["name"]!.GetValue<string>() == "payload")!;
        Assert.True(payload["hasSerializeReference"]!.GetValue<bool>());
        Assert.Equal("!0", payload["fieldTypeFullName"]!.GetValue<string>());
        JsonNode wrapperInterface = wrapperDef["interfaces"]!.AsArray()
            .Single(iface => iface!["fullName"]!.GetValue<string>().StartsWith("Game.IWrapper`1", StringComparison.Ordinal))!;
        Assert.Equal(
            "Game.IWrapper`1[[!0]]",
            wrapperInterface["fullName"]!.GetValue<string>());

        JsonNode concrete = types.Single(t => t!["fullName"]!.GetValue<string>() == "Game.ConcreteWrapper")!;
        Assert.StartsWith(
            "Game.Wrapper`1[[Game.IThing, ",
            concrete["baseTypeFullName"]!.GetValue<string>());
        Assert.Contains(
            concrete["interfaces"]!.AsArray(),
            iface => iface!["fullName"]!.GetValue<string>().StartsWith(
                "Game.IWrapper`1[[Game.IThing, ",
                StringComparison.Ordinal));
    }
}
