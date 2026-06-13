using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Locus.CompileServer;

// ── result models ────────────────────────────────────────────────────

/// <summary>One method (or accessor) the Unity side must redirect.</summary>
public sealed class HotDiffMethod
{
    /// <summary>Original type, CLR metadata style: "Ns.Outer+Inner".</summary>
    public string DeclaringType = "";

    /// <summary>Metadata member name: "Update", "get_Health", ".ctor", "op_Addition".</summary>
    public string Name = "";

    /// <summary>Reflection simple type names per parameter ("Int32", "String[]", "List`1", "Int32&" for ref/out/in).</summary>
    public string[] ParamTypeNames = Array.Empty<string>();

    public bool IsStatic;
    public bool IsCtor;

    /// <summary>Member exists only in the new text: compiled into the patch
    /// assembly but never detoured (only patched bodies can call it).</summary>
    public bool Added;

    /// <summary>The method's OWN generic arity (added members): disambiguates
    /// same-name-same-params generic/non-generic overloads when the shim
    /// declaration is located. Not part of the wire protocol.</summary>
    public int TypeParameterCount;
}

/// <summary>A member surface change that is hot only when every call site of
/// the OLD surface lives inside the current batch — the M3 caller scan
/// decides. While the scan is unavailable these map to cold.</summary>
public sealed class CallerCheckMember
{
    public string DeclaringType = "";

    /// <summary>Metadata member name ("M", "get_X"); empty for whole-type checks.</summary>
    public string Name = "";

    public string[] ParamTypeNames = Array.Empty<string>();

    /// <summary>"accessibility-narrowed" | "member-removed" | "signature-changed" | "type-removed".</summary>
    public string Kind = "";

    /// <summary>Human-readable description for cold reasons / tool output.</summary>
    public string Detail = "";

    /// <summary>Metadata member names the IL scan must look for (accessor
    /// pairs for properties/events); empty = scan the whole type.</summary>
    public string[] ScanMemberNames = Array.Empty<string>();
}

/// <summary>A member that exists only in the OLD text (M5 deletion, or the
/// "remove" half of a signature change). The original stays in place —
/// unreachable compiled code is harmless and in-flight delegates/coroutines
/// are legitimate callers — except Unity message methods, which the engine
/// keeps calling every frame: those get an empty-body stub detour.</summary>
public sealed class HotDiffRemovedMember
{
    public string DeclaringType = "";
    public string Name = "";
    public string[] ParamTypeNames = Array.Empty<string>();
    public bool IsStatic;
    public bool IsUnityMagic;

    /// <summary>For Unity message methods: the member declaration with an
    /// emptied body, ready to re-materialize in the patch type as the detour
    /// target that silences the engine's per-frame calls.</summary>
    public string? StubSource;
}

/// <summary>One virtualizable field change (M4): a plain instance/static
/// field added or removed (a retype is a remove+add pair). Added instance
/// fields live in a per-field LocusFieldStore keyed by the instance; added
/// static fields live in a patch-local holder class; removed instance
/// fields re-materialize as layout placeholders in the patch type.</summary>
public sealed class HotDiffFieldChange
{
    public string DeclaringType = "";
    public string Name = "";

    /// <summary>"added" | "removed".</summary>
    public string Kind = "";

    public bool IsStatic;

    /// <summary>Index among the OLD type's instance fields (removed
    /// instance fields) — the placeholder re-injection position.</summary>
    public int OldFieldIndex;
}

/// <summary>An enum member appended by the edit, with its resolved constant
/// value (H7e). Batch references compile as cast literals; Enum.Parse /
/// ToString only learn the name at the convergence recompile.</summary>
public sealed class HotDiffEnumAddition
{
    public string EnumType = "";
    public string MemberName = "";
    public long Value;
}

/// <summary>A type that exists only in the OLD text (file deletion / type
/// removal). The loaded type stays — in-flight references are legitimate —
/// but its Unity message methods detour to empty stubs in a synthesized
/// patch class so the engine stops driving it (M5/H7e).</summary>
public sealed class HotDiffRemovedType
{
    public string MetadataName = "";

    /// <summary>Ready-to-parse compilation-unit text declaring the stub
    /// class (old file's usings + namespace + empty magic methods); null
    /// when the type has no Unity message methods.</summary>
    public string? StubSource;

    /// <summary>Metadata name of the stub class declared by StubSource.</summary>
    public string StubTypeMetadataName = "";

    /// <summary>The magic methods stubbed by StubSource (detour mappings).</summary>
    public List<HotDiffRemovedMember> MagicMethods = new();
}

/// <summary>Hot/cold classification of one edited file.</summary>
public sealed class HotDiffFileResult
{
    public bool Hot;

    /// <summary>Cold reasons (empty when hot).</summary>
    public List<string> Reasons = new();

    /// <summary>Member surfaces that must pass the call-site scan (M3)
    /// before this file can be hot. `Hot` stays true for these — the
    /// compile/hotPatch pipeline runs the scan and folds the verdict into
    /// hot/cold; analyze/hotDiff surfaces the list for transparency.</summary>
    public List<CallerCheckMember> RequiresCallerCheck = new();

    /// <summary>Members that exist only in the old text (M5).</summary>
    public List<HotDiffRemovedMember> RemovedMembers = new();

    /// <summary>Virtualizable field additions/removals (M4).</summary>
    public List<HotDiffFieldChange> FieldChanges = new();

    /// <summary>Enum members appended by this edit (values resolved): batch
    /// references materialize as `(EnumType)value` literals.</summary>
    public List<HotDiffEnumAddition> EnumAdditions = new();

    /// <summary>Whole types that exist only in the old text (file deletion
    /// or type removal), kept hot via TypeRef-level caller scan + tombstone
    /// + magic-method stub class.</summary>
    public List<HotDiffRemovedType> RemovedTypes = new();

    /// <summary>Deterministic agent-facing error: the new text does not even
    /// parse. Not a cold reason — recompiling would fail identically.</summary>
    public string? SyntaxError;

    public List<HotDiffMethod> ChangedMethods = new();

    /// <summary>Metadata full names of types that exist only in the new text.</summary>
    public List<string> NewTypes = new();

    /// <summary>Metadata full names of pre-existing types with hot member
    /// changes — the rename set for the patch rewriter.</summary>
    public List<string> PatchedTypes = new();
}

// ── analysis ─────────────────────────────────────────────────────────

/// <summary>
/// Syntax-level member diff that decides whether an edited file can take the
/// hot-patch path (method/accessor/constructor body changes, new private
/// members, new types) or must go through a real Unity recompile (anything
/// that changes signatures, field layout, inlined constants, or type shape).
/// Pure function of (oldText, newText, parseOptions); no compilation.
/// </summary>
public static class HotDiff
{
    public static HotDiffFileResult Analyze(string oldText, string newText, CSharpParseOptions parseOptions)
    {
        var result = new HotDiffFileResult();

        SyntaxTree oldTree = CSharpSyntaxTree.ParseText(oldText, parseOptions);
        SyntaxTree newTree = CSharpSyntaxTree.ParseText(newText, parseOptions);

        var newErrors = newTree.GetDiagnostics()
            .Where(d => d.Severity == DiagnosticSeverity.Error)
            .ToList();
        if (newErrors.Count > 0)
        {
            result.SyntaxError = DiagnosticText.BuildDiagnosticErrorText(newErrors)
                ?? "new text does not parse";
            return result;
        }

        if (oldTree.GetDiagnostics().Any(d => d.Severity == DiagnosticSeverity.Error))
        {
            result.Reasons.Add("baseline text does not parse");
            return result;
        }

        var oldRoot = (CompilationUnitSyntax)oldTree.GetRoot();
        var newRoot = (CompilationUnitSyntax)newTree.GetRoot();

        // Assembly/module-level attributes change compiled metadata that no
        // detour can reproduce.
        if (AttributeListsText(oldRoot.AttributeLists) != AttributeListsText(newRoot.AttributeLists))
        {
            result.Reasons.Add("assembly-level attributes changed");
            return result;
        }
        // using/extern changes are NOT cold by themselves (M6): the risk is
        // unchanged members whose patch copies would bind differently under
        // the new directives — handled below by re-detouring the whole file.
        bool usingChanged = UsingAndExternText(oldRoot) != UsingAndExternText(newRoot);
        if (DelegateDeclarationsText(oldRoot) != DelegateDeclarationsText(newRoot))
        {
            result.Reasons.Add("delegate declarations changed");
            return result;
        }

        Dictionary<string, List<BaseTypeDeclarationSyntax>> oldTypes = CollectTypes(oldRoot, result.Reasons);
        Dictionary<string, List<BaseTypeDeclarationSyntax>> newTypes = CollectTypes(newRoot, result.Reasons);
        if (result.Reasons.Count > 0)
            return result;

        foreach (string removed in oldTypes.Keys.Where(k => !newTypes.ContainsKey(k)).OrderBy(k => k, StringComparer.Ordinal))
        {
            // B6: a vanished PARTIAL declaration is not a type removal — the
            // type usually lives on through its other parts (other files or
            // a generator), and the M5 tombstone/stub machinery would treat
            // a still-alive type as deleted.
            if (IsPartialParts(oldTypes[removed]))
            {
                result.Reasons.Add("partial type part removed: " + removed +
                    " (other parts may still declare the type; use unity_recompile)");
                return result;
            }
            ClassifyRemovedType(removed, oldTypes[removed][0], oldRoot, result);
            if (result.Reasons.Count > 0)
                return result;
        }

        foreach (var pair in newTypes)
        {
            if (oldTypes.ContainsKey(pair.Key))
                continue;
            // B6: a NEW partial declaration is ambiguous from one file — it
            // may be a part added to an EXISTING type (a layout/member merge
            // no patch can express) or a brand-new type whose other parts
            // (disk or generator) are invisible here. Fail closed (v1).
            if (IsPartialParts(pair.Value))
            {
                result.Reasons.Add("new partial type declaration: " + pair.Key +
                    " (other parts may exist on disk or come from a source generator; use unity_recompile)");
                return result;
            }
            result.NewTypes.Add(pair.Key);
        }

        foreach (var pair in newTypes)
        {
            if (!oldTypes.TryGetValue(pair.Key, out List<BaseTypeDeclarationSyntax>? oldParts))
                continue;

            DiffType(pair.Key, oldParts, pair.Value, result);
            if (result.Reasons.Count > 0)
                return result;
        }

        if (usingChanged)
        {
            // M6: re-detour every detourable member of every pre-existing
            // type so the whole file switches to the new binding semantics
            // together. Members whose compiled form cannot be re-detoured —
            // or whose already-materialized values cannot follow the new
            // directives — fail the file closed first.
            foreach (var pair in newTypes)
            {
                if (!oldTypes.ContainsKey(pair.Key))
                    continue;
                // B6 (v1): the whole-file re-detour cannot reason about
                // members declared by OTHER parts (their files keep the old
                // directives, yet initializers/ctors interleave with this
                // part's members in the compiled type).
                if (IsPartialParts(pair.Value))
                {
                    result.ChangedMethods.Clear();
                    result.PatchedTypes.Clear();
                    result.Reasons.Add("using directives changed in a file with a partial type: " +
                        pair.Key + " (the whole-file re-detour cannot cover the other parts; use unity_recompile)");
                    return result;
                }
                if (pair.Value[0] is not TypeDeclarationSyntax type)
                    continue;
                string? gate = UsingRehookGateReason(pair.Key, type);
                if (gate != null)
                {
                    result.ChangedMethods.Clear();
                    result.PatchedTypes.Clear();
                    result.Reasons.Add(gate);
                    return result;
                }
            }
            foreach (var pair in newTypes)
            {
                if (!oldTypes.ContainsKey(pair.Key) || pair.Value[0] is not TypeDeclarationSyntax type)
                    continue;
                if (type is InterfaceDeclarationSyntax)
                    continue; // gate guarantees no default implementations
                AddRehookMembers(pair.Key, type, result);
            }
        }

        result.NewTypes.Sort(StringComparer.Ordinal);
        result.PatchedTypes = result.PatchedTypes.Distinct().OrderBy(t => t, StringComparer.Ordinal).ToList();
        result.Hot = true;
        return result;
    }

    // ── type collection ──────────────────────────────────────────────

    /// <summary>One metadata type name → its declarations in THIS file, in
    /// source order. Non-partial types always have exactly one part; partial
    /// types (B6) may have several in one file — and further parts in OTHER
    /// files, which the batch pulls in as baseline siblings (the per-file
    /// diff only ever speaks about the members this file declares).</summary>
    private static Dictionary<string, List<BaseTypeDeclarationSyntax>> CollectTypes(
        CompilationUnitSyntax root,
        List<string> reasons)
    {
        var types = new Dictionary<string, List<BaseTypeDeclarationSyntax>>(StringComparer.Ordinal);

        foreach (BaseTypeDeclarationSyntax decl in root.DescendantNodes().OfType<BaseTypeDeclarationSyntax>())
        {
            if (decl is RecordDeclarationSyntax)
            {
                reasons.Add("record types are not hot-reloadable: " + decl.Identifier.Text);
                return types;
            }

            string metadataName = MetadataName(decl);
            if (!types.TryGetValue(metadataName, out List<BaseTypeDeclarationSyntax>? parts))
                types[metadataName] = parts = new List<BaseTypeDeclarationSyntax>();
            parts.Add(decl);
        }

        // Same-name declarations are only legal as partial parts; a duplicate
        // without the modifier would not compile (CS0101/CS0260) — fail
        // closed so a real compile surfaces the error.
        foreach (var pair in types)
        {
            if (pair.Value.Count > 1 &&
                pair.Value.Any(d => !d.Modifiers.Any(SyntaxKind.PartialKeyword)))
            {
                reasons.Add("duplicate type declaration: " + pair.Key);
                return types;
            }
        }

        return types;
    }

    /// <summary>Any part declared `partial` (a single partial declaration
    /// counts: the other parts may live in other files or a generator).</summary>
    private static bool IsPartialParts(List<BaseTypeDeclarationSyntax> parts) =>
        parts.Any(d => d.Modifiers.Any(SyntaxKind.PartialKeyword));

