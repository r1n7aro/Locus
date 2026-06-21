using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Locus.CompileServer;

/// <summary>Where calls to one ADDED member must be redirected (M2).</summary>
public sealed class ShimTarget
{
    public string ShimTypeFqn = "";
    public string ShimTypeMetadataName = "";
    public string ShimNamespace = "";
    public string MethodName = "";

    /// <summary>Original (un-renamed) declaring type, fully qualified.</summary>
    public string DeclaringTypeFqn = "";
    public string DeclaringTypeMetadataName = "";

    public bool HasSelf;
    public bool SelfIsValueType;
    public bool SelfIsRefLike;
    public bool GenericShim;

    /// <summary>Type parameters the shim method needs: the declaring type
    /// chain's parameters (outermost first) followed by the method's OWN
    /// type parameters (B1). Empty for non-generic surface.</summary>
    public string[] TypeParameters = Array.Empty<string>();

    /// <summary>How many of <see cref="TypeParameters"/> (from the end) are
    /// the method's own. Call sites of such shims materialize explicit type
    /// arguments — explicit arguments at the original call site cannot be
    /// partially re-applied to the flattened parameter list.</summary>
    public int MethodTypeParameterCount;

    /// <summary>Reflection-style parameter names of the SHIM method
    /// (self included) — the detour identity for re-edits.</summary>
    public string[] ShimParamTypeNames = Array.Empty<string>();

    /// <summary>Member identity across batches (original member shape).</summary>
    public string MemberKey = "";

    /// <summary>Set for the synthetic self-shim of a CHANGED INSTANCE method
    /// emitted only as a Release inline caller-refresh direct-call target
    /// (Option A source clone). Such shims are internal to the batch: they are
    /// NOT user surface, so they skip the MemberSurfaceRegistry registration
    /// and the re-edit continuity detour the ADDED-member shims get.</summary>
    public bool IsInlineRedirectShim;
}

/// <summary>Where calls to one CHANGED method in the same batch should bind so
/// a refreshed caller's body invokes the CURRENT patch copy instead of letting
/// Mono re-inline the stale original (Release inline caller refresh).
/// <para>Static methods bind directly to the patch copy
/// (<see cref="PatchTypeFqn"/>.<see cref="MethodName"/>(args)). Instance
/// methods cannot — the patch copy is a distinct type that can't be invoked on
/// the original receiver — so they bind to a static self-shim
/// (<see cref="ShimTypeFqn"/>.<see cref="MethodName"/>(self, args)), the same
/// shape the ADDED-member machinery uses.</para></summary>
public sealed class PatchedMethodTarget
{
    /// <summary>Static form: the patch copy type FQN. Empty for instance.</summary>
    public string PatchTypeFqn = "";

    /// <summary>Method (static form) or self-shim (instance form) name to call.</summary>
    public string MethodName = "";

    /// <summary>Instance form: prepend the receiver as the shim's `self`
    /// argument; the call binds to <see cref="ShimTypeFqn"/>.</summary>
    public bool HasSelf;

    /// <summary>Instance form: the static self-shim host class FQN
    /// (`global::Ns.Foo__LocusShims`).</summary>
    public string? ShimTypeFqn;

    /// <summary>Instance form: the original declaring type FQN, for the
    /// `((global::Ns.Foo)(object)this)` cast when the receiver is `this`.</summary>
    public string DeclaringTypeFqn = "";

    public bool SelfIsValueType;
    public bool SelfIsRefLike;
}

/// <summary>One Release inline caller-refresh self-shim clone of a CHANGED
/// instance method (Option A): a uniquely-named copy injected into the new-side
/// tree BEFORE binding, so the existing ADDED-member shim pipeline turns it
/// into a `static R Name(this Foo self, ...)` shim while the original method
/// keeps its normal detour. <see cref="Original"/> identifies the method whose
/// in-batch call sites redirect to the clone's shim.</summary>
public sealed class InlineRedirectClone
{
    public string FilePath = "";
    public HotDiffMethod Original = null!;
    public HotDiffMethod Clone = null!;
}

