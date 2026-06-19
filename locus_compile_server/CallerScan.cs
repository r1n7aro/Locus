using System.Reflection.Metadata;
using System.Reflection.Metadata.Ecma335;
using System.Reflection.PortableExecutable;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace Locus.CompileServer;

/// <summary>A member surface (or whole type) whose external call sites must
/// be located before a deletion/signature change can go hot (M3).</summary>
public sealed class CallerScanTarget
{
    /// <summary>CLR metadata name: "Ns.Outer+Inner".</summary>
    public string DeclaringType = "";

    /// <summary>Metadata member name ("M", "get_X"); empty = scan for any
    /// reference to the TYPE itself (type deletion).</summary>
    public string MemberName = "";

    public static string Key(string declaringType, string memberName) =>
        declaringType + "|" + memberName;
}

public sealed class CallerScanResult
{
    /// <summary>Target key (DeclaringType|MemberName) → source files whose
    /// compiled code references the target. Paths as recorded in the PDB
    /// (Unity: project-relative "Assets/...").</summary>
    public Dictionary<string, HashSet<string>> CallerFiles = new(StringComparer.Ordinal);

    /// <summary>Target key → exact compiled caller methods and their source
    /// files. Used by Release-inline caller refresh: refreshing the whole file
    /// would create avoidable detours, so the coordinator asks for the method
    /// that contains the call site.</summary>
    public Dictionary<string, List<CallerScanLocation>> CallerLocations = new(StringComparer.Ordinal);

    /// <summary>Fail-closed error (unreadable assembly, missing PDB): the
    /// caller must treat every target as unverifiable.</summary>
    public string? Error;
}

public sealed class CallerScanLocation
{
    public string File = "";
    public string CallerMethodKey = "";
    public string DeclaringType = "";
    public string MemberName = "";
}

/// <summary>
/// IL-level caller scan over the project's own assemblies (M3): finds every
/// method whose body references one of the target members (call / callvirt /
/// newobj / ldftn / ldvirtftn / ldtoken / field access for type targets) and
/// maps it back to its source file through the portable PDB. Matching is by
/// declaring type + member NAME (overload-insensitive): an over-approximation
/// that can only fail closed.
///
/// Known blind spots (callers must surface them in tool output): reflection
/// by name, SendMessage(string), UnityEvent serialized bindings, and inlined
/// const/enum values leave no metadata reference.
/// </summary>
public static class CallerScan
{
    private sealed class AssemblyCallerIndex
    {
        public string Path = "";
        public DateTime LastWriteUtc;
        public long Length;
        public Guid Mvid;
        public string? Error;
        public HashSet<string> TargetKeys = new(StringComparer.Ordinal);
        public Dictionary<string, List<CallerScanLocation>> LocationsByTarget = new(StringComparer.Ordinal);
        public Dictionary<string, string> UnmappedByTarget = new(StringComparer.Ordinal);
    }

    private static readonly object CacheLock = new();
    private static readonly Dictionary<string, AssemblyCallerIndex> Cache = new(StringComparer.OrdinalIgnoreCase);

    // ── Persistent tier ──────────────────────────────────────────────
    // The in-memory cache above is rebuilt from scratch every time the sidecar
    // restarts. The reverse caller graph of a large project assembly is the
    // most expensive thing this file does (a full IL walk of every method
    // body), so we also mirror each successfully-built index to disk, one JSON
    // file per assembly, keyed by the assembly's on-disk identity. A restart
    // then reloads instead of re-walking; editing one assembly only rewrites
    // that assembly's file (incremental). Any cache I/O failure is swallowed —
    // a missing/corrupt/locked cache simply rebuilds.

    private const int PersistFormatVersion = 1;

    private static readonly JsonSerializerOptions PersistJsonOptions = new()
    {
        // CallerScanLocation carries public FIELDS, not properties.
        IncludeFields = true,
    };

    private sealed class PersistedCallerIndex
    {
        public int FormatVersion { get; set; }
        public long LastWriteUtcTicks { get; set; }
        public long Length { get; set; }
        public string Mvid { get; set; } = "";
        public List<string> TargetKeys { get; set; } = new();
        public Dictionary<string, List<CallerScanLocation>> LocationsByTarget { get; set; } = new();
        public Dictionary<string, string> UnmappedByTarget { get; set; } = new();
    }

