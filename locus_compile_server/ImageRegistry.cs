using System.Collections.Immutable;
using Microsoft.CodeAnalysis;

namespace Locus.CompileServer;

/// <summary>
/// Session registry of in-memory assembly images, keyed by the Unity
/// AppDomain generation (a GUID minted per domain load, see the Unity-side
/// `get_compile_params`). Successfully emitted snippet / run_states
/// assemblies are registered here so a later compile can reference types
/// defined by an earlier snippet (`reference_session_images`) — the gap the
/// Unity in-process compiler has (CreateFromFile only, no CreateFromImage).
///
/// Registering under a new generation discards every image of older
/// generations: a Unity domain reload unloads those assemblies, so compiling
/// against them would produce assemblies that fail to resolve at runtime.
/// </summary>
public sealed class ImageRegistry
{
    private string? _generation;
    private readonly List<(string AssemblyName, MetadataReference Reference)> _images = new();

    public void Register(string generation, string assemblyName, byte[] image)
    {
        if (string.IsNullOrEmpty(generation))
            return;

        if (!string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            _images.Clear();
            _generation = generation;
        }

        _images.Add((
            assemblyName,
            MetadataReference.CreateFromImage(
                ImmutableArray.Create(image),
                filePath: assemblyName + ".dll")));
    }

    public IReadOnlyList<MetadataReference> ReferencesFor(string? generation)
    {
        if (string.IsNullOrEmpty(generation) ||
            !string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            return Array.Empty<MetadataReference>();
        }

        return _images.Select(image => image.Reference).ToList();
    }

    public int CountFor(string? generation)
    {
        return ReferencesFor(generation).Count;
    }

    /// <summary>True when <paramref name="assemblyName"/> is registered for the
    /// given generation — i.e. it is among <see cref="ReferencesFor"/> and a
    /// patch that references session images can resolve types defined in it.</summary>
    public bool Contains(string? generation, string assemblyName)
    {
        if (string.IsNullOrEmpty(generation) ||
            !string.Equals(_generation, generation, StringComparison.Ordinal))
        {
            return false;
        }

        return _images.Any(image => string.Equals(image.AssemblyName, assemblyName, StringComparison.Ordinal));
    }
}
