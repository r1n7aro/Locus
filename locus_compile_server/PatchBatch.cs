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
        bool allowUnsafe = false)
    {
        var context = new PatchBatchContext
        {
            Binding = CSharpCompilation.Create(
                "LocusHotPatchBinding",
                files.Select(f => f.Tree),
                references,
                new CSharpCompilationOptions(
                    OutputKind.DynamicallyLinkedLibrary,
                    allowUnsafe: allowUnsafe,
                    metadataImportOptions: MetadataImportOptions.All,
                    assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default)),
            EarlierShims = earlierShims,
            EarlierFieldStores = earlierFieldStores
                ?? new Dictionary<string, FieldStoreRegistry.StoreEntry>(StringComparer.Ordinal),
            StoreDiscriminator = storeDiscriminator,
        };

        foreach (var (_, tree, diff) in files)
        {
            SemanticModel model = context.Binding.GetSemanticModel(tree);
            var root = (CompilationUnitSyntax)tree.GetRoot();
            var newTypeNames = new HashSet<string>(diff.NewTypes, StringComparer.Ordinal);

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
                MethodDeclarationSyntax? decl = FindAddedMethodDeclaration(root, added);
                if (decl == null)
                    continue;
                if (model.GetDeclaredSymbol(decl) is not IMethodSymbol symbol)
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

        return context;
    }

    internal static MethodDeclarationSyntax? FindAddedMethodDeclaration(CompilationUnitSyntax root, HotDiffMethod added)
    {
        foreach (TypeDeclarationSyntax typeDecl in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
        {
            if (HotDiff.MetadataName(typeDecl) != added.DeclaringType)
                continue;
            foreach (MethodDeclarationSyntax method in typeDecl.Members.OfType<MethodDeclarationSyntax>())
            {
                if (method.Identifier.Text != added.Name)
                    continue;
                if (method.Modifiers.Any(SyntaxKind.StaticKeyword) != added.IsStatic)
                    continue;
                if ((method.TypeParameterList?.Parameters.Count ?? 0) != added.TypeParameterCount)
                    continue;
                if (!HotDiff.ParamTypeNames(method.ParameterList).SequenceEqual(added.ParamTypeNames, StringComparer.Ordinal))
                    continue;
                return method;
            }
        }
        return null;
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