    /// <summary>Per-project cache directory: under the project's own Library so
    /// it lives and dies with the project and is naturally invalidated when the
    /// user clears Library. Falls back to the OS temp dir for assemblies outside
    /// the Unity layout (unit tests). Null only if even temp is unavailable.</summary>
    private static string? PersistDir(string assemblyFullPath)
    {
        try
        {
            string normalized = assemblyFullPath.Replace('\\', '/');
            int idx = normalized.LastIndexOf("/Library/ScriptAssemblies/", StringComparison.OrdinalIgnoreCase);
            if (idx > 0)
            {
                string projectRoot = assemblyFullPath.Substring(0, idx);
                return Path.Combine(projectRoot, "Library", "Locus", "CallerIndex");
            }
        }
        catch
        {
        }
        try
        {
            return Path.Combine(Path.GetTempPath(), "Locus", "CallerIndex");
        }
        catch
        {
            return null;
        }
    }

    /// <summary>Stable per-assembly file name: sanitized assembly name plus a
    /// path hash (disambiguates same-named assemblies sharing the temp fallback)
    /// plus the format version (a bump orphans old files).</summary>
    private static string PersistFileName(string assemblyFullPath)
    {
        string name = Path.GetFileNameWithoutExtension(assemblyFullPath);
        var sanitized = new StringBuilder(name.Length);
        foreach (char ch in name)
            sanitized.Append(char.IsLetterOrDigit(ch) ? ch : '_');
        byte[] digest = SHA1.HashData(Encoding.UTF8.GetBytes(assemblyFullPath.ToLowerInvariant()));
        string hash = Convert.ToHexString(digest, 0, 6).ToLowerInvariant();
        return $"{sanitized}-{hash}-v{PersistFormatVersion}.json";
    }

    private static Guid ReadAssemblyMvid(string assemblyFullPath)
    {
        using FileStream stream = File.OpenRead(assemblyFullPath);
        using var peReader = new PEReader(stream);
        MetadataReader reader = peReader.GetMetadataReader();
        return reader.GetGuid(reader.GetModuleDefinition().Mvid);
    }

    private static AssemblyCallerIndex? TryLoadPersisted(string assemblyFullPath, DateTime lastWriteUtc, long length)
    {
        if (length < 0)
            return null;
        try
        {
            string? dir = PersistDir(assemblyFullPath);
            if (dir == null)
                return null;
            string file = Path.Combine(dir, PersistFileName(assemblyFullPath));
            if (!File.Exists(file))
                return null;

            PersistedCallerIndex? persisted =
                JsonSerializer.Deserialize<PersistedCallerIndex>(File.ReadAllText(file), PersistJsonOptions);
            if (persisted == null ||
                persisted.FormatVersion != PersistFormatVersion ||
                persisted.LastWriteUtcTicks != lastWriteUtc.Ticks ||
                persisted.Length != length)
            {
                return null;
            }

            if (!Guid.TryParse(persisted.Mvid, out Guid persistedMvid) ||
                persistedMvid != ReadAssemblyMvid(assemblyFullPath))
            {
                return null;
            }

            return new AssemblyCallerIndex
            {
                Path = assemblyFullPath,
                LastWriteUtc = lastWriteUtc,
                Length = length,
                Mvid = persistedMvid,
                TargetKeys = new HashSet<string>(persisted.TargetKeys ?? new List<string>(), StringComparer.Ordinal),
                LocationsByTarget = new Dictionary<string, List<CallerScanLocation>>(
                    persisted.LocationsByTarget ?? new(), StringComparer.Ordinal),
                UnmappedByTarget = new Dictionary<string, string>(
                    persisted.UnmappedByTarget ?? new(), StringComparer.Ordinal),
            };
        }
        catch
        {
            return null;
        }
    }