/// <summary>C0 measured (operation × visibility) JIT access matrix, projected
/// for classification (C2′a): the rewriter consults it to decide whether an
/// added member's non-public BODY reference may go hot through the existing
/// IgnoresAccessChecksTo + IgnoreAccessibility mechanism. Null when the
/// request carried no usable matrix (old plugin, failed probe) — consumers
/// must then keep today's conservative cold verdicts.</summary>
public sealed class AccessCaps
{
    private readonly Dictionary<string, bool> _cells;

    private AccessCaps(Dictionary<string, bool> cells) => _cells = cells;

    /// <summary>Null when absent or empty: "no measured caps" and "no green
    /// cells" gate identically (conservative).</summary>
    public static AccessCaps? FromCells(IReadOnlyDictionary<string, bool>? cells)
    {
        if (cells == null || cells.Count == 0)
            return null;
        return new AccessCaps(new Dictionary<string, bool>(cells, StringComparer.Ordinal));
    }

    /// <summary>The "{op}_{visibility}" probe cell measured green on the
    /// running editor's Mono (AccessProbeSource cell naming).</summary>
    public bool Allows(string op, string visibility) =>
        _cells.TryGetValue(op + "_" + visibility, out bool ok) && ok;

    /// <summary>Every canonical probe cell measured green (the universal
    /// result on every Unity tested so far). The C2′b kept-surface scan
    /// short-circuits then: nothing it could check can fail, so the
    /// rewrite pays zero scan cost on green runtimes.</summary>
    public bool CoversAllCells()
    {
        foreach (AccessProbeCell cell in AccessProbeSource.Cells)
        {
            if (!Allows(cell.Op, cell.Visibility))
                return false;
        }
        return true;
    }
}

/// <summary>
/// Batch-wide rewrite context (M1): ONE binding compilation over every hot
/// file's un-renamed tree (source declarations shadow the original metadata,
/// CS0436), so cross-file references — including calls to members added in a
/// different file of the same batch — bind symbolically and each file's
/// rewriter can resolve them against shared declaration sets.
/// </summary>
public sealed class PatchBatchContext
{
    public CSharpCompilation Binding = null!;

    /// <summary>Pre-existing file-local types across the WHOLE batch (their
    /// references rewrite to fully-qualified original names).</summary>
    public HashSet<INamedTypeSymbol> RenamedSymbols = new(SymbolEqualityComparer.Default);

    /// <summary>Added (new-surface) member symbols across the batch → shim
    /// target. Keyed by the ORIGINAL DEFINITION of the method symbol.</summary>
    public Dictionary<IMethodSymbol, ShimTarget> AddedMembers = new(SymbolEqualityComparer.Default);

    /// <summary>Changed ordinary static method symbols across the batch →
    /// patch type target. Used by Release inline caller refresh: a refreshed
    /// caller must call the current patch copy of an inlined callee, otherwise
    /// Mono can inline the original callee body again.</summary>
    public Dictionary<IMethodSymbol, PatchedMethodTarget> PatchedMethods = new(SymbolEqualityComparer.Default);

    /// <summary>Member keys that are REMOVED and re-ADDED with the same
    /// signature in this batch (B1 generic body changes): pre-existing
    /// compiled call sites exist, so every in-batch reference inside a KEPT
    /// member must drag that member into the detour set (or fail closed).</summary>
    public HashSet<string> ReAddedMemberKeys = new(StringComparer.Ordinal);

    /// <summary>Brand-new type symbols across the batch (the per-file
    /// NewTypes): their references never re-qualify to ORIGINAL names —
    /// nested ones under renamed containers re-qualify to the PATCH name
    /// (the original metadata type has no such nested member).</summary>
    public HashSet<INamedTypeSymbol> NewTypeSymbols = new(SymbolEqualityComparer.Default);

