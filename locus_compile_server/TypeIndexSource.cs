using System.Reflection;
using System.Reflection.Metadata;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;

namespace Locus.CompileServer;

/// <summary>
/// TI-B: build the Unity type index straight from the compile-reference
/// metadata (already prefetched in the ReferenceCache) instead of having the
/// Unity Editor reflect over every loaded assembly and ship a multi-MB JSON
/// across the pipe. Entry shape, skip rules, deduplication and ordering
/// mirror LocusBridge.TypeIndex.cs exactly — the Rust side treats both
/// sources as interchangeable.
///
/// Not covered here (and layered separately): in-memory skill-package
/// assemblies (delta channel on compile_skill_package) and hot-patch new
/// types (TI-C) — both are invisible to file-backed references.
/// </summary>
public static class TypeIndexSource
{
    public sealed class Entry
    {
        public string SimpleName = "";
        public string Ns = "";
        public string FullName = "";
        public string Assembly = "";
    }

    /// <summary>Mirror of the Unity-side ShouldSkipTypeIndexAssembly for
    /// assemblies that can appear in the reference path list.</summary>
    public static bool ShouldSkipAssembly(string assemblyName)
    {
        if (string.IsNullOrEmpty(assemblyName))
            return true;

        return assemblyName.StartsWith("__LocusRuntimeAsync_", StringComparison.Ordinal)
            || assemblyName.StartsWith("__LocusSnippet_", StringComparison.Ordinal)
            || assemblyName.StartsWith("__LocusView_", StringComparison.Ordinal)
            || assemblyName.StartsWith("__LocusRunStates_", StringComparison.Ordinal)
            || assemblyName.StartsWith("__LocusHotPatch_", StringComparison.Ordinal)
            || assemblyName.StartsWith("__LocusSkillPackage_", StringComparison.Ordinal)
            || assemblyName == "Locus.Editor"
            || assemblyName.StartsWith("Microsoft.CodeAnalysis", StringComparison.Ordinal)
            || assemblyName == "System.Collections.Immutable"
            || assemblyName == "System.Reflection.Metadata";
    }

    public static List<Entry> Build(IEnumerable<PortableExecutableReference> references)
    {
        // Assembly name → reader, sorted by assembly name (ordinal) so the
        // first-assembly-wins dedup attributes duplicates identically to the
        // Unity export (which sorts AppDomain assemblies by name).
        var readers = new List<(string AssemblyName, MetadataReader Reader)>();
        foreach (PortableExecutableReference reference in references)
        {
            try
            {
                if (reference.GetMetadata() is not AssemblyMetadata assembly)
                    continue;
                MetadataReader reader = assembly.GetModules()[0].GetMetadataReader();
                if (!reader.IsAssembly)
                    continue;
                string name = reader.GetString(reader.GetAssemblyDefinition().Name);
                if (ShouldSkipAssembly(name))
                    continue;
                readers.Add((name, reader));
            }
            catch
            {
                // Unreadable/native files mirror the Unity side's silent skip.
            }
        }
        readers.Sort((a, b) => string.CompareOrdinal(a.AssemblyName, b.AssemblyName));

        var entries = new List<Entry>(16384);
        var seen = new HashSet<string>(StringComparer.Ordinal);

        foreach (var (assemblyName, reader) in readers)
        {
            foreach (TypeDefinitionHandle handle in reader.TypeDefinitions)
            {
                TypeDefinition typeDef = reader.GetTypeDefinition(handle);

                // Top-level public only — nested visibilities fail the mask
                // compare, matching `type.IsNested || !type.IsPublic`.
                if ((typeDef.Attributes & TypeAttributes.VisibilityMask) != TypeAttributes.Public)
                    continue;

                string name = reader.GetString(typeDef.Name);
                string simpleName = StripGenericArity(name);
                if (string.IsNullOrEmpty(simpleName))
                    continue;

                string ns = typeDef.Namespace.IsNil ? "" : reader.GetString(typeDef.Namespace);
                string fullName = ns.Length == 0 ? simpleName : ns + "." + simpleName;
                if (!seen.Add(fullName))
                    continue;

                entries.Add(new Entry
                {
                    SimpleName = simpleName,
                    Ns = ns,
                    FullName = fullName,
                    Assembly = assemblyName,
                });
            }
        }

        entries.Sort(CompareEntries);
        return entries;
    }

    public static JsonObject ToJson(List<Entry> entries)
    {
        var types = new JsonArray();
        foreach (Entry entry in entries)
        {
            types.Add(new JsonObject
            {
                ["simpleName"] = entry.SimpleName,
                ["ns"] = entry.Ns,
                ["fullName"] = entry.FullName,
                ["assembly"] = entry.Assembly,
            });
        }
        return new JsonObject
        {
            ["count"] = entries.Count,
            ["types"] = types,
        };
    }

    private static int CompareEntries(Entry a, Entry b)
    {
        int byName = string.CompareOrdinal(a.SimpleName, b.SimpleName);
        if (byName != 0)
            return byName;
        int byNamespace = string.CompareOrdinal(a.Ns, b.Ns);
        if (byNamespace != 0)
            return byNamespace;
        return string.CompareOrdinal(a.Assembly, b.Assembly);
    }

    private static string StripGenericArity(string name)
    {
        int tick = name.IndexOf('`');
        return tick >= 0 ? name.Substring(0, tick) : name;
    }
}