    private static void TrySavePersisted(AssemblyCallerIndex index)
    {
        // Never persist a failed build: a transient read/PDB error must not
        // stick across restarts (re-deriving it is cheap).
        if (index.Error != null || index.Length < 0)
            return;
        try
        {
            string? dir = PersistDir(index.Path);
            if (dir == null)
                return;
            Directory.CreateDirectory(dir);
            string file = Path.Combine(dir, PersistFileName(index.Path));

            var persisted = new PersistedCallerIndex
            {
                FormatVersion = PersistFormatVersion,
                LastWriteUtcTicks = index.LastWriteUtc.Ticks,
                Length = index.Length,
                Mvid = index.Mvid.ToString(),
                TargetKeys = index.TargetKeys.ToList(),
                LocationsByTarget = index.LocationsByTarget,
                UnmappedByTarget = index.UnmappedByTarget,
            };
            string json = JsonSerializer.Serialize(persisted, PersistJsonOptions);

            // Write-then-rename so a crash mid-write never leaves a torn file
            // that the next load would reject (and rebuild) at best.
            string tmp = file + ".tmp." + Guid.NewGuid().ToString("N");
            File.WriteAllText(tmp, json);
            File.Move(tmp, file, overwrite: true);
        }
        catch
        {
        }
    }

    /// <summary>Is this reference path one of the project's own compiled
    /// assemblies (vs Unity/BCL references)?</summary>
    public static bool IsProjectAssemblyPath(string path)
    {
        string normalized = path.Replace('\\', '/').ToLowerInvariant();
        return normalized.Contains("/library/scriptassemblies/");
    }

    public static CallerScanResult Scan(IEnumerable<string> assemblyPaths, IReadOnlyList<CallerScanTarget> targets)
    {
        var result = new CallerScanResult();
        var targetKeys = new HashSet<string>(StringComparer.Ordinal);
        foreach (CallerScanTarget target in targets)
        {
            string key = CallerScanTarget.Key(target.DeclaringType, target.MemberName);
            targetKeys.Add(key);
            result.CallerFiles[key] = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            result.CallerLocations[key] = new List<CallerScanLocation>();
        }

        foreach (string assemblyPath in assemblyPaths)
        {
            AssemblyCallerIndex index = GetOrBuildIndex(assemblyPath);
            bool relevant = targetKeys.Any(key => index.TargetKeys.Contains(key));
            if (index.Error != null && relevant)
            {
                result.Error = index.Error;
                return result;
            }

            foreach (string key in targetKeys)
            {
                if (index.UnmappedByTarget.TryGetValue(key, out string? unmappedError))
                {
                    result.Error = unmappedError;
                    return result;
                }
                if (!index.LocationsByTarget.TryGetValue(key, out List<CallerScanLocation>? locations))
                    continue;
                foreach (CallerScanLocation location in locations)
                {
                    result.CallerFiles[key].Add(location.File);
                    if (!result.CallerLocations[key].Any(existing =>
                            string.Equals(existing.File, location.File, StringComparison.OrdinalIgnoreCase) &&
                            string.Equals(existing.CallerMethodKey, location.CallerMethodKey, StringComparison.Ordinal)))
                    {
                        result.CallerLocations[key].Add(location);
                    }
                }
            }
        }

        return result;
    }

    private static AssemblyCallerIndex GetOrBuildIndex(string assemblyPath)
    {
        FileInfo info = new(assemblyPath);
        string fullPath = info.FullName;
        DateTime lastWriteUtc = info.Exists ? info.LastWriteTimeUtc : DateTime.MinValue;
        long length = info.Exists ? info.Length : -1;

        lock (CacheLock)
        {
            if (Cache.TryGetValue(fullPath, out AssemblyCallerIndex? cached) &&
                cached.LastWriteUtc == lastWriteUtc &&
                cached.Length == length)
            {
                return cached;
            }
        }

        // Persistent tier: a sidecar restart reloads the reverse graph from
        // disk instead of re-walking every method body of a large assembly.
        AssemblyCallerIndex? persisted = TryLoadPersisted(fullPath, lastWriteUtc, length);
        if (persisted != null)
        {
            lock (CacheLock)
            {
                Cache[fullPath] = persisted;
            }
            return persisted;
        }

        AssemblyCallerIndex built;
        try
        {
            built = BuildIndex(fullPath, lastWriteUtc, length);
        }
        catch (Exception ex)
        {
            built = new AssemblyCallerIndex
            {
                Path = fullPath,
                LastWriteUtc = lastWriteUtc,
                Length = length,
                Error = "call-site scan failed for " + Path.GetFileName(assemblyPath) + ": " + ex.Message,
            };
        }

        lock (CacheLock)
        {
            Cache[fullPath] = built;
        }
        TrySavePersisted(built);
        return built;
    }