    /// <summary>Shims registered by earlier accepted batches of the same
    /// domain generation, keyed by member key.</summary>
    public IReadOnlyDictionary<string, MemberSurfaceRegistry.ShimEntry> EarlierShims =
        new Dictionary<string, MemberSurfaceRegistry.ShimEntry>(StringComparer.Ordinal);

    /// <summary>Field stores introduced by earlier accepted batches (M4),
    /// keyed by declaringType|fieldName — re-edits bind to these instead of
    /// regenerating (a new store would split the values).</summary>
    public IReadOnlyDictionary<string, FieldStoreRegistry.StoreEntry> EarlierFieldStores =
        new Dictionary<string, FieldStoreRegistry.StoreEntry>(StringComparer.Ordinal);

    /// <summary>Appended enum member symbols across the batch (H7e):
    /// references materialize as `(EnumFqn)value` cast literals.</summary>
    public Dictionary<IFieldSymbol, (string EnumFqn, long Value)> AddedEnumMembers =
        new(SymbolEqualityComparer.Default);

    /// <summary>Batch-unique suffix for NEW field-store holder names. Two
    /// batches adding fields to the SAME type would otherwise declare
    /// same-named holders, and the later patch's source declaration would
    /// shadow (CS0436) the earlier holder its re-sent fields still bind to.</summary>
    public string StoreDiscriminator = "";

    /// <summary>Measured runtime access caps for this domain generation
    /// (C2′a); null = conservative (non-public body references stay cold).</summary>
    public AccessCaps? RuntimeCaps;

    /// <summary>Files (keyed exactly as passed to <see cref="PatchRewriter.Rewrite"/>)
    /// whose types live only in a prior hot-patch assembly (play-mode-born): the
    /// value is that FIRST assembly. Redirections from such a file pin their
    /// OriginalAssembly to it, so Unity detours the FIRST loaded type (existing
    /// instances), not just newly created ones. Empty for ordinary batches.</summary>
    public IReadOnlyDictionary<string, string> ReeditFileAssemblies =
        new Dictionary<string, string>(StringComparer.Ordinal);

    /// <summary>Per-TYPE override of <see cref="ReeditFileAssemblies"/> (feature
    /// #5): a sibling type born into a play-mode-born file AFTER its first batch
    /// lives in its OWN assembly, not the file's first one. Keyed by the type's
    /// metadata name. Empty for ordinary batches and for files whose types are all
    /// first-batch.</summary>
    public IReadOnlyDictionary<string, string> ReeditTypeAssemblies =
        new Dictionary<string, string>(StringComparer.Ordinal);

    /// <summary>The play-mode-born detour ORIGINAL side for a member of
    /// <paramref name="declaringType"/> in <paramref name="path"/>: the sibling's
    /// own assembly if the type was born late (feature #5), else the file's first
    /// assembly, else null (an ordinary compiled type — default resolution).</summary>
    public string? ReeditAssemblyFor(string path, string declaringType) =>
        ReeditTypeAssembly(ReeditTypeAssemblies, declaringType)
        ?? (ReeditFileAssemblies.TryGetValue(path, out string? fileAssembly) ? fileAssembly : null);

    /// <summary>The late-born sibling assembly a <paramref name="declaringType"/>
    /// resolves to, or null. Only TOP-LEVEL siblings are registered, so a NESTED
    /// type (metadata names nest with '+') is matched to its parent sibling by
    /// prefix — without this a nested type's detour/driver falls back to the file's
    /// FIRST assembly, where the nested type does not exist (feature #5 / [P2]).</summary>
    public static string? ReeditTypeAssembly(
        IReadOnlyDictionary<string, string> typeAssemblies, string declaringType)
    {
        if (typeAssemblies.TryGetValue(declaringType, out string? typeAssembly))
            return typeAssembly;
        foreach (var pair in typeAssemblies)
            if (declaringType.StartsWith(pair.Key + "+", StringComparison.Ordinal))
                return pair.Value;
        return null;
    }

    public SemanticModel ModelFor(SyntaxTree tree) => Binding.GetSemanticModel(tree);

