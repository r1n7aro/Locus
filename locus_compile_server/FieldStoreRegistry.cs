namespace Locus.CompileServer;

/// <summary>
/// Session registry of virtualized fields (M4): a field ADDED by a hot patch
/// lives in a per-field store (instance) or holder field (static) inside the
/// FIRST patch assembly that introduced it. Later batches rewrite their
/// accesses to that same store — regenerating one would silently split the
/// values. Same lifecycle discipline as <see cref="ImageRegistry"/>: new
/// domain generation discards everything; a sidecar restart loses the
/// session and later references fail deterministically toward
/// unity_recompile.
/// </summary>
public sealed class FieldStoreRegistry
{
    public sealed class StoreEntry
    {
        /// <summary>Patch assembly that declared the store/holder.</summary>
        public string StoreAssembly = "";

        /// <summary>CLR metadata name, e.g. "Ns.__LocusFields_Foo".</summary>
        public string StoreTypeMetadataName = "";

        /// <summary>Fully-qualified C# name, e.g. "global::Ns.__LocusFields_Foo".</summary>
        public string StoreTypeFqn = "";

        /// <summary>Member on the store type: the LocusFieldStore&lt;T&gt;
        /// field (instance) or the plain holder field (static).</summary>
        public string MemberName = "";

        public bool IsStatic;

        public string FieldTypeFqn = "";
    }

    private string? _generation;
    private readonly Dictionary<string, StoreEntry> _fields = new(StringComparer.Ordinal);

    public static string FieldKey(string declaringType, string fieldName) =>
        declaringType + "|" + fieldName;

    public void Commit(string generation, IEnumerable<KeyValuePair<string, StoreEntry>> entries)
    {
        if (string.IsNullOrEmpty(generation))
            return;

        if (!string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            _fields.Clear();
            _generation = generation;
        }

        foreach (var pair in entries)
        {
            // First introducer wins: later batches bind to the original
            // store, never replace it (values would split).
            if (!_fields.ContainsKey(pair.Key))
                _fields[pair.Key] = pair.Value;
        }
    }

    public IReadOnlyDictionary<string, StoreEntry> SnapshotFor(string? generation)
    {
        if (string.IsNullOrEmpty(generation) ||
            !string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            return new Dictionary<string, StoreEntry>(StringComparer.Ordinal);
        }

        return new Dictionary<string, StoreEntry>(_fields, StringComparer.Ordinal);
    }
}