    private static AssemblyCallerIndex BuildIndex(string assemblyPath, DateTime lastWriteUtc, long length)
    {
        using FileStream stream = File.OpenRead(assemblyPath);
        using var peReader = new PEReader(stream);
        MetadataReader reader = peReader.GetMetadataReader();
        ModuleDefinition module = reader.GetModuleDefinition();

        var index = new AssemblyCallerIndex
        {
            Path = assemblyPath,
            LastWriteUtc = lastWriteUtc,
            Length = length,
            Mvid = reader.GetGuid(module.Mvid),
        };

        // token (int) → target keys it represents. Unlike the old per-target
        // pass, this maps every project-reference token once and caches the
        // reverse caller graph until the assembly changes on disk.
        var tokenTargets = new Dictionary<int, List<string>>();

        void AddToken(EntityHandle handle, string key)
        {
            int token = MetadataTokens.GetToken(handle);
            if (!tokenTargets.TryGetValue(token, out List<string>? keys))
                tokenTargets[token] = keys = new List<string>();
            if (!keys.Contains(key))
                keys.Add(key);
            index.TargetKeys.Add(key);
        }

        // External references (other project assemblies): every MemberRef is a
        // potential caller edge keyed by declaring type + member name. The
        // existing M3 scan is intentionally overload-insensitive; the cached
        // index keeps the same fail-closed over-approximation.
        foreach (MemberReferenceHandle memberRefHandle in reader.MemberReferences)
        {
            MemberReference memberRef = reader.GetMemberReference(memberRefHandle);
            string? parentType = ResolveParentTypeName(reader, memberRef.Parent);
            if (parentType == null)
                continue;

            string memberName = reader.GetString(memberRef.Name);
            AddToken(memberRefHandle, CallerScanTarget.Key(parentType, memberName));
            AddToken(memberRefHandle, CallerScanTarget.Key(parentType, ""));
        }

        // Type references (castclass/isinst/typeof of a deleted type).
        foreach (TypeReferenceHandle typeRefHandle in reader.TypeReferences)
        {
            string? name = TypeRefFullName(reader, typeRefHandle);
            if (name != null)
                AddToken(typeRefHandle, CallerScanTarget.Key(name, ""));
        }

        // Same-assembly references: direct MethodDef/FieldDef/TypeDef tokens.
        foreach (TypeDefinitionHandle typeDefHandle in reader.TypeDefinitions)
        {
            string typeName = TypeDefFullName(reader, typeDefHandle);
            TypeDefinition typeDef = reader.GetTypeDefinition(typeDefHandle);
            AddToken(typeDefHandle, CallerScanTarget.Key(typeName, ""));
            foreach (FieldDefinitionHandle fieldHandle in typeDef.GetFields())
                AddToken(fieldHandle, CallerScanTarget.Key(typeName, ""));
            foreach (MethodDefinitionHandle methodHandle in typeDef.GetMethods())
            {
                string methodName = reader.GetString(reader.GetMethodDefinition(methodHandle).Name);
                AddToken(methodHandle, CallerScanTarget.Key(typeName, methodName));
                AddToken(methodHandle, CallerScanTarget.Key(typeName, ""));
            }
        }

        // Generic METHOD call sites reference a MethodSpec token (the
        // instantiation), not the underlying MemberRef/MethodDef the loops
        // above registered: map every MethodSpec whose generic definition is
        // a target back to the same keys, otherwise calls like `Echo<int>(x)`
        // would scan as misses (fail-open).
        int methodSpecRows = reader.GetTableRowCount(TableIndex.MethodSpec);
        for (int row = 1; row <= methodSpecRows; row++)
        {
            MethodSpecificationHandle specHandle = MetadataTokens.MethodSpecificationHandle(row);
            MethodSpecification spec = reader.GetMethodSpecification(specHandle);
            if (tokenTargets.TryGetValue(MetadataTokens.GetToken(spec.Method), out List<string>? specKeys))
            {
                foreach (string key in specKeys)
                    AddToken(specHandle, key);
            }
        }

        if (tokenTargets.Count == 0)
            return index;

        // PDB up-front: fail closed BEFORE reporting hits without locations.
        using MetadataReaderProvider? pdbProvider = OpenPortablePdb(assemblyPath, peReader, out string? pdbError);
        if (pdbProvider == null)
        {
            index.Error = "cannot verify call sites: " + pdbError;
            return index;
        }
        MetadataReader pdbReader = pdbProvider.GetMetadataReader();

        foreach (TypeDefinitionHandle typeDefHandle in reader.TypeDefinitions)
        {
            TypeDefinition typeDef = reader.GetTypeDefinition(typeDefHandle);
            foreach (MethodDefinitionHandle methodHandle in typeDef.GetMethods())
            {
                MethodDefinition methodDef = reader.GetMethodDefinition(methodHandle);
                if (methodDef.RelativeVirtualAddress == 0)
                    continue;

                MethodBodyBlock body;
                try
                {
                    body = peReader.GetMethodBody(methodDef.RelativeVirtualAddress);
                }
                catch
                {
                    continue;
                }

                byte[]? il = body.GetILBytes();
                if (il == null)
                    continue;
                List<string>? hits = ScanIl(il, tokenTargets);
                if (hits == null)
                    continue;

                string? file = SourceFileOf(reader, pdbReader, methodHandle);
                string callerType = TypeDefFullName(reader, typeDefHandle);
                string callerName = reader.GetString(methodDef.Name);
                string callerKey = CallerMethodKey(reader, callerType, methodDef);
                foreach (string key in hits)
                {
                    // References from inside a deleted type don't count: the
                    // type goes away as a whole. The cached index is global,
                    // so apply that rule per emitted whole-type key.
                    if (string.Equals(key, CallerScanTarget.Key(callerType, ""), StringComparison.Ordinal))
                        continue;
                    if (file == null)
                    {
                        index.UnmappedByTarget.TryAdd(
                            key,
                            "cannot map a call site to its source file (no sequence points for " +
                            callerType + "." + callerName +
                            " in " + Path.GetFileName(assemblyPath) + ")");
                        continue;
                    }
                    if (!index.LocationsByTarget.TryGetValue(key, out List<CallerScanLocation>? locations))
                        index.LocationsByTarget[key] = locations = new List<CallerScanLocation>();
                    if (!locations.Any(existing =>
                            string.Equals(existing.File, file, StringComparison.OrdinalIgnoreCase) &&
                            string.Equals(existing.CallerMethodKey, callerKey, StringComparison.Ordinal)))
                    {
                        locations.Add(new CallerScanLocation
                        {
                            File = file,
                            CallerMethodKey = callerKey,
                            DeclaringType = callerType,
                            MemberName = callerName,
                        });
                    }
                }
            }
        }

        return index;
    }

