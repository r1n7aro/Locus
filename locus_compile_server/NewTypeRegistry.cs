namespace Locus.CompileServer;

/// <summary>
/// Session registry of FILES whose types live only in a hot-patch assembly: a
/// script written entirely during this domain (e.g. authored in Play Mode) and
/// applied via load_only, with no compiled-metadata original. Keyed by the
/// Unity AppDomain generation with the same lifecycle discipline as
/// <see cref="ImageRegistry"/> and <see cref="MemberSurfaceRegistry"/> — a new
/// generation discards older entries (the domain reload that bumped it unloaded
/// the patch assemblies), and a sidecar restart loses everything.
///
/// Why it exists: without it, every later edit of such a file re-diffs against
/// the empty coordinator baseline and re-classifies the whole type as new
/// (load_only into a FRESH assembly), so the live instances — which are of the
/// FIRST loaded type — never see the change; only newly created instances do.
/// The entry records the file's ORIGINAL text and the FIRST assembly it loaded
/// into. A later body-only re-edit (see the handler's IsBodyOnlyHotReedit gate)
/// diffs against that text and redirects the changed method BODIES onto that
/// first assembly's type, so existing instances pick up the new behavior. The
/// detour replaces rather than stacks (stable method identity), like the M2
/// shim discipline.
///
/// An entry is recorded at first load. The handler then routes a registered
/// file's re-edits three ways (see HandleCompileHotPatch): a body/using change
/// REDIRECTS onto the first assembly (and flips <see cref="FileEntry.Redirected"/>
/// so a later revert is handled right); an unchanged re-send is a clean NO-OP
/// (the coordinator re-ships every dirty file each convergence batch, so a
/// load_only'd file recurs unchanged); a structural change, or a revert to the
/// original text AFTER a redirect, is steered COLD so unity_recompile converges
/// (a load_only there would strand existing instances on a stale redirected
/// body — a false-positive "applied"). After that recompile the domain reloads,
/// this registry clears on the generation change, and the type becomes a real
/// compiled type.
///
/// Lifecycle coupling: the OriginalAssembly recorded here is committed in the
/// same image/register acceptance that registers the assembly in
/// <see cref="ImageRegistry"/> under the same generation, so a live entry here
/// implies the assembly is referenceable there. The re-edit redirect relies on
/// that (the patch's layout guard must resolve the original type); the handler
/// re-checks ImageRegistry before redirecting and steers cold if the assembly
/// is somehow absent, so the coupling failing is never fatal.
/// </summary>
public sealed class NewTypeRegistry
{
    public sealed class FileEntry
    {
        /// <summary>The file text the FIRST load_only emit compiled from — the
        /// effective diff baseline for every later re-edit of this file.</summary>
        public string OriginalText = "";

        /// <summary>The FIRST hot-patch assembly the file's types were loaded
        /// into — the detour ORIGINAL side for every later re-edit. Filled in
        /// after a successful emit, mirroring shim/field-store bookkeeping.</summary>
        public string OriginalAssembly = "";

        /// <summary>A body redirect has been applied to this file (a detour is
        /// live on the first assembly's methods). It distinguishes the two empty
        /// re-diffs: an UNCHANGED re-send of a never-redirected file is a clean
        /// no-op (the coordinator re-ships every dirty file each convergence
        /// batch), whereas a revert to the original text AFTER a redirect must
        /// be steered cold so a recompile clears the now-stale detour rather than
        /// leaving instances on the redirected body. Set true (committed on
        /// accept) the first time a redirect is emitted for the file.</summary>
        public bool Redirected;

        /// <summary>The most recent text successfully hot-applied for this file —
        /// the baseline for detecting members/messages REMOVED since the last
        /// apply. A play-mode-born type's POST-BIRTH additions (e.g. an added
        /// Update()) are absent from <see cref="OriginalText"/>, so a diff against
        /// the birth text can never reveal their later removal; a diff against this
        /// live text can. Initialized to the birth text at first load; re-committed
        /// to the new text on every hot apply / no-op. Empty (legacy entry) → the
        /// handler falls back to <see cref="OriginalText"/>.</summary>
        public string LastAppliedText = "";

        /// <summary>Sibling TOP-LEVEL types born into this file AFTER the first
        /// batch (feature #5). The first-batch types all live in
        /// <see cref="OriginalAssembly"/> and diff against <see cref="OriginalText"/>;
        /// a sibling added by a later re-edit lives in its OWN assembly and diffs
        /// against the whole-file text AT ITS BIRTH (so its body/additive re-edits
        /// redirect onto that assembly, exactly like a first-batch type). Keyed by
        /// the sibling's metadata name; null when the file only has first-batch
        /// types (the common case).</summary>
        public Dictionary<string, SiblingType>? Siblings;
    }

    /// <summary>A sibling top-level type born into a file after its first batch:
    /// it lives in its own hot-patch assembly (not the file's first one), so its
    /// re-edits resolve and redirect THERE.</summary>
    public sealed class SiblingType
    {
        /// <summary>The hot-patch assembly this sibling was load_only'd into — the
        /// detour ORIGINAL side for its re-edits. Empty until pinned after emit.</summary>
        public string Assembly = "";

        /// <summary>The whole-file text when this sibling was born — the diff
        /// baseline for redirecting/extending the sibling (a per-type diff is
        /// filtered out of the whole-file diff against this text).</summary>
        public string BirthText = "";
    }

    private string? _generation;
    private readonly Dictionary<string, FileEntry> _files = new(StringComparer.Ordinal);

    public void Commit(string generation, IEnumerable<KeyValuePair<string, FileEntry>> entries)
    {
        if (string.IsNullOrEmpty(generation))
            return;

        if (!string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            _files.Clear();
            _generation = generation;
        }

        foreach (var pair in entries)
        {
            // A file registers once (at first load): a registered file's later
            // re-edits either redirect (no new type → no re-registration) or are
            // steered cold, so this never overwrites a live entry within a
            // generation. The overwrite-on-equal-key is just last-wins
            // defensiveness. Never pin an empty origin — it would make the next
            // re-edit's resolution fail and roll the whole patch back.
            if (string.IsNullOrEmpty(pair.Value.OriginalAssembly))
                continue;
            _files[pair.Key] = pair.Value;
        }
    }

    public IReadOnlyDictionary<string, FileEntry> SnapshotFor(string? generation)
    {
        if (string.IsNullOrEmpty(generation) ||
            !string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            return new Dictionary<string, FileEntry>(StringComparer.Ordinal);
        }

        return new Dictionary<string, FileEntry>(_files, StringComparer.Ordinal);
    }
}