    /// <summary>
    /// Build the shared context for a batch of (path, tree, diff) files: the
    /// binding compilation, the renamed-symbol set and the added-member shim
    /// targets. Pure analysis; per-file rewriting happens afterwards.
    /// </summary>
    public static PatchBatchContext Build(
        IReadOnlyList<(string Path, SyntaxTree Tree, HotDiffFileResult Diff)> files,
        System.Collections.Immutable.ImmutableArray<MetadataReference> references,
        IReadOnlyDictionary<string, MemberSurfaceRegistry.ShimEntry> earlierShims,
        IReadOnlyDictionary<string, FieldStoreRegistry.StoreEntry>? earlierFieldStores = null,
        string storeDiscriminator = "",
        bool allowUnsafe = false,
        AccessCaps? runtimeCaps = null,
        IReadOnlyList<InlineRedirectClone>? inlineClones = null,
        IReadOnlyDictionary<string, string>? reeditFileAssemblies = null,
        IReadOnlyDictionary<string, string>? reeditTypeAssemblies = null)
    {
        // The binding model must RESOLVE what the emit will BIND: the emit
        // compilation has always carried IgnoreAccessibility (kept bodies
        // legitimately reach the original's privates), so the binding takes
        // the same flag — otherwise a non-public symbol from pure metadata
        // (another assembly, or an unedited file of the project assembly:
        // the batch is named LocusHotPatchBinding, so even "same-assembly"
        // internals are foreign here) binds to null, the access scans skip
        // it, and the reference ships hot UNGATED (C2′b). Known boundary,
        // unchanged from the emit side: an inaccessible overload may now
        // win resolution over an accessible one — binding and emit at least
        // agree on it.
        var bindingOptions = new CSharpCompilationOptions(
            OutputKind.DynamicallyLinkedLibrary,
            allowUnsafe: allowUnsafe,
            metadataImportOptions: MetadataImportOptions.All,
            assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default);
        CompileService.ApplyIgnoreAccessibility(bindingOptions);

        var context = new PatchBatchContext
        {
            Binding = CSharpCompilation.Create(
                "LocusHotPatchBinding",
                files.Select(f => f.Tree),
                references,
                bindingOptions),
            EarlierShims = earlierShims,
            EarlierFieldStores = earlierFieldStores
                ?? new Dictionary<string, FieldStoreRegistry.StoreEntry>(StringComparer.Ordinal),
            StoreDiscriminator = storeDiscriminator,
            RuntimeCaps = runtimeCaps,
            ReeditFileAssemblies = reeditFileAssemblies
                ?? new Dictionary<string, string>(StringComparer.Ordinal),
            ReeditTypeAssemblies = reeditTypeAssemblies
                ?? new Dictionary<string, string>(StringComparer.Ordinal),
        };

        foreach (var (path, tree, diff) in files)
        {
            SemanticModel model = context.Binding.GetSemanticModel(tree);
            var root = (CompilationUnitSyntax)tree.GetRoot();
            var newTypeNames = new HashSet<string>(diff.NewTypes, StringComparer.Ordinal);

            // Release inline caller-refresh self-shim clones live in the diff as
            // synthetic ADDED methods (so the M2 shim pipeline emits them), but
            // they are gated and wired separately below — keep the normal added
            // loop from treating them as ordinary user-added surface.
            var cloneNamesThisFile = new HashSet<string>(
                (inlineClones ?? Array.Empty<InlineRedirectClone>())
                    .Where(c => string.Equals(c.FilePath, path, StringComparison.OrdinalIgnoreCase))
                    .Select(c => c.Clone.Name),
                StringComparer.Ordinal);

            foreach (MemberDeclarationSyntax member in root.DescendantNodes().OfType<MemberDeclarationSyntax>())
            {
                switch (member)
                {
                    case BaseTypeDeclarationSyntax typeDecl:
                        if (newTypeNames.Contains(HotDiff.MetadataName(typeDecl)))
                        {
                            if (model.GetDeclaredSymbol(typeDecl) is INamedTypeSymbol newTypeSymbol)
                                context.NewTypeSymbols.Add(newTypeSymbol);
                            continue;
                        }
                        if (model.GetDeclaredSymbol(typeDecl) is INamedTypeSymbol typeSymbol)
                            context.RenamedSymbols.Add(typeSymbol);
                        break;
                    case DelegateDeclarationSyntax delegateDecl:
                        if (model.GetDeclaredSymbol(delegateDecl) is INamedTypeSymbol delegateSymbol)
                            context.RenamedSymbols.Add(delegateSymbol);
                        break;
                }
            }

            foreach (HotDiffMethod added in diff.ChangedMethods.Where(m => m.Added))
            {
                if (cloneNamesThisFile.Contains(added.Name))
                    continue; // handled by the inline-redirect clone loop below
                IMethodSymbol? symbol = null;
                MethodDeclarationSyntax? decl = FindMethodDeclaration(root, added);
                if (decl != null)
                    symbol = model.GetDeclaredSymbol(decl);
                else
                    symbol = FindAddedAccessorSymbol(root, model, added, out _, out _); // B2 accessors
                if (symbol == null)
                    continue;

                ShimTarget target = BuildShimTarget(symbol, added);
                context.AddedMembers[symbol.OriginalDefinition] = target;

                // Same-signature remove+add = a re-materialized member (B1):
                // old compiled call sites exist and must be redirected.
                if (diff.RemovedMembers.Any(removed =>
                    MemberSurfaceRegistry.MemberKey(
                        removed.DeclaringType, removed.Name, removed.ParamTypeNames, removed.IsStatic)
                    == target.MemberKey))
                {
                    context.ReAddedMemberKeys.Add(target.MemberKey);
                }
            }

            foreach (HotDiffMethod changed in diff.ChangedMethods.Where(m => !m.Added))
            {
                if (changed.IsCtor || !changed.IsStatic || changed.TypeParameterCount != 0)
                    continue;
                MethodDeclarationSyntax? decl = FindMethodDeclaration(root, changed);
                if (decl == null)
                    continue;
                if ((decl.TypeParameterList?.Parameters.Count ?? 0) != 0)
                    continue;
                if (model.GetDeclaredSymbol(decl) is not IMethodSymbol symbol)
                    continue;
                if (symbol.MethodKind != MethodKind.Ordinary || !symbol.IsStatic || symbol.IsGenericMethod)
                    continue;
                if (ContainingTypeHasTypeParameters(symbol.ContainingType))
                    continue;

                context.PatchedMethods[symbol.OriginalDefinition] = new PatchedMethodTarget
                {
                    PatchTypeFqn = PatchQualifiedDisplay(symbol.ContainingType),
                    MethodName = symbol.Name,
                };
            }

            foreach (HotDiffEnumAddition addition in diff.EnumAdditions)
            {
                foreach (EnumDeclarationSyntax enumDecl in root.DescendantNodes().OfType<EnumDeclarationSyntax>())
                {
                    if (HotDiff.MetadataName(enumDecl) != addition.EnumType)
                        continue;
                    foreach (EnumMemberDeclarationSyntax member in enumDecl.Members)
                    {
                        if (member.Identifier.Text != addition.MemberName)
                            continue;
                        if (model.GetDeclaredSymbol(member) is IFieldSymbol enumField)
                        {
                            string enumFqn = enumField.ContainingType
                                .ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat);
                            context.AddedEnumMembers[enumField] = (enumFqn, addition.Value);
                        }
                    }
                }
            }
        }