    // ── IL walk ──────────────────────────────────────────────────────

    /// <summary>Token-operand opcodes whose operands we inspect.</summary>
    private static bool IsTokenOpcode(int opcode)
    {
        switch (opcode)
        {
            case 0x27: // jmp
            case 0x28: // call
            case 0x29: // calli (StandAloneSig — never a target, but 4-byte token)
            case 0x6F: // callvirt
            case 0x70: // cpobj
            case 0x71: // ldobj
            case 0x73: // newobj
            case 0x74: // castclass
            case 0x75: // isinst
            case 0x79: // unbox
            case 0x7B: // ldfld
            case 0x7C: // ldflda
            case 0x7D: // stfld
            case 0x7E: // ldsfld
            case 0x7F: // ldsflda
            case 0x80: // stsfld
            case 0x81: // stobj
            case 0x8C: // box
            case 0x8D: // newarr
            case 0x8F: // ldelema
            case 0xA3: // ldelem
            case 0xA4: // stelem
            case 0xA5: // unbox.any
            case 0xC2: // refanyval
            case 0xC6: // mkrefany
            case 0xD0: // ldtoken
            case 0xFE06: // ldftn
            case 0xFE07: // ldvirtftn
            case 0xFE15: // initobj
            case 0xFE16: // constrained.
            case 0xFE1C: // sizeof
                return true;
            default:
                return false;
        }
    }