    /// <summary>"Ns.Sub.Outer+Inner`1" — CLR metadata naming.</summary>
    internal static string MetadataName(BaseTypeDeclarationSyntax decl)
    {
        var nesting = new List<string>();
        string name = decl.Identifier.Text;
        int arity = (decl as TypeDeclarationSyntax)?.TypeParameterList?.Parameters.Count ?? 0;
        nesting.Add(arity > 0 ? name + "`" + arity : name);

        SyntaxNode? current = decl.Parent;
        var namespaces = new List<string>();
        while (current != null)
        {
            switch (current)
            {
                case TypeDeclarationSyntax outer:
                    int outerArity = outer.TypeParameterList?.Parameters.Count ?? 0;
                    nesting.Add(outerArity > 0 ? outer.Identifier.Text + "`" + outerArity : outer.Identifier.Text);
                    break;
                case BaseNamespaceDeclarationSyntax ns:
                    namespaces.Add(ns.Name.ToString());
                    break;
            }
            current = current.Parent;
        }

        nesting.Reverse();
        namespaces.Reverse();

        string typePart = string.Join("+", nesting);
        return namespaces.Count == 0 ? typePart : string.Join(".", namespaces) + "." + typePart;
    }

    // ── per-type diff ────────────────────────────────────────────────

    private static void DiffType(
        string metadataName,
        List<BaseTypeDeclarationSyntax> oldParts,
        List<BaseTypeDeclarationSyntax> newParts,
        HotDiffFileResult result)
    {
        // B6: a part added to (or dropped from) THIS file changes how the
        // compiler interleaves the type's members — no per-file diff can
        // verify the result. Matching counts pair parts by source order.
        if (oldParts.Count != newParts.Count)
        {
            result.Reasons.Add("partial type part count changed in file: " + metadataName +
                " (parts cannot be hot-added or hot-removed; use unity_recompile)");
            return;
        }
        for (int i = 0; i < oldParts.Count; i++)
        {
            if (oldParts[i].RawKind != newParts[i].RawKind)
            {
                result.Reasons.Add("type kind changed: " + metadataName);
                return;
            }
        }

        // Enum values are inlined constants: changes/removals/reorders are
        // unverifiable (no metadata trace) and stay cold. APPEND-ONLY
        // additions are safe — existing code never references them — and
        // batch references materialize as cast literals (H7e). Enums cannot
        // be partial, so both sides are single-part here.
        if (oldParts[0] is EnumDeclarationSyntax || newParts[0] is EnumDeclarationSyntax)
        {
            if (TokenText(oldParts[0]) == TokenText(newParts[0]))
                return;
            DiffEnum(metadataName, (EnumDeclarationSyntax)oldParts[0], (EnumDeclarationSyntax)newParts[0], result);
            return;
        }

        // Interfaces: default-implementation bodies dispatch through the
        // IMT, where Mono detour reliability is unverified; signature-only
        // members are pure type surface. Any interface change stays cold.
        if (oldParts[0] is InterfaceDeclarationSyntax)
        {
            for (int i = 0; i < oldParts.Count; i++)
            {
                if (TokenText(oldParts[i]) != TokenText(newParts[i]))
                {
                    result.Reasons.Add(
                        "interface changed: " + metadataName +
                        " (interface dispatch cannot be hot-patched)");
                    return;
                }
            }
            return;
        }

        List<TypeDeclarationSyntax> oldTypeParts = oldParts.Cast<TypeDeclarationSyntax>().ToList();
        List<TypeDeclarationSyntax> newTypeParts = newParts.Cast<TypeDeclarationSyntax>().ToList();

        for (int i = 0; i < oldTypeParts.Count; i++)
        {
            if (TypeHeaderText(oldTypeParts[i]) != TypeHeaderText(newTypeParts[i]))
            {
                result.Reasons.Add("type declaration changed: " + metadataName);
                return;
            }
        }

        bool isPartial = IsPartialParts(oldParts) || IsPartialParts(newParts);
        // Generic/Burst context comes from the REAL parts (parent chains
        // intact); the merged views below are detached synthetic nodes.
        bool genericContext = newTypeParts.Any(IsGenericContext);
        bool burstContext = oldTypeParts.Any(HasBurstCompileAttribute) || newTypeParts.Any(HasBurstCompileAttribute);

        // B6: same-file parts merge into ONE member-level view (members
        // concatenated in source order) so every member diff below sees the
        // whole in-file surface. Members of OTHER files' parts are out of
        // scope by construction: they are unchanged baselines.
        TypeDeclarationSyntax oldType = MergeParts(oldTypeParts);
        TypeDeclarationSyntax newType = MergeParts(newTypeParts);

        // M4: plain field add/remove/retype virtualizes through stores;
        // everything else layout-shaped stays cold.
        int fieldChangesBefore = result.FieldChanges.Count;
        if (!DiffFieldLayout(metadataName, oldType, newType, genericContext, isPartial, result))
            return;
        bool fieldsChanged = result.FieldChanges.Count > fieldChangesBefore;

        // Constants are inlined at every use site; a recompile is the only
        // way to update consumers. Static initializers ran with the original
        // domain's static constructor and would silently not re-run.
        // Compared per-name so M4 static field additions/removals pass.
        if (!DiffConstAndStaticInit(metadataName, oldType, newType, result))
            return;

        bool instanceInitChanged = InstanceInitText(oldType) != InstanceInitText(newType);

        Dictionary<string, MemberDeclarationSyntax> oldMembers = ExecutableMembers(oldType, result.Reasons, metadataName);
        Dictionary<string, MemberDeclarationSyntax> newMembers = ExecutableMembers(newType, result.Reasons, metadataName);
        if (result.Reasons.Count > 0)
            return;

        bool patched = false;

        foreach (var pair in oldMembers)
        {
            if (newMembers.ContainsKey(pair.Key))
                continue;
            int before = result.RemovedMembers.Count;
            if (!ClassifyRemovedMember(metadataName, pair.Value, genericContext, burstContext, result))
                return;
            // A removed Unity message method re-materializes as an
            // empty-body stub in the patch type (the engine keeps calling
            // it every frame), which makes the type a patched type.
            for (int i = before; i < result.RemovedMembers.Count; i++)
            {
                if (result.RemovedMembers[i].IsUnityMagic)
                    patched = true;
            }
        }

        foreach (var pair in newMembers)
        {
            MemberDeclarationSyntax newMember = pair.Value;

            if (!oldMembers.TryGetValue(pair.Key, out MemberDeclarationSyntax? oldMember))
            {
                if (!ClassifyAddedMember(metadataName, newMember, genericContext, burstContext, result))
                    return;
                patched = true;
                continue;
            }

            int reasonsBefore = result.Reasons.Count;
            bool changed = DiffMember(metadataName, oldMember, newMember, genericContext, burstContext, result);
            if (result.Reasons.Count > reasonsBefore)
                return;
            patched |= changed;
        }

        if (instanceInitChanged)
        {
            if (newType is StructDeclarationSyntax)
            {
                result.Reasons.Add("struct initializer changed: " + metadataName);
                return;
            }
            if (genericContext)
            {
                result.Reasons.Add("generic type initializer changed: " + metadataName);
                return;
            }
            if (isPartial)
            {
                // B6 (v1): initializers compile into every non-chained
                // constructor — and a partial type's constructors (or the
                // lack of an explicit one) can only be seen across ALL
                // parts, which this per-file diff cannot.
                result.Reasons.Add("partial type instance initializer changed: " + metadataName +
                    " (initializers compile into constructors that may live in other parts; use unity_recompile)");
                return;
            }

            // Instance field/auto-property initializers compile into every
            // non-chained constructor: redirect them all (or the implicit
            // default constructor when none is declared).
            var ctors = newType.Members.OfType<ConstructorDeclarationSyntax>()
                .Where(c => !c.Modifiers.Any(SyntaxKind.StaticKeyword))
                .ToList();
            if (ctors.Count == 0)
            {
                AddMethod(result, metadataName, ".ctor", Array.Empty<string>(), isStatic: false, isCtor: true, added: false);
            }
            else
            {
                foreach (ConstructorDeclarationSyntax ctor in ctors)
                    AddMethod(result, metadataName, ".ctor", ParamTypeNames(ctor.ParameterList), isStatic: false, isCtor: true, added: false);
            }
            patched = true;
        }

        patched |= fieldsChanged;

        if (patched)
            result.PatchedTypes.Add(metadataName);
    }

    /// <summary>Single member-level view of same-file partial parts: the
    /// first part with every part's members concatenated in source order.
    /// DETACHED synthetic node when merging really happened — callers must
    /// not walk its parents (generic/Burst context is computed from the real
    /// parts before merging).</summary>
    private static TypeDeclarationSyntax MergeParts(List<TypeDeclarationSyntax> parts)
    {
        if (parts.Count == 1)
            return parts[0];
        return parts[0].WithMembers(SyntaxFactory.List(parts.SelectMany(p => p.Members)));
    }

    // ── enum additions (H7e) ─────────────────────────────────────────

    /// <summary>Allow strictly-appended enum members with resolvable,
    /// non-conflicting values; anything else stays cold.</summary>
    private static void DiffEnum(
        string metadataName,
        EnumDeclarationSyntax oldEnum,
        EnumDeclarationSyntax newEnum,
        HotDiffFileResult result)
    {
        // Header (attributes/modifiers/base) must be identical.
        string OldHeader(EnumDeclarationSyntax e) => string.Join(
            "|",
            AttributeListsText(e.AttributeLists),
            ModifiersText(e.Modifiers),
            e.Identifier.Text,
            e.BaseList != null ? TokenText(e.BaseList) : "");
        if (OldHeader(oldEnum) != OldHeader(newEnum))
        {
            result.Reasons.Add("enum changed: " + metadataName);
            return;
        }

        var oldMembers = oldEnum.Members.ToList();
        var newMembers = newEnum.Members.ToList();
        if (newMembers.Count <= oldMembers.Count)
        {
            result.Reasons.Add("enum changed: " + metadataName +
                " (values are inlined; only appended members are hot)");
            return;
        }
        for (int i = 0; i < oldMembers.Count; i++)
        {
            if (TokenText(oldMembers[i]) != TokenText(newMembers[i]))
            {
                result.Reasons.Add("enum changed: " + metadataName +
                    " (values are inlined; only appended members are hot)");
                return;
            }
        }

        // Resolve every member's value (literal / +1 chains). Unresolvable
        // EXISTING values only matter when an appended member's value
        // depends on them or could collide with them.
        long? current = null;
        bool anyExistingUnresolvable = false;
        var existingValues = new HashSet<long>();
        var additions = new List<HotDiffEnumAddition>();
        for (int i = 0; i < newMembers.Count; i++)
        {
            EnumMemberDeclarationSyntax member = newMembers[i];
            long? value = member.EqualsValue != null
                ? ResolveEnumLiteral(member.EqualsValue.Value)
                : (i == 0 ? 0 : current + 1); // null propagates from an unresolvable predecessor
            current = value;

            if (i < oldMembers.Count)
            {
                if (value is { } existing)
                    existingValues.Add(existing);
                else
                    anyExistingUnresolvable = true;
                continue;
            }

            if (value is not { } added)
            {
                result.Reasons.Add("enum member value not resolvable: " + metadataName + "." +
                    member.Identifier.Text + " (use an integer literal, or unity_recompile)");
                return;
            }
            if (existingValues.Contains(added) || anyExistingUnresolvable)
            {
                result.Reasons.Add("enum member value conflicts with (or cannot be checked against) " +
                    "an existing member: " + metadataName + "." + member.Identifier.Text);
                return;
            }
            additions.Add(new HotDiffEnumAddition
            {
                EnumType = metadataName,
                MemberName = member.Identifier.Text,
                Value = added,
            });
        }

        result.EnumAdditions.AddRange(additions);
    }

    private static long? ResolveEnumLiteral(ExpressionSyntax expression)
    {
        switch (expression)
        {
            case LiteralExpressionSyntax literal when literal.IsKind(SyntaxKind.NumericLiteralExpression):
                return literal.Token.Value switch
                {
                    int i => i,
                    long l => l,
                    uint u => u,
                    short s => s,
                    ushort us => us,
                    byte b => b,
                    sbyte sb => sb,
                    ulong ul when ul <= long.MaxValue => (long)ul,
                    _ => null,
                };
            case PrefixUnaryExpressionSyntax unary when unary.IsKind(SyntaxKind.UnaryMinusExpression):
                return -ResolveEnumLiteral(unary.Operand);
            case ParenthesizedExpressionSyntax parens:
                return ResolveEnumLiteral(parens.Expression);
            default:
                return null;
        }
    }

    // ── removed types (H7e) ──────────────────────────────────────────

    /// <summary>Classify a type that exists only in the old text. Hot path:
    /// TypeRef-level caller check + tombstone + (for classes) empty stubs
    /// for every Unity message method, so the engine stops driving scene
    /// instances immediately. Enums stay cold (inlined values).</summary>
    private static void ClassifyRemovedType(
        string metadataName,
        BaseTypeDeclarationSyntax oldDecl,
        CompilationUnitSyntax oldRoot,
        HotDiffFileResult result)
    {
        if (oldDecl is EnumDeclarationSyntax)
        {
            result.Reasons.Add("enum removed: " + metadataName +
                " (enum values are inlined at use sites and cannot be verified; use unity_recompile)");
            return;
        }

        var removedType = new HotDiffRemovedType { MetadataName = metadataName };

        if (oldDecl is ClassDeclarationSyntax oldClass && !IsGenericContext(oldClass))
        {
            var stubs = new List<MethodDeclarationSyntax>();
            foreach (MethodDeclarationSyntax method in oldClass.Members.OfType<MethodDeclarationSyntax>())
            {
                if (!UnityMagicMethods.Contains(method.Identifier.Text))
                    continue;
                if (method.Body == null && method.ExpressionBody == null)
                    continue;
                if ((method.TypeParameterList?.Parameters.Count ?? 0) > 0)
                    continue;

                stubs.Add(method
                    .WithAttributeLists(default)
                    .WithBody(SyntaxFactory.Block())
                    .WithExpressionBody(null)
                    .WithSemicolonToken(default));
                removedType.MagicMethods.Add(new HotDiffRemovedMember
                {
                    DeclaringType = metadataName,
                    Name = method.Identifier.Text,
                    ParamTypeNames = ParamTypeNames(method.ParameterList),
                    IsStatic = method.Modifiers.Any(SyntaxKind.StaticKeyword),
                    IsUnityMagic = true,
                });
            }

            if (stubs.Count > 0)
            {
                removedType.StubSource = BuildRemovedTypeStubSource(
                    metadataName, oldRoot, stubs, out string stubMetadataName);
                removedType.StubTypeMetadataName = stubMetadataName;
            }
        }

        result.RemovedTypes.Add(removedType);
        result.RequiresCallerCheck.Add(new CallerCheckMember
        {
            DeclaringType = metadataName,
            Name = "",
            Kind = "type-removed",
            Detail = "type removed: " + metadataName,
            ScanMemberNames = Array.Empty<string>(),
        });
    }

