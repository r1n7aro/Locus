using System.Collections.Immutable;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Locus.CompileServer;

// ── result models ────────────────────────────────────────────────────

/// <summary>One original→patch method pair for the Unity-side detour.</summary>
public sealed class PatchMethodMap
{
    public string DeclaringType = "";
    public string PatchDeclaringType = "";
    public string Name = "";
    public string[] ParamTypeNames = Array.Empty<string>();

    /// <summary>Enriched per-parameter identity (namespace + closed generic
    /// arguments) parallel to <see cref="ParamTypeNames"/>, for breaking ties
    /// between overloads that share simple names. Empty when no enrichment is
    /// available — Unity then matches on the simple names and, on a tie, falls
    /// back to a fail-closed cold verdict.</summary>
    public string[] ParamTypeSigs = Array.Empty<string>();

    public bool IsStatic;
    public bool IsCtor;

    /// <summary>When set, the "original" side lives in this specific
    /// assembly (an earlier patch's shim being re-edited) instead of the
    /// project assemblies.</summary>
    public string? OriginalAssembly;

    /// <summary>The patch side is a synthesized empty body (a deleted Unity
    /// message method being silenced) — observability for the tool output.</summary>
    public bool IsStub;
}

/// <summary>A type that only exists in the new text (TI-C / ImageRegistry).</summary>
public sealed class PatchNewType
{
    public string MetadataName = "";
    public string Namespace = "";
    public string SimpleName = "";
    public bool IsPublic;
    public bool IsTopLevel;
}

/// <summary>A shim materialized by THIS patch, to be committed into the
/// MemberSurfaceRegistry once Unity accepts the assembly.</summary>
public sealed class ShimRegistration
{
    public string MemberKey = "";
    public MemberSurfaceRegistry.ShimEntry Entry = new();
}

/// <summary>A field store/holder introduced by THIS patch (M4), committed
/// into the FieldStoreRegistry once Unity accepts the assembly.</summary>
public sealed class FieldStoreRegistration
{
    public string FieldKey = "";
    public FieldStoreRegistry.StoreEntry Entry = new();
}

public sealed class PatchRewriteResult
{
    /// <summary>Rewritten tree, ready for the patch compilation.</summary>
    public SyntaxTree? Tree;

    public List<PatchMethodMap> Methods = new();
    public List<PatchNewType> NewTypes = new();
    public List<ShimRegistration> ShimRegistrations = new();
    public List<FieldStoreRegistration> FieldStoreRegistrations = new();

    /// <summary>Assemblies whose non-public members the patch may touch
    /// (the original assemblies of the patched types).</summary>
    public List<string> OriginalAssemblies = new();

    /// <summary>Set when the file must take the cold path after all — e.g.
    /// the original assembly's field layout does not match the baseline.</summary>
    public string? ColdReason;

    /// <summary>Deterministic agent-facing rewrite error (e.g. a reference
    /// to a tombstoned member): recompiling would fail identically, so this
    /// is NOT a cold reason.</summary>
    public string? Error;
}

// ── rewriter ─────────────────────────────────────────────────────────

/// <summary>
/// Turns an edited source file into a hot-patch source:
///
///  1. every type declared in the file (except brand-new ones) is renamed
///     `Foo` → `Foo__LocusPatch`, so the patch assembly never collides with
///     the original;
///  2. every *reference* to batch-local pre-existing types — bodies,
///     signatures, base lists, typeof, attributes — is rewritten to a
///     fully-qualified name, which after the rename binds to the *original*
///     assembly's type: object identity, serialization and Unity APIs keep
///     seeing original types;
///  3. unqualified static field/property/event accesses are qualified back
///     to the original type, keeping static state single-sourced;
///  4. static constructors and static field initializers are emptied so the
///     patch type's cctor can never re-run side effects;
///  5. the original assembly's instance-field layout is compared against the
///     source as a guard against a stale baseline (fails closed to cold);
///  6. ADDED members (new surface, M2) are extracted into a static shim
///     class (`Foo__LocusShims`): instance members become
///     `static R M(this global::Ns.Foo self, ...)` with `this.` rewritten to
///     `self.`, and every batch call site binding to an added member is
///     rewritten to a fully-qualified DIRECT shim call (extension resolution
///     could be captured by an applicable original-metadata overload);
///  7. a standalone `this` inside kept members (escaping as an argument or
///     value) is rewritten to `((global::Ns.Foo)(object)this)` — the runtime
///     object IS an original instance, only the static type differs.
///
/// Instance fields keep their declarations (same ordered layout as the
/// original — guaranteed hot-side by HotDiff, original-side by the guard in
/// step 5), so patched bodies read correct offsets through original `this`.
/// </summary>
public static class PatchRewriter
{
    public const string TypeNameSuffix = "__LocusPatch";
    public const string ShimTypeSuffix = "__LocusShims";

    /// <summary>Single-file convenience (tests / callers without a batch):
    /// builds a one-file batch context and rewrites.</summary>
    public static PatchRewriteResult Rewrite(
        string path,
        string newText,
        HotDiffFileResult diff,
        CSharpParseOptions parseOptions,
        ImmutableArray<MetadataReference> references)
    {
        SyntaxTree tree = CSharpSyntaxTree.ParseText(newText, parseOptions, path: path);
        PatchBatchContext batch = PatchBatchContext.Build(
            new[] { (path, tree, diff) },
            references,
            new Dictionary<string, MemberSurfaceRegistry.ShimEntry>(StringComparer.Ordinal));
        return Rewrite(path, tree, diff, parseOptions, batch);
    }

    public static PatchRewriteResult Rewrite(
        string path,
        SyntaxTree tree,
        HotDiffFileResult diff,
        CSharpParseOptions parseOptions,
        PatchBatchContext batch)
    {
        var result = new PatchRewriteResult();

        var root = (CompilationUnitSyntax)tree.GetRoot();
        SemanticModel model = batch.ModelFor(tree);
        CSharpCompilation binding = batch.Binding;

        var newTypeNames = new HashSet<string>(diff.NewTypes, StringComparer.Ordinal);

        // File-local pre-existing types: ALL of them (nested included) get
        // their references rewritten; only TOP-LEVEL declarations get the
        // identifier rename — nested metadata names change through their
        // outer type ("Ns.Outer__LocusPatch+Inner"). The rename SYMBOL set
        // is batch-wide (cross-file references rewrite too).
        var localDecls = new List<BaseTypeDeclarationSyntax>();
        var topLevelDecls = new List<BaseTypeDeclarationSyntax>();
        HashSet<INamedTypeSymbol> renamedSymbols = batch.RenamedSymbols;
        var topLevelDelegates = new List<DelegateDeclarationSyntax>();

        bool IsTopLevel(SyntaxNode decl) =>
            decl.Parent is BaseNamespaceDeclarationSyntax || decl.Parent is CompilationUnitSyntax;

        foreach (MemberDeclarationSyntax member in root.DescendantNodes().OfType<MemberDeclarationSyntax>())
        {
            switch (member)
            {
                case BaseTypeDeclarationSyntax typeDecl:
                {
                    string metadataName = HotDiff.MetadataName(typeDecl);
                    if (newTypeNames.Contains(metadataName))
                    {
                        // A new NESTED type inside a pre-existing container
                        // lives inside the RENAMED patch copy: report the
                        // runtime metadata name.
                        int plus = metadataName.IndexOf('+');
                        string runtimeName = plus > 0 && !newTypeNames.Contains(metadataName[..plus])
                            ? PatchTypeName(metadataName)
                            : metadataName;
                        CollectNewType(model, typeDecl, runtimeName, result);
                        continue;
                    }
                    localDecls.Add(typeDecl);
                    if (IsTopLevel(typeDecl))
                        topLevelDecls.Add(typeDecl);
                    break;
                }
                case DelegateDeclarationSyntax delegateDecl:
                {
                    // Delegates: reference-rewrite like any other type so
                    // signatures keep matching originals; rename top-level
                    // declarations only.
                    if (IsTopLevel(delegateDecl))
                        topLevelDelegates.Add(delegateDecl);
                    break;
                }
            }
        }

        // M4: field changes per type, with each added field bound to its
        // store (an earlier batch's registered store, or a new one this
        // patch declares).
        var fieldChangesByType = diff.FieldChanges
            .GroupBy(c => c.DeclaringType, StringComparer.Ordinal)
            .ToDictionary(g => g.Key, g => g.ToList(), StringComparer.Ordinal);
        var addedFieldSymbols = new Dictionary<IFieldSymbol, AddedFieldInfo>(SymbolEqualityComparer.Default);
        var addedFieldsByType = new Dictionary<string, List<AddedFieldInfo>>(StringComparer.Ordinal);

        foreach (var pair in fieldChangesByType)
        {
            BaseTypeDeclarationSyntax? decl = localDecls
                .FirstOrDefault(d => HotDiff.MetadataName(d) == pair.Key);
            if (decl is not TypeDeclarationSyntax typeDecl)
                continue;

            foreach (HotDiffFieldChange change in pair.Value.Where(c => c.Kind == "added"))
            {
                VariableDeclaratorSyntax? declarator = FindFieldDeclarator(typeDecl, change.Name, change.IsStatic);
                if (declarator == null)
                {
                    // B2: an added AUTO-PROPERTY's backing field has no
                    // declarator — the store binds to the property
                    // declaration, and the accessor shims read/write it.
                    AddedFieldInfo? autoInfo = BuildAutoPropertyFieldInfo(pair.Key, typeDecl, change, model, batch);
                    if (autoInfo != null)
                    {
                        if (!addedFieldsByType.TryGetValue(pair.Key, out List<AddedFieldInfo>? autoList))
                            addedFieldsByType[pair.Key] = autoList = new List<AddedFieldInfo>();
                        autoList.Add(autoInfo);
                    }
                    continue;
                }
                if (model.GetDeclaredSymbol(declarator) is not IFieldSymbol fieldSymbol)
                    continue;

                AddedFieldInfo info = BuildAddedFieldInfo(pair.Key, typeDecl, change, fieldSymbol, declarator, batch);
                addedFieldSymbols[fieldSymbol] = info;
                if (!addedFieldsByType.TryGetValue(pair.Key, out List<AddedFieldInfo>? list))
                    addedFieldsByType[pair.Key] = list = new List<AddedFieldInfo>();
                list.Add(info);
            }
        }

        // ── locate ADDED members (M2) in this file ───────────────────

        var addedDecls = new Dictionary<MethodDeclarationSyntax, ShimTarget>();
        var addedAccessors = new List<AddedAccessorInfo>();
        foreach (HotDiffMethod added in diff.ChangedMethods.Where(m => m.Added))
        {
            MethodDeclarationSyntax? decl = PatchBatchContext.FindAddedMethodDeclaration(root, added);
            if (decl != null)
            {
                if (model.GetDeclaredSymbol(decl) is not IMethodSymbol symbol)
                    continue;
                if (batch.AddedMembers.TryGetValue(symbol.OriginalDefinition, out ShimTarget? target))
                    addedDecls[decl] = target;
                continue;
            }

            // B2: accessor-shaped additions (property/indexer/event).
            IMethodSymbol? accessorSymbol = PatchBatchContext.FindAddedAccessorSymbol(
                root, model, added, out BasePropertyDeclarationSyntax? container, out AccessorDeclarationSyntax? accessor);
            if (accessorSymbol == null || container == null)
                continue;
            if (!batch.AddedMembers.TryGetValue(accessorSymbol.OriginalDefinition, out ShimTarget? accessorTarget))
                continue;
            addedAccessors.Add(new AddedAccessorInfo
            {
                Target = accessorTarget,
                Symbol = accessorSymbol,
                Container = container,
                Accessor = accessor,
                IsAuto = container is PropertyDeclarationSyntax autoCandidate && HotDiff.IsAutoProperty(autoCandidate),
            });
        }

        // Auto-property accessors read/write their M4 backing store (the
        // FieldChange was recorded by HotDiff and materialized above).
        foreach (AddedAccessorInfo accessorInfo in addedAccessors.Where(a => a.IsAuto))
        {
            string backingName = HotDiff.AutoPropertyBackingFieldName(
                ((PropertyDeclarationSyntax)accessorInfo.Container).Identifier.Text);
            if (addedFieldsByType.TryGetValue(accessorInfo.Target.DeclaringTypeMetadataName, out List<AddedFieldInfo>? stores))
                accessorInfo.Store = stores.FirstOrDefault(i => i.RegistryFieldName == backingName);
            if (accessorInfo.Store == null)
            {
                // Defensive: HotDiff guarantees the FieldChange; without the
                // store the synthesized accessor body has nothing to touch.
                result.ColdReason = "added auto-property has no backing store: " +
                    accessorInfo.Target.DeclaringTypeMetadataName + "." + accessorInfo.Target.MethodName;
                return result;
            }
        }

        // Body spans of every added member (methods + accessor bodies): the
        // rewrite passes route `this` → self and implicit member access
        // through these.
        var addedBodySpans = new List<(SyntaxNode Node, ShimTarget Target)>();
        foreach (var addedPair in addedDecls)
            addedBodySpans.Add((addedPair.Key, addedPair.Value));
        foreach (AddedAccessorInfo accessorInfo in addedAccessors)
            addedBodySpans.Add((AccessorBodyNode(accessorInfo), accessorInfo.Target));

        bool InAddedMember(SyntaxNode node)
        {
            foreach (var (spanNode, _) in addedBodySpans)
            {
                if (spanNode.FullSpan.Contains(node.Span))
                    return true;
            }
            return false;
        }

        ShimTarget? EnclosingAddedTarget(SyntaxNode node)
        {
            foreach (var (spanNode, target) in addedBodySpans)
            {
                if (spanNode.FullSpan.Contains(node.Span))
                    return target;
            }
            return null;
        }

        // Mono reality check: shims run OUTSIDE the original type and the
        // Unity runtime enforces accessibility at JIT time. Whether a given
        // (operation × visibility) actually fails is MEASURED per runtime by
        // the C0 access probe (batch.RuntimeCaps): an added member's BODY may
        // reference non-public surface when every required cell is green
        // (C2′a) — caps absent or a red cell keeps the conservative cold
        // verdict, and non-public types in the shim's public SIGNATURE stay
        // cold regardless. Patch-materialized surface is always exempt.
        foreach (var addedPair in addedDecls)
        {
            if (model.GetDeclaredSymbol(addedPair.Key) is not IMethodSymbol addedSymbol)
                continue;
            string? accessViolation = FindShimAccessViolation(
                addedSymbol, addedPair.Key, model, batch, addedFieldSymbols, renamedSymbols);
            if (accessViolation != null)
            {
                result.ColdReason = accessViolation;
                return result;
            }
        }
        foreach (AddedAccessorInfo accessorInfo in addedAccessors)
        {
            // Auto accessors have no body — the signature checks still run
            // (the shim must NAME the property type in its declaration).
            string? accessViolation = FindShimAccessViolation(
                accessorInfo.Symbol, AccessorBodyNode(accessorInfo), model, batch, addedFieldSymbols, renamedSymbols);
            if (accessViolation != null)
            {
                result.ColdReason = accessViolation;
                return result;
            }
        }

        // ── B1: kept callers of RE-ADDED members must re-detour ──────
        // A re-added member (generic body change) keeps its old compiled
        // body live — nothing detours it. Its in-batch call sites rewrite
        // to direct shim calls below, but a rewrite inside a KEPT
        // (unchanged) member only takes effect once that member's patch
        // copy is detoured: ensure it, or fail the file closed when the
        // enclosing member cannot be re-detoured.
        var ensuredDetours = new List<HotDiffMethod>();
        if (batch.ReAddedMemberKeys.Count > 0)
        {
            string? ensureCold = EnsureReAddedCallerDetours(
                root, model, batch, diff, addedBodySpans, newTypeNames, ensuredDetours);
            if (ensureCold != null)
            {
                result.ColdReason = ensureCold;
                return result;
            }
        }

        // Layout guard + original assembly names for the patched types
        // (plus types whose kept members re-detour for re-added call sites).
        var guardTypes = new List<string>(diff.PatchedTypes);
        foreach (HotDiffMethod ensured in ensuredDetours)
        {
            if (!guardTypes.Contains(ensured.DeclaringType))
                guardTypes.Add(ensured.DeclaringType);
        }
        foreach (string patchedType in guardTypes)
        {
            BaseTypeDeclarationSyntax? decl = localDecls
                .FirstOrDefault(d => HotDiff.MetadataName(d) == patchedType);
            if (decl is not TypeDeclarationSyntax typeDecl)
                continue;
            if (model.GetDeclaredSymbol(typeDecl) is not INamedTypeSymbol sourceSymbol)
                continue;

            INamedTypeSymbol? original = FindOriginalType(binding, patchedType, out string? assemblyName);
            if (original == null)
            {
                result.ColdReason = "original type not found in references: " + patchedType;
                return result;
            }

            string? layoutError = fieldChangesByType.TryGetValue(patchedType, out List<HotDiffFieldChange>? typeChanges)
                ? VerifyVirtualizedLayout(sourceSymbol, original, typeChanges)
                : (InstanceFieldSequence(sourceSymbol).SequenceEqual(InstanceFieldSequence(original), StringComparer.Ordinal)
                    ? null
                    : "layout mismatch");
            if (layoutError != null)
            {
                result.ColdReason =
                    "original assembly field layout differs from the edited baseline for " + patchedType +
                    " (the file changed outside this session?); a unity_recompile will converge";
                return result;
            }
            if (assemblyName != null && !result.OriginalAssemblies.Contains(assemblyName))
                result.OriginalAssemblies.Add(assemblyName);
        }

        // ── collect rewrites ─────────────────────────────────────────

        // Strip targets first: nodes inside them are excluded from
        // reference rewriting (they get removed/emptied anyway).
        var strippedSpans = new List<Microsoft.CodeAnalysis.Text.TextSpan>();
        var nodeReplacements = new Dictionary<SyntaxNode, SyntaxNode>();
        var dynamicReplacements = new Dictionary<SyntaxNode, Func<SyntaxNode, SyntaxNode>>();

        foreach (BaseTypeDeclarationSyntax decl in localDecls)
        {
            if (decl is not TypeDeclarationSyntax typeDecl)
                continue;

            // Added static fields keep their initializers through the
            // rewrite: the (rewritten) expression moves into the holder
            // class, and the declaration itself is stripped afterwards.
            var addedStaticNames = new HashSet<string>(StringComparer.Ordinal);
            if (addedFieldsByType.TryGetValue(HotDiff.MetadataName(typeDecl), out List<AddedFieldInfo>? typeAdded))
            {
                foreach (AddedFieldInfo info in typeAdded.Where(i => i.IsStatic))
                    addedStaticNames.Add(info.Name);
            }

            foreach (MemberDeclarationSyntax member in typeDecl.Members)
            {
                switch (member)
                {
                    case ConstructorDeclarationSyntax ctor when ctor.Modifiers.Any(SyntaxKind.StaticKeyword):
                    {
                        // Empty static constructor: keeps beforefieldinit
                        // semantics inert without changing the member list.
                        // The identifier rename must happen on the
                        // replacement node itself — tokens inside a replaced
                        // node are gone before token replacement runs.
                        ConstructorDeclarationSyntax emptied = ctor
                            .WithExpressionBody(null)
                            .WithSemicolonToken(default);
                        if (IsTopLevel(typeDecl))
                        {
                            emptied = emptied.WithIdentifier(SyntaxFactory.Identifier(
                                ctor.Identifier.LeadingTrivia,
                                ctor.Identifier.Text + TypeNameSuffix,
                                ctor.Identifier.TrailingTrivia));
                        }
                        BlockSyntax emptyBody = SyntaxFactory.Block();
                        if (ctor.Body != null)
                            emptyBody = emptyBody.WithTriviaFrom(ctor.Body);
                        nodeReplacements[ctor] = emptied.WithBody(emptyBody);
                        strippedSpans.Add(ctor.FullSpan);
                        break;
                    }
                    case FieldDeclarationSyntax field when
                        field.Modifiers.Any(SyntaxKind.StaticKeyword) &&
                        !field.Modifiers.Any(SyntaxKind.ConstKeyword):
                    {
                        foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                        {
                            if (declarator.Initializer == null)
                                continue;
                            if (addedStaticNames.Contains(declarator.Identifier.Text))
                                continue;
                            nodeReplacements[declarator] = declarator
                                .WithInitializer(null)
                                .WithIdentifier(declarator.Identifier.WithTrailingTrivia());
                            strippedSpans.Add(declarator.Initializer.FullSpan);
                        }
                        break;
                    }
                    case PropertyDeclarationSyntax property when
                        property.Modifiers.Any(SyntaxKind.StaticKeyword) &&
                        property.Initializer != null:
                    {
                        // B2: an ADDED static auto-property keeps its
                        // initializer through the rewrite — the expression
                        // moves into the holder field, and the declaration
                        // itself is removed with the accessor extraction.
                        if (addedStaticNames.Contains(property.Identifier.Text))
                            break;
                        nodeReplacements[property] = property
                            .WithInitializer(null)
                            .WithSemicolonToken(default);
                        strippedSpans.Add(property.Initializer.FullSpan);
                        break;
                    }
                }
            }
        }

        bool InStrippedSpan(SyntaxNode node)
        {
            foreach (Microsoft.CodeAnalysis.Text.TextSpan span in strippedSpans)
            {
                if (span.Contains(node.Span))
                    return true;
            }
            return false;
        }

        // ── operators / conversions in renamed types ─────────────────
        // C# requires a binary operator parameter (and a conversion's
        // source or target) to BE the containing type — which the patch
        // copy renames. UNCHANGED operator/conversion declarations are
        // stripped from the copy (static surface, no layout impact; patch
        // bodies keep binding the ORIGINAL type's operators). CHANGED ones
        // stay: their containing-type parameter references rename with the
        // type (token-level for top-level declarations) and are excluded
        // from the original-name reference rewrite, so they bind to the
        // patch type and satisfy CS0563/CS0556.
        var operatorSelfRefs = new HashSet<SyntaxNode>();
        var operatorSelfRenameTokens = new List<SyntaxToken>();
        var strippedOperatorDecls = new List<BaseMethodDeclarationSyntax>();
        foreach (BaseTypeDeclarationSyntax hostDecl in localDecls)
        {
            if (hostDecl is not TypeDeclarationSyntax opHost)
                continue;
            string hostMetadataName = HotDiff.MetadataName(opHost);
            if (model.GetDeclaredSymbol(opHost) is not INamedTypeSymbol hostSymbol)
                continue;

            foreach (MemberDeclarationSyntax member in opHost.Members)
            {
                if (member is not OperatorDeclarationSyntax && member is not ConversionOperatorDeclarationSyntax)
                    continue;
                var operatorDecl = (BaseMethodDeclarationSyntax)member;

                bool changed = diff.ChangedMethods.Any(m =>
                    !m.Added &&
                    m.DeclaringType == hostMetadataName &&
                    m.Name.StartsWith("op_", StringComparison.Ordinal) &&
                    m.ParamTypeNames.SequenceEqual(
                        HotDiff.ParamTypeNames(operatorDecl.ParameterList), StringComparer.Ordinal));
                if (!changed)
                {
                    strippedOperatorDecls.Add(operatorDecl);
                    strippedSpans.Add(operatorDecl.FullSpan);
                    continue;
                }

                foreach (IdentifierNameSyntax selfRef in operatorDecl.ParameterList.DescendantNodes().OfType<IdentifierNameSyntax>())
                {
                    if (model.GetSymbolInfo(selfRef).Symbol is not INamedTypeSymbol refType)
                        continue;
                    if (!SymbolEqualityComparer.Default.Equals(refType.OriginalDefinition, hostSymbol.OriginalDefinition))
                        continue; // other-type references rewrite normally
                    operatorSelfRefs.Add(selfRef);
                    // Only top-level declarations rename their identifier; a
                    // nested patch type keeps its simple name, and lexical
                    // lookup resolves it once the rewrite is suppressed.
                    if (hostSymbol.ContainingType == null)
                        operatorSelfRenameTokens.Add(selfRef.Identifier);
                }
            }
        }

        // ── C2′b: kept-surface access gate (measured-red runtimes only) ──
        // Added members are gated above (C2′a — conservative when caps are
        // absent). Everything ELSE the patch compiles — kept bodies, new-
        // type bodies, added-field initializers — has ALWAYS shipped
        // non-public references through IgnoresAccessChecksTo, so a missing
        // probe keeps today's hot verdicts; but when this runtime MEASURED a
        // red cell, the original-token references those bodies emit would
        // crash at first JIT (non-detoured kept copies JIT on calls from
        // patched bodies, holder cctors on first store touch, new types on
        // first use — none of which the apply-time nets fully cover), so
        // name them cold up front. All cells green (the universal probe
        // result so far) skips the scan entirely: zero cost, zero behavior
        // change.
        if (batch.RuntimeCaps != null && !batch.RuntimeCaps.CoversAllCells())
        {
            string? keptViolation = FindKeptSurfaceAccessViolation(
                root, model, batch, addedFieldSymbols, renamedSymbols, InAddedMember, InStrippedSpan);
            if (keptViolation != null)
            {
                result.ColdReason = keptViolation;
                return result;
            }
        }

        // Types tombstoned by earlier batches (deleted files): a reference
        // binding to the still-loaded metadata type is a deterministic
        // error, not a silent half-alive call.
        var tombstonedTypes = new HashSet<string>(StringComparer.Ordinal);
        foreach (var pair in batch.EarlierShims)
        {
            if (pair.Value.Kind != "tombstone")
                continue;
            string[] parts = pair.Key.Split('|');
            if (parts.Length >= 2 && parts[1].Length == 0)
                tombstonedTypes.Add(parts[0]);
        }

        // Type references to renamed types → fully-qualified original names.
        // Candidates: name/member-access nodes whose symbol is a renamed
        // type; only the outermost such node is replaced.
        var typeRefCandidates = new Dictionary<SyntaxNode, INamedTypeSymbol>();
        foreach (SyntaxNode node in root.DescendantNodes())
        {
            if (node is not (IdentifierNameSyntax or GenericNameSyntax or QualifiedNameSyntax
                or MemberAccessExpressionSyntax or AliasQualifiedNameSyntax))
            {
                continue;
            }
            if (IsDeclarationName(node) || InStrippedSpan(node) || operatorSelfRefs.Contains(node))
                continue;

            ISymbol? symbol = model.GetSymbolInfo(node).Symbol;
            if (symbol is INamedTypeSymbol named)
            {
                if (tombstonedTypes.Count > 0 &&
                    !IsRenamedTypeSymbol(named, renamedSymbols) &&
                    tombstonedTypes.Contains(SymbolMetadataName(named.OriginalDefinition)))
                {
                    result.Error =
                        SymbolMetadataName(named.OriginalDefinition) +
                        " was deleted by an earlier hot patch in this session; remove the reference or run unity_recompile";
                    return result;
                }
                if (batch.NewTypeSymbols.Contains(named.OriginalDefinition))
                {
                    // Brand-new types never re-qualify to ORIGINAL names.
                    // Top-level ones keep their (unrenamed) names — lexical
                    // binding in the patch unit resolves them. Nested ones
                    // under a RENAMED container re-qualify to the PATCH
                    // name: the original type has no such nested member.
                    if (named.OriginalDefinition.ContainingType != null &&
                        IsRenamedTypeSymbol(named, renamedSymbols))
                    {
                        typeRefCandidates[node] = named;
                    }
                    continue;
                }
                if (IsRenamedTypeSymbol(named, renamedSymbols))
                    typeRefCandidates[node] = named;
            }
        }

        foreach (var pair in typeRefCandidates)
        {
            SyntaxNode node = pair.Key;
            // Keep only the outermost candidate node.
            bool nestedInCandidate = false;
            for (SyntaxNode? ancestor = node.Parent; ancestor != null; ancestor = ancestor.Parent)
            {
                if (typeRefCandidates.ContainsKey(ancestor))
                {
                    nestedInCandidate = true;
                    break;
                }
            }
            if (nestedInCandidate || nodeReplacements.ContainsKey(node))
                continue;

            string fqn = batch.NewTypeSymbols.Contains(pair.Value.OriginalDefinition)
                ? PatchQualifiedDisplay(pair.Value)
                : pair.Value.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat);
            bool expressionContext =
                node.Parent is MemberAccessExpressionSyntax memberAccess && memberAccess.Expression == node;
            SyntaxNode replacement = expressionContext
                ? SyntaxFactory.ParseExpression(fqn)
                : SyntaxFactory.ParseTypeName(fqn);
            nodeReplacements[node] = replacement.WithTriviaFrom(node);
        }

