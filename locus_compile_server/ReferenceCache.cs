using Microsoft.CodeAnalysis;

namespace Locus.CompileServer;

/// <summary>
/// Per-file MetadataReference cache keyed by (path, mtime, size).
///
/// References are created with PEStreamOptions.PrefetchMetadata so no file
/// handle outlives the load: the Unity Editor rewrites Library/ScriptAssemblies
/// on every recompile. Unlike MetadataReference.CreateFromFile, which prefetches
/// the entire image, only the metadata is retained here. Each returned reference
/// keeps its snapshot alive; cache eviction must not dispose metadata still
/// used by a compilation. Unreferenced snapshots are reclaimed by GC.
/// </summary>
public sealed class ReferenceCache
{
    private sealed class Entry
    {
        public long MtimeTicks;
        public long Size;
        public PortableExecutableReference Reference = null!;
    }

    private readonly Dictionary<string, Entry> _entries = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>
    /// Resolve a reference for `path`, reusing the cached instance while the
    /// file is unchanged. Returns null for missing/unreadable/non-assembly
    /// files — mirroring the silent skip in the Unity-side reference
    /// collection (TryAddMetadataReference).
    /// </summary>
    public PortableExecutableReference? GetOrCreate(string path)
    {
        FileInfo info;
        try
        {
            info = new FileInfo(path);
            if (!info.Exists)
                return null;
        }
        catch
        {
            return null;
        }

        long mtime = info.LastWriteTimeUtc.Ticks;
        long size = info.Length;

        if (_entries.TryGetValue(path, out var cached))
        {
            if (cached.MtimeTicks == mtime && cached.Size == size)
                return cached.Reference;
            _entries.Remove(path);
        }

        // One retry: Unity may be swapping the file at this instant
        // (ScriptAssemblies rewrite during a recompile).
        AssemblyMetadata? metadata = TryLoadMetadata(path);
        if (metadata == null)
        {
            Thread.Sleep(50);
            metadata = TryLoadMetadata(path);
        }
        if (metadata == null)
            return null;

        var entry = new Entry
        {
            MtimeTicks = mtime,
            Size = size,
            Reference = metadata.GetReference(filePath: path),
        };
        _entries[path] = entry;
        return entry.Reference;
    }

    /// <summary>Drop cache entries without invalidating references held by callers.</summary>
    public void PruneExcept(IReadOnlyCollection<string> alive)
    {
        var keep = new HashSet<string>(alive, StringComparer.OrdinalIgnoreCase);
        var stale = _entries.Keys.Where(k => !keep.Contains(k)).ToList();
        foreach (string key in stale)
        {
            _entries.Remove(key);
        }
    }

    public int Count => _entries.Count;

    private static AssemblyMetadata? TryLoadMetadata(string path)
    {
        try
        {
            using var stream = new FileStream(
                path,
                FileMode.Open,
                FileAccess.Read,
                FileShare.ReadWrite | FileShare.Delete);
            // PrefetchMetadata copies the metadata section so the stream (and
            // any lock on the file) is released before this method returns.
            var module = ModuleMetadata.CreateFromStream(
                stream,
                System.Reflection.PortableExecutable.PEStreamOptions.PrefetchMetadata);
            return AssemblyMetadata.Create(module);
        }
        catch
        {
            return null;
        }
    }
}