        // Release inline caller-refresh self-shim clones (Option A). A second
        // pass, AFTER the whole batch's renamed/added/new-type context exists,
        // so the caps gate sees the complete picture. Each clone is a synthetic
        // ADDED instance method already injected into the new-side tree; here it
        // is gated and, when shimmable, both wired as a shim (AddedMembers) and
        // recorded as the redirect target for the original method's call sites.
        if (inlineClones is { Count: > 0 })
        {
            var byPath = files.ToDictionary(
                f => f.Path,
                f => (Model: context.Binding.GetSemanticModel(f.Tree), Root: (CompilationUnitSyntax)f.Tree.GetRoot()),
                StringComparer.OrdinalIgnoreCase);

            foreach (InlineRedirectClone clone in inlineClones)
            {
                if (!byPath.TryGetValue(clone.FilePath, out var bound))
                    continue;
                SemanticModel model = bound.Model;
                CompilationUnitSyntax root = bound.Root;

                MethodDeclarationSyntax? cloneDecl = FindMethodDeclaration(root, clone.Clone);
                MethodDeclarationSyntax? originalDecl = FindMethodDeclaration(root, clone.Original);
                if (cloneDecl == null || originalDecl == null)
                    continue;
                if (model.GetDeclaredSymbol(cloneDecl) is not IMethodSymbol cloneSymbol ||
                    model.GetDeclaredSymbol(originalDecl) is not IMethodSymbol originalSymbol)
                    continue;

                // Value types (incl. record structs) are deferred — see
                // IsInlineCloneEligible. Skip the redirect; converge via recompile.
                if (originalSymbol.ContainingType.IsValueType)
                    continue;

                // Skip-not-cold: the static self-shim reaches the original
                // type's members WITHOUT the layout identity the instance patch
                // copy enjoys, so its non-public reach is gated by the measured
                // caps. A violation only means the inline-refresh optimization
                // can't apply here — drop the redirect and let the method
                // converge via the queued recompile; it is still hot through its
                // normal detour. Failing cold would REGRESS an otherwise-hot edit.
                SyntaxNode? cloneBody = (SyntaxNode?)cloneDecl.Body ?? cloneDecl.ExpressionBody;
                if (cloneBody != null &&
                    PatchRewriter.FindInlineShimAccessViolation(
                        cloneSymbol, cloneBody, model, context, context.RenamedSymbols) != null)
                {
                    continue;
                }

                ShimTarget shimTarget = BuildShimTarget(cloneSymbol, clone.Clone);
                shimTarget.IsInlineRedirectShim = true;
                context.AddedMembers[cloneSymbol.OriginalDefinition] = shimTarget;

                INamedTypeSymbol declaringType = originalSymbol.ContainingType;
                context.PatchedMethods[originalSymbol.OriginalDefinition] = new PatchedMethodTarget
                {
                    HasSelf = true,
                    ShimTypeFqn = shimTarget.ShimTypeFqn,
                    MethodName = shimTarget.MethodName,
                    DeclaringTypeFqn = declaringType.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat),
                    SelfIsValueType = declaringType.IsValueType,
                    SelfIsRefLike = declaringType.IsRefLikeType,
                };
            }
        }

        return context;
    }

    /// <summary>Locate the accessor declaration an added accessor-shaped
    /// HotDiffMethod (get_X/set_X/get_Item/set_Item/add_X/remove_X) refers
    /// to and return its method symbol (B2). `container` is the owning
    /// property/indexer/event declaration; `accessor` is null for an
    /// expression-bodied property/indexer (the implicit getter).</summary>
    internal static IMethodSymbol? FindAddedAccessorSymbol(
        CompilationUnitSyntax root,
        SemanticModel model,
        HotDiffMethod added,
        out BasePropertyDeclarationSyntax? container,
        out AccessorDeclarationSyntax? accessor)
    {
        container = null;
        accessor = null;

        foreach (TypeDeclarationSyntax typeDecl in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
        {
            if (HotDiff.MetadataName(typeDecl) != added.DeclaringType)
                continue;

            foreach (MemberDeclarationSyntax member in typeDecl.Members)
            {
                switch (member)
                {
                    case PropertyDeclarationSyntax property:
                    {
                        if (property.Modifiers.Any(SyntaxKind.StaticKeyword) != added.IsStatic)
                            continue;
                        if (added.Name == "get_" + property.Identifier.Text)
                        {
                            if (added.ParamTypeNames.Length != 0)
                                continue;
                            if (property.ExpressionBody != null)
                            {
                                container = property;
                                return (model.GetDeclaredSymbol(property) as IPropertySymbol)?.GetMethod;
                            }
                            AccessorDeclarationSyntax? getter = (property.AccessorList?.Accessors ?? default)
                                .FirstOrDefault(a => a.Keyword.Text == "get");
                            if (getter == null)
                                continue;
                            container = property;
                            accessor = getter;
                            return model.GetDeclaredSymbol(getter);
                        }
                        if (added.Name == "set_" + property.Identifier.Text)
                        {
                            AccessorDeclarationSyntax? setter = (property.AccessorList?.Accessors ?? default)
                                .FirstOrDefault(a => a.Keyword.Text is "set" or "init");
                            if (setter == null)
                                continue;
                            if (!HotDiff.AccessorParams(setter, null, HotDiff.TokenText(property.Type))
                                .SequenceEqual(added.ParamTypeNames, StringComparer.Ordinal))
                                continue;
                            container = property;
                            accessor = setter;
                            return model.GetDeclaredSymbol(setter);
                        }
                        continue;
                    }

                    case IndexerDeclarationSyntax indexer when
                        (added.Name == "get_Item" || added.Name == "set_Item") && !added.IsStatic:
                    {
                        if (added.Name == "get_Item")
                        {
                            if (indexer.ExpressionBody != null)
                            {
                                if (!HotDiff.ParamTypeNames(indexer.ParameterList)
                                    .SequenceEqual(added.ParamTypeNames, StringComparer.Ordinal))
                                    continue;
                                container = indexer;
                                return (model.GetDeclaredSymbol(indexer) as IPropertySymbol)?.GetMethod;
                            }
                            AccessorDeclarationSyntax? getter = (indexer.AccessorList?.Accessors ?? default)
                                .FirstOrDefault(a => a.Keyword.Text == "get");
                            if (getter == null ||
                                !HotDiff.AccessorParams(getter, indexer.ParameterList, HotDiff.TokenText(indexer.Type))
                                    .SequenceEqual(added.ParamTypeNames, StringComparer.Ordinal))
                            {
                                continue;
                            }
                            container = indexer;
                            accessor = getter;
                            return model.GetDeclaredSymbol(getter);
                        }
                        AccessorDeclarationSyntax? setter2 = (indexer.AccessorList?.Accessors ?? default)
                            .FirstOrDefault(a => a.Keyword.Text is "set" or "init");
                        if (setter2 == null ||
                            !HotDiff.AccessorParams(setter2, indexer.ParameterList, HotDiff.TokenText(indexer.Type))
                                .SequenceEqual(added.ParamTypeNames, StringComparer.Ordinal))
                        {
                            continue;
                        }
                        container = indexer;
                        accessor = setter2;
                        return model.GetDeclaredSymbol(setter2);
                    }

                    case EventDeclarationSyntax @event:
                    {
                        if (@event.Modifiers.Any(SyntaxKind.StaticKeyword) != added.IsStatic)
                            continue;
                        string? keyword =
                            added.Name == "add_" + @event.Identifier.Text ? "add"
                            : added.Name == "remove_" + @event.Identifier.Text ? "remove"
                            : null;
                        if (keyword == null)
                            continue;
                        AccessorDeclarationSyntax? eventAccessor = (@event.AccessorList?.Accessors ?? default)
                            .FirstOrDefault(a => a.Keyword.Text == keyword);
                        if (eventAccessor == null)
                            continue;
                        container = @event;
                        accessor = eventAccessor;
                        return model.GetDeclaredSymbol(eventAccessor);
                    }
                }
            }
        }
        return null;
    }

    internal static MethodDeclarationSyntax? FindAddedMethodDeclaration(CompilationUnitSyntax root, HotDiffMethod added) =>
        FindMethodDeclaration(root, added);

    private static MethodDeclarationSyntax? FindMethodDeclaration(CompilationUnitSyntax root, HotDiffMethod methodInfo)
    {
        foreach (TypeDeclarationSyntax typeDecl in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
        {
            if (HotDiff.MetadataName(typeDecl) != methodInfo.DeclaringType)
                continue;
            foreach (MethodDeclarationSyntax method in typeDecl.Members.OfType<MethodDeclarationSyntax>())
            {
                if (method.Identifier.Text != methodInfo.Name)
                    continue;
                if (method.Modifiers.Any(SyntaxKind.StaticKeyword) != methodInfo.IsStatic)
                    continue;
                if ((method.TypeParameterList?.Parameters.Count ?? 0) != methodInfo.TypeParameterCount)
                    continue;
                if (!HotDiff.ParamTypeNames(method.ParameterList).SequenceEqual(methodInfo.ParamTypeNames, StringComparer.Ordinal))
                    continue;
                return method;
            }
        }
        return null;
    }

    private static bool ContainingTypeHasTypeParameters(INamedTypeSymbol? type)
    {
        for (INamedTypeSymbol? current = type; current != null; current = current.ContainingType)
        {
            if (current.TypeParameters.Length > 0)
                return true;
        }
        return false;
    }

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
            ? prefix + PatchRewriter.TypeNameSuffix + display[prefix.Length..]
            : display;
    }

    private static ShimTarget BuildShimTarget(IMethodSymbol symbol, HotDiffMethod added)
    {
        INamedTypeSymbol declaring = symbol.ContainingType;

        // Top-level type of the declaring chain names the shim class.
        INamedTypeSymbol topLevel = declaring;
        while (topLevel.ContainingType != null)
            topLevel = topLevel.ContainingType;

        string ns = topLevel.ContainingNamespace is { IsGlobalNamespace: false } containing
            ? containing.ToDisplayString()
            : "";
        string shimSimpleName = topLevel.Name + "__LocusShims";

        var typeParameters = new List<string>();
        for (INamedTypeSymbol? current = declaring; current != null; current = current.ContainingType)
        {
            foreach (ITypeParameterSymbol parameter in current.TypeParameters.Reverse())
                typeParameters.Insert(0, parameter.Name);
        }
        // The method's own type parameters follow the chain's (B1):
        // HotDiff already failed shadowed names closed.
        foreach (ITypeParameterSymbol parameter in symbol.TypeParameters)
            typeParameters.Add(parameter.Name);

        var shimParams = new List<string>();
        if (!added.IsStatic)
            shimParams.Add(ReflectionSimpleName(declaring));
        shimParams.AddRange(added.ParamTypeNames);

        return new ShimTarget
        {
            ShimTypeFqn = ns.Length == 0 ? "global::" + shimSimpleName : "global::" + ns + "." + shimSimpleName,
            ShimTypeMetadataName = ns.Length == 0 ? shimSimpleName : ns + "." + shimSimpleName,
            ShimNamespace = ns,
            MethodName = added.Name,
            DeclaringTypeFqn = declaring.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat),
            DeclaringTypeMetadataName = added.DeclaringType,
            HasSelf = !added.IsStatic,
            SelfIsValueType = declaring.IsValueType,
            SelfIsRefLike = declaring.IsRefLikeType,
            GenericShim = typeParameters.Count > 0,
            TypeParameters = typeParameters.ToArray(),
            MethodTypeParameterCount = symbol.TypeParameters.Length,
            ShimParamTypeNames = shimParams.ToArray(),
            MemberKey = MemberSurfaceRegistry.MemberKey(
                added.DeclaringType, added.Name, added.ParamTypeNames, added.IsStatic),
        };
    }

    /// <summary>Reflection Type.Name-style simple name of a type symbol
    /// ("Foo", "List`1", nested "Inner").</summary>
    private static string ReflectionSimpleName(INamedTypeSymbol type)
    {
        return type.TypeParameters.Length > 0
            ? type.Name + "`" + type.TypeParameters.Length
            : type.Name;
    }
}