        // ── appended enum members (H7e) ──────────────────────────────
        // `E.NewMember` cannot bind against the original metadata enum:
        // materialize the resolved constant as a cast literal. Inside a
        // switch case label the cast stays unparenthesized (a parenthesized
        // pattern needs C# 9).
        if (batch.AddedEnumMembers.Count > 0)
        {
            foreach (SyntaxNode node in root.DescendantNodes())
            {
                if (node is not (IdentifierNameSyntax or MemberAccessExpressionSyntax))
                    continue;
                if (InStrippedSpan(node) || nodeReplacements.ContainsKey(node) || dynamicReplacements.ContainsKey(node))
                    continue;
                if (node.Parent is MemberAccessExpressionSyntax parentAccess && parentAccess.Name == node)
                    continue;
                if (node.Ancestors().Any(a => a is EnumDeclarationSyntax))
                    continue; // the declaration itself

                if (model.GetSymbolInfo(node).Symbol is not IFieldSymbol enumField ||
                    !batch.AddedEnumMembers.TryGetValue(enumField, out (string EnumFqn, long Value) enumTarget))
                {
                    continue;
                }

                InvocationExpressionSyntax? nameofInvocation = FindEnclosingNameOf(node, model);
                if (nameofInvocation != null)
                {
                    nodeReplacements[nameofInvocation] = SyntaxFactory.LiteralExpression(
                            SyntaxKind.StringLiteralExpression,
                            SyntaxFactory.Literal(enumField.Name))
                        .WithTriviaFrom(nameofInvocation);
                    continue;
                }

                bool inCaseLabel = node.Parent is CaseSwitchLabelSyntax;
                string cast = "(" + enumTarget.EnumFqn + ")" +
                    (enumTarget.Value < 0 ? "(" + enumTarget.Value + ")" : enumTarget.Value.ToString());
                string text = inCaseLabel ? cast : "(" + cast + ")";
                nodeReplacements[node] = SyntaxFactory.ParseExpression(text).WithTriviaFrom(node);
            }
        }

        // ── virtualized field accesses (M4) ─────────────────────────
        // Every access to an ADDED field rewrites to its store: instance
        // fields through `store.Ref(target)` (a ref-return, so reads,
        // writes, compound assignments and ref-arguments all work), static
        // fields through the holder's plain field.
        if (addedFieldSymbols.Count > 0)
        {
            foreach (SyntaxNode node in root.DescendantNodes())
            {
                if (node is not (IdentifierNameSyntax or MemberAccessExpressionSyntax))
                    continue;
                if (InStrippedSpan(node) || nodeReplacements.ContainsKey(node) || dynamicReplacements.ContainsKey(node))
                    continue;
                if (node.Parent is MemberAccessExpressionSyntax parentAccess && parentAccess.Name == node)
                    continue; // the whole member access handles it
                if (node is IdentifierNameSyntax && IsDeclarationName(node))
                    continue;

                if (model.GetSymbolInfo(node).Symbol is not IFieldSymbol fieldSymbol ||
                    !addedFieldSymbols.TryGetValue(fieldSymbol, out AddedFieldInfo? fieldInfo))
                {
                    continue;
                }

                // The declarator's own identifier is not a name node, but
                // `nameof(x)` is — materialize the constant.
                InvocationExpressionSyntax? nameofInvocation = FindEnclosingNameOf(node, model);
                if (nameofInvocation != null)
                {
                    nodeReplacements[nameofInvocation] = SyntaxFactory.LiteralExpression(
                            SyntaxKind.StringLiteralExpression,
                            SyntaxFactory.Literal(fieldInfo.Name))
                        .WithTriviaFrom(nameofInvocation);
                    continue;
                }

                AddedFieldInfo capturedField = fieldInfo;
                ShimTarget? enclosingAddedMember = EnclosingAddedTarget(node);
                dynamicReplacements[node] = rewrittenNode =>
                    BuildFieldStoreAccess(rewrittenNode, capturedField, enclosingAddedMember != null);
            }
        }

        // ── references to ADDED properties/indexers/events (B2) ─────
        // Batch references bind to the added property/indexer/event symbol;
        // reads, writes, compound assignments and event subscriptions
        // materialize as direct accessor-shim calls (same-file auto-
        // properties route straight to their lvalue-shaped backing store).
        // Any form the matrix cannot express with IDENTICAL semantics fails
        // the file closed with a pointed reason — never a wrong rewrite.
        {
            var autoStores = new Dictionary<IMethodSymbol, AddedFieldInfo>(SymbolEqualityComparer.Default);
            foreach (AddedAccessorInfo accessorInfo in addedAccessors)
            {
                if (accessorInfo.IsAuto && accessorInfo.Store != null)
                    autoStores[accessorInfo.Symbol.OriginalDefinition] = accessorInfo.Store;
            }
            string? matrixCold = RewriteAddedAccessorReferences(
                root, model, batch, autoStores, EnclosingAddedTarget, InStrippedSpan,
                nodeReplacements, dynamicReplacements);
            if (matrixCold != null)
            {
                result.ColdReason = matrixCold;
                return result;
            }
        }