    /// <summary>Operand byte size for non-token opcodes (token opcodes are 4).
    /// -1 marks the switch instruction (variable length).</summary>
    private static int OperandSize(int opcode)
    {
        switch (opcode)
        {
            case 0x0E: // ldarg.s
            case 0x0F: // ldarga.s
            case 0x10: // starg.s
            case 0x11: // ldloc.s
            case 0x12: // ldloca.s
            case 0x13: // stloc.s
            case 0x1F: // ldc.i4.s
            case 0xDE: // leave.s
            case 0xFE12: // unaligned.
                return 1;
            case 0xFE09: // ldarg
            case 0xFE0A: // ldarga
            case 0xFE0B: // starg
            case 0xFE0C: // ldloc
            case 0xFE0D: // ldloca
            case 0xFE0E: // stloc
                return 2;
            case 0x20: // ldc.i4
            case 0x22: // ldc.r4
            case 0x72: // ldstr
            case 0xDD: // leave
                return 4;
            case 0x21: // ldc.i8
            case 0x23: // ldc.r8
                return 8;
            case 0x45: // switch
                return -1;
            default:
                if (opcode >= 0x2B && opcode <= 0x37) // short branches
                    return 1;
                if (opcode >= 0x38 && opcode <= 0x44) // long branches
                    return 4;
                return 0;
        }
    }

    private static List<string>? ScanIl(byte[] il, Dictionary<int, List<string>> tokenTargets)
    {
        List<string>? hits = null;
        int i = 0;
        while (i < il.Length)
        {
            int opcode = il[i];
            i++;
            if (opcode == 0xFE)
            {
                if (i >= il.Length)
                    break;
                opcode = 0xFE00 | il[i];
                i++;
            }

            if (IsTokenOpcode(opcode))
            {
                if (i + 4 > il.Length)
                    break;
                int token = il[i] | (il[i + 1] << 8) | (il[i + 2] << 16) | (il[i + 3] << 24);
                i += 4;
                if (tokenTargets.TryGetValue(token, out List<string>? keys))
                {
                    hits ??= new List<string>();
                    foreach (string key in keys)
                    {
                        if (!hits.Contains(key))
                            hits.Add(key);
                    }
                }
                continue;
            }

            int size = OperandSize(opcode);
            if (size == -1) // switch: uint32 count + count * int32
            {
                if (i + 4 > il.Length)
                    break;
                int count = il[i] | (il[i + 1] << 8) | (il[i + 2] << 16) | (il[i + 3] << 24);
                i += 4 + count * 4;
                continue;
            }
            i += size;
        }
        return hits;
    }

    // ── name resolution ──────────────────────────────────────────────

    private static string? ResolveParentTypeName(MetadataReader reader, EntityHandle parent)
    {
        switch (parent.Kind)
        {
            case HandleKind.TypeReference:
                return TypeRefFullName(reader, (TypeReferenceHandle)parent);
            case HandleKind.TypeDefinition:
                return TypeDefFullName(reader, (TypeDefinitionHandle)parent);
            case HandleKind.TypeSpecification:
            {
                // Generic instantiation: GENERICINST CLASS|VALUETYPE TypeDefOrRef ...
                TypeSpecification spec = reader.GetTypeSpecification((TypeSpecificationHandle)parent);
                BlobReader blob = reader.GetBlobReader(spec.Signature);
                if (blob.RemainingBytes < 2)
                    return null;
                var typeCode = (SignatureTypeCode)blob.ReadCompressedInteger();
                if (typeCode != SignatureTypeCode.GenericTypeInstance)
                    return null;
                blob.ReadCompressedInteger(); // CLASS / VALUETYPE
                EntityHandle handle = blob.ReadTypeHandle();
                return handle.Kind switch
                {
                    HandleKind.TypeReference => TypeRefFullName(reader, (TypeReferenceHandle)handle),
                    HandleKind.TypeDefinition => TypeDefFullName(reader, (TypeDefinitionHandle)handle),
                    _ => null,
                };
            }
            default:
                return null;
        }
    }

    private static string? TypeRefFullName(MetadataReader reader, TypeReferenceHandle handle)
    {
        TypeReference typeRef = reader.GetTypeReference(handle);
        string name = reader.GetString(typeRef.Name);

        // Nested types chain through the resolution scope.
        if (typeRef.ResolutionScope.Kind == HandleKind.TypeReference)
        {
            string? outer = TypeRefFullName(reader, (TypeReferenceHandle)typeRef.ResolutionScope);
            return outer == null ? null : outer + "+" + name;
        }

        string ns = typeRef.Namespace.IsNil ? "" : reader.GetString(typeRef.Namespace);
        return ns.Length == 0 ? name : ns + "." + name;
    }

