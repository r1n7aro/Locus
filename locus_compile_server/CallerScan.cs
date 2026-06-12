using System.Reflection.Metadata;
using System.Reflection.Metadata.Ecma335;
using System.Reflection.PortableExecutable;

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

    /// <summary>Fail-closed error (unreadable assembly, missing PDB): the
    /// caller must treat every target as unverifiable.</summary>
    public string? Error;
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
        foreach (CallerScanTarget target in targets)
            result.CallerFiles[CallerScanTarget.Key(target.DeclaringType, target.MemberName)] = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        var memberTargets = new HashSet<(string Type, string Member)>();
        var typeTargets = new HashSet<string>(StringComparer.Ordinal);
        foreach (CallerScanTarget target in targets)
        {
            if (string.IsNullOrEmpty(target.MemberName))
                typeTargets.Add(target.DeclaringType);
            else
                memberTargets.Add((target.DeclaringType, target.MemberName));
        }

        foreach (string assemblyPath in assemblyPaths)
        {
            try
            {
                ScanAssembly(assemblyPath, memberTargets, typeTargets, result);
            }
            catch (Exception ex)
            {
                result.Error = "call-site scan failed for " + Path.GetFileName(assemblyPath) + ": " + ex.Message;
                return result;
            }
            if (result.Error != null)
                return result;
        }

        return result;
    }

    private static void ScanAssembly(
        string assemblyPath,
        HashSet<(string Type, string Member)> memberTargets,
        HashSet<string> typeTargets,
        CallerScanResult result)
    {
        using FileStream stream = File.OpenRead(assemblyPath);
        using var peReader = new PEReader(stream);
        MetadataReader reader = peReader.GetMetadataReader();

        // token (int) → target keys it represents.
        var tokenTargets = new Dictionary<int, List<string>>();

        void AddToken(EntityHandle handle, string key)
        {
            int token = MetadataTokens.GetToken(handle);
            if (!tokenTargets.TryGetValue(token, out List<string>? keys))
                tokenTargets[token] = keys = new List<string>();
            if (!keys.Contains(key))
                keys.Add(key);
        }

        // External references (other project assemblies): MemberRefs whose
        // parent TypeRef matches a target type.
        foreach (MemberReferenceHandle memberRefHandle in reader.MemberReferences)
        {
            MemberReference memberRef = reader.GetMemberReference(memberRefHandle);
            string? parentType = ResolveParentTypeName(reader, memberRef.Parent);
            if (parentType == null)
                continue;

            string memberName = reader.GetString(memberRef.Name);
            if (memberTargets.Contains((parentType, memberName)))
                AddToken(memberRefHandle, CallerScanTarget.Key(parentType, memberName));
            if (typeTargets.Contains(parentType))
                AddToken(memberRefHandle, CallerScanTarget.Key(parentType, ""));
        }

        // Type references (castclass/isinst/typeof of a deleted type).
        if (typeTargets.Count > 0)
        {
            foreach (TypeReferenceHandle typeRefHandle in reader.TypeReferences)
            {
                string? name = TypeRefFullName(reader, typeRefHandle);
                if (name != null && typeTargets.Contains(name))
                    AddToken(typeRefHandle, CallerScanTarget.Key(name, ""));
            }
        }

        // Same-assembly references: direct MethodDef/FieldDef/TypeDef tokens.
        var selfTargetTypes = new HashSet<TypeDefinitionHandle>();
        foreach (TypeDefinitionHandle typeDefHandle in reader.TypeDefinitions)
        {
            string typeName = TypeDefFullName(reader, typeDefHandle);
            bool wholeType = typeTargets.Contains(typeName);
            bool hasMembers = memberTargets.Any(t => t.Type == typeName);
            if (!wholeType && !hasMembers)
                continue;

            TypeDefinition typeDef = reader.GetTypeDefinition(typeDefHandle);
            if (wholeType)
            {
                selfTargetTypes.Add(typeDefHandle);
                AddToken(typeDefHandle, CallerScanTarget.Key(typeName, ""));
                foreach (FieldDefinitionHandle fieldHandle in typeDef.GetFields())
                    AddToken(fieldHandle, CallerScanTarget.Key(typeName, ""));
            }
            foreach (MethodDefinitionHandle methodHandle in typeDef.GetMethods())
            {
                string methodName = reader.GetString(reader.GetMethodDefinition(methodHandle).Name);
                if (memberTargets.Contains((typeName, methodName)))
                    AddToken(methodHandle, CallerScanTarget.Key(typeName, methodName));
                if (wholeType)
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
            return;

        // PDB up-front: fail closed BEFORE reporting hits without locations.
        using MetadataReaderProvider? pdbProvider = OpenPortablePdb(assemblyPath, peReader, out string? pdbError);
        if (pdbProvider == null)
        {
            result.Error = "cannot verify call sites: " + pdbError;
            return;
        }
        MetadataReader pdbReader = pdbProvider.GetMetadataReader();

        foreach (TypeDefinitionHandle typeDefHandle in reader.TypeDefinitions)
        {
            // References from inside a deleted type don't count: the type
            // goes away as a whole.
            if (selfTargetTypes.Contains(typeDefHandle))
                continue;

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
                if (file == null)
                {
                    result.Error =
                        "cannot map a call site to its source file (no sequence points for " +
                        TypeDefFullName(reader, typeDefHandle) + "." + reader.GetString(methodDef.Name) +
                        " in " + Path.GetFileName(assemblyPath) + ")";
                    return;
                }

                foreach (string key in hits)
                {
                    if (result.CallerFiles.TryGetValue(key, out HashSet<string>? files))
                        files.Add(file);
                }
            }
        }
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