    /// <summary>Self-contained compilation unit declaring the stub class
    /// (`T__LocusPatch` with empty magic methods), carrying the old file's
    /// usings so Unity parameter types resolve.</summary>
    private static string BuildRemovedTypeStubSource(
        string metadataName,
        CompilationUnitSyntax oldRoot,
        List<MethodDeclarationSyntax> stubs,
        out string stubMetadataName)
    {
        int plus = metadataName.IndexOf('+');
        string topLevel = plus < 0 ? metadataName : metadataName[..plus];
        int dot = topLevel.LastIndexOf('.');
        string ns = dot < 0 ? "" : topLevel[..dot];
        // Nested classes synthesize a FLAT stub class named after the full
        // chain — the stub never interacts with the original's nesting.
        string className = (plus < 0 ? topLevel[(dot + 1)..] : metadataName[(dot + 1)..].Replace('+', '_'))
            + "__LocusStub";
        stubMetadataName = ns.Length == 0 ? className : ns + "." + className;

        var sb = new System.Text.StringBuilder();
        foreach (UsingDirectiveSyntax usingDirective in oldRoot.Usings)
            sb.AppendLine(usingDirective.NormalizeWhitespace().ToFullString().Trim());
        if (ns.Length > 0)
            sb.AppendLine("namespace " + ns + " {");
        sb.AppendLine("internal sealed class " + className);
        sb.AppendLine("{");
        foreach (MethodDeclarationSyntax stub in stubs)
            sb.AppendLine("    " + stub.NormalizeWhitespace().ToFullString().Trim());
        sb.AppendLine("}");
        if (ns.Length > 0)
            sb.AppendLine("}");
        return sb.ToString();
    }

    // ── field-level layout diff (M4) ─────────────────────────────────

    private sealed class FieldDeclaratorEntry
    {
        public string Name = "";
        public string TypeText = "";
        public string ModifiersText = "";
        public string AttributesText = "";
        public int Index;
    }

    /// <summary>Diff the layout-affecting members. Plain instance/static
    /// FIELD additions, removals and retypes become FieldChanges (hot via
    /// store virtualization); auto-property/event-field changes, kept-field
    /// reorders, modifier/attribute changes, and any field change on a
    /// struct, generic or partial (B6 v1) type stay cold. Returns false on
    /// cold.</summary>
    private static bool DiffFieldLayout(
        string metadataName,
        TypeDeclarationSyntax oldType,
        TypeDeclarationSyntax newType,
        bool genericContext,
        bool isPartial,
        HotDiffFileResult result)
    {
        List<FieldDeclaratorEntry> oldFields = InstanceFieldEntries(oldType);
        List<FieldDeclaratorEntry> newFields = InstanceFieldEntries(newType);

        var oldByName = oldFields.ToDictionary(f => f.Name, StringComparer.Ordinal);
        var newByName = newFields.ToDictionary(f => f.Name, StringComparer.Ordinal);

        var added = new List<FieldDeclaratorEntry>();
        var removed = new List<FieldDeclaratorEntry>();

        foreach (FieldDeclaratorEntry field in newFields)
        {
            if (!oldByName.TryGetValue(field.Name, out FieldDeclaratorEntry? old))
            {
                added.Add(field);
                continue;
            }
            if (old.TypeText != field.TypeText)
            {
                // Retype = remove (placeholder keeps the old slot) + add
                // (new store slot); existing instances read default.
                removed.Add(old);
                added.Add(field);
            }
            else if (old.ModifiersText != field.ModifiersText || old.AttributesText != field.AttributesText)
            {
                result.Reasons.Add("field attributes or modifiers changed: " + metadataName + "." + field.Name +
                    " (metadata is immutable; use unity_recompile)");
                return false;
            }
        }
        foreach (FieldDeclaratorEntry field in oldFields)
        {
            if (!newByName.ContainsKey(field.Name))
                removed.Add(field);
        }

        // B2: a NEW field-like event would need compiler-generated accessors
        // and a backing delegate field in the original layout — point-name
        // it ahead of the generic skeleton verdict. (Removals/changes keep
        // the layout verdict below.)
        var oldEventFieldNames = EventFieldNames(oldType);
        foreach (string eventName in EventFieldNames(newType))
        {
            if (!oldEventFieldNames.Contains(eventName))
            {
                result.Reasons.Add("field-like event added: " + metadataName + "." + eventName +
                    " (the compiler-generated accessors and backing field need a real compile; " +
                    "declare explicit add/remove accessors or use unity_recompile)");
                return false;
            }
        }

        // B2: auto-properties ADDED by the edit virtualize their backing
        // field (accessor shims + M4 store) — exclude them from the skeleton
        // and record the backing-field change. A same-named pre-existing
        // property of ANY shape keeps the conservative layout verdict
        // (shape conversions move real backing storage).
        var oldPropertyNames = new HashSet<string>(
            oldType.Members.OfType<PropertyDeclarationSyntax>().Select(p => p.Identifier.Text),
            StringComparer.Ordinal);
        var addedAutoProps = newType.Members.OfType<PropertyDeclarationSyntax>()
            .Where(p => IsAutoProperty(p) && !oldPropertyNames.Contains(p.Identifier.Text))
            .ToList();
        var addedAutoNames = new HashSet<string>(
            addedAutoProps.Select(p => p.Identifier.Text), StringComparer.Ordinal);

        // Kept fields, auto-properties and field-like events must keep
        // their exact shape and relative order: the skeleton sequence
        // (added/removed fields and added auto-properties excluded) is
        // compared verbatim.
        var addedNames = new HashSet<string>(added.Select(f => f.Name), StringComparer.Ordinal);
        var removedNames = new HashSet<string>(removed.Select(f => f.Name), StringComparer.Ordinal);
        List<string> oldSkeleton = LayoutSkeleton(oldType, removedNames, new HashSet<string>(StringComparer.Ordinal));
        List<string> newSkeleton = LayoutSkeleton(newType, addedNames, addedAutoNames);
        if (!oldSkeleton.SequenceEqual(newSkeleton, StringComparer.Ordinal))
        {
            result.Reasons.Add("field layout changed: " + metadataName +
                " (only plain field additions/removals/retypes are hot-virtualizable)");
            return false;
        }

        // Static (non-const) fields: per-name add/remove diff. Common-name
        // changes are handled by DiffConstAndStaticInit.
        var oldStatics = StaticFieldEntries(oldType);
        var newStatics = StaticFieldEntries(newType);
        var staticAdded = newStatics.Values.Where(f => !oldStatics.ContainsKey(f.Name)).ToList();
        var staticRemoved = oldStatics.Values.Where(f => !newStatics.ContainsKey(f.Name)).ToList();

        if (added.Count == 0 && removed.Count == 0 && staticAdded.Count == 0 && staticRemoved.Count == 0 &&
            addedAutoProps.Count == 0)
            return true;

        if (newType is StructDeclarationSyntax)
        {
            result.Reasons.Add("struct field layout changed: " + metadataName +
                " (value-copy semantics cannot be store-virtualized; use unity_recompile)");
            return false;
        }
        if (genericContext)
        {
            result.Reasons.Add("generic type field layout changed: " + metadataName);
            return false;
        }
        if (isPartial)
        {
            // B6 (v1): field virtualization re-materializes initializers
            // into the type's constructors and anchors removed-field
            // placeholders by in-file position — both undefined when other
            // parts (other files, generators) contribute to the layout.
            result.Reasons.Add("partial type field layout changed: " + metadataName +
                " (cross-part field virtualization is not supported; use unity_recompile)");
            return false;
        }

        foreach (FieldDeclaratorEntry field in added)
        {
            result.FieldChanges.Add(new HotDiffFieldChange
            {
                DeclaringType = metadataName,
                Name = field.Name,
                Kind = "added",
                IsStatic = false,
            });
        }
        foreach (FieldDeclaratorEntry field in removed)
        {
            result.FieldChanges.Add(new HotDiffFieldChange
            {
                DeclaringType = metadataName,
                Name = field.Name,
                Kind = "removed",
                IsStatic = false,
                OldFieldIndex = field.Index,
            });
        }
        foreach (FieldDeclaratorEntry field in staticAdded)
        {
            result.FieldChanges.Add(new HotDiffFieldChange
            {
                DeclaringType = metadataName,
                Name = field.Name,
                Kind = "added",
                IsStatic = true,
            });
        }
        // Removed statics need no action: the original keeps holding the
        // (now unreferenced) value until the convergence recompile.
        foreach (FieldDeclaratorEntry field in staticRemoved)
        {
            result.FieldChanges.Add(new HotDiffFieldChange
            {
                DeclaringType = metadataName,
                Name = field.Name,
                Kind = "removed",
                IsStatic = true,
            });
        }
        // B2: added auto-properties virtualize their backing field. The
        // FieldChange carries the METADATA backing name (layout
        // verification); the rewriter binds it to the property declaration.
        foreach (PropertyDeclarationSyntax property in addedAutoProps)
        {
            result.FieldChanges.Add(new HotDiffFieldChange
            {
                DeclaringType = metadataName,
                Name = AutoPropertyBackingFieldName(property.Identifier.Text),
                Kind = "added",
                IsStatic = property.Modifiers.Any(SyntaxKind.StaticKeyword),
            });
        }

        return true;
    }

    private static HashSet<string> EventFieldNames(TypeDeclarationSyntax type)
    {
        var names = new HashSet<string>(StringComparer.Ordinal);
        foreach (EventFieldDeclarationSyntax eventField in type.Members.OfType<EventFieldDeclarationSyntax>())
        {
            foreach (VariableDeclaratorSyntax declarator in eventField.Declaration.Variables)
                names.Add(declarator.Identifier.Text);
        }
        return names;
    }

