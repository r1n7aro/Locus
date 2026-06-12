namespace Locus.CompileServer;

/// <summary>
/// Session registry of NEW-SURFACE members (M2): members that do not exist
/// in the original metadata but were materialized as static shims inside a
/// hot-patch assembly. Keyed by the Unity AppDomain generation with the same
/// lifecycle discipline as <see cref="ImageRegistry"/>: registering under a
/// new generation discards older entries (a domain reload unloaded the patch
/// assemblies), and a sidecar restart loses everything — later references
/// then fail deterministically and the tool points at unity_recompile.
///
/// Entries let later batches (a) detour an OLD shim to its re-edited
/// replacement so in-flight delegates pick up new behavior, and (b) resolve
/// calls to members added by earlier batches whose files are not part of the
/// current batch. Tombstones (H7c) mark deleted members so a later binding
/// against them is a deterministic rewrite-time error.
/// </summary>
public sealed class MemberSurfaceRegistry
{
    public sealed class ShimEntry
    {
        /// <summary>"added" | "tombstone".</summary>
        public string Kind = "added";

        /// <summary>Patch assembly that hosts the (latest) shim.</summary>
        public string ShimAssembly = "";

        /// <summary>CLR metadata name, e.g. "Ns.Foo__LocusShims".</summary>
        public string ShimTypeMetadataName = "";

        /// <summary>Fully-qualified C# name, e.g. "global::Ns.Foo__LocusShims".</summary>
        public string ShimTypeFqn = "";

        public string ShimMethod = "";

        /// <summary>Reflection-style parameter names of the shim method,
        /// INCLUDING the leading self parameter for instance members.</summary>
        public string[] ParamTypeNames = System.Array.Empty<string>();

        /// <summary>Fully-qualified original declaring type, e.g. "global::Ns.Foo".</summary>
        public string DeclaringTypeFqn = "";

        public bool HasSelf;
        public bool SelfIsValueType;

        /// <summary>The shim method carries type parameters (declaring type
        /// is generic): direct calls work via inference, but re-edit detours
        /// are skipped — generic method detours are the unreliable case.</summary>
        public bool GenericShim;
    }

    private string? _generation;
    private readonly Dictionary<string, ShimEntry> _members = new(StringComparer.Ordinal);

    /// <summary>Identity of a member surface across batches. Matches the
    /// HotDiff method identity (declaring type + metadata name + reflection
    /// parameter names + staticness of the ORIGINAL member, not the shim).</summary>
    public static string MemberKey(string declaringType, string name, IEnumerable<string> paramTypeNames, bool isStatic)
    {
        return declaringType + "|" + name + "|" + string.Join(",", paramTypeNames) + (isStatic ? "|s" : "|i");
    }

    public void Commit(string generation, IEnumerable<KeyValuePair<string, ShimEntry>> entries)
    {
        if (string.IsNullOrEmpty(generation))
            return;

        if (!string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            _members.Clear();
            _generation = generation;
        }

        foreach (var pair in entries)
            _members[pair.Key] = pair.Value;
    }

    public IReadOnlyDictionary<string, ShimEntry> SnapshotFor(string? generation)
    {
        if (string.IsNullOrEmpty(generation) ||
            !string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            return new Dictionary<string, ShimEntry>(StringComparer.Ordinal);
        }

        return new Dictionary<string, ShimEntry>(_members, StringComparer.Ordinal);
    }
}