    private static string TypeDefFullName(MetadataReader reader, TypeDefinitionHandle handle)
    {
        TypeDefinition typeDef = reader.GetTypeDefinition(handle);
        string name = reader.GetString(typeDef.Name);
        TypeDefinitionHandle declaring = typeDef.GetDeclaringType();
        if (!declaring.IsNil)
            return TypeDefFullName(reader, declaring) + "+" + name;
        string ns = typeDef.Namespace.IsNil ? "" : reader.GetString(typeDef.Namespace);
        return ns.Length == 0 ? name : ns + "." + name;
    }

    /// <summary>
    /// Caller refresh only needs a stable method identity inside the source
    /// file, so use declaring type + metadata name + parameter COUNT +
    /// static/instance. When a source file has ambiguous overloads with the
    /// same count, the force-detour phase fails that file closed.
    /// </summary>
    private static string CallerMethodKey(MetadataReader reader, string declaringType, MethodDefinition method)
    {
        string name = reader.GetString(method.Name);
        int parameterCount = 0;
        try
        {
            BlobReader blob = reader.GetBlobReader(method.Signature);
            SignatureHeader header = blob.ReadSignatureHeader();
            if (header.IsGeneric)
                blob.ReadCompressedInteger();
            parameterCount = blob.ReadCompressedInteger();
        }
        catch
        {
            parameterCount = method.GetParameters().Count;
        }
        bool isStatic = (method.Attributes & System.Reflection.MethodAttributes.Static) != 0;
        return declaringType + "|" + name + "|" + parameterCount + (isStatic ? "|s" : "|i");
    }

    // ── PDB ──────────────────────────────────────────────────────────

    private static MetadataReaderProvider? OpenPortablePdb(string assemblyPath, PEReader peReader, out string? error)
    {
        error = null;

        // Embedded portable PDB first.
        foreach (DebugDirectoryEntry entry in peReader.ReadDebugDirectory())
        {
            if (entry.Type == DebugDirectoryEntryType.EmbeddedPortablePdb)
            {
                try
                {
                    return peReader.ReadEmbeddedPortablePdbDebugDirectoryData(entry);
                }
                catch (Exception ex)
                {
                    error = "embedded PDB unreadable for " + Path.GetFileName(assemblyPath) + ": " + ex.Message;
                    return null;
                }
            }
        }

        string pdbPath = Path.ChangeExtension(assemblyPath, ".pdb");
        if (!File.Exists(pdbPath))
        {
            error = "missing PDB for " + Path.GetFileName(assemblyPath) +
                " (call sites cannot be verified; use unity_recompile)";
            return null;
        }
        try
        {
            // Read fully into memory so the file handle is not held.
            byte[] bytes = File.ReadAllBytes(pdbPath);
            return MetadataReaderProvider.FromPortablePdbStream(new MemoryStream(bytes));
        }
        catch (Exception ex)
        {
            error = "PDB unreadable for " + Path.GetFileName(assemblyPath) + ": " + ex.Message;
            return null;
        }
    }

    private static string? SourceFileOf(MetadataReader reader, MetadataReader pdbReader, MethodDefinitionHandle methodHandle)
    {
        MethodDebugInformationHandle debugHandle = methodHandle.ToDebugInformationHandle();
        if (debugHandle.IsNil)
            return null;

        MethodDebugInformation debugInfo;
        try
        {
            debugInfo = pdbReader.GetMethodDebugInformation(debugHandle);
        }
        catch
        {
            return null;
        }

        DocumentHandle documentHandle = debugInfo.Document;
        if (documentHandle.IsNil)
        {
            // Methods spanning documents record per-sequence-point docs.
            foreach (SequencePoint point in debugInfo.GetSequencePoints())
            {
                if (!point.Document.IsNil)
                {
                    documentHandle = point.Document;
                    break;
                }
            }
        }
        if (documentHandle.IsNil)
            return null;

        Document document = pdbReader.GetDocument(documentHandle);
        return pdbReader.GetString(document.Name);
    }
}