    private static List<FieldDeclaratorEntry> InstanceFieldEntries(TypeDeclarationSyntax type)
    {
        var entries = new List<FieldDeclaratorEntry>();
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            if (member is not FieldDeclarationSyntax field ||
                field.Modifiers.Any(SyntaxKind.ConstKeyword) ||
                field.Modifiers.Any(SyntaxKind.StaticKeyword))
            {
                continue;
            }
            foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
            {
                entries.Add(new FieldDeclaratorEntry
                {
                    Name = declarator.Identifier.Text,
                    TypeText = TokenText(field.Declaration.Type),
                    ModifiersText = ModifiersText(field.Modifiers),
                    AttributesText = AttributeListsText(field.AttributeLists),
                    Index = entries.Count,
                });
            }
        }
        return entries;
    }

    private static Dictionary<string, FieldDeclaratorEntry> StaticFieldEntries(TypeDeclarationSyntax type)
    {
        var entries = new Dictionary<string, FieldDeclaratorEntry>(StringComparer.Ordinal);
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            if (member is not FieldDeclarationSyntax field ||
                field.Modifiers.Any(SyntaxKind.ConstKeyword) ||
                !field.Modifiers.Any(SyntaxKind.StaticKeyword))
            {
                continue;
            }
            foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
            {
                entries[declarator.Identifier.Text] = new FieldDeclaratorEntry
                {
                    Name = declarator.Identifier.Text,
                    TypeText = TokenText(field.Declaration.Type),
                    ModifiersText = ModifiersText(field.Modifiers),
                    AttributesText = AttributeListsText(field.AttributeLists),
                };
            }
        }
        return entries;
    }

    /// <summary>The layout sequence with added/removed plain fields (and B2:
    /// added auto-properties) excluded: kept fields shrink to name markers
    /// (their shape equality is checked separately), auto-properties and
    /// field-like events keep their full shape text — so any change or
    /// reorder among them stays cold.</summary>
    private static List<string> LayoutSkeleton(
        TypeDeclarationSyntax type,
        HashSet<string> excludedFieldNames,
        HashSet<string> excludedAutoPropertyNames)
    {
        var sequence = new List<string>();
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when
                    !field.Modifiers.Any(SyntaxKind.ConstKeyword) &&
                    !field.Modifiers.Any(SyntaxKind.StaticKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        if (!excludedFieldNames.Contains(declarator.Identifier.Text))
                            sequence.Add("F|" + declarator.Identifier.Text);
                    }
                    break;

                case EventFieldDeclarationSyntax eventField:
                    foreach (VariableDeclaratorSyntax declarator in eventField.Declaration.Variables)
                    {
                        sequence.Add(
                            "eventfield|" + ModifiersText(eventField.Modifiers) + "|" +
                            AttributeListsText(eventField.AttributeLists) + "|" +
                            TokenText(eventField.Declaration.Type) + "|" + declarator.Identifier.Text);
                    }
                    break;

                case PropertyDeclarationSyntax property when IsAutoProperty(property):
                    if (excludedAutoPropertyNames.Contains(property.Identifier.Text))
                        break;
                    sequence.Add(
                        "autoprop|" + ModifiersText(property.Modifiers) + "|" +
                        AttributeListsText(property.AttributeLists) + "|" +
                        TokenText(property.Type) + "|" + property.Identifier.Text + "|" +
                        string.Join(",", property.AccessorList!.Accessors.Select(a =>
                            ModifiersText(a.Modifiers) + a.Keyword.Text + AttributeListsText(a.AttributeLists))));
                    break;
            }
        }
        return sequence;
    }

    /// <summary>Per-name comparison of consts and static initializers.
    /// Common-name changes stay cold (inlined values / already-ran cctor);
    /// ADDED consts and the add/remove of static fields pass (the latter
    /// flow through DiffFieldLayout). Removed consts stay cold: their
    /// inlined values leave no metadata trace to verify.</summary>
    private static bool DiffConstAndStaticInit(
        string metadataName,
        TypeDeclarationSyntax oldType,
        TypeDeclarationSyntax newType,
        HotDiffFileResult result)
    {
        Dictionary<string, string> oldEntries = ConstAndStaticInitEntries(oldType);
        Dictionary<string, string> newEntries = ConstAndStaticInitEntries(newType);

        foreach (var pair in oldEntries)
        {
            if (!newEntries.TryGetValue(pair.Key, out string? newText))
            {
                if (pair.Value.StartsWith("const|", StringComparison.Ordinal))
                {
                    result.Reasons.Add("const removed: " + metadataName + "." + pair.Key +
                        " (inlined values cannot be verified; use unity_recompile)");
                    return false;
                }
                continue; // removed static field/initializer: DiffFieldLayout decides
            }
            if (pair.Value != newText)
            {
                result.Reasons.Add("const or static initializer changed: " + metadataName);
                return false;
            }
        }
        // Added consts are fine (the patch inlines them; no pre-existing
        // call sites can reference them). Added static fields flow through
        // DiffFieldLayout into holder classes.
        return true;
    }

    private static Dictionary<string, string> ConstAndStaticInitEntries(TypeDeclarationSyntax type)
    {
        var entries = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when field.Modifiers.Any(SyntaxKind.ConstKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        entries[declarator.Identifier.Text] =
                            "const|" + ModifiersText(field.Modifiers) + "|" +
                            TokenText(field.Declaration.Type) + "|" +
                            (declarator.Initializer != null ? TokenText(declarator.Initializer) : "");
                    }
                    break;
                case FieldDeclarationSyntax field when field.Modifiers.Any(SyntaxKind.StaticKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        entries[declarator.Identifier.Text] =
                            "static|" + ModifiersText(field.Modifiers) + "|" +
                            TokenText(field.Declaration.Type) + "|" +
                            (declarator.Initializer != null ? TokenText(declarator.Initializer) : "");
                    }
                    break;
                case PropertyDeclarationSyntax property when
                    IsAutoProperty(property) &&
                    property.Modifiers.Any(SyntaxKind.StaticKeyword):
                    entries["P:" + property.Identifier.Text] =
                        "sprop|" + ModifiersText(property.Modifiers) + "|" + TokenText(property.Type) + "|" +
                        (property.Initializer != null ? TokenText(property.Initializer) : "");
                    break;
            }
        }
        return entries;
    }

    private static bool IsGenericContext(TypeDeclarationSyntax type)
    {
        for (SyntaxNode? current = type; current != null; current = current.Parent)
        {
            if (current is TypeDeclarationSyntax decl && (decl.TypeParameterList?.Parameters.Count ?? 0) > 0)
                return true;
        }
        return false;
    }

    private static string TypeHeaderText(TypeDeclarationSyntax type)
    {
        return string.Join(
            "|",
            AttributeListsText(type.AttributeLists),
            string.Join(" ", type.Modifiers.Select(m => m.Text)),
            type.Keyword.Text,
            type.Identifier.Text,
            type.TypeParameterList?.ToString() ?? "",
            type.BaseList != null ? TokenText(type.BaseList) : "",
            string.Join(",", type.ConstraintClauses.Select(TokenText)));
    }

    // ── layout & initializer text ────────────────────────────────────

    internal static bool IsAutoProperty(PropertyDeclarationSyntax property)
    {
        if (property.ExpressionBody != null || property.AccessorList == null)
            return false;
        return property.AccessorList.Accessors.All(a => a.Body == null && a.ExpressionBody == null);
    }

    private static string InstanceInitText(TypeDeclarationSyntax type)
    {
        var parts = new List<string>();
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when
                    !field.Modifiers.Any(SyntaxKind.ConstKeyword) &&
                    !field.Modifiers.Any(SyntaxKind.StaticKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        if (declarator.Initializer != null)
                            parts.Add(declarator.Identifier.Text + "|" + TokenText(declarator.Initializer));
                    }
                    break;
                case PropertyDeclarationSyntax property when
                    IsAutoProperty(property) &&
                    !property.Modifiers.Any(SyntaxKind.StaticKeyword) &&
                    property.Initializer != null:
                    parts.Add(property.Identifier.Text + "|" + TokenText(property.Initializer));
                    break;
            }
        }
        return string.Join("\n", parts);
    }

    // ── M6: using-change whole-file re-detour ────────────────────────

    /// <summary>Why this type blocks the using-change re-detour path (null
    /// when safe). The patch copy of EVERY member binds under the new
    /// directives, so any member whose compiled body cannot be re-detoured —
    /// or whose already-materialized value cannot follow the new bindings —
    /// fails the file closed.</summary>
    private static string? UsingRehookGateReason(string metadataName, TypeDeclarationSyntax type)
    {
        string Reason(string what) =>
            "using directives changed and " + what + ": " + metadataName;

        if (type is InterfaceDeclarationSyntax)
        {
            bool hasBodies = type.Members.Any(member =>
                member.DescendantNodes().Any(n => n is BlockSyntax || n is ArrowExpressionClauseSyntax));
            return hasBodies
                ? Reason("the interface has default implementations that cannot be re-detoured")
                : null;
        }

        if (HasBurstCompileAttribute(type))
            return Reason("the type is Burst-compiled");

        bool genericContext = IsGenericContext(type);

        // Values frozen under the old bindings: inlined consts and
        // already-ran static initializers cannot re-bind without a real
        // compile, unless they are pure literals (binding-independent).
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when field.Modifiers.Any(SyntaxKind.ConstKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        if (declarator.Initializer != null && !IsLiteralInitializer(declarator.Initializer.Value))
                            return Reason("a non-literal const value is inlined under the old bindings");
                    }
                    break;
                case FieldDeclarationSyntax field when field.Modifiers.Any(SyntaxKind.StaticKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        if (declarator.Initializer != null && !IsLiteralInitializer(declarator.Initializer.Value))
                            return Reason("a non-literal static initializer already ran under the old bindings");
                    }
                    break;
                case PropertyDeclarationSyntax property when
                    IsAutoProperty(property) &&
                    property.Modifiers.Any(SyntaxKind.StaticKeyword) &&
                    property.Initializer != null &&
                    !IsLiteralInitializer(property.Initializer.Value):
                    return Reason("a non-literal static initializer already ran under the old bindings");
            }
        }

        foreach (MemberDeclarationSyntax member in type.Members)
        {
            bool hasBody = member.DescendantNodes().Any(n => n is BlockSyntax || n is ArrowExpressionClauseSyntax);
            if (!hasBody)
                continue;

            switch (member)
            {
                case MethodDeclarationSyntax method:
                    if (genericContext || (method.TypeParameterList?.Parameters.Count ?? 0) > 0)
                        return Reason("generic members cannot be re-detoured");
                    if (HasBurstCompileAttribute(method))
                        return Reason("a Burst-compiled member cannot be re-detoured");
                    if (method.ExplicitInterfaceSpecifier != null)
                        return Reason("an explicit interface implementation cannot be re-detoured");
                    break;
                case ConstructorDeclarationSyntax ctor:
                    if (genericContext && !ctor.Modifiers.Any(SyntaxKind.StaticKeyword))
                        return Reason("generic members cannot be re-detoured");
                    break;
                case DestructorDeclarationSyntax:
                    return Reason("a finalizer cannot be re-detoured");
                case OperatorDeclarationSyntax op:
                    if (genericContext)
                        return Reason("generic members cannot be re-detoured");
                    if (HasBurstCompileAttribute(op))
                        return Reason("a Burst-compiled member cannot be re-detoured");
                    if (OperatorMetadataName(op.OperatorToken.Text, op.ParameterList.Parameters.Count) == null)
                        return Reason("an unsupported operator cannot be re-detoured");
                    break;
                case ConversionOperatorDeclarationSyntax conv:
                    if (genericContext)
                        return Reason("generic members cannot be re-detoured");
                    if (HasBurstCompileAttribute(conv))
                        return Reason("a Burst-compiled member cannot be re-detoured");
                    break;
                case PropertyDeclarationSyntax property when !IsAutoProperty(property):
                    if (genericContext)
                        return Reason("generic members cannot be re-detoured");
                    if (property.ExplicitInterfaceSpecifier != null)
                        return Reason("an explicit interface implementation cannot be re-detoured");
                    if (HasBurstCompileAttribute(property) ||
                        (property.AccessorList?.Accessors.Any(HasBurstCompileAttribute) ?? false))
                        return Reason("a Burst-compiled member cannot be re-detoured");
                    break;
                case IndexerDeclarationSyntax indexer:
                    if (genericContext)
                        return Reason("generic members cannot be re-detoured");
                    if (indexer.ExplicitInterfaceSpecifier != null)
                        return Reason("an explicit interface implementation cannot be re-detoured");
                    if (HasBurstCompileAttribute(indexer) ||
                        (indexer.AccessorList?.Accessors.Any(HasBurstCompileAttribute) ?? false))
                        return Reason("a Burst-compiled member cannot be re-detoured");
                    break;
                case EventDeclarationSyntax @event:
                    if (genericContext)
                        return Reason("generic members cannot be re-detoured");
                    if (@event.ExplicitInterfaceSpecifier != null)
                        return Reason("an explicit interface event cannot be re-detoured");
                    if (HasBurstCompileAttribute(@event) ||
                        (@event.AccessorList?.Accessors.Any(HasBurstCompileAttribute) ?? false))
                        return Reason("a Burst-compiled member cannot be re-detoured");
                    break;
                case FieldDeclarationSyntax:
                case EventFieldDeclarationSyntax:
                    // Initializers with lambdas: instance ones re-detour via
                    // constructors; static ones were gated above (a lambda is
                    // never a literal initializer).
                    break;
            }
        }

        return null;
    }

    /// <summary>True for initializer expressions that cannot bind
    /// differently under changed using directives.</summary>
    private static bool IsLiteralInitializer(ExpressionSyntax expression)
    {
        return expression switch
        {
            LiteralExpressionSyntax => true,
            PrefixUnaryExpressionSyntax unary when
                unary.IsKind(SyntaxKind.UnaryMinusExpression) ||
                unary.IsKind(SyntaxKind.UnaryPlusExpression) => IsLiteralInitializer(unary.Operand),
            _ => false,
        };
    }

    /// <summary>Add every detourable member of a pre-existing type to the
    /// re-detour set (deduplicated against body-diff entries).</summary>
    private static void AddRehookMembers(string metadataName, TypeDeclarationSyntax type, HotDiffFileResult result)
    {
        static string Key(string declaringType, string name, string[] paramNames, bool isStatic) =>
            declaringType + "|" + name + "|" + string.Join(",", paramNames) + (isStatic ? "|s" : "|i");

        var seen = new HashSet<string>(
            result.ChangedMethods.Select(m => Key(m.DeclaringType, m.Name, m.ParamTypeNames, m.IsStatic)),
            StringComparer.Ordinal);
        bool any = false;

        void Add(string name, string[] paramNames, bool isStatic, bool isCtor)
        {
            if (!seen.Add(Key(metadataName, name, paramNames, isStatic)))
                return;
            AddMethod(result, metadataName, name, paramNames, isStatic, isCtor, added: false);
            any = true;
        }

        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case MethodDeclarationSyntax method when method.Body != null || method.ExpressionBody != null:
                    Add(method.Identifier.Text, ParamTypeNames(method.ParameterList),
                        method.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false);
                    break;

                case ConstructorDeclarationSyntax ctor when
                    !ctor.Modifiers.Any(SyntaxKind.StaticKeyword) &&
                    (ctor.Body != null || ctor.ExpressionBody != null):
                    Add(".ctor", ParamTypeNames(ctor.ParameterList), isStatic: false, isCtor: true);
                    break;

                case PropertyDeclarationSyntax property when !IsAutoProperty(property):
                    if (property.ExpressionBody != null)
                    {
                        Add("get_" + property.Identifier.Text, Array.Empty<string>(),
                            property.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false);
                        break;
                    }
                    foreach (AccessorDeclarationSyntax accessor in property.AccessorList?.Accessors ?? default)
                    {
                        if (accessor.Body == null && accessor.ExpressionBody == null)
                            continue;
                        Add(AccessorName(accessor, property.Identifier.Text),
                            AccessorParams(accessor, null, TokenText(property.Type)),
                            property.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false);
                    }
                    break;

                case IndexerDeclarationSyntax indexer:
                    if (indexer.ExpressionBody != null)
                    {
                        Add("get_Item", ParamTypeNames(indexer.ParameterList), isStatic: false, isCtor: false);
                        break;
                    }
                    foreach (AccessorDeclarationSyntax accessor in indexer.AccessorList?.Accessors ?? default)
                    {
                        if (accessor.Body == null && accessor.ExpressionBody == null)
                            continue;
                        Add(AccessorName(accessor, "Item"),
                            AccessorParams(accessor, indexer.ParameterList, TokenText(indexer.Type)),
                            isStatic: false, isCtor: false);
                    }
                    break;

                case EventDeclarationSyntax @event:
                    foreach (AccessorDeclarationSyntax accessor in @event.AccessorList?.Accessors ?? default)
                    {
                        if (accessor.Body == null && accessor.ExpressionBody == null)
                            continue;
                        string prefix = accessor.Keyword.Text == "add" ? "add_" : "remove_";
                        Add(prefix + @event.Identifier.Text, new[] { SimpleTypeName(@event.Type) },
                            @event.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false);
                    }
                    break;

                case OperatorDeclarationSyntax op when op.Body != null || op.ExpressionBody != null:
                {
                    string? opName = OperatorMetadataName(op.OperatorToken.Text, op.ParameterList.Parameters.Count);
                    if (opName != null)
                        Add(opName, ParamTypeNames(op.ParameterList), isStatic: true, isCtor: false);
                    break;
                }

                case ConversionOperatorDeclarationSyntax conv when conv.Body != null || conv.ExpressionBody != null:
                    Add(conv.ImplicitOrExplicitKeyword.IsKind(SyntaxKind.ImplicitKeyword) ? "op_Implicit" : "op_Explicit",
                        ParamTypeNames(conv.ParameterList), isStatic: true, isCtor: false);
                    break;
            }
        }

        // Instance initializers compile into the implicit default ctor when
        // none is declared; re-detour it so initializer bindings follow too.
        bool hasInstanceInit = InstanceInitText(type).Length > 0;
        bool hasExplicitInstanceCtor = type.Members.OfType<ConstructorDeclarationSyntax>()
            .Any(c => !c.Modifiers.Any(SyntaxKind.StaticKeyword));
        if (hasInstanceInit && !hasExplicitInstanceCtor && type is not StructDeclarationSyntax)
            Add(".ctor", Array.Empty<string>(), isStatic: false, isCtor: true);

        if (any)
            result.PatchedTypes.Add(metadataName);
    }

    // ── executable member diff ───────────────────────────────────────

    private static Dictionary<string, MemberDeclarationSyntax> ExecutableMembers(
        TypeDeclarationSyntax type,
        List<string> reasons,
        string metadataName)
    {
        var members = new Dictionary<string, MemberDeclarationSyntax>(StringComparer.Ordinal);

        foreach (MemberDeclarationSyntax member in type.Members)
        {
            string? key = member switch
            {
                MethodDeclarationSyntax method =>
                    "M|" + (method.ExplicitInterfaceSpecifier != null ? TokenText(method.ExplicitInterfaceSpecifier) : "") +
                    method.Identifier.Text + "`" + (method.TypeParameterList?.Parameters.Count ?? 0) +
                    "|" + ParamKey(method.ParameterList),
                ConstructorDeclarationSyntax ctor =>
                    (ctor.Modifiers.Any(SyntaxKind.StaticKeyword) ? "CC|" : "C|") + ParamKey(ctor.ParameterList),
                DestructorDeclarationSyntax => "D|",
                OperatorDeclarationSyntax op =>
                    "O|" + op.OperatorToken.Text + "|" + ParamKey(op.ParameterList),
                ConversionOperatorDeclarationSyntax conv =>
                    "V|" + conv.ImplicitOrExplicitKeyword.Text + "|" + TokenText(conv.Type) + "|" + ParamKey(conv.ParameterList),
                // Auto-properties join the member diff for the ADDED case
                // (B2); every kept-pair change is intercepted earlier by the
                // layout skeleton / initializer diffs (they run first).
                PropertyDeclarationSyntax property =>
                    "P|" + (property.ExplicitInterfaceSpecifier != null ? TokenText(property.ExplicitInterfaceSpecifier) : "") +
                    property.Identifier.Text,
                IndexerDeclarationSyntax indexer =>
                    "I|" + (indexer.ExplicitInterfaceSpecifier != null ? TokenText(indexer.ExplicitInterfaceSpecifier) : "") +
                    ParamKey(indexer.ParameterList),
                EventDeclarationSyntax @event =>
                    "E|" + (@event.ExplicitInterfaceSpecifier != null ? TokenText(@event.ExplicitInterfaceSpecifier) : "") +
                    @event.Identifier.Text,
                _ => null,
            };

            if (key == null)
                continue;

            if (members.ContainsKey(key))
            {
                // B6 (v1): a partial METHOD whose defining and implementing
                // declarations live in the SAME file (merged part view)
                // legally carries one signature twice — the member pairing
                // below cannot tell the halves apart, so fail closed by
                // name. (Split across FILES each side diffs alone and flows
                // normally.) Any other duplicate would not compile.
                bool partialMethodPair =
                    member is MethodDeclarationSyntax dup && dup.Modifiers.Any(SyntaxKind.PartialKeyword);
                reasons.Add(partialMethodPair
                    ? "partial method declared twice in this file: " + metadataName +
                      " (move the implementing declaration to another part or use unity_recompile)"
                    : "duplicate member signature: " + metadataName);
                return members;
            }
            members.Add(key, member);
        }

        return members;
    }

    private static string ParamKey(BaseParameterListSyntax? parameters)
    {
        if (parameters == null)
            return "";
        return string.Join(
            ",",
            parameters.Parameters.Select(p =>
                string.Join(" ", p.Modifiers.Select(m => m.Text)) + " " + (p.Type != null ? TokenText(p.Type) : "")));
    }

    /// <summary>Classify a member that only exists in the new text. Returns
    /// false (with a reason) when it forces the cold path. Added METHODS of
    /// any accessibility are hot: they materialize as static shims
    /// (`__LocusShims`) in the patch assembly, batch call sites rewrite to
    /// direct shim calls, and no detour is needed (M2) — so even members of
    /// generic types are fine (the shim is just a generic static method).</summary>
    private static bool ClassifyAddedMember(
        string metadataName,
        MemberDeclarationSyntax member,
        bool genericContext,
        bool burstContext,
        HotDiffFileResult result)
    {
        switch (member)
        {
            case ConstructorDeclarationSyntax:
                // `new Foo(...)` in patched bodies binds to the *original*
                // type, which does not have the new overload.
                result.Reasons.Add("constructor added: " + metadataName +
                    " (constructor surface cannot be hot-added; use unity_recompile)");
                return false;

            case DestructorDeclarationSyntax:
                result.Reasons.Add("finalizer added: " + metadataName);
                return false;

            case MethodDeclarationSyntax method:
                if (burstContext || HasBurstCompileAttribute(method))
                {
                    result.Reasons.Add("Burst-compiled member added: " + metadataName + "." + method.Identifier.Text);
                    return false;
                }
                if (method.ExplicitInterfaceSpecifier != null)
                {
                    result.Reasons.Add("explicit interface implementation added: " + metadataName);
                    return false;
                }
                if (UnityMagicMethods.Contains(method.Identifier.Text))
                {
                    // Unity discovered the original type's message set at
                    // load; a new message method would never be called.
                    result.Reasons.Add("new Unity message method: " + metadataName + "." + method.Identifier.Text +
                        " (Unity only discovers message methods at a real compile; use unity_recompile)");
                    return false;
                }
                if ((method.TypeParameterList?.Parameters.Count ?? 0) > 0)
                {
                    // Generic methods materialize as generic static shims
                    // (B1): direct calls re-bind every batch and re-edit
                    // detours are skipped (H7b), so the only structural
                    // blocker is a method type parameter shadowing a
                    // declaring-chain parameter — the shim would have to
                    // declare the name twice.
                    string? shadowed = FindShadowedMethodTypeParameter(method);
                    if (shadowed != null)
                    {
                        result.Reasons.Add("generic method type parameter shadows the declaring type's: " +
                            metadataName + "." + method.Identifier.Text + "<" + shadowed +
                            "> (rename the method type parameter or use unity_recompile)");
                        return false;
                    }
                }
                if (method.Modifiers.Any(m =>
                    m.IsKind(SyntaxKind.VirtualKeyword) || m.IsKind(SyntaxKind.OverrideKeyword) ||
                    m.IsKind(SyntaxKind.AbstractKeyword) || m.IsKind(SyntaxKind.SealedKeyword)))
                {
                    // Shims are static dispatch; a new virtual slot (or an
                    // override of one) cannot be reproduced without a real
                    // compile.
                    result.Reasons.Add("virtual member added: " + metadataName + "." + method.Identifier.Text +
                        " (virtual dispatch needs a real compile; use unity_recompile)");
                    return false;
                }
                if (method.DescendantNodes().OfType<BaseExpressionSyntax>().Any())
                {
                    // A static shim cannot express a non-virtual `base.X`
                    // call on behalf of the instance.
                    result.Reasons.Add("added member uses base access: " + metadataName + "." + method.Identifier.Text +
                        " (base calls cannot be expressed by a shim; use unity_recompile)");
                    return false;
                }
                AddMethod(
                    result, metadataName, method.Identifier.Text, ParamTypeNames(method.ParameterList),
                    method.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false, added: true,
                    typeParameterCount: method.TypeParameterList?.Parameters.Count ?? 0);
                return true;

            case PropertyDeclarationSyntax property:
            {
                // B2: an added property decomposes into accessor shims
                // (get_X/set_X) — call sites only exist inside the batch
                // (the surface is new), and the rewriter materializes them
                // as direct shim calls. Auto-properties additionally
                // virtualize their backing field through an M4 store
                // (DiffFieldLayout recorded the FieldChange).
                string display = property.Identifier.Text;
                string? guard = AddedAccessorMemberGuard(
                    metadataName, display, property, property.Modifiers,
                    property.ExplicitInterfaceSpecifier != null, burstContext,
                    property.AccessorList?.Accessors ?? default);
                if (guard != null)
                {
                    result.Reasons.Add(guard);
                    return false;
                }
                bool propertyStatic = property.Modifiers.Any(SyntaxKind.StaticKeyword);
                if (property.ExpressionBody != null)
                {
                    AddMethod(result, metadataName, "get_" + display, Array.Empty<string>(),
                        propertyStatic, isCtor: false, added: true);
                    return true;
                }
                bool auto = IsAutoProperty(property);
                foreach (AccessorDeclarationSyntax accessor in property.AccessorList?.Accessors ?? default)
                {
                    if (!auto && accessor.Body == null && accessor.ExpressionBody == null)
                    {
                        // Mixed auto/bodied accessors do not compile; fail
                        // closed so a real compile surfaces the error.
                        result.Reasons.Add("added property accessor has no body: " + metadataName + "." + display);
                        return false;
                    }
                    AddMethod(result, metadataName, AccessorName(accessor, display),
                        AccessorParams(accessor, null, TokenText(property.Type)),
                        propertyStatic, isCtor: false, added: true);
                }
                return true;
            }

            case IndexerDeclarationSyntax indexer:
            {
                // B2: get_Item/set_Item accessor shims, parameterized by the
                // indexer's own parameter list.
                string? guard = AddedAccessorMemberGuard(
                    metadataName, "this[]", indexer, indexer.Modifiers,
                    indexer.ExplicitInterfaceSpecifier != null, burstContext,
                    indexer.AccessorList?.Accessors ?? default);
                if (guard != null)
                {
                    result.Reasons.Add(guard);
                    return false;
                }
                if (indexer.ExpressionBody != null)
                {
                    AddMethod(result, metadataName, "get_Item", ParamTypeNames(indexer.ParameterList),
                        isStatic: false, isCtor: false, added: true);
                    return true;
                }
                foreach (AccessorDeclarationSyntax accessor in indexer.AccessorList?.Accessors ?? default)
                {
                    if (accessor.Body == null && accessor.ExpressionBody == null)
                    {
                        result.Reasons.Add("added indexer accessor has no body: " + metadataName + ".this[]");
                        return false;
                    }
                    AddMethod(result, metadataName, AccessorName(accessor, "Item"),
                        AccessorParams(accessor, indexer.ParameterList, TokenText(indexer.Type)),
                        isStatic: false, isCtor: false, added: true);
                }
                return true;
            }

            case EventDeclarationSyntax @event:
            {
                // B2: add_X/remove_X accessor shims. Field-like events
                // (EventFieldDeclaration) never reach here — DiffFieldLayout
                // names them cold (compiler-generated accessors + backing
                // delegate field).
                string display = @event.Identifier.Text;
                string? guard = AddedAccessorMemberGuard(
                    metadataName, display, @event, @event.Modifiers,
                    @event.ExplicitInterfaceSpecifier != null, burstContext,
                    @event.AccessorList?.Accessors ?? default);
                if (guard != null)
                {
                    result.Reasons.Add(guard);
                    return false;
                }
                bool eventStatic = @event.Modifiers.Any(SyntaxKind.StaticKeyword);
                foreach (AccessorDeclarationSyntax accessor in @event.AccessorList?.Accessors ?? default)
                {
                    if (accessor.Body == null && accessor.ExpressionBody == null)
                    {
                        result.Reasons.Add("added event accessor has no body: " + metadataName + "." + display);
                        return false;
                    }
                    string prefix = accessor.Keyword.Text == "add" ? "add_" : "remove_";
                    AddMethod(result, metadataName, prefix + display, new[] { SimpleTypeName(@event.Type) },
                        eventStatic, isCtor: false, added: true);
                }
                return true;
            }

            case OperatorDeclarationSyntax:
            case ConversionOperatorDeclarationSyntax:
                // Adding these is rare and their call sites live outside the
                // patch; keep the matrix small and recompile.
                result.Reasons.Add("member kind addition not hot-reloadable: " + metadataName + "." + DisplayName(member));
                return false;

            default:
                return true;
        }
    }

    /// <summary>Shared B2 gate for added accessor-shaped members
    /// (property/indexer/event): the same structural blockers as added
    /// methods — Burst, explicit interface, virtual slots, `base.` access
    /// (a static shim cannot express any of them). Null when hot.</summary>
    private static string? AddedAccessorMemberGuard(
        string metadataName,
        string displayName,
        MemberDeclarationSyntax member,
        SyntaxTokenList modifiers,
        bool explicitInterface,
        bool burstContext,
        SyntaxList<AccessorDeclarationSyntax> accessors)
    {
        if (burstContext || HasBurstCompileAttribute(member) || accessors.Any(HasBurstCompileAttribute))
            return "Burst-compiled member added: " + metadataName + "." + displayName;
        if (explicitInterface)
            return "explicit interface implementation added: " + metadataName;
        if (modifiers.Any(m =>
            m.IsKind(SyntaxKind.VirtualKeyword) || m.IsKind(SyntaxKind.OverrideKeyword) ||
            m.IsKind(SyntaxKind.AbstractKeyword) || m.IsKind(SyntaxKind.SealedKeyword)))
        {
            // Shims are static dispatch; a new virtual slot (or an override
            // of one) cannot be reproduced without a real compile.
            return "virtual member added: " + metadataName + "." + displayName +
                " (virtual dispatch needs a real compile; use unity_recompile)";
        }
        if (member.DescendantNodes().OfType<BaseExpressionSyntax>().Any())
        {
            return "added member uses base access: " + metadataName + "." + displayName +
                " (base calls cannot be expressed by a shim; use unity_recompile)";
        }
        return null;
    }

    /// <summary>Compiler naming of an auto-property's backing field.</summary>
    internal static string AutoPropertyBackingFieldName(string propertyName) =>
        "<" + propertyName + ">k__BackingField";

    /// <summary>Classify a member that only exists in the OLD text (M5).
    /// Returns false (with a reason) when it forces the cold path; otherwise
    /// records removal entries + caller-check entries. The original member
    /// stays in the loaded assembly (in-flight delegates/coroutines are
    /// legitimate callers); Unity message methods additionally get an
    /// empty-body stub detour so the engine stops reaching the old body.</summary>
    private static bool ClassifyRemovedMember(
        string metadataName,
        MemberDeclarationSyntax member,
        bool genericContext,
        bool burstContext,
        HotDiffFileResult result)
    {
        void AddRemoval(string name, string[] paramTypeNames, bool isStatic, bool isUnityMagic, string? stubSource)
        {
            result.RemovedMembers.Add(new HotDiffRemovedMember
            {
                DeclaringType = metadataName,
                Name = name,
                ParamTypeNames = paramTypeNames,
                IsStatic = isStatic,
                IsUnityMagic = isUnityMagic,
                StubSource = stubSource,
            });
        }

        void AddCheck(string displayName, string[] scanNames, string[] paramTypeNames)
        {
            string detail = "member removed (or signature changed): " + metadataName + "." + displayName;
            result.RequiresCallerCheck.Add(new CallerCheckMember
            {
                DeclaringType = metadataName,
                Name = displayName,
                ParamTypeNames = paramTypeNames,
                Kind = "member-removed",
                Detail = detail,
                ScanMemberNames = scanNames,
            });
        }

        switch (member)
        {
            case MethodDeclarationSyntax method:
            {
                if (burstContext || HasBurstCompileAttribute(method))
                {
                    result.Reasons.Add("Burst-compiled member removed: " + metadataName + "." + method.Identifier.Text);
                    return false;
                }
                if (method.ExplicitInterfaceSpecifier != null)
                {
                    result.Reasons.Add("explicit interface implementation removed: " + metadataName);
                    return false;
                }
                if (method.Modifiers.Any(m =>
                    m.IsKind(SyntaxKind.VirtualKeyword) || m.IsKind(SyntaxKind.OverrideKeyword) ||
                    m.IsKind(SyntaxKind.AbstractKeyword) || m.IsKind(SyntaxKind.SealedKeyword)))
                {
                    // Override relations are metadata, not call sites: the
                    // IL scan cannot prove no out-of-batch override exists.
                    result.Reasons.Add("virtual member removed: " + metadataName + "." + method.Identifier.Text +
                        " (override relations cannot be verified; use unity_recompile)");
                    return false;
                }

                bool magic = UnityMagicMethods.Contains(method.Identifier.Text);
                string? stub = null;
                if (magic)
                {
                    if (genericContext)
                    {
                        result.Reasons.Add("Unity message method removed from generic type: " + metadataName);
                        return false;
                    }
                    stub = method
                        .WithAttributeLists(default)
                        .WithBody(SyntaxFactory.Block())
                        .WithExpressionBody(null)
                        .WithSemicolonToken(default)
                        .NormalizeWhitespace()
                        .ToFullString();
                }

                AddRemoval(method.Identifier.Text, ParamTypeNames(method.ParameterList),
                    method.Modifiers.Any(SyntaxKind.StaticKeyword), magic, stub);
                AddCheck(method.Identifier.Text, new[] { method.Identifier.Text },
                    ParamTypeNames(method.ParameterList));
                return true;
            }

            case PropertyDeclarationSyntax property when !IsAutoProperty(property):
            {
                if (burstContext || property.ExplicitInterfaceSpecifier != null || genericContext)
                {
                    result.Reasons.Add("member removed: " + metadataName + "." + property.Identifier.Text);
                    return false;
                }
                if (property.Modifiers.Any(m =>
                    m.IsKind(SyntaxKind.VirtualKeyword) || m.IsKind(SyntaxKind.OverrideKeyword) ||
                    m.IsKind(SyntaxKind.AbstractKeyword) || m.IsKind(SyntaxKind.SealedKeyword)))
                {
                    result.Reasons.Add("virtual member removed: " + metadataName + "." + property.Identifier.Text);
                    return false;
                }
                bool isStatic = property.Modifiers.Any(SyntaxKind.StaticKeyword);
                var scanNames = new List<string>();
                if (property.ExpressionBody != null)
                {
                    scanNames.Add("get_" + property.Identifier.Text);
                    AddRemoval("get_" + property.Identifier.Text, Array.Empty<string>(), isStatic, false, null);
                }
                foreach (AccessorDeclarationSyntax accessor in property.AccessorList?.Accessors ?? default)
                {
                    string accessorName = AccessorName(accessor, property.Identifier.Text);
                    scanNames.Add(accessorName);
                    AddRemoval(accessorName,
                        AccessorParams(accessor, null, TokenText(property.Type)), isStatic, false, null);
                }
                AddCheck(property.Identifier.Text, scanNames.ToArray(), Array.Empty<string>());
                return true;
            }

            case IndexerDeclarationSyntax indexer:
            {
                if (burstContext || indexer.ExplicitInterfaceSpecifier != null || genericContext)
                {
                    result.Reasons.Add("member removed: " + metadataName + ".this[]");
                    return false;
                }
                var scanNames = new List<string>();
                if (indexer.ExpressionBody != null)
                {
                    scanNames.Add("get_Item");
                    AddRemoval("get_Item", ParamTypeNames(indexer.ParameterList), false, false, null);
                }
                foreach (AccessorDeclarationSyntax accessor in indexer.AccessorList?.Accessors ?? default)
                {
                    string accessorName = AccessorName(accessor, "Item");
                    scanNames.Add(accessorName);
                    AddRemoval(accessorName,
                        AccessorParams(accessor, indexer.ParameterList, TokenText(indexer.Type)), false, false, null);
                }
                AddCheck("this[]", scanNames.ToArray(), ParamTypeNames(indexer.ParameterList));
                return true;
            }

            case EventDeclarationSyntax @event:
            {
                if (burstContext || @event.ExplicitInterfaceSpecifier != null || genericContext)
                {
                    result.Reasons.Add("member removed: " + metadataName + "." + @event.Identifier.Text);
                    return false;
                }
                bool isStatic = @event.Modifiers.Any(SyntaxKind.StaticKeyword);
                string[] scanNames = { "add_" + @event.Identifier.Text, "remove_" + @event.Identifier.Text };
                foreach (string accessorName in scanNames)
                    AddRemoval(accessorName, new[] { SimpleTypeName(@event.Type) }, isStatic, false, null);
                AddCheck(@event.Identifier.Text, scanNames, Array.Empty<string>());
                return true;
            }

            case OperatorDeclarationSyntax op:
            {
                string? opName = OperatorMetadataName(op.OperatorToken.Text, op.ParameterList.Parameters.Count);
                if (burstContext || genericContext || opName == null)
                {
                    result.Reasons.Add("member removed: " + metadataName + ".operator" + op.OperatorToken.Text);
                    return false;
                }
                AddRemoval(opName, ParamTypeNames(op.ParameterList), true, false, null);
                AddCheck("operator" + op.OperatorToken.Text, new[] { opName }, ParamTypeNames(op.ParameterList));
                return true;
            }

            case ConversionOperatorDeclarationSyntax conv:
            {
                if (burstContext || genericContext)
                {
                    result.Reasons.Add("member removed: " + metadataName + ".conversion");
                    return false;
                }
                string convName = conv.ImplicitOrExplicitKeyword.IsKind(SyntaxKind.ImplicitKeyword)
                    ? "op_Implicit"
                    : "op_Explicit";
                AddRemoval(convName, ParamTypeNames(conv.ParameterList), true, false, null);
                AddCheck("conversion", new[] { convName }, ParamTypeNames(conv.ParameterList));
                return true;
            }

            case ConstructorDeclarationSyntax:
                result.Reasons.Add("constructor removed: " + metadataName +
                    " (constructor surface changes need a real compile)");
                return false;

            case DestructorDeclarationSyntax:
                result.Reasons.Add("finalizer removed: " + metadataName);
                return false;

            default:
                result.Reasons.Add("member removed: " + metadataName + "." + DisplayName(member));
                return false;
        }
    }

    /// <summary>Diff one matched member; returns true when it contributed a
    /// hot change. Adds a reason (cold) for anything not provably safe.</summary>
    private static bool DiffMember(
        string metadataName,
        MemberDeclarationSyntax oldMember,
        MemberDeclarationSyntax newMember,
        bool genericContext,
        bool burstContext,
        HotDiffFileResult result)
    {
        if (HeaderText(oldMember) != HeaderText(newMember))
        {
            // Same-key signature changes (return type, static flip,
            // ref-kind-preserving rewrites) decompose into REMOVE(old) +
            // ADD(new): the add side shims (M2), the remove side tombstones
            // (M5), and the M3 caller scan verifies every call site of the
            // old surface lives in this batch. Attribute-only differences
            // stay cold (metadata is immutable).
            if (oldMember is MethodDeclarationSyntax oldMethodDecl &&
                newMember is MethodDeclarationSyntax newMethodDecl &&
                SignatureText(oldMethodDecl) != SignatureText(newMethodDecl))
            {
                if (UnityMagicMethods.Contains(oldMethodDecl.Identifier.Text) ||
                    UnityMagicMethods.Contains(newMethodDecl.Identifier.Text))
                {
                    result.Reasons.Add("Unity message method signature changed: " + metadataName + "." +
                        newMethodDecl.Identifier.Text + " (Unity matches message signatures at a real compile)");
                    return false;
                }
                if (!ClassifyRemovedMember(metadataName, oldMember, genericContext, burstContext, result))
                    return false;
                if (!ClassifyAddedMember(metadataName, newMember, genericContext, burstContext, result))
                    return false;
                return true;
            }

            result.Reasons.Add("member declaration changed: " + metadataName + "." + DisplayName(newMember));
            return false;
        }

        // Accessibility sits outside the header: WIDENING changes nothing at
        // runtime until a real compile (original metadata keeps the old
        // accessibility; patched bodies bypass access checks anyway), so an
        // unchanged body is a noop and a changed body is a plain hot edit.
        // NARROWING can strand call sites outside this batch — the M3
        // caller scan verifies it at compile/hotPatch time (the file stays
        // conditionally hot here).
        MemberAccess oldAccess = DeclaredAccess(oldMember);
        MemberAccess newAccess = DeclaredAccess(newMember);
        if (oldAccess != newAccess && !IsAccessWideningOrEqual(oldAccess, newAccess))
        {
            string detail = "member accessibility narrowed: " + metadataName + "." + DisplayName(newMember);
            result.RequiresCallerCheck.Add(new CallerCheckMember
            {
                DeclaringType = metadataName,
                Name = DisplayName(newMember),
                ParamTypeNames = newMember is BaseMethodDeclarationSyntax m ? ParamTypeNames(m.ParameterList) : Array.Empty<string>(),
                Kind = "accessibility-narrowed",
                Detail = detail,
                ScanMemberNames = ScanNamesFor(newMember),
            });
        }

        bool burstMember = burstContext || HasBurstCompileAttribute(oldMember) || HasBurstCompileAttribute(newMember);

        switch (newMember)
        {
            case MethodDeclarationSyntax newMethod:
            {
                var oldMethod = (MethodDeclarationSyntax)oldMember;
                if (MethodBodyText(oldMethod) == MethodBodyText(newMethod))
                    return false;
                if (burstMember)
                {
                    result.Reasons.Add("Burst-compiled method body changed: " + metadataName + "." + newMethod.Identifier.Text);
                    return false;
                }
                if (newMethod.ExplicitInterfaceSpecifier != null)
                {
                    result.Reasons.Add("explicit interface implementation changed: " + metadataName + "." + newMethod.Identifier.Text);
                    return false;
                }
                if (newMethod.Body == null && newMethod.ExpressionBody == null)
                {
                    // abstract/extern: no body to patch, header equality
                    // already ensured — nothing to do.
                    return false;
                }
                if (genericContext || (newMethod.TypeParameterList?.Parameters.Count ?? 0) > 0)
                {
                    return DecomposeGenericBodyChange(
                        metadataName, oldMethod, newMethod, genericContext, burstContext, result);
                }
                AddMethod(
                    result, metadataName, newMethod.Identifier.Text, ParamTypeNames(newMethod.ParameterList),
                    newMethod.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false, added: false);
                return true;
            }

            case ConstructorDeclarationSyntax newCtor:
            {
                var oldCtor = (ConstructorDeclarationSyntax)oldMember;
                string oldBody = (oldCtor.Initializer != null ? TokenText(oldCtor.Initializer) : "") + BodyText(oldCtor.Body, oldCtor.ExpressionBody);
                string newBody = (newCtor.Initializer != null ? TokenText(newCtor.Initializer) : "") + BodyText(newCtor.Body, newCtor.ExpressionBody);
                if (oldBody == newBody)
                    return false;
                if (burstMember)
                {
                    result.Reasons.Add("Burst-compiled constructor changed: " + metadataName);
                    return false;
                }
                if (newCtor.Modifiers.Any(SyntaxKind.StaticKeyword))
                {
                    result.Reasons.Add("static constructor changed: " + metadataName);
                    return false;
                }
                if (genericContext)
                {
                    result.Reasons.Add("generic type constructor changed: " + metadataName);
                    return false;
                }
                AddMethod(
                    result, metadataName, ".ctor", ParamTypeNames(newCtor.ParameterList),
                    isStatic: false, isCtor: true, added: false);
                return true;
            }

            case DestructorDeclarationSyntax newDtor:
            {
                var oldDtor = (DestructorDeclarationSyntax)oldMember;
                if (BodyText(oldDtor.Body, oldDtor.ExpressionBody) == BodyText(newDtor.Body, newDtor.ExpressionBody))
                    return false;
                result.Reasons.Add("finalizer changed: " + metadataName);
                return false;
            }

            case OperatorDeclarationSyntax newOp:
            {
                var oldOp = (OperatorDeclarationSyntax)oldMember;
                if (BodyText(oldOp.Body, oldOp.ExpressionBody) == BodyText(newOp.Body, newOp.ExpressionBody))
                    return false;
                if (burstMember)
                {
                    result.Reasons.Add("Burst-compiled operator changed: " + metadataName);
                    return false;
                }
                if (genericContext)
                {
                    result.Reasons.Add("generic type operator changed: " + metadataName);
                    return false;
                }
                string? opName = OperatorMetadataName(newOp.OperatorToken.Text, newOp.ParameterList.Parameters.Count);
                if (opName == null)
                {
                    result.Reasons.Add("unsupported operator changed: " + metadataName + ".operator" + newOp.OperatorToken.Text);
                    return false;
                }
                AddMethod(result, metadataName, opName, ParamTypeNames(newOp.ParameterList), isStatic: true, isCtor: false, added: false);
                return true;
            }

            case ConversionOperatorDeclarationSyntax newConv:
            {
                var oldConv = (ConversionOperatorDeclarationSyntax)oldMember;
                if (BodyText(oldConv.Body, oldConv.ExpressionBody) == BodyText(newConv.Body, newConv.ExpressionBody))
                    return false;
                if (burstMember)
                {
                    result.Reasons.Add("Burst-compiled conversion changed: " + metadataName);
                    return false;
                }
                if (genericContext)
                {
                    result.Reasons.Add("generic type conversion changed: " + metadataName);
                    return false;
                }
                if (SimpleTypeName(newConv.Type) == SimpleNameOfMetadata(metadataName))
                {
                    // The patch copy renames the type, and a conversion that
                    // RETURNS the declaring type would have to construct the
                    // patch type — semantically a different object. Only
                    // conversions FROM the declaring type stay hot.
                    result.Reasons.Add("conversion to the declaring type changed: " + metadataName +
                        " (the patch copy cannot re-declare it; use unity_recompile)");
                    return false;
                }
                string convName = newConv.ImplicitOrExplicitKeyword.IsKind(SyntaxKind.ImplicitKeyword)
                    ? "op_Implicit"
                    : "op_Explicit";
                AddMethod(result, metadataName, convName, ParamTypeNames(newConv.ParameterList), isStatic: true, isCtor: false, added: false);
                return true;
            }

            case PropertyDeclarationSyntax newProperty:
            {
                var oldProperty = (PropertyDeclarationSyntax)oldMember;
                return DiffAccessors(
                    metadataName,
                    oldProperty.AccessorList,
                    oldProperty.ExpressionBody,
                    newProperty.AccessorList,
                    newProperty.ExpressionBody,
                    newProperty.Identifier.Text,
                    indexerParams: null,
                    TokenText(newProperty.Type),
                    newProperty.Modifiers.Any(SyntaxKind.StaticKeyword),
                    newProperty.ExplicitInterfaceSpecifier != null,
                    genericContext,
                    burstMember,
                    result);
            }

            case IndexerDeclarationSyntax newIndexer:
            {
                var oldIndexer = (IndexerDeclarationSyntax)oldMember;
                return DiffAccessors(
                    metadataName,
                    oldIndexer.AccessorList,
                    oldIndexer.ExpressionBody,
                    newIndexer.AccessorList,
                    newIndexer.ExpressionBody,
                    "Item",
                    newIndexer.ParameterList,
                    TokenText(newIndexer.Type),
                    isStatic: false,
                    newIndexer.ExplicitInterfaceSpecifier != null,
                    genericContext,
                    burstMember,
                    result);
            }

            case EventDeclarationSyntax newEvent:
            {
                var oldEvent = (EventDeclarationSyntax)oldMember;
                bool changed = false;
                foreach (AccessorDeclarationSyntax newAccessor in newEvent.AccessorList?.Accessors ?? default)
                {
                    AccessorDeclarationSyntax? oldAccessor = oldEvent.AccessorList?.Accessors
                        .FirstOrDefault(a => a.Keyword.Text == newAccessor.Keyword.Text);
                    if (oldAccessor == null ||
                        BodyText(oldAccessor.Body, oldAccessor.ExpressionBody) != BodyText(newAccessor.Body, newAccessor.ExpressionBody))
                    {
                        if (burstMember ||
                            (oldAccessor != null && HasBurstCompileAttribute(oldAccessor)) ||
                            HasBurstCompileAttribute(newAccessor))
                        {
                            result.Reasons.Add("Burst-compiled event changed: " + metadataName + "." + newEvent.Identifier.Text);
                            return false;
                        }
                        if (genericContext)
                        {
                            result.Reasons.Add("generic type event changed: " + metadataName);
                            return false;
                        }
                        if (newEvent.ExplicitInterfaceSpecifier != null)
                        {
                            result.Reasons.Add("explicit interface event changed: " + metadataName);
                            return false;
                        }
                        string prefix = newAccessor.Keyword.Text == "add" ? "add_" : "remove_";
                        AddMethod(
                            result, metadataName, prefix + newEvent.Identifier.Text,
                            new[] { SimpleTypeName(newEvent.Type) },
                            newEvent.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false, added: false);
                        changed = true;
                    }
                }
                return changed;
            }

            default:
                return false;
        }
    }

    /// <summary>B1: a generic body cannot be re-detoured (Mono JITs generic
    /// methods per instantiation), but the edit decomposes into REMOVE
    /// (tombstone, M5 — no stub) + ADD (same-signature generic shim, M2),
    /// with the M3 caller scan verifying every compiled call site of the old
    /// body lives in this batch; in-batch call sites rewrite to direct shim
    /// calls and their kept containing members re-detour (PatchRewriter).
    /// Virtual/explicit-interface/Burst forms fall out cold through the
    /// remove/add classifiers; Unity messages are guarded here (the engine's
    /// dispatch cannot be replaced by a shim direct call).</summary>
    private static bool DecomposeGenericBodyChange(
        string metadataName,
        MethodDeclarationSyntax oldMethod,
        MethodDeclarationSyntax newMethod,
        bool genericContext,
        bool burstContext,
        HotDiffFileResult result)
    {
        if (UnityMagicMethods.Contains(newMethod.Identifier.Text))
        {
            result.Reasons.Add("generic method body changed: " + metadataName + "." + newMethod.Identifier.Text +
                " (Unity message methods cannot take the remove+add path; use unity_recompile)");
            return false;
        }

        int checksBefore = result.RequiresCallerCheck.Count;
        if (!ClassifyRemovedMember(metadataName, oldMethod, genericContext, burstContext, result))
            return false;
        if (!ClassifyAddedMember(metadataName, newMethod, genericContext, burstContext, result))
            return false;

        // The caller check came from the REMOVE half; relabel so a cold
        // verdict names the actual edit instead of "member removed".
        for (int i = checksBefore; i < result.RequiresCallerCheck.Count; i++)
        {
            result.RequiresCallerCheck[i].Detail =
                "generic method body changed: " + metadataName + "." + newMethod.Identifier.Text;
        }
        return true;
    }

    private static bool DiffAccessors(
        string metadataName,
        AccessorListSyntax? oldAccessors,
        ArrowExpressionClauseSyntax? oldExpressionBody,
        AccessorListSyntax? newAccessors,
        ArrowExpressionClauseSyntax? newExpressionBody,
        string propertyName,
        BaseParameterListSyntax? indexerParams,
        string propertyTypeText,
        bool isStatic,
        bool explicitInterface,
        bool genericContext,
        bool burstContext,
        HotDiffFileResult result)
    {
        // `int X => expr;` is a get_X body.
        if (oldExpressionBody != null || newExpressionBody != null)
        {
            string oldBody = oldExpressionBody != null ? TokenText(oldExpressionBody) : "";
            string newBody = newExpressionBody != null ? TokenText(newExpressionBody) : "";
            if (oldBody == newBody)
                return false;
            if (burstContext)
            {
                result.Reasons.Add("Burst-compiled property changed: " + metadataName + "." + propertyName);
                return false;
            }
            if (genericContext || explicitInterface)
            {
                result.Reasons.Add(
                    (explicitInterface ? "explicit interface property changed: " : "generic type property changed: ") +
                    metadataName + "." + propertyName);
                return false;
            }
            AddMethod(
                result, metadataName, "get_" + propertyName,
                indexerParams != null ? ParamTypeNames(indexerParams) : Array.Empty<string>(),
                isStatic, isCtor: false, added: false);
            return true;
        }

        bool changed = false;
        foreach (AccessorDeclarationSyntax newAccessor in newAccessors?.Accessors ?? default)
        {
            AccessorDeclarationSyntax? oldAccessor = oldAccessors?.Accessors
                .FirstOrDefault(a => a.Keyword.Text == newAccessor.Keyword.Text);
            // Header equality already guarantees the accessor sets match for
            // non-auto properties (accessor list is part of the header).
            if (oldAccessor == null)
                continue;

            if (BodyText(oldAccessor.Body, oldAccessor.ExpressionBody) == BodyText(newAccessor.Body, newAccessor.ExpressionBody))
                continue;

            if (burstContext || HasBurstCompileAttribute(oldAccessor) || HasBurstCompileAttribute(newAccessor))
            {
                result.Reasons.Add("Burst-compiled property changed: " + metadataName + "." + propertyName);
                return false;
            }

            if (genericContext || explicitInterface)
            {
                result.Reasons.Add(
                    (explicitInterface ? "explicit interface property changed: " : "generic type property changed: ") +
                    metadataName + "." + propertyName);
                return false;
            }

            AddMethod(
                result, metadataName,
                AccessorName(newAccessor, propertyName),
                AccessorParams(newAccessor, indexerParams, propertyTypeText),
                isStatic, isCtor: false, added: false);
            changed = true;
        }
        return changed;
    }

    internal static string AccessorName(AccessorDeclarationSyntax accessor, string propertyName)
    {
        // init-only accessors compile to set_X (with a modreq).
        string prefix = accessor.Keyword.Text == "get" ? "get_" : "set_";
        return prefix + propertyName;
    }

    internal static string[] AccessorParams(
        AccessorDeclarationSyntax accessor,
        BaseParameterListSyntax? indexerParams,
        string propertyTypeText)
    {
        var names = new List<string>();
        if (indexerParams != null)
            names.AddRange(ParamTypeNames(indexerParams));
        if (accessor.Keyword.Text != "get")
            names.Add(SimpleTypeNameFromText(propertyTypeText));
        return names.ToArray();
    }

    private static void AddMethod(
        HotDiffFileResult result,
        string declaringType,
        string name,
        string[] paramTypeNames,
        bool isStatic,
        bool isCtor,
        bool added,
        int typeParameterCount = 0)
    {
        result.ChangedMethods.Add(new HotDiffMethod
        {
            DeclaringType = declaringType,
            Name = name,
            ParamTypeNames = paramTypeNames,
            IsStatic = isStatic,
            IsCtor = isCtor,
            Added = added,
            TypeParameterCount = typeParameterCount,
        });
    }

    /// <summary>The first method type parameter whose name shadows a
    /// declaring-chain type parameter (CS0693 source — legal C#, but the
    /// flattened generic shim would declare the name twice), or null.</summary>
    private static string? FindShadowedMethodTypeParameter(MethodDeclarationSyntax method)
    {
        if (method.TypeParameterList == null)
            return null;
        var chainNames = new HashSet<string>(StringComparer.Ordinal);
        for (SyntaxNode? current = method.Parent; current != null; current = current.Parent)
        {
            if (current is TypeDeclarationSyntax type && type.TypeParameterList != null)
            {
                foreach (TypeParameterSyntax parameter in type.TypeParameterList.Parameters)
                    chainNames.Add(parameter.Identifier.Text);
            }
        }
        foreach (TypeParameterSyntax parameter in method.TypeParameterList.Parameters)
        {
            if (chainNames.Contains(parameter.Identifier.Text))
                return parameter.Identifier.Text;
        }
        return null;
    }

    // ── text helpers ─────────────────────────────────────────────────

    /// <summary>Token-level text: whitespace and comments do not count as
    /// changes (they cannot affect IL). Tokens are joined with an
    /// explicit separator so adjacent tokens never merge into a different
    /// token stream ("--x" vs "- -x").</summary>
    internal static string TokenText(SyntaxNode node)
    {
        return string.Join("\u0001", node.DescendantTokens().Select(t => t.Text));
    }

    private static string BodyText(BlockSyntax? body, ArrowExpressionClauseSyntax? expressionBody)
    {
        if (body != null)
            return "B" + TokenText(body);
        if (expressionBody != null)
            return "E" + TokenText(expressionBody);
        return "";
    }

    /// <summary>The member's declaration with every body removed — the
    /// signature/modifier/attribute surface whose change forces a recompile.
    /// Accessibility modifiers are stripped (compared separately: widening
    /// is hot/noop, narrowing needs the caller check); the async modifier is
    /// stripped from methods (it is part of the BODY comparison — an
    /// async↔sync flip changes the compiled body, not the signature).</summary>
    private static string HeaderText(MemberDeclarationSyntax member)
    {
        SyntaxNode stripped = member switch
        {
            MethodDeclarationSyntax m => m
                .WithModifiers(StripHeaderModifiers(m.Modifiers, stripAsync: true))
                .WithBody(null).WithExpressionBody(null).WithSemicolonToken(default),
            ConstructorDeclarationSyntax c => c
                .WithModifiers(StripHeaderModifiers(c.Modifiers, stripAsync: false))
                .WithBody(null).WithExpressionBody(null).WithInitializer(null).WithSemicolonToken(default),
            DestructorDeclarationSyntax d => d.WithBody(null).WithExpressionBody(null).WithSemicolonToken(default),
            OperatorDeclarationSyntax o => o.WithBody(null).WithExpressionBody(null).WithSemicolonToken(default),
            ConversionOperatorDeclarationSyntax v => v.WithBody(null).WithExpressionBody(null).WithSemicolonToken(default),
            PropertyDeclarationSyntax p => p
                .WithModifiers(StripHeaderModifiers(p.Modifiers, stripAsync: false))
                .WithExpressionBody(null)
                .WithInitializer(null)
                .WithSemicolonToken(default)
                .WithAccessorList(StripAccessorBodies(p.AccessorList)),
            IndexerDeclarationSyntax i => i
                .WithModifiers(StripHeaderModifiers(i.Modifiers, stripAsync: false))
                .WithExpressionBody(null)
                .WithSemicolonToken(default)
                .WithAccessorList(StripAccessorBodies(i.AccessorList)),
            EventDeclarationSyntax e => e
                .WithModifiers(StripHeaderModifiers(e.Modifiers, stripAsync: false))
                .WithAccessorList(StripAccessorBodies(e.AccessorList)),
            _ => member,
        };
        return TokenText(stripped);
    }

    /// <summary>The method's signature surface only — header minus member
    /// AND parameter attribute lists. When two headers differ but their
    /// signatures match, the difference is attribute-only (stays cold);
    /// when signatures differ, the change decomposes into remove+add.</summary>
    private static string SignatureText(MethodDeclarationSyntax method)
    {
        MethodDeclarationSyntax stripped = method
            .WithAttributeLists(default)
            .WithModifiers(StripHeaderModifiers(method.Modifiers, stripAsync: true))
            .WithParameterList(method.ParameterList.WithParameters(SyntaxFactory.SeparatedList(
                method.ParameterList.Parameters.Select(p => p.WithAttributeLists(default)))))
            .WithBody(null)
            .WithExpressionBody(null)
            .WithSemicolonToken(default);
        return TokenText(stripped);
    }

    /// <summary>Metadata member names the IL caller scan should look for.</summary>
    private static string[] ScanNamesFor(MemberDeclarationSyntax member)
    {
        return member switch
        {
            MethodDeclarationSyntax method => new[] { method.Identifier.Text },
            ConstructorDeclarationSyntax => new[] { ".ctor" },
            PropertyDeclarationSyntax property => new[]
            {
                "get_" + property.Identifier.Text,
                "set_" + property.Identifier.Text,
            },
            IndexerDeclarationSyntax => new[] { "get_Item", "set_Item" },
            EventDeclarationSyntax @event => new[]
            {
                "add_" + @event.Identifier.Text,
                "remove_" + @event.Identifier.Text,
            },
            _ => Array.Empty<string>(),
        };
    }

    private static SyntaxTokenList StripHeaderModifiers(SyntaxTokenList modifiers, bool stripAsync)
    {
        return SyntaxFactory.TokenList(modifiers.Where(m =>
            !m.IsKind(SyntaxKind.PublicKeyword) &&
            !m.IsKind(SyntaxKind.PrivateKeyword) &&
            !m.IsKind(SyntaxKind.ProtectedKeyword) &&
            !m.IsKind(SyntaxKind.InternalKeyword) &&
            !(stripAsync && m.IsKind(SyntaxKind.AsyncKeyword))));
    }

    /// <summary>Async modifier folded into the body text: an async↔sync flip
    /// with an identical token body still swaps state machine for direct
    /// code and must re-detour.</summary>
    private static string MethodBodyText(MethodDeclarationSyntax method)
    {
        return (method.Modifiers.Any(SyntaxKind.AsyncKeyword) ? "A|" : "") +
            BodyText(method.Body, method.ExpressionBody);
    }

    // ── member accessibility (widen = hot, narrow = caller check) ────

    private enum MemberAccess
    {
        Private,
        PrivateProtected,
        Protected,
        Internal,
        ProtectedInternal,
        Public,
    }

    private static MemberAccess DeclaredAccess(MemberDeclarationSyntax member)
    {
        SyntaxTokenList modifiers = member.Modifiers;
        bool isPublic = modifiers.Any(SyntaxKind.PublicKeyword);
        bool isPrivate = modifiers.Any(SyntaxKind.PrivateKeyword);
        bool isProtected = modifiers.Any(SyntaxKind.ProtectedKeyword);
        bool isInternal = modifiers.Any(SyntaxKind.InternalKeyword);
        if (isPublic)
            return MemberAccess.Public;
        if (isPrivate && isProtected)
            return MemberAccess.PrivateProtected;
        if (isProtected && isInternal)
            return MemberAccess.ProtectedInternal;
        if (isProtected)
            return MemberAccess.Protected;
        if (isInternal)
            return MemberAccess.Internal;
        // Explicit private or the class/struct member default. (Interface
        // members never reach this: interfaces diff as a whole.)
        return MemberAccess.Private;
    }

    /// <summary>Old ⊆ new in the accessibility-domain partial order.
    /// protected↔internal are incomparable and count as narrowing.</summary>
    private static bool IsAccessWideningOrEqual(MemberAccess oldAccess, MemberAccess newAccess)
    {
        if (oldAccess == newAccess)
            return true;
        return (oldAccess, newAccess) switch
        {
            (MemberAccess.Private, _) => true,
            (MemberAccess.PrivateProtected,
                MemberAccess.Protected or MemberAccess.Internal or
                MemberAccess.ProtectedInternal or MemberAccess.Public) => true,
            (MemberAccess.Protected, MemberAccess.ProtectedInternal or MemberAccess.Public) => true,
            (MemberAccess.Internal, MemberAccess.ProtectedInternal or MemberAccess.Public) => true,
            (MemberAccess.ProtectedInternal, MemberAccess.Public) => true,
            _ => false,
        };
    }

    private static AccessorListSyntax? StripAccessorBodies(AccessorListSyntax? accessors)
    {
        if (accessors == null)
            return null;
        return accessors.WithAccessors(
            SyntaxFactory.List(accessors.Accessors.Select(a =>
                a.WithBody(null).WithExpressionBody(null).WithSemicolonToken(
                    SyntaxFactory.Token(SyntaxKind.SemicolonToken)))));
    }

    private static string AttributeListsText(SyntaxList<AttributeListSyntax> lists)
    {
        return string.Join("\n", lists.Select(TokenText));
    }

    private static string UsingAndExternText(CompilationUnitSyntax root)
    {
        var parts = new List<string>();
        parts.AddRange(root.Externs.Select(TokenText));
        parts.AddRange(root.Usings.Select(TokenText));
        return string.Join("\n", parts);
    }

    private static string DelegateDeclarationsText(CompilationUnitSyntax root)
    {
        return string.Join(
            "\n",
            root.DescendantNodes()
                .OfType<DelegateDeclarationSyntax>()
                .Select(TokenText)
                .OrderBy(text => text, StringComparer.Ordinal));
    }

    private static string ModifiersText(SyntaxTokenList modifiers)
    {
        return string.Join(" ", modifiers.Select(m => m.Text));
    }

    private static bool IsPrivateOrImplicitPrivate(SyntaxTokenList modifiers)
    {
        bool hasPublicSurface = modifiers.Any(m =>
            m.IsKind(SyntaxKind.PublicKeyword) ||
            m.IsKind(SyntaxKind.ProtectedKeyword) ||
            m.IsKind(SyntaxKind.InternalKeyword));
        return !hasPublicSurface;
    }

    internal static bool HasBurstCompileAttribute(SyntaxNode node)
    {
        SyntaxList<AttributeListSyntax> lists = node switch
        {
            BaseTypeDeclarationSyntax type => type.AttributeLists,
            DelegateDeclarationSyntax del => del.AttributeLists,
            MethodDeclarationSyntax method => method.AttributeLists,
            ConstructorDeclarationSyntax ctor => ctor.AttributeLists,
            OperatorDeclarationSyntax op => op.AttributeLists,
            ConversionOperatorDeclarationSyntax conv => conv.AttributeLists,
            PropertyDeclarationSyntax property => property.AttributeLists,
            IndexerDeclarationSyntax indexer => indexer.AttributeLists,
            EventDeclarationSyntax @event => @event.AttributeLists,
            AccessorDeclarationSyntax accessor => accessor.AttributeLists,
            _ => default,
        };
        foreach (AttributeListSyntax list in lists)
        {
            foreach (AttributeSyntax attribute in list.Attributes)
            {
                string name = TokenText(attribute.Name).Replace("\u0001", "", StringComparison.Ordinal);
                if (name.EndsWith("BurstCompile", StringComparison.Ordinal) ||
                    name.EndsWith("BurstCompileAttribute", StringComparison.Ordinal))
                {
                    return true;
                }
            }
        }
        return false;
    }

    private static string DisplayName(MemberDeclarationSyntax member)
    {
        return member switch
        {
            MethodDeclarationSyntax m => m.Identifier.Text,
            ConstructorDeclarationSyntax => ".ctor",
            DestructorDeclarationSyntax => "~dtor",
            OperatorDeclarationSyntax o => "operator" + o.OperatorToken.Text,
            ConversionOperatorDeclarationSyntax => "conversion",
            PropertyDeclarationSyntax p => p.Identifier.Text,
            IndexerDeclarationSyntax => "this[]",
            EventDeclarationSyntax e => e.Identifier.Text,
            _ => member.Kind().ToString(),
        };
    }

    // ── reflection-style parameter type names ────────────────────────

    internal static string[] ParamTypeNames(BaseParameterListSyntax? parameters)
    {
        if (parameters == null)
            return Array.Empty<string>();

        return parameters.Parameters
            .Select(p =>
            {
                string name = p.Type != null ? SimpleTypeName(p.Type) : "";
                bool byRef = p.Modifiers.Any(m =>
                    m.IsKind(SyntaxKind.RefKeyword) || m.IsKind(SyntaxKind.OutKeyword) || m.IsKind(SyntaxKind.InKeyword));
                return byRef ? name + "&" : name;
            })
            .ToArray();
    }

    /// <summary>Reflection `Type.Name`-style simple name from syntax:
    /// `int` → "Int32", `List&lt;int&gt;` → "List`1", `string[]` → "String[]",
    /// `int?` → "Nullable`1", `(int, string)` → "ValueTuple`2".</summary>
    internal static string SimpleTypeName(TypeSyntax type)
    {
        switch (type)
        {
            case PredefinedTypeSyntax predefined:
                return PredefinedName(predefined.Keyword.Text);
            case NullableTypeSyntax:
                return "Nullable`1";
            case ArrayTypeSyntax array:
            {
                string element = SimpleTypeName(array.ElementType);
                foreach (ArrayRankSpecifierSyntax rank in array.RankSpecifiers)
                    element += "[" + new string(',', rank.Rank - 1) + "]";
                return element;
            }
            case GenericNameSyntax generic:
                return generic.Identifier.Text + "`" + generic.TypeArgumentList.Arguments.Count;
            case IdentifierNameSyntax identifier:
                return identifier.Identifier.Text == "dynamic" ? "Object" : identifier.Identifier.Text;
            case QualifiedNameSyntax qualified:
                return SimpleTypeName(qualified.Right);
            case AliasQualifiedNameSyntax alias:
                return SimpleTypeName(alias.Name);
            case TupleTypeSyntax tuple:
                return "ValueTuple`" + tuple.Elements.Count;
            case RefTypeSyntax refType:
                return SimpleTypeName(refType.Type) + "&";
            default:
                return type.ToString();
        }
    }

    private static string SimpleTypeNameFromText(string typeText)
    {
        TypeSyntax parsed = SyntaxFactory.ParseTypeName(typeText);
        return SimpleTypeName(parsed);
    }

    /// <summary>Simple (arity-stripped) name of a metadata type name's last
    /// segment: "Ns.Outer+Inner`1" → "Inner".</summary>
    private static string SimpleNameOfMetadata(string metadataName)
    {
        int separator = Math.Max(metadataName.LastIndexOf('.'), metadataName.LastIndexOf('+'));
        string simple = separator < 0 ? metadataName : metadataName[(separator + 1)..];
        int backtick = simple.IndexOf('`');
        return backtick < 0 ? simple : simple[..backtick];
    }

    private static string PredefinedName(string keyword)
    {
        return keyword switch
        {
            "bool" => "Boolean",
            "byte" => "Byte",
            "sbyte" => "SByte",
            "char" => "Char",
            "decimal" => "Decimal",
            "double" => "Double",
            "float" => "Single",
            "int" => "Int32",
            "uint" => "UInt32",
            "long" => "Int64",
            "ulong" => "UInt64",
            "short" => "Int16",
            "ushort" => "UInt16",
            "object" => "Object",
            "string" => "String",
            "void" => "Void",
            _ => keyword,
        };
    }

    internal static string? OperatorMetadataName(string token, int paramCount)
    {
        return token switch
        {
            "+" => paramCount == 1 ? "op_UnaryPlus" : "op_Addition",
            "-" => paramCount == 1 ? "op_UnaryNegation" : "op_Subtraction",
            "*" => "op_Multiply",
            "/" => "op_Division",
            "%" => "op_Modulus",
            "!" => "op_LogicalNot",
            "~" => "op_OnesComplement",
            "++" => "op_Increment",
            "--" => "op_Decrement",
            "true" => "op_True",
            "false" => "op_False",
            "&" => "op_BitwiseAnd",
            "|" => "op_BitwiseOr",
            "^" => "op_ExclusiveOr",
            "<<" => "op_LeftShift",
            ">>" => "op_RightShift",
            "==" => "op_Equality",
            "!=" => "op_Inequality",
            "<" => "op_LessThan",
            ">" => "op_GreaterThan",
            "<=" => "op_LessThanOrEqual",
            ">=" => "op_GreaterThanOrEqual",
            _ => null,
        };
    }

    /// <summary>MonoBehaviour/Editor message names Unity discovers per type
    /// at load time — adding one can never take effect through a detour.
    /// Curated SUPERSET across Unity versions (UI/Canvas, animation and
    /// particle-job callbacks included): a name listed here but unused by
    /// the project's Unity version only costs a false cold, while a missing
    /// name would report hot success that silently never runs.</summary>
    internal static readonly HashSet<string> UnityMagicMethods = new(StringComparer.Ordinal)
    {
        "Awake", "FixedUpdate", "LateUpdate", "OnAnimatorIK", "OnAnimatorMove",
        "OnApplicationFocus", "OnApplicationPause", "OnApplicationQuit",
        "OnAudioFilterRead", "OnBecameInvisible", "OnBecameVisible",
        "OnBeforeTransformParentChanged", "OnCanvasGroupChanged", "OnCanvasHierarchyChanged",
        "OnCollisionEnter", "OnCollisionEnter2D", "OnCollisionExit", "OnCollisionExit2D",
        "OnCollisionStay", "OnCollisionStay2D", "OnControllerColliderHit",
        "OnDestroy", "OnDidApplyAnimationProperties", "OnDisable",
        "OnDrawGizmos", "OnDrawGizmosSelected",
        "OnEnable", "OnGUI", "OnJointBreak", "OnJointBreak2D", "OnLevelWasLoaded",
        "OnMouseDown", "OnMouseDrag", "OnMouseEnter", "OnMouseExit",
        "OnMouseOver", "OnMouseUp", "OnMouseUpAsButton",
        "OnParticleCollision", "OnParticleSystemStopped", "OnParticleTrigger",
        "OnParticleUpdateJobScheduled",
        "OnPostRender", "OnPreCull", "OnPreRender",
        "OnRectTransformDimensionsChange", "OnRenderImage", "OnRenderObject",
        "OnTransformChildrenChanged", "OnTransformParentChanged",
        "OnTriggerEnter", "OnTriggerEnter2D", "OnTriggerExit", "OnTriggerExit2D",
        "OnTriggerStay", "OnTriggerStay2D", "OnValidate", "OnWillRenderObject",
        "Reset", "Start", "Update",
    };
}