        // Unqualified static field/property/event reads of renamed types →
        // qualified back to the original type (single static state source).
        foreach (IdentifierNameSyntax identifier in root.DescendantNodes().OfType<IdentifierNameSyntax>())
        {
            if (nodeReplacements.ContainsKey(identifier) || dynamicReplacements.ContainsKey(identifier) ||
                InStrippedSpan(identifier))
                continue;
            if (identifier.Parent is MemberAccessExpressionSyntax access && access.Name == identifier)
                continue; // already qualified; the qualifier rewrite covers it
            if (identifier.Parent is QualifiedNameSyntax || identifier.Parent is AliasQualifiedNameSyntax)
                continue;
            if (IsDeclarationName(identifier))
                continue;
            // Inside a candidate type-ref node (e.g. the `Foo` of `Foo.Bar`):
            // skip — the outer replacement handles it.
            bool insideTypeRef = false;
            for (SyntaxNode? ancestor = identifier.Parent; ancestor != null; ancestor = ancestor.Parent)
            {
                if (nodeReplacements.ContainsKey(ancestor))
                {
                    insideTypeRef = true;
                    break;
                }
            }
            if (insideTypeRef)
                continue;

            ISymbol? symbol = model.GetSymbolInfo(identifier).Symbol;
            bool isStaticState = symbol switch
            {
                IFieldSymbol field => field.IsStatic && !field.IsConst,
                IPropertySymbol property => property.IsStatic,
                IEventSymbol @event => @event.IsStatic,
                _ => false,
            };
            if (!isStaticState || symbol!.ContainingType == null)
                continue;
            if (HasAddedAccessor(symbol, batch))
                continue; // B2: the accessor matrix owns added properties/events
            if (!IsRenamedTypeSymbol(symbol.ContainingType, renamedSymbols))
                continue;

            string qualified =
                symbol.ContainingType.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat) +
                "." + symbol.Name;
            nodeReplacements[identifier] = SyntaxFactory.ParseExpression(qualified).WithTriviaFrom(identifier);
        }

        // ── `this` handling ──────────────────────────────────────────
        // Inside ADDED members every `this` becomes `self` (the shim's
        // explicit receiver). Inside KEPT members of renamed types, a
        // STANDALONE `this` (escaping as an argument/value) becomes
        // `((global::Ns.Foo)(object)this)`: the runtime object is an
        // original-type instance, only the static type differs.
        foreach (ThisExpressionSyntax thisNode in root.DescendantNodes().OfType<ThisExpressionSyntax>())
        {
            if (InStrippedSpan(thisNode) || nodeReplacements.ContainsKey(thisNode))
                continue;

            if (InAddedMember(thisNode))
            {
                nodeReplacements[thisNode] = SyntaxFactory.IdentifierName("self").WithTriviaFrom(thisNode);
                continue;
            }

            bool isReceiver =
                (thisNode.Parent is MemberAccessExpressionSyntax mae && mae.Expression == thisNode) ||
                (thisNode.Parent is ElementAccessExpressionSyntax eae && eae.Expression == thisNode);
            if (isReceiver)
                continue;

            TypeDeclarationSyntax? enclosingType = thisNode.Ancestors().OfType<TypeDeclarationSyntax>().FirstOrDefault();
            if (enclosingType == null || !localDecls.Contains(enclosingType))
                continue;
            if (model.GetDeclaredSymbol(enclosingType) is not INamedTypeSymbol enclosingSymbol)
                continue;

            string fqn = enclosingSymbol.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat);
            nodeReplacements[thisNode] = SyntaxFactory
                .ParseExpression("((" + fqn + ")(object)this)")
                .WithTriviaFrom(thisNode);
        }

        // ── implicit member references inside ADDED members ─────────
        // The shim body runs outside the type: implicit instance refs need
        // `self.`, implicit refs that stay implicit-static need the original
        // type qualifier (the static-state pass above only covers
        // field/property/event of renamed types). Accessor bodies (B2) are
        // added members too.
        if (addedBodySpans.Count > 0)
        {
            foreach (IdentifierNameSyntax identifier in root.DescendantNodes().OfType<IdentifierNameSyntax>())
            {
                if (!InAddedMember(identifier))
                    continue;
                if (nodeReplacements.ContainsKey(identifier) || dynamicReplacements.ContainsKey(identifier) ||
                    InStrippedSpan(identifier))
                    continue;
                if (identifier.Parent is MemberAccessExpressionSyntax accessParent && accessParent.Name == identifier)
                    continue;
                if (identifier.Parent is MemberBindingExpressionSyntax)
                    continue;
                if (identifier.Parent is QualifiedNameSyntax || identifier.Parent is AliasQualifiedNameSyntax)
                    continue;
                if (IsDeclarationName(identifier))
                    continue;
                bool insideReplaced = false;
                for (SyntaxNode? ancestor = identifier.Parent; ancestor != null; ancestor = ancestor.Parent)
                {
                    if (nodeReplacements.ContainsKey(ancestor))
                    {
                        insideReplaced = true;
                        break;
                    }
                }
                if (insideReplaced)
                    continue;

                ISymbol? symbol = model.GetSymbolInfo(identifier).Symbol;
                if (symbol is not (IFieldSymbol or IPropertySymbol or IEventSymbol or IMethodSymbol))
                    continue;
                if (symbol is IMethodSymbol candidateMethod &&
                    TryGetAddedTarget(candidateMethod, batch, out _, out _))
                {
                    continue; // the shim-call rewrite owns these
                }
                if (HasAddedAccessor(symbol, batch))
                    continue; // B2: the accessor matrix owns these
                if (symbol.ContainingType == null)
                    continue;
                if (symbol is IMethodSymbol { MethodKind: not MethodKind.Ordinary })
                    continue;

                // Only references that were IMPLICIT member lookups on the
                // declaring chain (this. or static context).
                ShimTarget? enclosing = EnclosingAddedTarget(identifier);
                if (enclosing == null)
                    continue;

                if (symbol.IsStatic)
                {
                    string qualifier = symbol.ContainingType.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat);
                    nodeReplacements[identifier] = SyntaxFactory
                        .ParseExpression(qualifier + "." + identifier.Identifier.Text)
                        .WithTriviaFrom(identifier);
                }
                else if (enclosing.HasSelf)
                {
                    nodeReplacements[identifier] = SyntaxFactory.MemberAccessExpression(
                            SyntaxKind.SimpleMemberAccessExpression,
                            SyntaxFactory.IdentifierName("self"),
                            SyntaxFactory.IdentifierName(identifier.Identifier.Text))
                        .WithTriviaFrom(identifier);
                }
            }
        }

        // ── calls to CHANGED static methods in this batch → patch copy ─
        // Release inline caller refresh compiles an unchanged caller together
        // with the current callee diff. Without this narrow rewrite, the
        // caller's type reference rewrite points back at the original callee
        // and Mono can inline the stale body into the refreshed caller.
        if (batch.PatchedMethods.Count > 0)
        {
            foreach (InvocationExpressionSyntax invocation in root.DescendantNodes().OfType<InvocationExpressionSyntax>())
            {
                if (InStrippedSpan(invocation) || dynamicReplacements.ContainsKey(invocation))
                    continue;
                if (!TryResolvePatchedMethodTarget(
                    model.GetSymbolInfo(invocation), batch,
                    out PatchedMethodTarget? target))
                {
                    continue;
                }

                PatchedMethodTarget capturedTarget = target;
                if (!target.HasSelf)
                {
                    // Static callee → bind to the patch copy directly.
                    dynamicReplacements[invocation] = rewrittenNode =>
                        BuildPatchedMethodInvocation((InvocationExpressionSyntax)rewrittenNode, capturedTarget);
                    continue;
                }

                // Instance callee → bind to the static self-shim, passing the
                // receiver as `self`. Receiver shapes the shim cannot express
                // (`base.M` has no base argument; `obj?.M()` would lose the
                // null-propagation) keep the original call: it is still hot via
                // the normal detour and converges fully at the queued recompile.
                if (!PatchedInstanceReceiverExpressible(invocation.Expression))
                    continue;
                ShimTarget? enclosingAdded = EnclosingAddedTarget(invocation);
                dynamicReplacements[invocation] = rewrittenNode =>
                    BuildPatchedInstanceInvocation(
                        (InvocationExpressionSyntax)rewrittenNode, capturedTarget, enclosingAdded);
            }
        }

        // ── calls to ADDED members → direct shim calls (M2) ──────────
        foreach (InvocationExpressionSyntax invocation in root.DescendantNodes().OfType<InvocationExpressionSyntax>())
        {
            if (InStrippedSpan(invocation))
                continue;
            if (!TryResolveAddedTarget(
                model.GetSymbolInfo(invocation), batch,
                out ShimTarget? target, out IMethodSymbol? invoked, out bool reduced))
            {
                continue;
            }

            if (invocation.Expression is MemberBindingExpressionSyntax)
            {
                result.ColdReason =
                    "added member called through ?. (conditional access): " + target.DeclaringTypeMetadataName +
                    "." + target.MethodName + " — rewrite the call without ?. or use unity_recompile";
                return result;
            }

            ShimTarget? enclosingAdded = EnclosingAddedTarget(invocation);
            ShimTarget capturedTarget = target;
            string typeArgumentText = ShimTypeArgumentText(target, invoked);
            string? reducedReceiverCastFqn = reduced && invoked.Parameters.Length > 0
                ? invoked.Parameters[0].Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)
                : null;
            dynamicReplacements[invocation] = rewrittenNode =>
                BuildShimInvocation(
                    (InvocationExpressionSyntax)rewrittenNode, capturedTarget, enclosingAdded,
                    typeArgumentText, reducedReceiverCastFqn);
        }

        // Method groups of added members (delegate conversions) → lambdas
        // that call the shim; nameof(added) → string literal.
        foreach (SyntaxNode node in root.DescendantNodes())
        {
            if (node is not (IdentifierNameSyntax or MemberAccessExpressionSyntax))
                continue;
            if (InStrippedSpan(node) || nodeReplacements.ContainsKey(node) || dynamicReplacements.ContainsKey(node))
                continue;
            if (node.Parent is InvocationExpressionSyntax parentInvocation && parentInvocation.Expression == node)
                continue; // the invocation pass owns it
            if (node.Parent is MemberAccessExpressionSyntax outerAccess && outerAccess.Name == node)
                continue;

            if (!TryResolveAddedTarget(
                model.GetSymbolInfo(node), batch,
                out ShimTarget? groupTarget, out IMethodSymbol? groupSymbol, out bool groupReduced))
            {
                continue;
            }

            // nameof(NewMethod): the added member is extracted from the
            // patch type, so materialize the constant string instead.
            InvocationExpressionSyntax? nameofInvocation = FindEnclosingNameOf(node, model);
            if (nameofInvocation != null)
            {
                nodeReplacements[nameofInvocation] = SyntaxFactory.LiteralExpression(
                        SyntaxKind.StringLiteralExpression,
                        SyntaxFactory.Literal(groupTarget.MethodName))
                    .WithTriviaFrom(nameofInvocation);
                continue;
            }

            if (model.GetTypeInfo(node).ConvertedType is not INamedTypeSymbol delegateType ||
                delegateType.DelegateInvokeMethod == null)
            {
                result.ColdReason =
                    "method group of added member needs an explicit delegate context: " +
                    groupTarget.DeclaringTypeMetadataName + "." + groupTarget.MethodName +
                    " — wrap it in a lambda or use unity_recompile";
                return result;
            }

            ShimTarget capturedGroupTarget = groupTarget;
            IMethodSymbol invoke = delegateType.DelegateInvokeMethod;
            ShimTarget? enclosingAdded = EnclosingAddedTarget(node);
            string groupTypeArgumentText = ShimTypeArgumentText(groupTarget, groupSymbol);
            string? groupReceiverCastFqn = groupReduced && groupSymbol.Parameters.Length > 0
                ? groupSymbol.Parameters[0].Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)
                : null;
            dynamicReplacements[node] = rewrittenNode =>
                BuildShimLambda(
                    rewrittenNode, capturedGroupTarget, invoke, enclosingAdded,
                    groupTypeArgumentText, groupReceiverCastFqn);
        }

        // ── calls to shims from EARLIER batches (registry fallback) ──
        // A call to a member added by an earlier batch whose file is NOT in
        // this batch usually binds through the session image's extension
        // method and needs no rewrite. The cases that do NOT bind that way —
        // static members (`Foo.M()`) and out-of-scope namespaces — show up
        // as unresolved member accesses; resolve them against the registry.
        if (batch.EarlierShims.Count > 0)
        {
            foreach (InvocationExpressionSyntax invocation in root.DescendantNodes().OfType<InvocationExpressionSyntax>())
            {
                if (InStrippedSpan(invocation) || dynamicReplacements.ContainsKey(invocation))
                    continue;
                if (invocation.Expression is not MemberAccessExpressionSyntax memberAccess)
                    continue;
                if (memberAccess.Name is not IdentifierNameSyntax nameNode)
                    continue;
                SymbolInfo info = model.GetSymbolInfo(invocation);
                if (info.Symbol != null || info.CandidateSymbols.Length > 0)
                    continue;

                MemberSurfaceRegistry.ShimEntry? entry = ResolveRegistryShim(
                    model, batch, memberAccess, nameNode.Identifier.Text,
                    invocation.ArgumentList.Arguments.Count, out bool staticForm, out string? tombstoneError);
                if (tombstoneError != null)
                {
                    result.Error = tombstoneError;
                    return result;
                }
                if (entry == null)
                    continue;

                MemberSurfaceRegistry.ShimEntry capturedEntry = entry;
                bool capturedStaticForm = staticForm;
                dynamicReplacements[invocation] = rewrittenNode =>
                    BuildRegistryShimInvocation(
                        (InvocationExpressionSyntax)rewrittenNode, capturedEntry, capturedStaticForm);
            }
        }

        // Declaration identifiers: top-level type declarations plus their
        // constructor and destructor name tokens must rename together.
        var tokenReplacements = new Dictionary<SyntaxToken, SyntaxToken>();
        foreach (BaseTypeDeclarationSyntax decl in topLevelDecls)
        {
            tokenReplacements[decl.Identifier] = SyntaxFactory.Identifier(
                decl.Identifier.LeadingTrivia,
                decl.Identifier.Text + TypeNameSuffix,
                decl.Identifier.TrailingTrivia);

            if (decl is not TypeDeclarationSyntax typeDecl)
                continue;
            foreach (MemberDeclarationSyntax member in typeDecl.Members)
            {
                switch (member)
                {
                    // Static ctors are node-replaced (emptied) above and
                    // carry their rename inside the replacement.
                    case ConstructorDeclarationSyntax ctor when !ctor.Modifiers.Any(SyntaxKind.StaticKeyword):
                        tokenReplacements[ctor.Identifier] = SyntaxFactory.Identifier(
                            ctor.Identifier.LeadingTrivia,
                            ctor.Identifier.Text + TypeNameSuffix,
                            ctor.Identifier.TrailingTrivia);
                        break;
                    case DestructorDeclarationSyntax dtor:
                        tokenReplacements[dtor.Identifier] = SyntaxFactory.Identifier(
                            dtor.Identifier.LeadingTrivia,
                            dtor.Identifier.Text + TypeNameSuffix,
                            dtor.Identifier.TrailingTrivia);
                        break;
                }
            }
        }
        foreach (DelegateDeclarationSyntax delegateDecl in topLevelDelegates)
        {
            tokenReplacements[delegateDecl.Identifier] = SyntaxFactory.Identifier(
                delegateDecl.Identifier.LeadingTrivia,
                delegateDecl.Identifier.Text + TypeNameSuffix,
                delegateDecl.Identifier.TrailingTrivia);
        }
        // Containing-type parameter references of CHANGED operators rename
        // together with their (top-level) type declaration.
        foreach (SyntaxToken selfRefToken in operatorSelfRenameTokens)
        {
            tokenReplacements[selfRefToken] = SyntaxFactory.Identifier(
                selfRefToken.LeadingTrivia,
                selfRefToken.Text + TypeNameSuffix,
                selfRefToken.TrailingTrivia);
        }

        // Record index paths of added members, stripped operators and
        // added-field initializers BEFORE the rewrite (replace operations
        // preserve node counts and order), so the rewritten nodes can be
        // located afterwards.
        var addedPaths = new List<(List<int> Path, ShimTarget Target)>();
        foreach (var pair in addedDecls)
            addedPaths.Add((IndexPath(pair.Key), pair.Value));
        var addedAccessorPaths = new List<(List<int> ContainerPath, AddedAccessorInfo Info)>();
        foreach (AddedAccessorInfo accessorInfo in addedAccessors)
            addedAccessorPaths.Add((IndexPath(accessorInfo.Container), accessorInfo));
        var strippedOperatorPaths = new List<List<int>>();
        foreach (BaseMethodDeclarationSyntax operatorDecl in strippedOperatorDecls)
            strippedOperatorPaths.Add(IndexPath(operatorDecl));
        var initializerPaths = new List<(List<int> Path, AddedFieldInfo Field)>();
        foreach (AddedFieldInfo info in addedFieldsByType.Values.SelectMany(list => list))
        {
            if (info.InitializerValue != null)
                initializerPaths.Add((IndexPath(info.InitializerValue), info));
        }

        SyntaxNode rewritten = root.ReplaceSyntax(
            nodeReplacements.Keys.Concat(dynamicReplacements.Keys),
            (original, rewrittenNode) =>
                nodeReplacements.TryGetValue(original, out SyntaxNode? fixedNode)
                    ? fixedNode
                    : dynamicReplacements[original](rewrittenNode),
            tokenReplacements.Keys,
            (original, _) => tokenReplacements[original],
            null,
            null);

        // Fetch the REWRITTEN initializer expressions first — later surgery
        // (member extraction, declaration stripping) invalidates the paths.
        foreach (var (pathIndices, info) in initializerPaths)
        {
            if (NodeAtPath(rewritten, pathIndices) is ExpressionSyntax expression)
                info.RewrittenInitializerText = expression.ToFullString();
        }

        // ── extract added members into shim classes (M2) and drop the
        // unchanged operator/conversion copies in one removal pass ───────
        {
            // Shim methods are built from the REWRITTEN declarations before
            // the removal pass detaches them from the tree.
            var extracted = new List<(MemberDeclarationSyntax Shim, ShimTarget Target)>();
            var removableNodes = new List<SyntaxNode>();
            foreach (var (pathIndices, target) in addedPaths)
            {
                if (NodeAtPath(rewritten, pathIndices) is not MethodDeclarationSyntax method)
                    continue;
                extracted.Add((BuildShimMethod(method, target, root), target));
                removableNodes.Add(method);
            }
            // B2: accessor shims — one per added accessor; the owning
            // property/indexer/event declaration is removed ONCE (the patch
            // copy must not re-declare the new surface: an auto backing
            // field would break the layout guard, and the original metadata
            // lacks the member anyway).
            var removedContainers = new HashSet<SyntaxNode>();
            foreach (var (containerPath, accessorInfo) in addedAccessorPaths)
            {
                if (NodeAtPath(rewritten, containerPath) is not BasePropertyDeclarationSyntax container)
                    continue;
                extracted.Add((BuildAccessorShimMethod(container, accessorInfo, root), accessorInfo.Target));
                if (removedContainers.Add(container))
                    removableNodes.Add(container);
            }
            foreach (List<int> pathIndices in strippedOperatorPaths)
            {
                if (NodeAtPath(rewritten, pathIndices) is BaseMethodDeclarationSyntax strippedOperator)
                    removableNodes.Add(strippedOperator);
            }

            if (removableNodes.Count > 0)
            {
                rewritten = rewritten.RemoveNodes(removableNodes, SyntaxRemoveOptions.KeepNoTrivia)!;
            }

            // One shim class per top-level type, grouped by namespace.
            foreach (var group in extracted.GroupBy(e => e.Target.ShimTypeMetadataName, StringComparer.Ordinal))
            {
                ShimTarget first = group.First().Target;
                var shimMethods = new List<MemberDeclarationSyntax>();
                foreach (var (shim, _) in group)
                    shimMethods.Add(shim);

                string shimSimpleName = first.ShimTypeMetadataName.Contains('.')
                    ? first.ShimTypeMetadataName[(first.ShimTypeMetadataName.LastIndexOf('.') + 1)..]
                    : first.ShimTypeMetadataName;

                // `partial`: two files of a batch can both add members to the
                // SAME (partial, B6) top-level type — each file's rewrite
                // emits its own shim-class declaration and they must merge.
                ClassDeclarationSyntax shimClass = SyntaxFactory.ClassDeclaration(shimSimpleName)
                    .WithModifiers(SyntaxFactory.TokenList(
                        SyntaxFactory.Token(SyntaxKind.PublicKeyword),
                        SyntaxFactory.Token(SyntaxKind.StaticKeyword),
                        SyntaxFactory.Token(SyntaxKind.PartialKeyword)))
                    .WithMembers(SyntaxFactory.List(shimMethods))
                    .NormalizeWhitespace()
                    .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);

                rewritten = AppendTypeToNamespace((CompilationUnitSyntax)rewritten, shimClass, first.ShimNamespace);

                foreach (var (_, target) in group)
                {
                    // Inline-redirect self-shims are internal batch targets, not
                    // user surface: the method is emitted (above) but it must NOT
                    // register in the MemberSurfaceRegistry or take a re-edit
                    // continuity detour — its only callers are the same batch's
                    // refreshed bodies, rewritten by name.
                    if (target.IsInlineRedirectShim)
                        continue;

                    result.ShimRegistrations.Add(new ShimRegistration
                    {
                        MemberKey = target.MemberKey,
                        Entry = new MemberSurfaceRegistry.ShimEntry
                        {
                            Kind = "added",
                            ShimTypeMetadataName = target.ShimTypeMetadataName,
                            ShimTypeFqn = target.ShimTypeFqn,
                            ShimMethod = target.MethodName,
                            ParamTypeNames = target.ShimParamTypeNames,
                            DeclaringTypeFqn = target.DeclaringTypeFqn,
                            HasSelf = target.HasSelf,
                            SelfIsValueType = target.SelfIsValueType,
                            GenericShim = target.GenericShim,
                        },
                    });

                    // Re-edit continuity: a member already shimmed by an
                    // earlier batch gets a detour OLD shim → NEW shim, so
                    // in-flight delegates pick up the new behavior. Generic
                    // shims skip this (generic method detours are the
                    // unreliable case; direct calls re-bind every batch).
                    if (batch.EarlierShims.TryGetValue(target.MemberKey, out MemberSurfaceRegistry.ShimEntry? earlier) &&
                        earlier.Kind == "added" &&
                        !earlier.GenericShim &&
                        !target.GenericShim)
                    {
                        result.Methods.Add(new PatchMethodMap
                        {
                            DeclaringType = earlier.ShimTypeMetadataName,
                            PatchDeclaringType = target.ShimTypeMetadataName,
                            Name = earlier.ShimMethod,
                            ParamTypeNames = earlier.ParamTypeNames,
                            IsStatic = true,
                            IsCtor = false,
                            OriginalAssembly = earlier.ShimAssembly,
                        });
                    }
                }
            }
        }

        // Play-mode-born re-edit: this file's types live only in a prior
        // hot-patch assembly, so every detour ORIGINAL side — body redirects
        // (below), AND the deletion/removed-type magic-method stubs — must be
        // pinned to it, or Unity's default resolver (which skips
        // __LocusHotPatch_ assemblies) cannot find the live type to detour.
        // batch.ReeditAssemblyFor(path, type) resolves the right assembly per
        // type: a late-born sibling (feature #5) uses its own, not the file's.

        // ── deletions (M5) ───────────────────────────────────────────
        // Removed members stay untouched in the loaded original (in-flight
        // delegates/coroutines are legitimate callers) and tombstone in the
        // registry. Removed UNITY MESSAGE methods additionally get an
        // empty-body stub re-materialized in the patch type + a normal
        // detour mapping: the engine calls the original every frame, and
        // the stub is what makes the deletion observable immediately.
        foreach (HotDiffRemovedMember removed in diff.RemovedMembers)
        {
            string removedKey = MemberSurfaceRegistry.MemberKey(
                removed.DeclaringType, removed.Name, removed.ParamTypeNames, removed.IsStatic);

            // A same-signature re-add (B1 generic body change) materialized
            // this very member as a shim above: that "added" registration is
            // the live surface — a tombstone would overwrite it in the
            // registry (last write wins) and break later batches.
            if (!batch.ReAddedMemberKeys.Contains(removedKey))
            {
                result.ShimRegistrations.Add(new ShimRegistration
                {
                    MemberKey = removedKey,
                    Entry = new MemberSurfaceRegistry.ShimEntry { Kind = "tombstone" },
                });
            }

            if (!removed.IsUnityMagic || removed.StubSource == null)
                continue;

            if (SyntaxFactory.ParseMemberDeclaration(removed.StubSource) is not MethodDeclarationSyntax stub)
                continue;

            string renamedMetadataName = PatchTypeName(removed.DeclaringType);
            TypeDeclarationSyntax? host = ((CompilationUnitSyntax)rewritten)
                .DescendantNodes()
                .OfType<TypeDeclarationSyntax>()
                .FirstOrDefault(t => HotDiff.MetadataName(t) == renamedMetadataName);
            if (host == null)
                continue;

            rewritten = rewritten.ReplaceNode(
                host,
                host.AddMembers(stub
                    .NormalizeWhitespace()
                    .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed)
                    .WithTrailingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed)));

            result.Methods.Add(new PatchMethodMap
            {
                DeclaringType = removed.DeclaringType,
                PatchDeclaringType = renamedMetadataName,
                Name = removed.Name,
                ParamTypeNames = removed.ParamTypeNames,
                IsStatic = removed.IsStatic,
                IsCtor = false,
                IsStub = true,
                OriginalAssembly = batch.ReeditAssemblyFor(path, removed.DeclaringType),
            });
        }

        // ── removed types (H7e: file deletion / type removal) ───────
        // The loaded type stays (in-flight references are legitimate), but
        // every Unity message method detours to an empty stub in a
        // synthesized flat class — the engine stops driving scene instances
        // immediately. The stub never touches instance state, so layout
        // compatibility is irrelevant (NativeDetour jump).
        foreach (HotDiffRemovedType removedType in diff.RemovedTypes)
        {
            result.ShimRegistrations.Add(new ShimRegistration
            {
                MemberKey = MemberSurfaceRegistry.MemberKey(
                    removedType.MetadataName, "", Array.Empty<string>(), isStatic: false),
                Entry = new MemberSurfaceRegistry.ShimEntry { Kind = "tombstone" },
            });

            if (removedType.StubSource == null)
                continue;

            CompilationUnitSyntax stubUnit;
            try
            {
                stubUnit = (CompilationUnitSyntax)CSharpSyntaxTree
                    .ParseText(removedType.StubSource, parseOptions)
                    .GetRoot();
            }
            catch
            {
                continue;
            }

            var merged = (CompilationUnitSyntax)rewritten;
            foreach (UsingDirectiveSyntax stubUsing in stubUnit.Usings)
            {
                if (!merged.Usings.Any(u => u.ToString() == stubUsing.ToString()))
                    merged = merged.AddUsings(stubUsing);
            }
            rewritten = merged.AddMembers(stubUnit.Members.ToArray());

            foreach (HotDiffRemovedMember magic in removedType.MagicMethods)
            {
                result.Methods.Add(new PatchMethodMap
                {
                    DeclaringType = removedType.MetadataName,
                    PatchDeclaringType = removedType.StubTypeMetadataName,
                    Name = magic.Name,
                    ParamTypeNames = magic.ParamTypeNames,
                    IsStatic = magic.IsStatic,
                    IsCtor = false,
                    IsStub = true,
                    OriginalAssembly = batch.ReeditAssemblyFor(path, removedType.MetadataName),
                });
            }
        }

        // ── virtualized-field surgery (M4) ───────────────────────────
        // Strip added field declarations, re-inject placeholders for the
        // removed ones (the patch type's instance layout must equal the
        // original exactly), materialize added-field initializers into the
        // instance constructors, and declare the new stores/holders.
        foreach (var pair in fieldChangesByType)
        {
            string declaringType = pair.Key;
            List<HotDiffFieldChange> changes = pair.Value;
            List<AddedFieldInfo> addedInfos = addedFieldsByType.TryGetValue(declaringType, out List<AddedFieldInfo>? list)
                ? list
                : new List<AddedFieldInfo>();

            INamedTypeSymbol? original = FindOriginalType(binding, declaringType, out _);
            if (original == null)
                continue; // patched-type guard above already handled

            rewritten = ApplyFieldSurgery(
                (CompilationUnitSyntax)rewritten, declaringType, changes, addedInfos, original, result);
        }

        result.Tree = CSharpSyntaxTree.Create(
            (CSharpSyntaxNode)rewritten,
            parseOptions,
            path: path,
            encoding: System.Text.Encoding.UTF8);

        // Detour map: changed (non-added) members, original → patch type,
        // plus kept members re-detoured for their re-added call sites (B1).
        // ReeditAssemblyFor pins the detour ORIGINAL side to the play-mode-born
        // assembly (the file's first, or a late-born sibling's own — feature #5)
        // so Unity resolves and redirects the FIRST loaded type (existing
        // instances); null (the ordinary case) leaves resolution against the
        // project assemblies.
        foreach (HotDiffMethod method in diff.ChangedMethods.Where(m => !m.Added).Concat(ensuredDetours))
        {
            result.Methods.Add(new PatchMethodMap
            {
                DeclaringType = method.DeclaringType,
                PatchDeclaringType = PatchTypeName(method.DeclaringType),
                Name = method.Name,
                ParamTypeNames = method.ParamTypeNames,
                ParamTypeSigs = method.ParamTypeSigs,
                IsStatic = method.IsStatic,
                IsCtor = method.IsCtor,
                OriginalAssembly = batch.ReeditAssemblyFor(path, method.DeclaringType),
            });
        }

        return result;
    }

    // ── virtualized fields (M4) ──────────────────────────────────────

    private sealed class AddedFieldInfo
    {
        public string Name = "";
        public string DeclaringType = "";
        public string FieldTypeFqn = "";
        public bool IsStatic;
        public string StoreFqn = "";
        public string StoreMetadataName = "";
        public string StoreNamespace = "";

        /// <summary>Field name the store registers under across batches: the
        /// metadata backing-field name for auto-properties (B2), the member
        /// name itself for plain fields.</summary>
        public string RegistryFieldName = "";

        /// <summary>False when an earlier batch already declared the store —
        /// accesses bind to it and no new declaration is generated.</summary>
        public bool IsNewStore = true;

        public ExpressionSyntax? InitializerValue;
        public string? RewrittenInitializerText;
    }

    /// <summary>One added property/indexer/event ACCESSOR of this file (B2):
    /// its shim target, the body-owning syntax, and — for auto-property
    /// accessors — the backing store the synthesized shim body reads/writes.</summary>
    private sealed class AddedAccessorInfo
    {
        public ShimTarget Target = null!;
        public IMethodSymbol Symbol = null!;
        public BasePropertyDeclarationSyntax Container = null!;

        /// <summary>Null for an expression-bodied property/indexer (the
        /// implicit getter — the body is the container's arrow clause).</summary>
        public AccessorDeclarationSyntax? Accessor;

        public bool IsAuto;
        public AddedFieldInfo? Store;
    }

    /// <summary>The syntax node that OWNS an added accessor's executable
    /// body — the accessor declaration, or the container's arrow clause for
    /// expression-bodied properties/indexers.</summary>
    private static SyntaxNode AccessorBodyNode(AddedAccessorInfo info)
    {
        if (info.Accessor != null)
            return info.Accessor;
        return info.Container switch
        {
            PropertyDeclarationSyntax { ExpressionBody: { } arrow } => arrow,
            IndexerDeclarationSyntax { ExpressionBody: { } arrow } => arrow,
            _ => info.Container,
        };
    }

    /// <summary>True when the symbol is a property/event whose accessor(s)
    /// were ADDED by this batch — references rewrite to direct shim calls
    /// (or store accesses), so every other pass must leave them alone.</summary>
    private static bool HasAddedAccessor(ISymbol symbol, PatchBatchContext batch)
    {
        static bool Added(IMethodSymbol? accessor, PatchBatchContext batch) =>
            accessor != null && batch.AddedMembers.ContainsKey(accessor.OriginalDefinition);
        return symbol switch
        {
            IPropertySymbol property => Added(property.GetMethod, batch) || Added(property.SetMethod, batch),
            IEventSymbol @event => Added(@event.AddMethod, batch) || Added(@event.RemoveMethod, batch),
            _ => false,
        };
    }

    private static VariableDeclaratorSyntax? FindFieldDeclarator(TypeDeclarationSyntax type, string name, bool isStatic)
    {
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            if (member is not FieldDeclarationSyntax field ||
                field.Modifiers.Any(SyntaxKind.ConstKeyword) ||
                field.Modifiers.Any(SyntaxKind.StaticKeyword) != isStatic)
            {
                continue;
            }
            foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
            {
                if (declarator.Identifier.Text == name)
                    return declarator;
            }
        }
        return null;
    }

    private static AddedFieldInfo BuildAddedFieldInfo(
        string declaringType,
        TypeDeclarationSyntax typeDecl,
        HotDiffFieldChange change,
        IFieldSymbol fieldSymbol,
        VariableDeclaratorSyntax declarator,
        PatchBatchContext batch)
    {
        var info = new AddedFieldInfo
        {
            Name = change.Name,
            RegistryFieldName = change.Name,
            DeclaringType = declaringType,
            FieldTypeFqn = fieldSymbol.Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat),
            IsStatic = change.IsStatic,
            InitializerValue = declarator.Initializer?.Value,
        };

        string fieldKey = FieldStoreRegistry.FieldKey(declaringType, change.Name);
        if (batch.EarlierFieldStores.TryGetValue(fieldKey, out FieldStoreRegistry.StoreEntry? earlier))
        {
            info.StoreFqn = earlier.StoreTypeFqn;
            info.StoreMetadataName = earlier.StoreTypeMetadataName;
            info.IsNewStore = false;
            return info;
        }

        // "Ns.Outer+Inner" → holder "__LocusFields_Outer_Inner{_disc}" in Ns.
        // The name stays inside the __LocusHotPatch_ assembly, so the
        // type-index skip list covers it without a new prefix entry. The
        // batch discriminator keeps a LATER batch's holder for the same type
        // from shadowing the earlier batch's (re-sent fields bind to the
        // earlier holder by its registered FQN).
        int dot = declaringType.LastIndexOf('.');
        string ns = dot < 0 ? "" : declaringType[..dot];
        string chain = (dot < 0 ? declaringType : declaringType[(dot + 1)..]).Replace('+', '_');
        string storeName = "__LocusFields_" + chain
            + (batch.StoreDiscriminator.Length > 0 ? "_" + batch.StoreDiscriminator : "");
        info.StoreNamespace = ns;
        info.StoreMetadataName = ns.Length == 0 ? storeName : ns + "." + storeName;
        info.StoreFqn = "global::" + info.StoreMetadataName;
        _ = typeDecl;
        return info;
    }

    /// <summary>B2: the backing store of an added AUTO-PROPERTY. The
    /// FieldChange carries the metadata backing name ("&lt;P&gt;k__BackingField"
    /// — the layout-verification identity and the cross-batch registry key);
    /// the holder member is named after the property (a valid identifier).
    /// The synthesized accessor shims and the kept-body access matrix both
    /// route through this store.</summary>
    private static AddedFieldInfo? BuildAutoPropertyFieldInfo(
        string declaringType,
        TypeDeclarationSyntax typeDecl,
        HotDiffFieldChange change,
        SemanticModel model,
        PatchBatchContext batch)
    {
        if (!change.Name.StartsWith("<", StringComparison.Ordinal))
            return null;
        int end = change.Name.IndexOf('>');
        if (end <= 1)
            return null;
        string propertyName = change.Name[1..end];

        PropertyDeclarationSyntax? property = typeDecl.Members.OfType<PropertyDeclarationSyntax>()
            .FirstOrDefault(p => p.Identifier.Text == propertyName && HotDiff.IsAutoProperty(p));
        if (property == null)
            return null;
        if (model.GetDeclaredSymbol(property) is not IPropertySymbol propertySymbol)
            return null;

        var info = new AddedFieldInfo
        {
            Name = propertyName,
            RegistryFieldName = change.Name,
            DeclaringType = declaringType,
            FieldTypeFqn = propertySymbol.Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat),
            IsStatic = change.IsStatic,
            InitializerValue = property.Initializer?.Value,
        };

        string fieldKey = FieldStoreRegistry.FieldKey(declaringType, change.Name);
        if (batch.EarlierFieldStores.TryGetValue(fieldKey, out FieldStoreRegistry.StoreEntry? earlier))
        {
            info.StoreFqn = earlier.StoreTypeFqn;
            info.StoreMetadataName = earlier.StoreTypeMetadataName;
            if (earlier.MemberName.Length > 0)
                info.Name = earlier.MemberName;
            info.IsNewStore = false;
            return info;
        }

        int dot = declaringType.LastIndexOf('.');
        string ns = dot < 0 ? "" : declaringType[..dot];
        string chain = (dot < 0 ? declaringType : declaringType[(dot + 1)..]).Replace('+', '_');
        string storeName = "__LocusFields_" + chain
            + (batch.StoreDiscriminator.Length > 0 ? "_" + batch.StoreDiscriminator : "");
        info.StoreNamespace = ns;
        info.StoreMetadataName = ns.Length == 0 ? storeName : ns + "." + storeName;
        info.StoreFqn = "global::" + info.StoreMetadataName;
        return info;
    }

    // C2′a: probe columns (AccessProbeSource cell ops) required per body
    // reference shape. Field reads and writes are hard to split exactly
    // (compound assignment, ref receivers), so any field touch requires
    // BOTH the load and the store column; calls require both call flavors.
    private static readonly string[] InstanceFieldOps = { "ldfld", "stfld" };
    private static readonly string[] StaticFieldOps = { "ldsfld", "stsfld" };
    private static readonly string[] CallOps = { "call", "callvirt" };
    private static readonly string[] CtorOps = { "newobj" };
    private static readonly string[] TypeReferenceOps = { "castclass", "ldtoken" };
    private static readonly string[] TypeCreationOps = { "castclass", "ldtoken", "newobj" };

    /// <summary>True when `name` is (part of) the type of a `new T(...)`
    /// expression — the JIT then also resolves T's constructor (newobj) on
    /// top of the type token itself.</summary>
    private static bool IsObjectCreationType(SimpleNameSyntax name)
    {
        SyntaxNode current = name;
        while (true)
        {
            switch (current.Parent)
            {
                case QualifiedNameSyntax qualified when qualified.Right == current:
                    current = qualified;
                    continue;
                case AliasQualifiedNameSyntax alias when alias.Name == current:
                    current = alias;
                    continue;
            }
            break;
        }
        return current.Parent is ObjectCreationExpressionSyntax creation && creation.Type == current;
    }

    /// <summary>Mono-reality check for ADDED members (M2): the generated
    /// shim runs OUTSIDE the original type and Unity's Mono enforces
    /// accessibility at JIT time (a violating shim throws
    /// FieldAccessException / MethodAccessException when first jitted).
    ///
    /// C2′a: whether a given (operation × visibility) actually fails is
    /// MEASURED per runtime by the C0 access probe. A BODY reference to
    /// non-public surface goes hot when every required probe cell is green —
    /// it rides the IgnoresAccessChecksTo + IgnoreAccessibility mechanism
    /// the patch already compiles with, and Unity's PrepareHotPatchShims
    /// force-JITs every shim at apply time, so a runtime that rejects after
    /// all rolls the whole batch back instead of poisoning it. Caps absent
    /// (old plugin / failed probe) or any red cell keeps today's cold
    /// verdict. Non-public types in the shim's public SIGNATURE (declaring
    /// type, return/parameter types) stay cold regardless: the C0 matrix has
    /// no cell for declaration-site type loading yet. Patch-materialized
    /// surface (added members/fields, appended enum members, new types) is
    /// always exempt.</summary>
    /// <summary>Caps gate for a Release inline caller-refresh self-shim clone
    /// (Option A), callable from <c>PatchBatchContext.Build</c> without exposing
    /// the private added-field bookkeeping. A clone re-emits a CHANGED instance
    /// method's body as a static self-shim, so it adds no fields — the empty
    /// added-field map is correct, and conservatively treats any same-batch
    /// added field the body touches as a capped access (safe: a violation only
    /// drops the optimization, never fails the patch). Returns the violation
    /// reason, or null when the shim is safe to emit.</summary>
    internal static string? FindInlineShimAccessViolation(
        IMethodSymbol method,
        SyntaxNode bodyNode,
        SemanticModel model,
        PatchBatchContext batch,
        HashSet<INamedTypeSymbol> renamedSymbols)
        => FindShimAccessViolation(
            method,
            bodyNode,
            model,
            batch,
            new Dictionary<IFieldSymbol, AddedFieldInfo>(SymbolEqualityComparer.Default),
            renamedSymbols);

    private static string? FindShimAccessViolation(
        IMethodSymbol declared,
        SyntaxNode bodyNode,
        SemanticModel model,
        PatchBatchContext batch,
        Dictionary<IFieldSymbol, AddedFieldInfo> addedFieldSymbols,
        HashSet<INamedTypeSymbol> renamedSymbols)
    {
        AccessCaps? caps = batch.RuntimeCaps;
        string memberDisplay = SymbolMetadataName(declared.ContainingType) + "." + declared.Name;

        string Violation(ISymbol symbol, string detail) =>
            "added member references non-public surface: " + memberDisplay + " uses " +
            symbol.ToDisplayString(SymbolDisplayFormat.CSharpShortErrorMessageFormat) +
            " (" + detail + ")";

        const string SignatureDetail =
            "signature-level non-public type; not yet probed — the public shim must name it " +
            "in its own declaration; make it public or use unity_recompile";

        const string CapsAbsentDetail =
            "the shim runs outside the type; runtime caps absent (old plugin or a failed " +
            "access probe) — update the Locus plugin, or use unity_recompile";

        string CellRedDetail(string operation, string bucket) =>
            "this runtime's Mono blocks " + operation + " access to " + bucket +
            " members (probe cell " + operation + "_" + bucket + " is red); make the " +
            "referenced surface public or use unity_recompile";

        bool IsPatchLocal(ISymbol symbol)
        {
            switch (symbol)
            {
                case IMethodSymbol method when TryGetAddedTarget(method, batch, out _, out _):
                    return true;
                case IFieldSymbol field when
                    addedFieldSymbols.ContainsKey(field) || batch.AddedEnumMembers.ContainsKey(field):
                    return true;
                case IPropertySymbol or IEventSymbol when HasAddedAccessor(symbol, batch):
                    // B2: rewrites to a direct accessor-shim call / store
                    // access (patch-materialized surface).
                    return true;
            }
            // Declared in batch source but NOT a renamed pre-existing type:
            // a NEW type whose code compiles into the patch assembly itself
            // (same-assembly access never hits the runtime checks).
            if (symbol.Locations.Any(location => location.IsInSource))
            {
                INamedTypeSymbol? top = symbol as INamedTypeSymbol ?? symbol.ContainingType;
                while (top?.ContainingType != null)
                    top = top.ContainingType;
                if (top != null && !IsRenamedTypeSymbol(top, renamedSymbols))
                    return true;
            }
            return false;
        }

        // C2′a relaxation for BODY references: hot when every required
        // (operation × visibility) cell measured green; otherwise the
        // conservative cold verdict stands, naming the failing cell.
        string? CheckBodyAccess(ISymbol symbol, string[] operations)
        {
            List<string> buckets = RequiredBuckets(symbol, ShimVisibilityBucket);
            if (buckets.Count == 0)
                return null; // public all the way down
            if (caps == null)
                return Violation(symbol, CapsAbsentDetail);
            foreach (string operation in operations)
            {
                foreach (string bucket in buckets)
                {
                    if (!caps.Allows(operation, bucket))
                        return Violation(symbol, CellRedDetail(operation, bucket));
                }
            }
            return null;
        }

        string? CheckBodyType(ITypeSymbol? type, string[] operations)
        {
            switch (type)
            {
                case IArrayTypeSymbol array:
                    return CheckBodyType(array.ElementType, operations);
                case IPointerTypeSymbol pointer:
                    return CheckBodyType(pointer.PointedAtType, operations);
                case INamedTypeSymbol named:
                {
                    if (IsPatchLocal(named))
                        return null;
                    string? violation = CheckBodyAccess(named, operations);
                    if (violation != null)
                        return violation;
                    foreach (ITypeSymbol argument in named.TypeArguments)
                    {
                        violation = CheckBodyType(argument, operations);
                        if (violation != null)
                            return violation;
                    }
                    return null;
                }
                default:
                    return null; // type parameters, dynamic, null
            }
        }

        string? CheckSignatureType(ITypeSymbol? type)
        {
            switch (type)
            {
                case IArrayTypeSymbol array:
                    return CheckSignatureType(array.ElementType);
                case IPointerTypeSymbol pointer:
                    return CheckSignatureType(pointer.PointedAtType);
                case INamedTypeSymbol named:
                {
                    if (IsPatchLocal(named))
                        return null;
                    if (!IsAccessiblePublicly(named))
                        return Violation(named, SignatureDetail);
                    foreach (ITypeSymbol argument in named.TypeArguments)
                    {
                        string? nested = CheckSignatureType(argument);
                        if (nested != null)
                            return nested;
                    }
                    return null;
                }
                default:
                    return null; // type parameters, dynamic, null
            }
        }

        // Signature surface first: the (public) shim must be able to NAME
        // these types in its own declaration — never relaxed by the body
        // matrix (no probe cell covers declaration-site loading yet).
        if (!declared.IsStatic && !IsAccessiblePublicly(declared.ContainingType))
        {
            return "added instance member on a non-public type: " + memberDisplay +
                " (the shim cannot name the declaring type — signature-level non-public type; " +
                "not yet probed; use unity_recompile)";
        }
        string? signatureViolation = CheckSignatureType(declared.ReturnType);
        if (signatureViolation != null)
            return signatureViolation;
        foreach (IParameterSymbol parameter in declared.Parameters)
        {
            signatureViolation = CheckSignatureType(parameter.Type);
            if (signatureViolation != null)
                return signatureViolation;
        }

        // Body references: every bound member/type the shim would touch.
        foreach (SyntaxNode node in bodyNode.DescendantNodes())
        {
            if (node is not SimpleNameSyntax name)
                continue;
            ISymbol? symbol = model.GetSymbolInfo(name).Symbol;
            if (symbol == null)
                continue;
            if (symbol.Kind is not SymbolKind.Method and not SymbolKind.Property
                and not SymbolKind.Field and not SymbolKind.Event and not SymbolKind.NamedType)
            {
                continue;
            }
            if (IsPatchLocal(symbol))
                continue;
            if (symbol is INamedTypeSymbol typeRef)
            {
                // typeof/cast/expression type → the type token itself must
                // JIT (castclass + ldtoken columns); `new T(...)` adds the
                // constructor's newobj on top.
                string? typeViolation = CheckBodyType(
                    typeRef, IsObjectCreationType(name) ? TypeCreationOps : TypeReferenceOps);
                if (typeViolation != null)
                    return typeViolation;
                continue;
            }
            string? memberViolation = symbol switch
            {
                IFieldSymbol field => CheckBodyAccess(
                    field, field.IsStatic ? StaticFieldOps : InstanceFieldOps),
                IMethodSymbol { MethodKind: MethodKind.Constructor } ctor =>
                    CheckBodyAccess(ctor, CtorOps),
                IMethodSymbol method => CheckBodyAccess(method, CallOps),
                IPropertySymbol property => CheckBodyAccess(property, CallOps),
                IEventSymbol @event => CheckBodyAccess(@event, CallOps),
                _ => CheckBodyAccess(symbol, CallOps), // unreachable (kind-filtered)
            };
            if (memberViolation != null)
                return memberViolation;
        }

        // `new T(...)` resolves T's CONSTRUCTOR on top of the type token,
        // and the ctor symbol hangs on the creation node — a non-public
        // ctor on a public type never surfaces through the name scan above
        // (the type-name check only folds the TYPE's own visibility).
        foreach (BaseObjectCreationExpressionSyntax creation in bodyNode.DescendantNodes().OfType<BaseObjectCreationExpressionSyntax>())
        {
            if (model.GetSymbolInfo(creation).Symbol is not IMethodSymbol creationCtor)
                continue;
            if (IsPatchLocal(creationCtor))
                continue;
            string? ctorViolation = CheckBodyAccess(creationCtor, CtorOps);
            if (ctorViolation != null)
                return ctorViolation;
        }

        return null;
    }

    /// <summary>Public (or accessibility-less) all the way up the
    /// containing-type chain — the only surface a PUBLIC shim signature may
    /// name, and the only one the JIT never access-checks.</summary>
    private static bool IsAccessiblePublicly(ISymbol? symbol)
    {
        for (ISymbol? current = symbol;
            current is not null and not INamespaceSymbol;
            current = current.ContainingType)
        {
            if (current.DeclaredAccessibility is not Accessibility.Public
                and not Accessibility.NotApplicable)
            {
                return false;
            }
        }
        return true;
    }

    /// <summary>Probe matrix column(s) one accessibility level maps to for
    /// ADDED members (M2 shims). The shim is an unrelated type, so
    /// `protected` degrades to the private column; IgnoresAccessChecksTo
    /// makes the patch assembly-equivalent, so every internal flavor maps to
    /// the internal column.</summary>
    private static string? ShimVisibilityBucket(Accessibility accessibility) => accessibility switch
    {
        Accessibility.Private => "private",
        Accessibility.Protected => "private",
        Accessibility.Internal => "internal",
        Accessibility.ProtectedOrInternal => "internal",
        Accessibility.ProtectedAndInternal => "internal",
        _ => null,
    };

    /// <summary>Probe matrix column(s) for KEPT bodies (the patch COPY of
    /// the edited type): the copy derives from the same original base chain,
    /// so plain `protected` (and the protected leg of `protected internal`)
    /// stays legal by inheritance and never needs a cell; `private
    /// protected` still needs the same-assembly leg → internal column.</summary>
    private static string? KeptVisibilityBucket(Accessibility accessibility) => accessibility switch
    {
        Accessibility.Private => "private",
        Accessibility.Internal => "internal",
        Accessibility.ProtectedAndInternal => "internal",
        _ => null,
    };

    /// <summary>The JIT resolves the member AND its whole containing-type
    /// chain: every non-public level on the way up contributes its column.</summary>
    private static List<string> RequiredBuckets(ISymbol symbol, Func<Accessibility, string?> bucket)
    {
        var buckets = new List<string>(2);
        for (ISymbol? current = symbol;
            current is not null and not INamespaceSymbol;
            current = current.ContainingType)
        {
            string? column = bucket(current.DeclaredAccessibility);
            if (column != null && !buckets.Contains(column))
                buckets.Add(column);
        }
        return buckets;
    }

    /// <summary>C2′b: scan every produced body OUTSIDE added members (kept
    /// member bodies, new-type bodies, added-field initializers) for
    /// references that end up as ORIGINAL-metadata tokens of non-public
    /// surface, and cold-name the first one whose probe cell measured red.
    /// Only invoked when the matrix has a red cell (callers short-circuit
    /// the all-green/absent cases — those keep today's hot verdicts).
    ///
    /// What emits an original token from a kept body: every reference to
    /// non-batch metadata (other assemblies, unedited files — the patch is
    /// its own assembly either way); and, for the RENAMED batch types
    /// themselves, whatever the rewrite re-qualifies to the original —
    /// type tokens (typeof/casts/creations), constructors, unqualified
    /// static STATE, and any access through a non-`this` receiver
    /// (receivers are original-typed after the rewrite). `this.`-routed
    /// member access keeps the PATCH COPY's same-assembly token and plain
    /// `protected` stays legal through the copy's inheritance, so neither
    /// needs a cell. Known boundary (over to the detour-time JIT net):
    /// operator/indexer/foreach/await pattern calls bind to no name node
    /// and are not scanned; member-declaration signatures (parameter/
    /// return/field types) load at type-load, which no cell measures.</summary>
    private static string? FindKeptSurfaceAccessViolation(
        CompilationUnitSyntax root,
        SemanticModel model,
        PatchBatchContext batch,
        Dictionary<IFieldSymbol, AddedFieldInfo> addedFieldSymbols,
        HashSet<INamedTypeSymbol> renamedSymbols,
        Func<SyntaxNode, bool> inAddedMember,
        Func<SyntaxNode, bool> inStrippedSpan)
    {
        AccessCaps caps = batch.RuntimeCaps!;

        string Violation(SyntaxNode node, ISymbol symbol, string operation, string bucket) =>
            "patched body references non-public surface: " + EnclosingMemberDisplay(node) + " uses " +
            symbol.ToDisplayString(SymbolDisplayFormat.CSharpShortErrorMessageFormat) +
            " (this runtime's Mono blocks " + operation + " access to " + bucket +
            " members — probe cell " + operation + "_" + bucket +
            " is red; make the referenced surface public or use unity_recompile)";

        string? CheckAccess(SyntaxNode node, ISymbol symbol, string[] operations)
        {
            List<string> buckets = RequiredBuckets(symbol, KeptVisibilityBucket);
            if (buckets.Count == 0)
                return null;
            foreach (string operation in operations)
            {
                foreach (string bucket in buckets)
                {
                    if (!caps.Allows(operation, bucket))
                        return Violation(node, symbol, operation, bucket);
                }
            }
            return null;
        }

        bool IsKeptExempt(ISymbol symbol, bool thisReceiver)
        {
            switch (symbol)
            {
                case IMethodSymbol method when TryGetAddedTarget(method, batch, out _, out _):
                    return true; // rewrites to a direct shim call (patch-materialized)
                case IPropertySymbol or IEventSymbol when HasAddedAccessor(symbol, batch):
                    return true; // B2: rewrites to a shim call / store access
                case IFieldSymbol field when
                    addedFieldSymbols.ContainsKey(field) || batch.AddedEnumMembers.ContainsKey(field):
                    return true; // rewrites to store access / cast literal
                case IFieldSymbol { IsConst: true }:
                    return true; // inlined at compile time; no runtime token
            }
            if (!symbol.Locations.Any(location => location.IsInSource))
                return false; // original metadata: always an original token

            // A brand-new NESTED type under a renamed container re-qualifies
            // to the PATCH name (PatchQualifiedDisplay): patch tokens.
            for (INamedTypeSymbol? container = symbol as INamedTypeSymbol ?? symbol.ContainingType;
                container != null; container = container.ContainingType)
            {
                if (batch.NewTypeSymbols.Contains(container.OriginalDefinition))
                    return true;
            }

            INamedTypeSymbol? top = symbol as INamedTypeSymbol ?? symbol.ContainingType;
            while (top?.ContainingType != null)
                top = top.ContainingType;
            if (top == null || !IsRenamedTypeSymbol(top, renamedSymbols))
                return true; // brand-new surface: compiles into the patch itself

            // Declared on a RENAMED batch type: only `this.`-routed
            // instance/static-method references keep patch-copy tokens.
            if (symbol is INamedTypeSymbol)
                return false; // type refs re-qualify to the original name
            if (symbol is IMethodSymbol { MethodKind: MethodKind.Constructor })
                return false; // `new Foo()` re-qualifies to the original ctor
            if (!thisReceiver)
                return false; // receivers are original-typed after the rewrite
            bool staticState = symbol switch
            {
                IFieldSymbol f => f.IsStatic,
                IPropertySymbol p => p.IsStatic,
                IEventSymbol e => e.IsStatic,
                _ => false,
            };
            return !staticState; // unqualified static state re-qualifies too
        }

        string? CheckTypeRef(SyntaxNode node, ITypeSymbol? type, string[] operations)
        {
            switch (type)
            {
                case IArrayTypeSymbol array:
                    return CheckTypeRef(node, array.ElementType, operations);
                case IPointerTypeSymbol pointer:
                    return CheckTypeRef(node, pointer.PointedAtType, operations);
                case INamedTypeSymbol named:
                {
                    if (IsKeptExempt(named, thisReceiver: false))
                        return null;
                    string? violation = CheckAccess(node, named, operations);
                    if (violation != null)
                        return violation;
                    foreach (ITypeSymbol argument in named.TypeArguments)
                    {
                        violation = CheckTypeRef(node, argument, operations);
                        if (violation != null)
                            return violation;
                    }
                    return null;
                }
                default:
                    return null; // type parameters, dynamic, null
            }
        }

        foreach (SyntaxNode node in root.DescendantNodes())
        {
            if (node is not SimpleNameSyntax name)
                continue;
            if (inAddedMember(name) || inStrippedSpan(name) || IsDeclarationName(name))
                continue;
            if (name.Ancestors().Any(a => a is UsingDirectiveSyntax))
                continue; // compile-time lookup only; no token
            ISymbol? symbol = model.GetSymbolInfo(name).Symbol;
            if (symbol == null)
                continue;
            if (symbol.Kind is not SymbolKind.Method and not SymbolKind.Property
                and not SymbolKind.Field and not SymbolKind.Event and not SymbolKind.NamedType)
            {
                continue;
            }

            if (symbol is INamedTypeSymbol typeRef)
            {
                // A pure qualifier emits no type token of its own — the
                // qualified MEMBER's token folds the containing chain
                // (RequiredBuckets); only typeof/cast/creation/argument
                // positions load the type.
                if (name.Parent is MemberAccessExpressionSyntax qualifier && qualifier.Expression == name)
                    continue;
                if (name.Parent is QualifiedNameSyntax qualified && qualified.Left == name)
                    continue;
                if (InMemberSignature(name) || FindEnclosingNameOf(name, model) != null)
                    continue;
                string? typeViolation = CheckTypeRef(
                    name, typeRef, IsObjectCreationType(name) ? TypeCreationOps : TypeReferenceOps);
                if (typeViolation != null)
                    return typeViolation;
                continue;
            }

            if (IsKeptExempt(symbol, HasThisOrImplicitReceiver(name)))
                continue;
            if (InMemberSignature(name) || FindEnclosingNameOf(name, model) != null)
                continue;
            string? memberViolation = symbol switch
            {
                IFieldSymbol field => CheckAccess(
                    name, field, field.IsStatic ? StaticFieldOps : InstanceFieldOps),
                IMethodSymbol { MethodKind: MethodKind.Constructor } ctor =>
                    CheckAccess(name, ctor, CtorOps),
                IMethodSymbol method => CheckAccess(name, method, CallOps),
                IPropertySymbol property => CheckAccess(name, property, CallOps),
                IEventSymbol @event => CheckAccess(name, @event, CallOps),
                _ => null,
            };
            if (memberViolation != null)
                return memberViolation;
        }

        // `new T(...)` — the constructor symbol hangs on the creation node.
        foreach (BaseObjectCreationExpressionSyntax creation in root.DescendantNodes().OfType<BaseObjectCreationExpressionSyntax>())
        {
            if (inAddedMember(creation) || inStrippedSpan(creation))
                continue;
            if (model.GetSymbolInfo(creation).Symbol is not IMethodSymbol creationCtor)
                continue;
            if (IsKeptExempt(creationCtor, thisReceiver: false))
                continue;
            string? ctorViolation = CheckAccess(creation, creationCtor, CtorOps);
            if (ctorViolation != null)
                return ctorViolation;
        }

        // A standalone `this` in a kept member rewrites to
        // `((global::Ns.Foo)(object)this)` — a castclass against the
        // ORIGINAL type, which may itself be non-public.
        foreach (ThisExpressionSyntax thisNode in root.DescendantNodes().OfType<ThisExpressionSyntax>())
        {
            if (inAddedMember(thisNode) || inStrippedSpan(thisNode))
                continue;
            bool isReceiver =
                (thisNode.Parent is MemberAccessExpressionSyntax mae && mae.Expression == thisNode) ||
                (thisNode.Parent is ElementAccessExpressionSyntax eae && eae.Expression == thisNode);
            if (isReceiver)
                continue;
            TypeDeclarationSyntax? enclosingType = thisNode.Ancestors().OfType<TypeDeclarationSyntax>().FirstOrDefault();
            if (enclosingType == null)
                continue;
            if (model.GetDeclaredSymbol(enclosingType) is not INamedTypeSymbol enclosingSymbol)
                continue;
            if (!IsRenamedTypeSymbol(enclosingSymbol, renamedSymbols))
                continue;
            string? escapeViolation = CheckAccess(thisNode, enclosingSymbol, TypeReferenceOps);
            if (escapeViolation != null)
                return escapeViolation;
        }

        return null;
    }

    /// <summary>Receiver shape of a (member) name node: `this.X` and
    /// unqualified `X` (implicit this / implicit static context) bind to the
    /// patch copy's own tokens; everything else goes through an expression
    /// that is ORIGINAL-typed after the rewrite.</summary>
    private static bool HasThisOrImplicitReceiver(SimpleNameSyntax name)
    {
        if (name.Parent is MemberAccessExpressionSyntax access && access.Name == name)
            return access.Expression is ThisExpressionSyntax;
        if (name.Parent is MemberBindingExpressionSyntax)
            return false; // x?.Member
        return true;
    }

    /// <summary>True when the name sits in a member-declaration SIGNATURE
    /// position (parameter/return/field/property types, base lists,
    /// constraints, attributes): those load at type-load time — outside the
    /// per-operation probe matrix — and kept signatures have always shipped,
    /// so the kept-surface scan leaves them alone. Body positions (including
    /// local declarations and initializer expressions) return false.</summary>
    private static bool InMemberSignature(SyntaxNode node)
    {
        foreach (SyntaxNode ancestor in node.Ancestors())
        {
            switch (ancestor)
            {
                case ParameterSyntax:
                case BaseListSyntax:
                case TypeParameterConstraintClauseSyntax:
                case ExplicitInterfaceSpecifierSyntax:
                case AttributeListSyntax:
                case DelegateDeclarationSyntax:
                    return true;
                case MethodDeclarationSyntax method when method.ReturnType.FullSpan.Contains(node.Span):
                    return true;
                case BasePropertyDeclarationSyntax property when property.Type.FullSpan.Contains(node.Span):
                    return true;
                case BaseFieldDeclarationSyntax field when field.Declaration.Type.FullSpan.Contains(node.Span):
                    return true;
                case StatementSyntax:
                case BaseTypeDeclarationSyntax:
                    return false;
            }
        }
        return false;
    }

    /// <summary>"Type.Member" display of the member declaration enclosing
    /// `node`, for kept-surface violation messages.</summary>
    private static string EnclosingMemberDisplay(SyntaxNode node)
    {
        string member = "";
        foreach (SyntaxNode ancestor in node.Ancestors())
        {
            if (member.Length == 0)
            {
                member = ancestor switch
                {
                    MethodDeclarationSyntax method => method.Identifier.Text,
                    ConstructorDeclarationSyntax => ".ctor",
                    DestructorDeclarationSyntax => "~dtor",
                    PropertyDeclarationSyntax property => property.Identifier.Text,
                    IndexerDeclarationSyntax => "this[]",
                    EventDeclarationSyntax @event => @event.Identifier.Text,
                    OperatorDeclarationSyntax op => "operator " + op.OperatorToken.Text,
                    ConversionOperatorDeclarationSyntax => "conversion",
                    VariableDeclaratorSyntax declarator when
                        declarator.Parent?.Parent is BaseFieldDeclarationSyntax => declarator.Identifier.Text,
                    _ => "",
                };
            }
            if (ancestor is BaseTypeDeclarationSyntax typeDecl)
            {
                string typeName = HotDiff.MetadataName(typeDecl);
                return member.Length == 0 ? typeName : typeName + "." + member;
            }
        }
        return member.Length == 0 ? "(file)" : member;
    }

    /// <summary>Constructive layout verification for a type with M4 field
    /// changes: source fields minus ADDED must equal original fields minus
    /// REMOVED (names and types, in order — auto-property backing fields
    /// included). Returns an error string on mismatch (stale baseline).</summary>
    private static string? VerifyVirtualizedLayout(
        INamedTypeSymbol sourceSymbol,
        INamedTypeSymbol original,
        List<HotDiffFieldChange> changes)
    {
        var addedNames = new HashSet<string>(
            changes.Where(c => c.Kind == "added" && !c.IsStatic).Select(c => c.Name),
            StringComparer.Ordinal);
        var removedNames = new HashSet<string>(
            changes.Where(c => c.Kind == "removed" && !c.IsStatic).Select(c => c.Name),
            StringComparer.Ordinal);

        static string EntryName(string entry) => entry[..entry.IndexOf('|')];

        List<string> source = InstanceFieldSequence(sourceSymbol)
            .Where(e => !addedNames.Contains(EntryName(e)))
            .ToList();
        List<string> originalSeq = InstanceFieldSequence(original);
        List<string> originalFiltered = originalSeq
            .Where(e => !removedNames.Contains(EntryName(e)))
            .ToList();

        if (!source.SequenceEqual(originalFiltered, StringComparer.Ordinal))
            return "constructed layout mismatch";
        foreach (string removed in removedNames)
        {
            if (!originalSeq.Any(e => EntryName(e) == removed))
                return "removed field not present in the original: " + removed;
        }
        return null;
    }

    /// <summary>Strip added-field declarations, inject placeholders for the
    /// removed fields at their original positions, materialize added-field
    /// initializers into instance constructors, and declare new
    /// stores/holders. Operates purely syntactically on the rewritten unit.</summary>
    private static CompilationUnitSyntax ApplyFieldSurgery(
        CompilationUnitSyntax unit,
        string declaringType,
        List<HotDiffFieldChange> changes,
        List<AddedFieldInfo> addedInfos,
        INamedTypeSymbol original,
        PatchRewriteResult result)
    {
        string renamedMetadataName = PatchTypeName(declaringType);

        TypeDeclarationSyntax? Locate(CompilationUnitSyntax current) => current
            .DescendantNodes()
            .OfType<TypeDeclarationSyntax>()
            .FirstOrDefault(t => HotDiff.MetadataName(t) == renamedMetadataName);

        // 1. Strip added-field declarators (instance AND static).
        TypeDeclarationSyntax? typeDecl = Locate(unit);
        if (typeDecl == null)
            return unit;

        var addedNames = new HashSet<string>(addedInfos.Select(i => i.Name), StringComparer.Ordinal);
        if (addedNames.Count > 0)
        {
            var removeDeclarations = new List<SyntaxNode>();
            var declarationRewrites = new Dictionary<SyntaxNode, SyntaxNode>();
            foreach (FieldDeclarationSyntax field in typeDecl.Members.OfType<FieldDeclarationSyntax>())
            {
                if (field.Modifiers.Any(SyntaxKind.ConstKeyword))
                    continue;
                var kept = field.Declaration.Variables
                    .Where(v => !addedNames.Contains(v.Identifier.Text))
                    .ToList();
                if (kept.Count == field.Declaration.Variables.Count)
                    continue;
                if (kept.Count == 0)
                    removeDeclarations.Add(field);
                else
                    declarationRewrites[field] = field.WithDeclaration(
                        field.Declaration.WithVariables(SyntaxFactory.SeparatedList(kept)));
            }

            if (declarationRewrites.Count > 0)
            {
                unit = unit.ReplaceNodes(declarationRewrites.Keys, (orig, _) => declarationRewrites[orig]);
                typeDecl = Locate(unit)!;
                removeDeclarations = typeDecl.Members.OfType<FieldDeclarationSyntax>()
                    .Where(f => !f.Modifiers.Any(SyntaxKind.ConstKeyword) &&
                                f.Declaration.Variables.All(v => addedNames.Contains(v.Identifier.Text)))
                    .Cast<SyntaxNode>()
                    .ToList();
            }
            if (removeDeclarations.Count > 0)
            {
                unit = unit.RemoveNodes(removeDeclarations, SyntaxRemoveOptions.KeepNoTrivia)!;
                typeDecl = Locate(unit);
                if (typeDecl == null)
                    return unit;
            }
        }

        // 2. Placeholders for removed instance fields, at original order.
        var removedInstance = changes
            .Where(c => c.Kind == "removed" && !c.IsStatic)
            .OrderBy(c => c.OldFieldIndex)
            .ToList();
        if (removedInstance.Count > 0)
        {
            List<(string Name, string TypeFqn)> originalFields = InstanceFieldSequence(original)
                .Select(e =>
                {
                    int bar = e.IndexOf('|');
                    return (e[..bar], e[(bar + 1)..]);
                })
                .ToList();

            foreach (HotDiffFieldChange removed in removedInstance)
            {
                typeDecl = Locate(unit);
                if (typeDecl == null)
                    return unit;

                int originalIndex = originalFields.FindIndex(f => f.Name == removed.Name);
                if (originalIndex < 0)
                    continue;
                string typeFqn = originalFields[originalIndex].TypeFqn;

                MemberDeclarationSyntax placeholder = SyntaxFactory.ParseMemberDeclaration(
                        "private " + typeFqn + " " + removed.Name + ";")!
                    .NormalizeWhitespace()
                    .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed)
                    .WithTrailingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);

                // Anchor: the first original field AFTER this one that still
                // exists in the patch type (backing fields anchor on their
                // auto-property declaration).
                MemberDeclarationSyntax? anchor = null;
                for (int i = originalIndex + 1; i < originalFields.Count && anchor == null; i++)
                {
                    string nextName = originalFields[i].Name;
                    if (removedInstance.Any(r => r.Name == nextName))
                        continue;
                    anchor = FindLayoutMember(typeDecl, nextName);
                }

                TypeDeclarationSyntax updated;
                if (anchor != null)
                {
                    updated = typeDecl.WithMembers(typeDecl.Members.Insert(
                        typeDecl.Members.IndexOf(anchor), placeholder));
                }
                else
                {
                    // No following field: append after the LAST layout
                    // member (or at the top when none remain).
                    int insertIndex = 0;
                    for (int i = 0; i < typeDecl.Members.Count; i++)
                    {
                        MemberDeclarationSyntax member = typeDecl.Members[i];
                        if (member is FieldDeclarationSyntax f && !f.Modifiers.Any(SyntaxKind.ConstKeyword) &&
                            !f.Modifiers.Any(SyntaxKind.StaticKeyword))
                        {
                            insertIndex = i + 1;
                        }
                        else if (member is PropertyDeclarationSyntax p && HotDiff.IsAutoProperty(p) &&
                                 !p.Modifiers.Any(SyntaxKind.StaticKeyword))
                        {
                            insertIndex = i + 1;
                        }
                        else if (member is EventFieldDeclarationSyntax)
                        {
                            insertIndex = i + 1;
                        }
                    }
                    updated = typeDecl.WithMembers(typeDecl.Members.Insert(insertIndex, placeholder));
                }
                unit = unit.ReplaceNode(typeDecl, updated);
            }
        }

        // 3. Materialize added-field initializers into the instance ctors.
        var initialized = addedInfos
            .Where(i => !i.IsStatic && i.RewrittenInitializerText != null)
            .ToList();
        if (initialized.Count > 0)
        {
            typeDecl = Locate(unit);
            if (typeDecl == null)
                return unit;

            var statements = new List<StatementSyntax>();
            foreach (AddedFieldInfo info in initialized)
            {
                statements.Add(SyntaxFactory.ParseStatement(
                        info.StoreFqn + "." + info.Name + ".Ref(this) = " +
                        info.RewrittenInitializerText!.Trim() + ";")
                    .NormalizeWhitespace()
                    .WithTrailingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed));
            }

            var ctors = typeDecl.Members.OfType<ConstructorDeclarationSyntax>()
                .Where(c => !c.Modifiers.Any(SyntaxKind.StaticKeyword))
                .ToList();
            if (ctors.Count == 0)
            {
                // The original implicit default ctor is being detoured (the
                // diff added .ctor): synthesize its patch-side counterpart.
                ConstructorDeclarationSyntax synthesized = SyntaxFactory.ConstructorDeclaration(typeDecl.Identifier.Text)
                    .WithModifiers(SyntaxFactory.TokenList(SyntaxFactory.Token(SyntaxKind.PublicKeyword)))
                    .WithBody(SyntaxFactory.Block(statements))
                    .NormalizeWhitespace()
                    .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed)
                    .WithTrailingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);
                unit = unit.ReplaceNode(typeDecl, typeDecl.AddMembers(synthesized));
            }
            else
            {
                var ctorRewrites = new Dictionary<SyntaxNode, SyntaxNode>();
                foreach (ConstructorDeclarationSyntax ctor in ctors)
                {
                    // Chained `: this(...)` ctors delegate initialization,
                    // mirroring Roslyn's initializer emission.
                    if (ctor.Initializer != null && ctor.Initializer.IsKind(SyntaxKind.ThisConstructorInitializer))
                        continue;
                    if (ctor.Body != null)
                    {
                        ctorRewrites[ctor] = ctor.WithBody(
                            ctor.Body.WithStatements(ctor.Body.Statements.InsertRange(0, statements)));
                    }
                    else if (ctor.ExpressionBody != null)
                    {
                        var bodyStatements = new List<StatementSyntax>(statements)
                        {
                            SyntaxFactory.ExpressionStatement(ctor.ExpressionBody.Expression),
                        };
                        ctorRewrites[ctor] = ctor
                            .WithExpressionBody(null)
                            .WithSemicolonToken(default)
                            .WithBody(SyntaxFactory.Block(bodyStatements));
                    }
                }
                if (ctorRewrites.Count > 0)
                    unit = unit.ReplaceNodes(ctorRewrites.Keys, (orig, _) => ctorRewrites[orig]);
            }
        }

        // 4. Declare new stores/holders + registrations.
        var newStores = addedInfos.Where(i => i.IsNewStore).ToList();
        if (newStores.Count > 0)
        {
            var members = new List<MemberDeclarationSyntax>();
            foreach (AddedFieldInfo info in newStores)
            {
                string declaration;
                if (info.IsStatic)
                {
                    declaration = "public static " + info.FieldTypeFqn + " " + info.Name +
                        (info.RewrittenInitializerText != null
                            ? " = " + info.RewrittenInitializerText.Trim() + ";"
                            : ";");
                }
                else
                {
                    declaration =
                        "public static readonly global::Locus.HotReload.LocusFieldStore<" + info.FieldTypeFqn + "> " +
                        info.Name + " = new global::Locus.HotReload.LocusFieldStore<" + info.FieldTypeFqn + ">();";
                }
                members.Add(SyntaxFactory.ParseMemberDeclaration(declaration)!);
            }

            AddedFieldInfo first = newStores[0];
            string storeSimpleName = first.StoreMetadataName.Contains('.')
                ? first.StoreMetadataName[(first.StoreMetadataName.LastIndexOf('.') + 1)..]
                : first.StoreMetadataName;
            ClassDeclarationSyntax storeClass = SyntaxFactory.ClassDeclaration(storeSimpleName)
                .WithModifiers(SyntaxFactory.TokenList(
                    SyntaxFactory.Token(SyntaxKind.PublicKeyword),
                    SyntaxFactory.Token(SyntaxKind.StaticKeyword)))
                .WithMembers(SyntaxFactory.List(members))
                .NormalizeWhitespace()
                .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);

            unit = AppendTypeToNamespace(unit, storeClass, first.StoreNamespace);

            foreach (AddedFieldInfo info in newStores)
            {
                result.FieldStoreRegistrations.Add(new FieldStoreRegistration
                {
                    FieldKey = FieldStoreRegistry.FieldKey(info.DeclaringType, info.RegistryFieldName),
                    Entry = new FieldStoreRegistry.StoreEntry
                    {
                        StoreTypeMetadataName = info.StoreMetadataName,
                        StoreTypeFqn = info.StoreFqn,
                        MemberName = info.Name,
                        IsStatic = info.IsStatic,
                        FieldTypeFqn = info.FieldTypeFqn,
                    },
                });
            }
        }

        return unit;
    }

    /// <summary>Find the member declaration that carries a layout entry by
    /// field name ("&lt;X&gt;k__BackingField" anchors on auto-property X).</summary>
    private static MemberDeclarationSyntax? FindLayoutMember(TypeDeclarationSyntax type, string fieldName)
    {
        string? propertyName = null;
        if (fieldName.StartsWith("<", StringComparison.Ordinal))
        {
            int end = fieldName.IndexOf('>');
            if (end > 1)
                propertyName = fieldName[1..end];
        }

        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when propertyName == null:
                    if (field.Declaration.Variables.Any(v => v.Identifier.Text == fieldName))
                        return field;
                    break;
                case EventFieldDeclarationSyntax eventField when propertyName == null:
                    if (eventField.Declaration.Variables.Any(v => v.Identifier.Text == fieldName))
                        return eventField;
                    break;
                case PropertyDeclarationSyntax property when propertyName != null:
                    if (property.Identifier.Text == propertyName)
                        return property;
                    break;
            }
        }
        return null;
    }

    /// <summary>`x` / `expr.x` → `Store.x.Ref(target)` (instance) or
    /// `Holder.x` (static).</summary>
    private static SyntaxNode BuildFieldStoreAccess(
        SyntaxNode rewrittenNode,
        AddedFieldInfo field,
        bool inAddedMember)
    {
        if (field.IsStatic)
        {
            return SyntaxFactory.ParseExpression(field.StoreFqn + "." + field.Name)
                .WithTriviaFrom(rewrittenNode);
        }

        ExpressionSyntax target;
        if (rewrittenNode is MemberAccessExpressionSyntax memberAccess)
        {
            // `this.x` inside kept members keeps `this` (the store key is
            // the object identity; the patch-typed reference converts to
            // object implicitly). Inside added members `this` already became
            // `self`.
            target = memberAccess.Expression.WithoutTrivia();
        }
        else
        {
            target = inAddedMember
                ? SyntaxFactory.IdentifierName("self")
                : SyntaxFactory.ThisExpression();
        }

        return SyntaxFactory.InvocationExpression(
                SyntaxFactory.ParseExpression(field.StoreFqn + "." + field.Name + ".Ref"),
                SyntaxFactory.ArgumentList(SyntaxFactory.SingletonSeparatedList(SyntaxFactory.Argument(target))))
            .WithTriviaFrom(rewrittenNode);
    }

    // ── B2: added property/indexer/event call-site materialization ───

    /// <summary>Compound-assignment operator token for the supported
    /// assignment kinds (??= is excluded: the expansion would always call
    /// the setter, losing the null-skip semantics).</summary>
    private static SyntaxKind? CompoundBinaryKind(SyntaxKind assignmentKind) => assignmentKind switch
    {
        SyntaxKind.AddAssignmentExpression => SyntaxKind.AddExpression,
        SyntaxKind.SubtractAssignmentExpression => SyntaxKind.SubtractExpression,
        SyntaxKind.MultiplyAssignmentExpression => SyntaxKind.MultiplyExpression,
        SyntaxKind.DivideAssignmentExpression => SyntaxKind.DivideExpression,
        SyntaxKind.ModuloAssignmentExpression => SyntaxKind.ModuloExpression,
        SyntaxKind.AndAssignmentExpression => SyntaxKind.BitwiseAndExpression,
        SyntaxKind.OrAssignmentExpression => SyntaxKind.BitwiseOrExpression,
        SyntaxKind.ExclusiveOrAssignmentExpression => SyntaxKind.ExclusiveOrExpression,
        SyntaxKind.LeftShiftAssignmentExpression => SyntaxKind.LeftShiftExpression,
        SyntaxKind.RightShiftAssignmentExpression => SyntaxKind.RightShiftExpression,
        _ => null,
    };

    private static ExpressionSyntax UnwrapParens(ExpressionSyntax expression)
    {
        while (expression is ParenthesizedExpressionSyntax parens)
            expression = parens.Expression;
        return expression;
    }

    /// <summary>Rewrite every reference that binds to an accessor ADDED by
    /// this batch (B2). Reads become `Shims.get_X(recv, …)`, statement
    /// assignments become `Shims.set_X(recv, …, value)`, compound statement
    /// assignments expand to get+op+set (receiver/index arguments gated to
    /// repeatable shapes), event subscriptions become add_/remove_ calls,
    /// and same-file AUTO-property references route to the lvalue-shaped
    /// backing store. Returns a pointed cold reason for any form whose
    /// semantics the expansion cannot preserve.</summary>
    private static string? RewriteAddedAccessorReferences(
        CompilationUnitSyntax root,
        SemanticModel model,
        PatchBatchContext batch,
        Dictionary<IMethodSymbol, AddedFieldInfo> autoStores,
        Func<SyntaxNode, ShimTarget?> enclosingAddedTarget,
        Func<SyntaxNode, bool> inStrippedSpan,
        Dictionary<SyntaxNode, SyntaxNode> nodeReplacements,
        Dictionary<SyntaxNode, Func<SyntaxNode, SyntaxNode>> dynamicReplacements)
    {
        if (batch.AddedMembers.Count == 0)
            return null;

        ShimTarget? Added(IMethodSymbol? accessor)
        {
            if (accessor == null)
                return null;
            return batch.AddedMembers.TryGetValue(accessor.OriginalDefinition, out ShimTarget? target) ? target : null;
        }

        foreach (SyntaxNode node in root.DescendantNodes())
        {
            if (node is not (IdentifierNameSyntax or MemberAccessExpressionSyntax or ElementAccessExpressionSyntax
                or MemberBindingExpressionSyntax or ElementBindingExpressionSyntax))
            {
                continue;
            }
            if (inStrippedSpan(node) || nodeReplacements.ContainsKey(node) || dynamicReplacements.ContainsKey(node))
                continue;
            if (node.Parent is MemberAccessExpressionSyntax parentAccess && parentAccess.Name == node)
                continue; // the outer member access owns it
            if (node is IdentifierNameSyntax && IsDeclarationName(node))
                continue;

            SymbolInfo symbolInfo = model.GetSymbolInfo(node);
            ISymbol? symbol = symbolInfo.Symbol;
            if (symbol is not (IPropertySymbol or IEventSymbol))
            {
                // Invalid-form uses (`ref obj.P`, `obj.E = x`) bind with
                // candidates only — resolve them so the matrix can NAME the
                // unsupported form instead of leaking a CS1061 on the
                // extracted member.
                symbol = symbolInfo.CandidateSymbols.FirstOrDefault(c => c is IPropertySymbol or IEventSymbol);
            }
            ShimTarget? getTarget = null;
            ShimTarget? setTarget = null;
            IMethodSymbol? getAccessor = null;
            AddedFieldInfo? store = null;
            bool isEvent = false;
            ITypeSymbol? valueType = null;
            string display;

            switch (symbol)
            {
                case IPropertySymbol property:
                {
                    getTarget = Added(property.GetMethod);
                    setTarget = Added(property.SetMethod);
                    if (getTarget == null && setTarget == null)
                        continue;
                    getAccessor = property.GetMethod ?? property.SetMethod;
                    valueType = property.Type;
                    display = (getTarget ?? setTarget)!.DeclaringTypeMetadataName + "." +
                        (property.IsIndexer ? "this[]" : property.Name);
                    if (property.GetMethod != null &&
                        autoStores.TryGetValue(property.GetMethod.OriginalDefinition, out AddedFieldInfo? getStore))
                    {
                        store = getStore;
                    }
                    else if (property.SetMethod != null &&
                        autoStores.TryGetValue(property.SetMethod.OriginalDefinition, out AddedFieldInfo? setStore))
                    {
                        store = setStore;
                    }
                    break;
                }
                case IEventSymbol @event:
                {
                    getTarget = Added(@event.AddMethod);     // +=
                    setTarget = Added(@event.RemoveMethod);  // -=
                    if (getTarget == null && setTarget == null)
                        continue;
                    getAccessor = @event.AddMethod ?? @event.RemoveMethod;
                    isEvent = true;
                    display = (getTarget ?? setTarget)!.DeclaringTypeMetadataName + "." + @event.Name;
                    break;
                }
                default:
                    continue;
            }

            // Conditional access has no expressible receiver for the shim.
            if (node is MemberBindingExpressionSyntax or ElementBindingExpressionSyntax)
            {
                return "added member accessed through ?. (conditional access): " + display +
                    " — rewrite the access without ?. or use unity_recompile";
            }

            // nameof(P) — the member is extracted from the patch copy, so
            // materialize the constant string.
            InvocationExpressionSyntax? nameofInvocation = FindEnclosingNameOf(node, model);
            if (nameofInvocation != null)
            {
                if (!nodeReplacements.ContainsKey(nameofInvocation))
                {
                    string constant = symbol is IEventSymbol evt ? evt.Name : ((IPropertySymbol)symbol!).Name;
                    nodeReplacements[nameofInvocation] = SyntaxFactory.LiteralExpression(
                            SyntaxKind.StringLiteralExpression,
                            SyntaxFactory.Literal(constant))
                        .WithTriviaFrom(nameofInvocation);
                }
                continue;
            }

            if (node.Ancestors().Any(a => a is AttributeListSyntax))
            {
                return "added member referenced inside an attribute: " + display +
                    " (attributes need compile-time constants; use unity_recompile)";
            }
            if (node.Parent is NameColonSyntax)
            {
                return "added property used in a pattern: " + display +
                    " (property patterns bind to original metadata; use unity_recompile)";
            }

            var reference = (ExpressionSyntax)node;
            ExpressionSyntax effective = reference;
            while (effective.Parent is ParenthesizedExpressionSyntax parens)
                effective = parens;
            SyntaxNode? use = effective.Parent;
            ShimTarget? enclosingAdded = enclosingAddedTarget(node);
            ShimTarget anyTarget = (getTarget ?? setTarget)!;
            string typeArgumentText = AccessorTypeArgumentText(anyTarget, getAccessor!);

            // ── events: only += / -= are expressible ────────────────
            if (isEvent)
            {
                if (use is AssignmentExpressionSyntax eventAssign && eventAssign.Left == effective &&
                    (eventAssign.IsKind(SyntaxKind.AddAssignmentExpression) ||
                     eventAssign.IsKind(SyntaxKind.SubtractAssignmentExpression)))
                {
                    ShimTarget? eventAccessor = eventAssign.IsKind(SyntaxKind.AddAssignmentExpression)
                        ? getTarget
                        : setTarget;
                    if (eventAccessor == null)
                    {
                        return "added event accessor not available for " +
                            (eventAssign.IsKind(SyntaxKind.AddAssignmentExpression) ? "+=" : "-=") +
                            ": " + display + " — use unity_recompile";
                    }
                    string? receiverCold = AccessorReceiverGuard(
                        reference, eventAccessor, model, enclosingAdded, display);
                    if (receiverCold != null)
                        return receiverCold;
                    ShimTarget capturedEvent = eventAccessor;
                    ShimTarget? capturedEnclosing = enclosingAdded;
                    string capturedTypeArgs = typeArgumentText;
                    dynamicReplacements[eventAssign] = rewrittenNode => BuildAccessorAssignmentCall(
                        (AssignmentExpressionSyntax)rewrittenNode, capturedEvent, capturedEnclosing, capturedTypeArgs);
                    continue;
                }
                return "added event can only be subscribed with += / -=: " + display +
                    " — use unity_recompile";
            }

            // ── same-file AUTO-property: lvalue-shaped store access ──
            // `Store.P.Ref(target)` (or the holder field) carries field
            // semantics, so reads, writes, compound assignments, ++/-- and
            // deconstruction targets all materialize from ONE rewrite.
            if (store != null)
            {
                if (use is AssignmentExpressionSyntax storeAssign && storeAssign.Left == effective &&
                    storeAssign.Parent is InitializerExpressionSyntax)
                {
                    return "added property used in an object initializer: " + display +
                        " — assign it in a separate statement or use unity_recompile";
                }
                if (IsByRefUse(effective))
                {
                    // Would compile against the ref-returning store but NOT
                    // against the eventual real compile (CS0206) — diverging
                    // semantics, so fail closed.
                    return "added property passed by ref/out: " + display + " — use unity_recompile";
                }
                AddedFieldInfo capturedStore = store;
                bool insideAdded = enclosingAdded != null;
                dynamicReplacements[node] = rewrittenNode =>
                    BuildFieldStoreAccess(rewrittenNode, capturedStore, insideAdded);
                continue;
            }

            // ── full property / indexer: the shim-call matrix ────────
            if (use is AssignmentExpressionSyntax assign && assign.Left == effective)
            {
                if (assign.Parent is InitializerExpressionSyntax)
                {
                    return "added property used in an object initializer: " + display +
                        " — assign it in a separate statement or use unity_recompile";
                }

                SyntaxKind assignKind = assign.Kind();
                if (assignKind == SyntaxKind.SimpleAssignmentExpression)
                {
                    if (setTarget == null)
                    {
                        return "added property has no set accessor to assign through: " + display +
                            " — use unity_recompile";
                    }
                    if (!AssignmentResultDiscarded(assign))
                    {
                        return "assignment to an added property is used as a value: " + display +
                            " (the set shim returns void) — split the statement or use unity_recompile";
                    }
                    string? receiverCold = AccessorReceiverGuard(reference, setTarget, model, enclosingAdded, display);
                    if (receiverCold != null)
                        return receiverCold;
                    ShimTarget capturedSet = setTarget;
                    ShimTarget? capturedEnclosing = enclosingAdded;
                    string capturedTypeArgs = typeArgumentText;
                    dynamicReplacements[assign] = rewrittenNode => BuildAccessorAssignmentCall(
                        (AssignmentExpressionSyntax)rewrittenNode, capturedSet, capturedEnclosing, capturedTypeArgs);
                    continue;
                }

                if (assignKind == SyntaxKind.CoalesceAssignmentExpression)
                {
                    return "??= on an added property cannot preserve its set-skip semantics: " + display +
                        " — use unity_recompile";
                }

                SyntaxKind? binaryKind = CompoundBinaryKind(assignKind);
                if (binaryKind != null)
                {
                    if (getTarget == null || setTarget == null)
                    {
                        return "compound assignment needs both accessors of the added property: " + display +
                            " — use unity_recompile";
                    }
                    if (!AssignmentResultDiscarded(assign))
                    {
                        return "compound assignment to an added property is used as a value: " + display +
                            " — split the statement or use unity_recompile";
                    }
                    string? receiverCold = AccessorReceiverGuard(reference, setTarget, model, enclosingAdded, display);
                    if (receiverCold != null)
                        return receiverCold;
                    // The expansion evaluates receiver and index arguments
                    // TWICE (get, then set): only repeatable shapes pass.
                    string? repeatCold = CompoundRepeatableGuard(reference, setTarget, model, display);
                    if (repeatCold != null)
                        return repeatCold;
                    string castFqn = valueType!.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat);
                    ShimTarget capturedGet = getTarget;
                    ShimTarget capturedSet = setTarget;
                    ShimTarget? capturedEnclosing = enclosingAdded;
                    string capturedTypeArgs = typeArgumentText;
                    SyntaxKind capturedBinary = binaryKind.Value;
                    dynamicReplacements[assign] = rewrittenNode => BuildAccessorCompoundCall(
                        (AssignmentExpressionSyntax)rewrittenNode, capturedGet, capturedSet,
                        capturedBinary, castFqn, capturedEnclosing, capturedTypeArgs);
                    continue;
                }

                return "unsupported assignment to an added property: " + display + " — use unity_recompile";
            }

            if ((use is PrefixUnaryExpressionSyntax pre &&
                    (pre.IsKind(SyntaxKind.PreIncrementExpression) || pre.IsKind(SyntaxKind.PreDecrementExpression))) ||
                (use is PostfixUnaryExpressionSyntax post &&
                    (post.IsKind(SyntaxKind.PostIncrementExpression) || post.IsKind(SyntaxKind.PostDecrementExpression))))
            {
                return "increment/decrement of an added property: " + display +
                    " — rewrite it as '+= 1' / '-= 1' or use unity_recompile";
            }

            if (IsByRefUse(effective))
                return "added property passed by ref/out: " + display + " — use unity_recompile";

            if (InAssignmentTargetChain(effective))
            {
                return "added property in a deconstruction or nested assignment target: " + display +
                    " — use unity_recompile";
            }

            // Plain READ in value position.
            if (getTarget == null)
            {
                return "added property has no get accessor to read through: " + display +
                    " — use unity_recompile";
            }
            {
                string? receiverCold = AccessorReceiverGuard(reference, getTarget, model, enclosingAdded, display);
                if (receiverCold != null)
                    return receiverCold;
                ShimTarget capturedGet = getTarget;
                ShimTarget? capturedEnclosing = enclosingAdded;
                string capturedTypeArgs = typeArgumentText;
                dynamicReplacements[node] = rewrittenNode => BuildAccessorGetCall(
                    rewrittenNode, capturedGet, capturedEnclosing, capturedTypeArgs);
            }
        }

        return null;
    }

    /// <summary>True when the (paren-climbed) use site passes the expression
    /// by reference: a ref/out argument or a `ref` expression (ref locals,
    /// ref returns).</summary>
    private static bool IsByRefUse(ExpressionSyntax effective)
    {
        if (effective.Parent is RefExpressionSyntax)
            return true;
        return effective.Parent is ArgumentSyntax argument &&
            argument.Expression == effective &&
            !argument.RefKindKeyword.IsKind(SyntaxKind.None);
    }

    /// <summary>True when the assignment's value is provably discarded:
    /// expression statements and for-statement initializer/incrementor
    /// positions. Everything else (chained assignments, lambda bodies,
    /// arrow bodies) stays conservative.</summary>
    private static bool AssignmentResultDiscarded(AssignmentExpressionSyntax assign) =>
        assign.Parent is ExpressionStatementSyntax || assign.Parent is ForStatementSyntax;

    /// <summary>True when the (paren-climbed) expression sits inside an
    /// assignment TARGET through tuple/deconstruction nesting.</summary>
    private static bool InAssignmentTargetChain(ExpressionSyntax effective)
    {
        SyntaxNode current = effective;
        while (true)
        {
            switch (current.Parent)
            {
                case ParenthesizedExpressionSyntax parens:
                    current = parens;
                    continue;
                case ArgumentSyntax { Parent: TupleExpressionSyntax tuple }:
                    current = tuple;
                    continue;
                case AssignmentExpressionSyntax assignment when assignment.Left == current:
                    return true;
                default:
                    return false;
            }
        }
    }

    /// <summary>Receiver shapes the accessor-shim call cannot express:
    /// `base.P` (no base argument exists), and value-type receivers that are
    /// not plainly an lvalue (the shim takes self by ref) — including patch-
    /// typed `this` in KEPT members, whose `(T)(object)this` cast boxes a
    /// COPY for structs.</summary>
    private static string? AccessorReceiverGuard(
        ExpressionSyntax reference,
        ShimTarget target,
        SemanticModel model,
        ShimTarget? enclosingAdded,
        string display)
    {
        ExpressionSyntax? receiver = reference switch
        {
            MemberAccessExpressionSyntax memberAccess => memberAccess.Expression,
            ElementAccessExpressionSyntax elementAccess => elementAccess.Expression,
            _ => null,
        };

        if (receiver is BaseExpressionSyntax)
        {
            return "added member accessed through base: " + display +
                " — access it through this or use unity_recompile";
        }
        if (!target.HasSelf || !target.SelfIsValueType || target.SelfIsRefLike)
            return null;

        // By-ref self: the receiver must be a variable.
        if (receiver == null || receiver is ThisExpressionSyntax)
        {
            return enclosingAdded != null
                ? null // `self` is already the by-ref parameter
                : "added struct member referenced through this in a kept member: " + display +
                  " (the patch-typed this cannot be passed by ref to the shim; use unity_recompile)";
        }
        if (receiver is IdentifierNameSyntax identifier &&
            model.GetSymbolInfo(identifier).Symbol is ILocalSymbol or IParameterSymbol or IFieldSymbol)
        {
            return null;
        }
        return "added struct member needs a simple variable receiver: " + display +
            " (the shim takes the receiver by ref) — hoist it into a local or use unity_recompile";
    }

    /// <summary>Compound expansion repeats the receiver and index arguments
    /// (get, then set): only shapes that cannot observe the duplication
    /// pass — this/self, locals, parameters, fields, literals and type
    /// qualifiers (static accessors drop the receiver entirely).</summary>
    private static string? CompoundRepeatableGuard(
        ExpressionSyntax reference,
        ShimTarget target,
        SemanticModel model,
        string display)
    {
        bool Repeatable(ExpressionSyntax? expression)
        {
            switch (expression)
            {
                case null:
                case ThisExpressionSyntax:
                case LiteralExpressionSyntax:
                    return true;
                case IdentifierNameSyntax identifier:
                    return model.GetSymbolInfo(identifier).Symbol
                        is ILocalSymbol or IParameterSymbol or IFieldSymbol or INamedTypeSymbol;
                case MemberAccessExpressionSyntax { Expression: ThisExpressionSyntax } thisMember:
                    return model.GetSymbolInfo(thisMember).Symbol is IFieldSymbol;
                default:
                    return false;
            }
        }

        if (target.HasSelf)
        {
            ExpressionSyntax? receiver = reference switch
            {
                MemberAccessExpressionSyntax memberAccess => memberAccess.Expression,
                ElementAccessExpressionSyntax elementAccess => elementAccess.Expression,
                _ => null,
            };
            if (!Repeatable(receiver))
            {
                return "compound assignment to an added property through a receiver with possible " +
                    "side effects: " + display + " — hoist the receiver into a local or use unity_recompile";
            }
        }
        if (reference is ElementAccessExpressionSyntax element)
        {
            foreach (ArgumentSyntax argument in element.ArgumentList.Arguments)
            {
                if (!argument.RefKindKeyword.IsKind(SyntaxKind.None))
                    return "added indexer takes a by-ref index argument in a compound assignment: " + display +
                        " — use unity_recompile";
                if (!Repeatable(argument.Expression))
                {
                    return "compound assignment to an added indexer with non-trivial index arguments: " +
                        display + " — hoist them into locals or use unity_recompile";
                }
            }
        }
        return null;
    }

    /// <summary>Explicit type arguments for an accessor shim on a generic
    /// declaring chain. Instance accessors infer everything from `self`;
    /// STATIC accessors have no argument to infer from, so the chain's type
    /// arguments materialize explicitly.</summary>
    private static string AccessorTypeArgumentText(ShimTarget target, IMethodSymbol accessor)
    {
        if (!target.GenericShim || target.HasSelf)
            return "";

        var arguments = new List<ITypeSymbol>();
        var chain = new List<INamedTypeSymbol>();
        for (INamedTypeSymbol? current = accessor.ContainingType; current != null; current = current.ContainingType)
            chain.Insert(0, current);
        foreach (INamedTypeSymbol type in chain)
            arguments.AddRange(type.TypeArguments);

        if (arguments.Count != target.TypeParameters.Length)
            return "";
        foreach (ITypeSymbol argument in arguments)
        {
            if (!IsSpeakableType(argument))
                return "";
        }
        return "<" + string.Join(
            ", ",
            arguments.Select(a => a.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat))) + ">";
    }

    /// <summary>Receiver and index arguments of a rewritten reference node:
    /// `obj.P` → (obj, []), `obj[i, j]` → (obj, [i, j]), bare `P` → (null, []).</summary>
    private static ExpressionSyntax? AccessorReceiver(
        ExpressionSyntax rewrittenReference,
        out IReadOnlyList<ArgumentSyntax> indexArguments)
    {
        switch (rewrittenReference)
        {
            case MemberAccessExpressionSyntax memberAccess:
                indexArguments = Array.Empty<ArgumentSyntax>();
                return memberAccess.Expression;
            case ElementAccessExpressionSyntax elementAccess:
                indexArguments = elementAccess.ArgumentList.Arguments;
                return elementAccess.Expression;
            default:
                indexArguments = Array.Empty<ArgumentSyntax>();
                return null;
        }
    }

    /// <summary>The shim call's `self` argument (mirrors
    /// BuildShimInvocation): null receiver / `this` becomes `self` inside
    /// added members or the patch-cast `this` in kept members; explicit
    /// receivers pass through; value types add `ref`. Static targets drop
    /// the (type-qualifier) receiver entirely — callers return null.</summary>
    private static ArgumentSyntax? AccessorSelfArgument(
        ExpressionSyntax? rewrittenReceiver,
        ShimTarget target,
        ShimTarget? enclosingAdded)
    {
        if (!target.HasSelf)
            return null;

        ExpressionSyntax selfExpression;
        if (rewrittenReceiver == null || rewrittenReceiver is ThisExpressionSyntax)
        {
            selfExpression = enclosingAdded != null
                ? SyntaxFactory.IdentifierName("self")
                : SyntaxFactory.ParseExpression("((" + target.DeclaringTypeFqn + ")(object)this)");
        }
        else
        {
            selfExpression = rewrittenReceiver.WithoutTrivia();
        }

        ArgumentSyntax argument = SyntaxFactory.Argument(selfExpression);
        if (target.SelfIsValueType && !target.SelfIsRefLike)
            argument = argument.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.RefKeyword));
        return argument;
    }

    private static InvocationExpressionSyntax AccessorShimInvocation(
        ShimTarget target,
        string typeArgumentText,
        List<ArgumentSyntax> arguments)
    {
        return SyntaxFactory.InvocationExpression(
            SyntaxFactory.ParseExpression(target.ShimTypeFqn + "." + target.MethodName + typeArgumentText),
            SyntaxFactory.ArgumentList(SyntaxFactory.SeparatedList(arguments)));
    }

    /// <summary>`obj.P` / `P` / `obj[i]` → `Shims.get_X(obj, i…)`.</summary>
    private static SyntaxNode BuildAccessorGetCall(
        SyntaxNode rewrittenReference,
        ShimTarget getTarget,
        ShimTarget? enclosingAdded,
        string typeArgumentText)
    {
        ExpressionSyntax? receiver = AccessorReceiver(
            (ExpressionSyntax)rewrittenReference, out IReadOnlyList<ArgumentSyntax> indexArguments);
        var arguments = new List<ArgumentSyntax>();
        ArgumentSyntax? self = AccessorSelfArgument(receiver, getTarget, enclosingAdded);
        if (self != null)
            arguments.Add(self);
        arguments.AddRange(indexArguments);
        return AccessorShimInvocation(getTarget, typeArgumentText, arguments)
            .WithTriviaFrom(rewrittenReference);
    }

    /// <summary>`obj.P = v` → `Shims.set_P(obj, v)`; `obj[i] = v` →
    /// `Shims.set_Item(obj, i, v)`; `obj.E += h` → `Shims.add_E(obj, h)`.
    /// Statement positions only (registration gated).</summary>
    private static SyntaxNode BuildAccessorAssignmentCall(
        AssignmentExpressionSyntax rewrittenAssign,
        ShimTarget accessorTarget,
        ShimTarget? enclosingAdded,
        string typeArgumentText)
    {
        ExpressionSyntax left = UnwrapParens(rewrittenAssign.Left);
        ExpressionSyntax? receiver = AccessorReceiver(left, out IReadOnlyList<ArgumentSyntax> indexArguments);
        var arguments = new List<ArgumentSyntax>();
        ArgumentSyntax? self = AccessorSelfArgument(receiver, accessorTarget, enclosingAdded);
        if (self != null)
            arguments.Add(self);
        arguments.AddRange(indexArguments);
        arguments.Add(SyntaxFactory.Argument(rewrittenAssign.Right.WithoutTrivia()));
        return AccessorShimInvocation(accessorTarget, typeArgumentText, arguments)
            .WithTriviaFrom(rewrittenAssign);
    }

    /// <summary>`obj.P op= v` → `Shims.set_P(obj, (T)(Shims.get_P(obj) op
    /// (v)))` — the cast reproduces the compound assignment's implicit
    /// narrowing (byte/short/char/enum stay compilable). Receiver and index
    /// arguments repeat; the registration gated them to effect-free shapes.</summary>
    private static SyntaxNode BuildAccessorCompoundCall(
        AssignmentExpressionSyntax rewrittenAssign,
        ShimTarget getTarget,
        ShimTarget setTarget,
        SyntaxKind binaryKind,
        string valueTypeCastFqn,
        ShimTarget? enclosingAdded,
        string typeArgumentText)
    {
        ExpressionSyntax left = UnwrapParens(rewrittenAssign.Left);
        ExpressionSyntax? receiver = AccessorReceiver(left, out IReadOnlyList<ArgumentSyntax> indexArguments);

        var getArguments = new List<ArgumentSyntax>();
        ArgumentSyntax? getSelf = AccessorSelfArgument(receiver, getTarget, enclosingAdded);
        if (getSelf != null)
            getArguments.Add(getSelf);
        getArguments.AddRange(indexArguments);
        InvocationExpressionSyntax getCall = AccessorShimInvocation(getTarget, typeArgumentText, getArguments);

        ExpressionSyntax computed = SyntaxFactory.CastExpression(
            SyntaxFactory.ParseTypeName(valueTypeCastFqn),
            SyntaxFactory.ParenthesizedExpression(SyntaxFactory.BinaryExpression(
                binaryKind,
                getCall,
                SyntaxFactory.ParenthesizedExpression(rewrittenAssign.Right.WithoutTrivia()))));

        var setArguments = new List<ArgumentSyntax>();
        ArgumentSyntax? setSelf = AccessorSelfArgument(receiver, setTarget, enclosingAdded);
        if (setSelf != null)
            setArguments.Add(setSelf);
        setArguments.AddRange(indexArguments);
        setArguments.Add(SyntaxFactory.Argument(computed));
        return AccessorShimInvocation(setTarget, typeArgumentText, setArguments)
            .WithTriviaFrom(rewrittenAssign);
    }

    // ── B1: kept callers of re-added members ─────────────────────────

    /// <summary>Walk every reference to a RE-ADDED member (same-signature
    /// remove+add, i.e. a generic body change) and make sure the enclosing
    /// member's patch copy will actually run: members already in the detour
    /// set pass, added members pass (they ARE the shims), kept detourable
    /// members join <paramref name="ensured"/>, and anything that cannot be
    /// re-detoured returns a cold reason naming the exact site. Static
    /// initializers/cctors are skipped: they never re-run by design (M5
    /// frozen-value semantics).</summary>
    private static string? EnsureReAddedCallerDetours(
        CompilationUnitSyntax root,
        SemanticModel model,
        PatchBatchContext batch,
        HotDiffFileResult diff,
        List<(SyntaxNode Node, ShimTarget Target)> addedBodySpans,
        HashSet<string> newTypeNames,
        List<HotDiffMethod> ensured)
    {
        foreach (SyntaxNode node in root.DescendantNodes())
        {
            SymbolInfo info;
            switch (node)
            {
                case InvocationExpressionSyntax invocation:
                    info = model.GetSymbolInfo(invocation);
                    break;
                case IdentifierNameSyntax or MemberAccessExpressionSyntax:
                {
                    // Method groups (delegate conversions): same shape the
                    // lambda rewrite pass matches.
                    if (node.Parent is InvocationExpressionSyntax parentInvocation && parentInvocation.Expression == node)
                        continue;
                    if (node.Parent is MemberAccessExpressionSyntax outerAccess && outerAccess.Name == node)
                        continue;
                    info = model.GetSymbolInfo(node);
                    break;
                }
                default:
                    continue;
            }

            if (!TryResolveAddedTarget(info, batch, out ShimTarget? target, out _, out _))
                continue;
            if (!batch.ReAddedMemberKeys.Contains(target.MemberKey))
                continue;
            // nameof(...) materializes as a constant — nothing to redirect.
            if (FindEnclosingNameOf(node, model) != null)
                continue;
            if (addedBodySpans.Any(span => span.Node.FullSpan.Contains(node.Span)))
                continue; // inside an added member: the shim body itself

            string? cold = EnsureSiteDetour(node, target, diff, newTypeNames, ensured);
            if (cold != null)
                return cold;
        }
        return null;
    }

    /// <summary>Classify one re-added-member reference site's enclosing
    /// member and append the detour it needs (null = fine / handled);
    /// returns a cold reason when the enclosure cannot be re-detoured.</summary>
    private static string? EnsureSiteDetour(
        SyntaxNode node,
        ShimTarget target,
        HotDiffFileResult diff,
        HashSet<string> newTypeNames,
        List<HotDiffMethod> ensured)
    {
        MemberDeclarationSyntax? member = null;
        TypeDeclarationSyntax? hostType = null;
        for (SyntaxNode? current = node.Parent; current != null; current = current.Parent)
        {
            if (current is MemberDeclarationSyntax candidate && current.Parent is TypeDeclarationSyntax owner)
            {
                member = candidate;
                hostType = owner;
                break;
            }
        }
        if (member == null || hostType == null)
            return null; // not executable code (attribute argument etc.)

        string hostMetadataName = HotDiff.MetadataName(hostType);
        if (newTypeNames.Contains(hostMetadataName))
            return null; // brand-new types live wholly in the patch assembly

        string reAdded = target.DeclaringTypeMetadataName + "." + target.MethodName;
        string Cold(string memberDisplay, string why) =>
            "a changed generic body is still reachable through " + hostMetadataName + "." + memberDisplay +
            ", which " + why + ": it calls " + reAdded +
            " — edit that member's body in the same batch or use unity_recompile";

        bool genericHost = false;
        for (SyntaxNode? current = hostType; current != null; current = current.Parent)
        {
            if (current is TypeDeclarationSyntax outer && (outer.TypeParameterList?.Parameters.Count ?? 0) > 0)
            {
                genericHost = true;
                break;
            }
        }

        void Add(string name, string[] paramNames, bool isStatic, bool isCtor)
        {
            bool exists =
                diff.ChangedMethods.Any(m => !m.Added &&
                    m.DeclaringType == hostMetadataName && m.Name == name && m.IsStatic == isStatic &&
                    m.IsCtor == isCtor && m.ParamTypeNames.SequenceEqual(paramNames, StringComparer.Ordinal)) ||
                ensured.Any(m =>
                    m.DeclaringType == hostMetadataName && m.Name == name && m.IsStatic == isStatic &&
                    m.IsCtor == isCtor && m.ParamTypeNames.SequenceEqual(paramNames, StringComparer.Ordinal));
            if (!exists)
            {
                ensured.Add(new HotDiffMethod
                {
                    DeclaringType = hostMetadataName,
                    Name = name,
                    ParamTypeNames = paramNames,
                    IsStatic = isStatic,
                    IsCtor = isCtor,
                });
            }
        }

        // Instance field/auto-property initializers compile into every
        // non-chained constructor; static ones already ran and stay frozen.
        string? AddInstanceInitializerCtors()
        {
            if (genericHost)
                return Cold("(initializer)", "is a generic type initializer that cannot be re-detoured");
            var ctors = hostType.Members.OfType<ConstructorDeclarationSyntax>()
                .Where(c => !c.Modifiers.Any(SyntaxKind.StaticKeyword))
                .ToList();
            if (ctors.Count == 0)
            {
                Add(".ctor", Array.Empty<string>(), isStatic: false, isCtor: true);
            }
            else
            {
                foreach (ConstructorDeclarationSyntax ctor in ctors)
                    Add(".ctor", HotDiff.ParamTypeNames(ctor.ParameterList), isStatic: false, isCtor: true);
            }
            return null;
        }

        switch (member)
        {
            case MethodDeclarationSyntax method:
            {
                if (genericHost || (method.TypeParameterList?.Parameters.Count ?? 0) > 0)
                    return Cold(method.Identifier.Text, "is generic and cannot be re-detoured");
                if (HotDiff.HasBurstCompileAttribute(method))
                    return Cold(method.Identifier.Text, "is Burst-compiled");
                if (method.ExplicitInterfaceSpecifier != null)
                    return Cold(method.Identifier.Text, "is an explicit interface implementation");
                Add(method.Identifier.Text, HotDiff.ParamTypeNames(method.ParameterList),
                    method.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false);
                return null;
            }

            case ConstructorDeclarationSyntax ctor:
            {
                if (ctor.Modifiers.Any(SyntaxKind.StaticKeyword))
                    return null; // emptied in the copy; ran already (frozen)
                if (genericHost)
                    return Cold(".ctor", "is a generic type constructor that cannot be re-detoured");
                Add(".ctor", HotDiff.ParamTypeNames(ctor.ParameterList), isStatic: false, isCtor: true);
                return null;
            }

            case DestructorDeclarationSyntax:
                return Cold("~dtor", "is a finalizer that never detours");

            case OperatorDeclarationSyntax op:
            {
                string? opName = HotDiff.OperatorMetadataName(op.OperatorToken.Text, op.ParameterList.Parameters.Count);
                bool changed = opName != null && diff.ChangedMethods.Any(m => !m.Added &&
                    m.DeclaringType == hostMetadataName && m.Name == opName &&
                    m.ParamTypeNames.SequenceEqual(HotDiff.ParamTypeNames(op.ParameterList), StringComparer.Ordinal));
                return changed
                    ? null // stays in the copy and detours
                    : Cold("operator" + op.OperatorToken.Text, "is an unchanged operator (stripped from the patch copy)");
            }

            case ConversionOperatorDeclarationSyntax conv:
            {
                string convName = conv.ImplicitOrExplicitKeyword.IsKind(SyntaxKind.ImplicitKeyword)
                    ? "op_Implicit"
                    : "op_Explicit";
                bool changed = diff.ChangedMethods.Any(m => !m.Added &&
                    m.DeclaringType == hostMetadataName && m.Name == convName &&
                    m.ParamTypeNames.SequenceEqual(HotDiff.ParamTypeNames(conv.ParameterList), StringComparer.Ordinal));
                return changed
                    ? null
                    : Cold("conversion", "is an unchanged conversion (stripped from the patch copy)");
            }

            case PropertyDeclarationSyntax property:
            {
                if (property.Initializer != null && property.Initializer.FullSpan.Contains(node.Span))
                {
                    return property.Modifiers.Any(SyntaxKind.StaticKeyword)
                        ? null // static initializer: frozen by design
                        : AddInstanceInitializerCtors();
                }
                if (genericHost)
                    return Cold(property.Identifier.Text, "is a generic type property that cannot be re-detoured");
                if (property.ExplicitInterfaceSpecifier != null)
                    return Cold(property.Identifier.Text, "is an explicit interface implementation");
                bool isStatic = property.Modifiers.Any(SyntaxKind.StaticKeyword);
                if (property.ExpressionBody != null && property.ExpressionBody.FullSpan.Contains(node.Span))
                {
                    Add("get_" + property.Identifier.Text, Array.Empty<string>(), isStatic, isCtor: false);
                    return null;
                }
                foreach (AccessorDeclarationSyntax accessor in property.AccessorList?.Accessors ?? default)
                {
                    if (!accessor.FullSpan.Contains(node.Span))
                        continue;
                    Add(HotDiff.AccessorName(accessor, property.Identifier.Text),
                        HotDiff.AccessorParams(accessor, null, HotDiff.TokenText(property.Type)),
                        isStatic, isCtor: false);
                    return null;
                }
                return null;
            }

            case IndexerDeclarationSyntax indexer:
            {
                if (genericHost)
                    return Cold("this[]", "is a generic type indexer that cannot be re-detoured");
                if (indexer.ExplicitInterfaceSpecifier != null)
                    return Cold("this[]", "is an explicit interface implementation");
                if (indexer.ExpressionBody != null && indexer.ExpressionBody.FullSpan.Contains(node.Span))
                {
                    Add("get_Item", HotDiff.ParamTypeNames(indexer.ParameterList), isStatic: false, isCtor: false);
                    return null;
                }
                foreach (AccessorDeclarationSyntax accessor in indexer.AccessorList?.Accessors ?? default)
                {
                    if (!accessor.FullSpan.Contains(node.Span))
                        continue;
                    Add(HotDiff.AccessorName(accessor, "Item"),
                        HotDiff.AccessorParams(accessor, indexer.ParameterList, HotDiff.TokenText(indexer.Type)),
                        isStatic: false, isCtor: false);
                    return null;
                }
                return null;
            }

            case EventDeclarationSyntax @event:
            {
                if (genericHost)
                    return Cold(@event.Identifier.Text, "is a generic type event that cannot be re-detoured");
                if (@event.ExplicitInterfaceSpecifier != null)
                    return Cold(@event.Identifier.Text, "is an explicit interface event");
                foreach (AccessorDeclarationSyntax accessor in @event.AccessorList?.Accessors ?? default)
                {
                    if (!accessor.FullSpan.Contains(node.Span))
                        continue;
                    string prefix = accessor.Keyword.Text == "add" ? "add_" : "remove_";
                    Add(prefix + @event.Identifier.Text, new[] { HotDiff.SimpleTypeName(@event.Type) },
                        @event.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false);
                    return null;
                }
                return null;
            }

            case FieldDeclarationSyntax field:
                return field.Modifiers.Any(SyntaxKind.StaticKeyword) || field.Modifiers.Any(SyntaxKind.ConstKeyword)
                    ? null // static initializer: frozen by design
                    : AddInstanceInitializerCtors();

            case EventFieldDeclarationSyntax eventField:
                return eventField.Modifiers.Any(SyntaxKind.StaticKeyword)
                    ? null
                    : AddInstanceInitializerCtors();

            default:
                return null;
        }
    }

    // ── shim construction ────────────────────────────────────────────

    /// <summary>Build the static shim method from the (already rewritten)
    /// added-member declaration: `static R M(this global::Ns.Foo self, ...)`.</summary>
    private static MethodDeclarationSyntax BuildShimMethod(
        MethodDeclarationSyntax decl,
        ShimTarget target,
        CompilationUnitSyntax originalRoot)
    {
        var modifiers = new List<SyntaxToken>
        {
            SyntaxFactory.Token(SyntaxKind.PublicKeyword),
            SyntaxFactory.Token(SyntaxKind.StaticKeyword),
        };
        foreach (SyntaxToken modifier in decl.Modifiers)
        {
            if (modifier.IsKind(SyntaxKind.AsyncKeyword) || modifier.IsKind(SyntaxKind.UnsafeKeyword))
                modifiers.Add(SyntaxFactory.Token(modifier.Kind()));
        }

        var parameters = new List<ParameterSyntax>();
        if (target.HasSelf)
            parameters.Add(BuildSelfParameter(target));
        parameters.AddRange(decl.ParameterList.Parameters);

        MethodDeclarationSyntax shim = SyntaxFactory.MethodDeclaration(decl.ReturnType.WithLeadingTrivia(SyntaxFactory.Space), decl.Identifier.Text)
            .WithAttributeLists(decl.AttributeLists)
            .WithModifiers(SyntaxFactory.TokenList(modifiers))
            .WithParameterList(SyntaxFactory.ParameterList(SyntaxFactory.SeparatedList(parameters)))
            .WithBody(decl.Body)
            .WithExpressionBody(decl.ExpressionBody)
            .WithSemicolonToken(decl.ExpressionBody != null
                ? SyntaxFactory.Token(SyntaxKind.SemicolonToken)
                : default);

        shim = WithShimTypeParameters(shim, target, originalRoot, decl.ConstraintClauses);

        return shim.NormalizeWhitespace().WithTrailingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);
    }

    /// <summary>The shim's leading `self` parameter (extension receiver;
    /// by-ref for value types, plain by-value for ref structs).</summary>
    private static ParameterSyntax BuildSelfParameter(ShimTarget target)
    {
        var selfModifiers = new List<SyntaxToken>();
        if (target.SelfIsRefLike)
        {
            // ref-struct receivers cannot be extension receivers in this
            // language surface: plain by-value static shim.
        }
        else if (target.SelfIsValueType)
        {
            selfModifiers.Add(SyntaxFactory.Token(SyntaxKind.RefKeyword));
            selfModifiers.Add(SyntaxFactory.Token(SyntaxKind.ThisKeyword));
        }
        else
        {
            selfModifiers.Add(SyntaxFactory.Token(SyntaxKind.ThisKeyword));
        }

        return SyntaxFactory.Parameter(SyntaxFactory.Identifier("self"))
            .WithModifiers(SyntaxFactory.TokenList(selfModifiers))
            .WithType(SyntaxFactory.ParseTypeName(target.DeclaringTypeFqn).WithTrailingTrivia(SyntaxFactory.Space));
    }

    /// <summary>Apply the flattened type-parameter list (declaring chain
    /// + method's own, B1) and carry the chain's constraint clauses so
    /// `A&lt;T&gt; self` stays well-formed (e.g. `where T : class`).</summary>
    private static MethodDeclarationSyntax WithShimTypeParameters(
        MethodDeclarationSyntax shim,
        ShimTarget target,
        CompilationUnitSyntax originalRoot,
        SyntaxList<TypeParameterConstraintClauseSyntax> ownConstraints)
    {
        if (!target.GenericShim || target.TypeParameters.Length == 0)
            return shim;

        shim = shim.WithTypeParameterList(SyntaxFactory.TypeParameterList(
            SyntaxFactory.SeparatedList(target.TypeParameters.Select(SyntaxFactory.TypeParameter))));

        var constraints = new List<TypeParameterConstraintClauseSyntax>();
        foreach (TypeDeclarationSyntax typeDecl in originalRoot.DescendantNodes().OfType<TypeDeclarationSyntax>())
        {
            if (HotDiff.MetadataName(typeDecl) != target.DeclaringTypeMetadataName &&
                !target.DeclaringTypeMetadataName.StartsWith(HotDiff.MetadataName(typeDecl) + "+", StringComparison.Ordinal))
            {
                continue;
            }
            constraints.AddRange(typeDecl.ConstraintClauses);
        }
        // The member's OWN constraints (B1) come from the already
        // rewritten declaration, so type references are requalified.
        constraints.AddRange(ownConstraints);
        if (constraints.Count > 0)
            shim = shim.WithConstraintClauses(SyntaxFactory.List(constraints));
        return shim;
    }

    /// <summary>B2: build one accessor shim from the (already rewritten)
    /// added property/indexer/event declaration:
    ///   get_P → `static T get_P(self)`        (body = accessor body)
    ///   set_P → `static void set_P(self, T value)`
    ///   get_Item/set_Item carry the indexer's parameter list;
    ///   add_E/remove_E take the handler as `value`.
    /// Auto-property accessors synthesize a store read/write body.</summary>
    private static MethodDeclarationSyntax BuildAccessorShimMethod(
        BasePropertyDeclarationSyntax rewrittenContainer,
        AddedAccessorInfo info,
        CompilationUnitSyntax originalRoot)
    {
        ShimTarget target = info.Target;
        bool isGet = target.MethodName.StartsWith("get_", StringComparison.Ordinal);

        var modifiers = new List<SyntaxToken>
        {
            SyntaxFactory.Token(SyntaxKind.PublicKeyword),
            SyntaxFactory.Token(SyntaxKind.StaticKeyword),
        };
        foreach (SyntaxToken modifier in rewrittenContainer.Modifiers)
        {
            if (modifier.IsKind(SyntaxKind.UnsafeKeyword))
                modifiers.Add(SyntaxFactory.Token(SyntaxKind.UnsafeKeyword));
        }

        TypeSyntax returnType = isGet
            ? rewrittenContainer.Type.WithoutTrivia()
            : SyntaxFactory.PredefinedType(SyntaxFactory.Token(SyntaxKind.VoidKeyword));

        var parameters = new List<ParameterSyntax>();
        if (target.HasSelf)
            parameters.Add(BuildSelfParameter(target));
        if (rewrittenContainer is IndexerDeclarationSyntax rewrittenIndexer)
            parameters.AddRange(rewrittenIndexer.ParameterList.Parameters);
        if (!isGet)
        {
            // set_/add_/remove_: the value parameter comes last (`value` is
            // a legal parameter name — accessor bodies keep compiling).
            parameters.Add(SyntaxFactory.Parameter(SyntaxFactory.Identifier("value"))
                .WithType(rewrittenContainer.Type.WithoutTrivia().WithTrailingTrivia(SyntaxFactory.Space)));
        }

        MethodDeclarationSyntax shim = SyntaxFactory
            .MethodDeclaration(returnType.WithLeadingTrivia(SyntaxFactory.Space), target.MethodName)
            .WithModifiers(SyntaxFactory.TokenList(modifiers))
            .WithParameterList(SyntaxFactory.ParameterList(SyntaxFactory.SeparatedList(parameters)));

        // Locate the rewritten body (the original accessor's KEYWORD finds
        // its rewritten counterpart — replace passes keep accessor order).
        AccessorDeclarationSyntax? accessor = null;
        ArrowExpressionClauseSyntax? arrow = null;
        if (info.Accessor != null)
        {
            string keyword = info.Accessor.Keyword.Text;
            accessor = (rewrittenContainer.AccessorList?.Accessors ?? default)
                .FirstOrDefault(a => a.Keyword.Text == keyword);
        }
        else
        {
            arrow = rewrittenContainer switch
            {
                PropertyDeclarationSyntax p => p.ExpressionBody,
                IndexerDeclarationSyntax i => i.ExpressionBody,
                _ => null,
            };
        }

        if (info.IsAuto && info.Store != null)
        {
            string access = info.Store.IsStatic
                ? info.Store.StoreFqn + "." + info.Store.Name
                : info.Store.StoreFqn + "." + info.Store.Name + ".Ref(self)";
            ExpressionSyntax body = SyntaxFactory.ParseExpression(isGet ? access : access + " = value");
            shim = shim
                .WithExpressionBody(SyntaxFactory.ArrowExpressionClause(body))
                .WithSemicolonToken(SyntaxFactory.Token(SyntaxKind.SemicolonToken));
        }
        else if (arrow != null)
        {
            shim = shim
                .WithExpressionBody(arrow)
                .WithSemicolonToken(SyntaxFactory.Token(SyntaxKind.SemicolonToken));
        }
        else if (accessor?.Body != null)
        {
            shim = shim.WithBody(accessor.Body);
        }
        else if (accessor?.ExpressionBody != null)
        {
            shim = shim
                .WithExpressionBody(accessor.ExpressionBody)
                .WithSemicolonToken(SyntaxFactory.Token(SyntaxKind.SemicolonToken));
        }
        else
        {
            // Unreachable (HotDiff fails body-less non-auto accessors
            // closed); an empty body beats a malformed declaration.
            shim = shim.WithBody(SyntaxFactory.Block());
        }

        shim = WithShimTypeParameters(shim, target, originalRoot, default);

        return shim.NormalizeWhitespace().WithTrailingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);
    }

    /// <summary>Append the shim class inside namespace `ns` of the rewritten
    /// unit (reusing an existing namespace declaration when present, so
    /// file-scoped namespaces stay legal), or at the top level for the
    /// global namespace. Namespace-scoped usings of the original file apply
    /// unchanged because the class joins the same declaration.</summary>
    private static CompilationUnitSyntax AppendTypeToNamespace(
        CompilationUnitSyntax unit,
        MemberDeclarationSyntax type,
        string ns)
    {
        if (string.IsNullOrEmpty(ns))
            return unit.AddMembers(type);

        BaseNamespaceDeclarationSyntax? existing = unit.DescendantNodes()
            .OfType<BaseNamespaceDeclarationSyntax>()
            .FirstOrDefault(n => n.Name.ToString() == ns);
        if (existing != null)
            return unit.ReplaceNode(existing, existing.AddMembers(type));

        NamespaceDeclarationSyntax block = SyntaxFactory
            .NamespaceDeclaration(SyntaxFactory.ParseName(ns))
            .AddMembers(type)
            .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);
        return unit.AddMembers(block);
    }

    /// <summary>The shim target for a method symbol, with extension-REDUCED
    /// forms normalized back to their static declaration (the AddedMembers
    /// key): `5.Tripled()` binds the reduced symbol whose OriginalDefinition
    /// is NOT the declared method.</summary>
    private static bool TryGetAddedTarget(
        IMethodSymbol method,
        PatchBatchContext batch,
        out ShimTarget target,
        out IMethodSymbol normalized)
    {
        normalized = method.ReducedFrom != null
            ? method.GetConstructedReducedFrom() ?? method.ReducedFrom
            : method;
        return batch.AddedMembers.TryGetValue(normalized.OriginalDefinition, out target!);
    }

    /// <summary>Resolve a reference to an ADDED member from a SymbolInfo,
    /// looking through CandidateSymbols too: once an earlier patch's image
    /// is referenced, an extension-syntax call to a re-emitted added
    /// extension method binds AMBIGUOUSLY between the batch source and the
    /// image's identical shim — the source one is the live surface and the
    /// rewrite to a direct call removes the ambiguity from the patch text.</summary>
    private static bool TryResolveAddedTarget(
        SymbolInfo info,
        PatchBatchContext batch,
        out ShimTarget target,
        out IMethodSymbol normalized,
        out bool reduced)
    {
        if (info.Symbol is IMethodSymbol direct && TryGetAddedTarget(direct, batch, out target, out normalized))
        {
            reduced = direct.ReducedFrom != null;
            return true;
        }
        foreach (ISymbol candidate in info.CandidateSymbols)
        {
            if (candidate is IMethodSymbol method && TryGetAddedTarget(method, batch, out target, out normalized))
            {
                reduced = method.ReducedFrom != null;
                return true;
            }
        }
        target = null!;
        normalized = null!;
        reduced = false;
        return false;
    }

    private static bool TryResolvePatchedMethodTarget(
        SymbolInfo info,
        PatchBatchContext batch,
        out PatchedMethodTarget target)
    {
        if (info.Symbol is IMethodSymbol direct &&
            batch.PatchedMethods.TryGetValue(direct.OriginalDefinition, out target!))
        {
            return true;
        }
        foreach (ISymbol candidate in info.CandidateSymbols)
        {
            if (candidate is IMethodSymbol method &&
                batch.PatchedMethods.TryGetValue(method.OriginalDefinition, out target!))
            {
                return true;
            }
        }
        target = null!;
        return false;
    }

    /// <summary>Explicit type arguments for a shim call whose target carries
    /// METHOD type parameters (B1): the original call's explicit/inferred
    /// arguments cannot be partially re-applied to the flattened
    /// chain+method parameter list, so the full list materializes — unless
    /// any argument is unspeakable (anonymous types only ever arrive via
    /// inference, which the shim call then relies on too). Chain-only
    /// generic shims keep relying on inference from `self`.</summary>
    private static string ShimTypeArgumentText(ShimTarget target, IMethodSymbol invoked)
    {
        if (target.MethodTypeParameterCount == 0)
            return "";

        var arguments = new List<ITypeSymbol>();
        var chain = new List<INamedTypeSymbol>();
        for (INamedTypeSymbol? current = invoked.ContainingType; current != null; current = current.ContainingType)
            chain.Insert(0, current);
        foreach (INamedTypeSymbol type in chain)
            arguments.AddRange(type.TypeArguments);
        arguments.AddRange(invoked.TypeArguments);

        if (arguments.Count != target.TypeParameters.Length)
            return "";
        foreach (ITypeSymbol argument in arguments)
        {
            if (!IsSpeakableType(argument))
                return "";
        }
        return "<" + string.Join(
            ", ",
            arguments.Select(a => a.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat))) + ">";
    }

    private static bool IsSpeakableType(ITypeSymbol type)
    {
        switch (type)
        {
            case IErrorTypeSymbol:
                return false;
            case ITypeParameterSymbol:
                return true;
            case IArrayTypeSymbol array:
                return IsSpeakableType(array.ElementType);
            case IPointerTypeSymbol pointer:
                return IsSpeakableType(pointer.PointedAtType);
            case INamedTypeSymbol named:
            {
                if (named.IsAnonymousType)
                    return false;
                foreach (ITypeSymbol argument in named.TypeArguments)
                {
                    if (!IsSpeakableType(argument))
                        return false;
                }
                return true;
            }
            default:
                return type.TypeKind != TypeKind.Error;
        }
    }

    /// <summary>`expr.M(args)` / `M(args)` → `global::Ns.Foo__LocusShims.M(self, args)`.
    /// For an added EXTENSION method called in reduced form (`expr.M(args)`
    /// where M is `static R M(this T t, ...)`), `reducedReceiverCastFqn`
    /// (the `this`-parameter type) folds the receiver back into the first
    /// argument of the static shim — a `this` receiver inside a kept member
    /// is patch-typed and needs the cast to the parameter's original type.</summary>
    private static SyntaxNode BuildShimInvocation(
        InvocationExpressionSyntax rewrittenInvocation,
        ShimTarget target,
        ShimTarget? enclosingAdded,
        string typeArgumentText,
        string? reducedReceiverCastFqn = null)
    {
        ExpressionSyntax? receiver = rewrittenInvocation.Expression switch
        {
            MemberAccessExpressionSyntax memberAccess => memberAccess.Expression,
            _ => null,
        };

        var arguments = new List<ArgumentSyntax>();
        if (target.HasSelf || reducedReceiverCastFqn != null)
        {
            ExpressionSyntax selfExpression;
            if (receiver == null || receiver is ThisExpressionSyntax)
            {
                string castFqn = reducedReceiverCastFqn ?? target.DeclaringTypeFqn;
                selfExpression = enclosingAdded != null
                    ? SyntaxFactory.IdentifierName("self")
                    : SyntaxFactory.ParseExpression("((" + castFqn + ")(object)this)");
            }
            else
            {
                selfExpression = receiver.WithoutTrivia();
            }

            ArgumentSyntax selfArgument = SyntaxFactory.Argument(selfExpression);
            if (target.HasSelf && target.SelfIsValueType && !target.SelfIsRefLike)
                selfArgument = selfArgument.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.RefKeyword));
            arguments.Add(selfArgument);
        }
        arguments.AddRange(rewrittenInvocation.ArgumentList.Arguments);

        return SyntaxFactory.InvocationExpression(
                SyntaxFactory.ParseExpression(target.ShimTypeFqn + "." + target.MethodName + typeArgumentText),
                SyntaxFactory.ArgumentList(SyntaxFactory.SeparatedList(arguments)))
            .WithTriviaFrom(rewrittenInvocation);
    }

    private static SyntaxNode BuildPatchedMethodInvocation(
        InvocationExpressionSyntax rewrittenInvocation,
        PatchedMethodTarget target)
    {
        return SyntaxFactory.InvocationExpression(
                SyntaxFactory.ParseExpression(target.PatchTypeFqn + "." + target.MethodName),
                rewrittenInvocation.ArgumentList)
            .WithTriviaFrom(rewrittenInvocation);
    }

    /// <summary>Receiver shapes the instance self-shim redirect can express:
    /// an explicit or `this`/implicit receiver. `base.M` (no base argument) and
    /// `obj?.M()` (null-propagation) are not expressible — the original call is
    /// kept (still hot via the normal detour; converges at recompile).</summary>
    private static bool PatchedInstanceReceiverExpressible(ExpressionSyntax invocationExpression) =>
        invocationExpression switch
        {
            MemberAccessExpressionSyntax ma => ma.Expression is not BaseExpressionSyntax,
            MemberBindingExpressionSyntax => false,
            SimpleNameSyntax => true,
            _ => false,
        };

    /// <summary>`recv.M(args)` / `M(args)` → `Foo__LocusShims.&lt;shim&gt;(self,
    /// args)`, where the inlined callee's CHANGED body lives in the self-shim.
    /// `self` mirrors AccessorSelfArgument: an explicit receiver passes through;
    /// `this`/implicit becomes `self` inside another shim or `((Foo)(object)this)`
    /// in a kept body; value-type receivers go by `ref`.</summary>
    private static SyntaxNode BuildPatchedInstanceInvocation(
        InvocationExpressionSyntax rewrittenInvocation,
        PatchedMethodTarget target,
        ShimTarget? enclosingAdded)
    {
        ExpressionSyntax? rewrittenReceiver = rewrittenInvocation.Expression is MemberAccessExpressionSyntax memberAccess
            ? memberAccess.Expression
            : null;

        ExpressionSyntax selfExpression;
        if (rewrittenReceiver == null || rewrittenReceiver is ThisExpressionSyntax)
        {
            selfExpression = enclosingAdded != null
                ? SyntaxFactory.IdentifierName("self")
                : SyntaxFactory.ParseExpression("((" + target.DeclaringTypeFqn + ")(object)this)");
        }
        else
        {
            selfExpression = rewrittenReceiver.WithoutTrivia();
        }

        ArgumentSyntax selfArgument = SyntaxFactory.Argument(selfExpression);
        if (target.SelfIsValueType && !target.SelfIsRefLike)
            selfArgument = selfArgument.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.RefKeyword));

        var arguments = new List<ArgumentSyntax> { selfArgument };
        arguments.AddRange(rewrittenInvocation.ArgumentList.Arguments);

        return SyntaxFactory.InvocationExpression(
                SyntaxFactory.ParseExpression(target.ShimTypeFqn + "." + target.MethodName),
                SyntaxFactory.ArgumentList(SyntaxFactory.SeparatedList(arguments)))
            .WithTriviaFrom(rewrittenInvocation);
    }

    /// <summary>Method group `foo.M` → `(a0, ...) => Shims.M(foo, a0, ...)`.
    /// The receiver evaluates at INVOCATION time instead of delegate
    /// creation, and delegate equality differs — both documented. A reduced
    /// extension group folds its receiver into the first call argument
    /// (`reducedReceiverCastFqn` = the `this`-parameter type, see
    /// BuildShimInvocation).</summary>
    private static SyntaxNode BuildShimLambda(
        SyntaxNode rewrittenGroup,
        ShimTarget target,
        IMethodSymbol invoke,
        ShimTarget? enclosingAdded,
        string typeArgumentText,
        string? reducedReceiverCastFqn = null)
    {
        ExpressionSyntax? receiver = rewrittenGroup switch
        {
            MemberAccessExpressionSyntax memberAccess => memberAccess.Expression,
            _ => null,
        };

        var lambdaParams = new List<ParameterSyntax>();
        var callArgs = new List<ArgumentSyntax>();

        if (target.HasSelf || reducedReceiverCastFqn != null)
        {
            ExpressionSyntax selfExpression;
            if (receiver == null || receiver is ThisExpressionSyntax)
            {
                string castFqn = reducedReceiverCastFqn ?? target.DeclaringTypeFqn;
                selfExpression = enclosingAdded != null
                    ? SyntaxFactory.IdentifierName("self")
                    : SyntaxFactory.ParseExpression("((" + castFqn + ")(object)this)");
            }
            else
            {
                selfExpression = receiver.WithoutTrivia();
            }
            ArgumentSyntax selfArgument = SyntaxFactory.Argument(selfExpression);
            if (target.HasSelf && target.SelfIsValueType && !target.SelfIsRefLike)
                selfArgument = selfArgument.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.RefKeyword));
            callArgs.Add(selfArgument);
        }

        for (int i = 0; i < invoke.Parameters.Length; i++)
        {
            IParameterSymbol parameter = invoke.Parameters[i];
            string name = "__a" + i;
            ParameterSyntax lambdaParam = SyntaxFactory.Parameter(SyntaxFactory.Identifier(name))
                .WithType(SyntaxFactory.ParseTypeName(
                        parameter.Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat))
                    .WithTrailingTrivia(SyntaxFactory.Space));
            ArgumentSyntax callArg = SyntaxFactory.Argument(SyntaxFactory.IdentifierName(name));
            switch (parameter.RefKind)
            {
                case RefKind.Ref:
                    lambdaParam = lambdaParam.AddModifiers(SyntaxFactory.Token(SyntaxKind.RefKeyword));
                    callArg = callArg.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.RefKeyword));
                    break;
                case RefKind.Out:
                    lambdaParam = lambdaParam.AddModifiers(SyntaxFactory.Token(SyntaxKind.OutKeyword));
                    callArg = callArg.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.OutKeyword));
                    break;
                case RefKind.In:
                    lambdaParam = lambdaParam.AddModifiers(SyntaxFactory.Token(SyntaxKind.InKeyword));
                    callArg = callArg.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.InKeyword));
                    break;
            }
            lambdaParams.Add(lambdaParam);
            callArgs.Add(callArg);
        }

        InvocationExpressionSyntax call = SyntaxFactory.InvocationExpression(
            SyntaxFactory.ParseExpression(target.ShimTypeFqn + "." + target.MethodName + typeArgumentText),
            SyntaxFactory.ArgumentList(SyntaxFactory.SeparatedList(callArgs)));

        return SyntaxFactory.ParenthesizedLambdaExpression(
                SyntaxFactory.ParameterList(SyntaxFactory.SeparatedList(lambdaParams)),
                call)
            .WithTriviaFrom(rewrittenGroup);
    }

    private static SyntaxNode BuildRegistryShimInvocation(
        InvocationExpressionSyntax rewrittenInvocation,
        MemberSurfaceRegistry.ShimEntry entry,
        bool staticForm)
    {
        var arguments = new List<ArgumentSyntax>();
        if (entry.HasSelf && !staticForm && rewrittenInvocation.Expression is MemberAccessExpressionSyntax memberAccess)
        {
            ArgumentSyntax selfArgument = SyntaxFactory.Argument(memberAccess.Expression.WithoutTrivia());
            if (entry.SelfIsValueType)
                selfArgument = selfArgument.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.RefKeyword));
            arguments.Add(selfArgument);
        }
        arguments.AddRange(rewrittenInvocation.ArgumentList.Arguments);

        return SyntaxFactory.InvocationExpression(
                SyntaxFactory.ParseExpression(entry.ShimTypeFqn + "." + entry.ShimMethod),
                SyntaxFactory.ArgumentList(SyntaxFactory.SeparatedList(arguments)))
            .WithTriviaFrom(rewrittenInvocation);
    }

    /// <summary>Match an unresolved `recv.M(...)` against registry shims of
    /// the receiver's type (or the named type for static form).</summary>
    private static MemberSurfaceRegistry.ShimEntry? ResolveRegistryShim(
        SemanticModel model,
        PatchBatchContext batch,
        MemberAccessExpressionSyntax memberAccess,
        string memberName,
        int argumentCount,
        out bool staticForm,
        out string? tombstoneError)
    {
        staticForm = false;
        tombstoneError = null;

        INamedTypeSymbol? receiverType = null;
        if (model.GetSymbolInfo(memberAccess.Expression).Symbol is INamedTypeSymbol namedType)
        {
            staticForm = true;
            receiverType = namedType;
        }
        else if (model.GetTypeInfo(memberAccess.Expression).Type is INamedTypeSymbol instanceType)
        {
            receiverType = instanceType;
        }
        if (receiverType == null)
            return null;

        for (INamedTypeSymbol? current = receiverType.OriginalDefinition;
             current != null;
             current = current.BaseType?.OriginalDefinition)
        {
            string metadataName = SymbolMetadataName(current);
            // Param-type identity is unknowable without overload resolution:
            // match on declaring type + name + arity + staticness instead.
            foreach (var pair in batch.EarlierShims)
            {
                if (!pair.Key.StartsWith(metadataName + "|" + memberName + "|", StringComparison.Ordinal))
                    continue;
                bool entryStatic = pair.Key.EndsWith("|s", StringComparison.Ordinal);
                if (entryStatic != staticForm)
                    continue;
                MemberSurfaceRegistry.ShimEntry entry = pair.Value;
                if (entry.Kind == "tombstone")
                {
                    tombstoneError =
                        metadataName + "." + memberName + " was deleted by an earlier hot patch in this " +
                        "session; remove the call or run unity_recompile";
                    return null;
                }
                int memberParamCount = entry.ParamTypeNames.Length - (entry.HasSelf ? 1 : 0);
                if (memberParamCount != argumentCount)
                    continue;
                return entry;
            }
        }
        return null;
    }

    private static InvocationExpressionSyntax? FindEnclosingNameOf(SyntaxNode node, SemanticModel model)
    {
        for (SyntaxNode? current = node.Parent; current != null; current = current.Parent)
        {
            if (current is StatementSyntax || current is MemberDeclarationSyntax)
                return null;
            if (current is InvocationExpressionSyntax invocation &&
                invocation.Expression is IdentifierNameSyntax { Identifier.Text: "nameof" } &&
                model.GetSymbolInfo(invocation).Symbol == null)
            {
                return invocation;
            }
        }
        return null;
    }

    // ── tree navigation helpers ──────────────────────────────────────

    /// <summary>Child-index path from the root to `node` (replace passes
    /// preserve node order and counts, so the path survives ReplaceSyntax).</summary>
    private static List<int> IndexPath(SyntaxNode node)
    {
        var path = new List<int>();
        SyntaxNode current = node;
        while (current.Parent != null)
        {
            int index = 0;
            foreach (SyntaxNode sibling in current.Parent.ChildNodes())
            {
                if (sibling == current)
                    break;
                index++;
            }
            path.Insert(0, index);
            current = current.Parent;
        }
        return path;
    }

    private static SyntaxNode? NodeAtPath(SyntaxNode root, List<int> path)
    {
        SyntaxNode current = root;
        foreach (int index in path)
        {
            SyntaxNode? next = null;
            int i = 0;
            foreach (SyntaxNode child in current.ChildNodes())
            {
                if (i == index)
                {
                    next = child;
                    break;
                }
                i++;
            }
            if (next == null)
                return null;
            current = next;
        }
        return current;
    }

    /// <summary>"Ns.Outer+Inner" metadata name from a symbol.</summary>
    private static string SymbolMetadataName(INamedTypeSymbol type)
    {
        var nesting = new List<string>();
        INamedTypeSymbol outermost = type;
        for (INamedTypeSymbol? current = type; current != null; current = current.ContainingType)
        {
            nesting.Insert(0, current.MetadataName);
            outermost = current;
        }
        string ns = outermost.ContainingNamespace is { IsGlobalNamespace: false } containing
            ? containing.ToDisplayString()
            : "";
        string typePart = string.Join("+", nesting);
        return ns.Length == 0 ? typePart : ns + "." + typePart;
    }

    /// <summary>Fully-qualified display with the TOP-LEVEL container renamed
    /// to its patch copy: a brand-new NESTED type lives inside the renamed
    /// copy of its pre-existing container, so references must spell the
    /// patch name (e.g. `global::Game.Player__LocusPatch.Inner2`).</summary>
    private static string PatchQualifiedDisplay(INamedTypeSymbol type)
    {
        INamedTypeSymbol top = type;
        while (top.ContainingType != null)
            top = top.ContainingType;

        string display = type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat);
        string prefix = top.ContainingNamespace is { IsGlobalNamespace: false } ns
            ? "global::" + ns.ToDisplayString() + "." + top.Name
            : "global::" + top.Name;
        return display.StartsWith(prefix, StringComparison.Ordinal)
            ? prefix + TypeNameSuffix + display[prefix.Length..]
            : display; // unexpected display shape: a deterministic compile error beats a silent wrong bind
    }

    /// <summary>"Ns.Outer+Inner" → "Ns.Outer__LocusPatch+Inner" (the rename
    /// applies to top-level declarations; nested names follow along).</summary>
    public static string PatchTypeName(string metadataName)
    {
        int plus = metadataName.IndexOf('+');
        string topLevel = plus < 0 ? metadataName : metadataName.Substring(0, plus);
        string rest = plus < 0 ? "" : metadataName.Substring(plus);
        return topLevel + TypeNameSuffix + rest;
    }

    private static bool IsRenamedTypeSymbol(INamedTypeSymbol symbol, HashSet<INamedTypeSymbol> renamedSymbols)
    {
        INamedTypeSymbol definition = symbol.OriginalDefinition;
        if (renamedSymbols.Contains(definition))
            return true;
        // Constructed/nested forms: walk containing types to catch
        // references like Outer.Inner where Outer is renamed.
        for (INamedTypeSymbol? container = definition; container != null; container = container.ContainingType)
        {
            if (renamedSymbols.Contains(container.OriginalDefinition))
                return true;
        }
        return false;
    }

    private static bool IsDeclarationName(SyntaxNode node)
    {
        // Binding-name positions: the identifier NAMES a thing being introduced
        // (anonymous-object member `new { X = e }`, using alias `using X = T`,
        // named argument / tuple element `f(x: e)`), not a reference. It is
        // syntactically REQUIRED to stay an IdentifierName, so requalifying it to a
        // member access / qualified name builds an invalid tree that Roslyn's
        // rewriter rejects — it casts NameEquals.Name / NameColon.Name back to
        // IdentifierNameSyntax and throws InvalidCastException, aborting the whole
        // patch compile (observed as the inline caller-refresh failure behind R05).
        if (node.Parent is NameEqualsSyntax nameEquals && nameEquals.Name == node)
            return true;
        if (node.Parent is NameColonSyntax nameColon && nameColon.Name == node)
            return true;

        // The identifier inside a declaration header is a token, not a name
        // node, so the only name *nodes* to protect are explicit interface
        // specifiers (rewritten as type refs is fine) — nothing to do — and
        // namespace declaration names.
        for (SyntaxNode? current = node; current != null; current = current.Parent)
        {
            if (current is BaseNamespaceDeclarationSyntax ns && ns.Name.Span.Contains(node.Span))
                return true;
            if (current is UsingDirectiveSyntax)
                return false;
            if (current is MemberDeclarationSyntax)
                return false;
        }
        return false;
    }

    private static void CollectNewType(
        SemanticModel model,
        BaseTypeDeclarationSyntax decl,
        string metadataName,
        PatchRewriteResult result)
    {
        bool isPublic = decl.Modifiers.Any(SyntaxKind.PublicKeyword);
        string simpleName = decl.Identifier.Text;
        string ns = "";
        if (model.GetDeclaredSymbol(decl) is INamedTypeSymbol symbol)
        {
            ns = symbol.ContainingNamespace?.IsGlobalNamespace == false
                ? symbol.ContainingNamespace.ToDisplayString()
                : "";
            isPublic = symbol.DeclaredAccessibility == Accessibility.Public;
        }

        result.NewTypes.Add(new PatchNewType
        {
            MetadataName = metadataName,
            Namespace = ns,
            SimpleName = simpleName,
            IsPublic = isPublic,
            IsTopLevel = !metadataName.Contains('+'),
        });
    }

    /// <summary>Resolve a patched type's ORIGINAL symbol across the whole
    /// reference set — any project assembly works (Assembly-CSharp or an
    /// asmdef assembly; B3 relies on this being assembly-agnostic).
    ///
    /// KNOWN BOUNDARY: matching is by metadata name, FIRST reference wins.
    /// The request carries no source→assembly attribution, so two project
    /// assemblies declaring the SAME metadata name cannot be told apart —
    /// when the first match is not the edited type's home, the layout guard
    /// fails CLOSED (cold) for differing layouts; layout-identical
    /// duplicates would detour whichever same-named type Unity's own
    /// name-based domain scan finds first. The configuration is already
    /// pathological in user code (Assembly-CSharp auto-references every
    /// autoReferenced asmdef, so an unqualified use of such a name is a
    /// CS0433 ambiguity). Pinned by HotPatchTests
    /// .Same_name_type_in_two_assemblies_binds_first_and_fails_closed.
    /// Internal for the B6 partial passes (layout ordering, completeness).</summary>
    internal static INamedTypeSymbol? FindOriginalType(
        CSharpCompilation binding,
        string metadataName,
        out string? assemblyName)
    {
        foreach (MetadataReference reference in binding.References)
        {
            if (binding.GetAssemblyOrModuleSymbol(reference) is not IAssemblySymbol assembly)
                continue;
            INamedTypeSymbol? type = assembly.GetTypeByMetadataName(metadataName);
            if (type != null)
            {
                assemblyName = assembly.Name;
                return type;
            }
        }
        assemblyName = null;
        return null;
    }

    /// <summary>Ordered instance-field shape (explicit fields + synthesized
    /// auto-property/event backing fields, in declaration order — the order
    /// Roslyn emits and Mono lays out).</summary>
    private static List<string> InstanceFieldSequence(INamedTypeSymbol type)
    {
        var sequence = new List<string>();
        foreach (ISymbol member in type.GetMembers())
        {
            if (member is not IFieldSymbol field || field.IsStatic || field.IsConst)
                continue;
            sequence.Add(field.Name + "|" + field.Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat));
        }
        return sequence;
    }
}
