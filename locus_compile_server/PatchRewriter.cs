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
    public bool IsStatic;
    public bool IsCtor;
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

public sealed class PatchRewriteResult
{
    /// <summary>Rewritten tree, ready for the patch compilation.</summary>
    public SyntaxTree? Tree;

    public List<PatchMethodMap> Methods = new();
    public List<PatchNewType> NewTypes = new();

    /// <summary>Assemblies whose non-public members the patch may touch
    /// (the original assemblies of the patched types).</summary>
    public List<string> OriginalAssemblies = new();

    /// <summary>Set when the file must take the cold path after all — e.g.
    /// the original assembly's field layout does not match the baseline.</summary>
    public string? ColdReason;
}

// ── rewriter ─────────────────────────────────────────────────────────

/// <summary>
/// Turns an edited source file into a hot-patch source:
///
///  1. every type declared in the file (except brand-new ones) is renamed
///     `Foo` → `Foo__LocusPatch`, so the patch assembly never collides with
///     the original;
///  2. every *reference* to those types — bodies, signatures, base lists,
///     typeof, attributes — is rewritten to a fully-qualified name, which
///     after the rename binds to the *original* assembly's type: object
///     identity, serialization and Unity APIs keep seeing original types;
///  3. unqualified static field/property/event accesses are qualified back
///     to the original type, keeping static state single-sourced;
///  4. static constructors and static field initializers are emptied so the
///     patch type's cctor can never re-run side effects;
///  5. the original assembly's instance-field layout is compared against the
///     source as a guard against a stale baseline (fails closed to cold).
///
/// Instance fields keep their declarations (same ordered layout as the
/// original — guaranteed hot-side by HotDiff, original-side by the guard in
/// step 5), so patched bodies read correct offsets through original `this`.
/// </summary>
public static class PatchRewriter
{
    public const string TypeNameSuffix = "__LocusPatch";

    public static PatchRewriteResult Rewrite(
        string path,
        string newText,
        HotDiffFileResult diff,
        CSharpParseOptions parseOptions,
        ImmutableArray<MetadataReference> references)
    {
        var result = new PatchRewriteResult();

        SyntaxTree tree = CSharpSyntaxTree.ParseText(newText, parseOptions, path: path);
        var root = (CompilationUnitSyntax)tree.GetRoot();

        // Binding pass: the un-renamed file against the real reference set.
        // Source declarations shadow the originals (CS0436), which is exactly
        // what lets us detect "reference to a file-local type" symbolically.
        // Binding *errors* are ignored here — pass B reports the real ones.
        CSharpCompilation binding = CSharpCompilation.Create(
            "LocusHotPatchBinding",
            new[] { tree },
            references,
            new CSharpCompilationOptions(
                OutputKind.DynamicallyLinkedLibrary,
                allowUnsafe: false,
                metadataImportOptions: MetadataImportOptions.All,
                assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default));
        SemanticModel model = binding.GetSemanticModel(tree);

        var newTypeNames = new HashSet<string>(diff.NewTypes, StringComparer.Ordinal);

        // File-local pre-existing types: ALL of them (nested included) get
        // their references rewritten; only TOP-LEVEL declarations get the
        // identifier rename — nested metadata names change through their
        // outer type ("Ns.Outer__LocusPatch+Inner").
        var localDecls = new List<BaseTypeDeclarationSyntax>();
        var topLevelDecls = new List<BaseTypeDeclarationSyntax>();
        var renamedSymbols = new HashSet<INamedTypeSymbol>(SymbolEqualityComparer.Default);
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
                        CollectNewType(model, typeDecl, metadataName, result);
                        continue;
                    }
                    localDecls.Add(typeDecl);
                    if (IsTopLevel(typeDecl))
                        topLevelDecls.Add(typeDecl);
                    if (model.GetDeclaredSymbol(typeDecl) is INamedTypeSymbol symbol)
                        renamedSymbols.Add(symbol);
                    break;
                }
                case DelegateDeclarationSyntax delegateDecl:
                {
                    // Delegates: reference-rewrite like any other type so
                    // signatures keep matching originals; rename top-level
                    // declarations only.
                    if (IsTopLevel(delegateDecl))
                        topLevelDelegates.Add(delegateDecl);
                    if (model.GetDeclaredSymbol(delegateDecl) is INamedTypeSymbol symbol)
                        renamedSymbols.Add(symbol);
                    break;
                }
            }
        }

        // Layout guard + original assembly names for the patched types.
        foreach (string patchedType in diff.PatchedTypes)
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
            if (!InstanceFieldSequence(sourceSymbol).SequenceEqual(InstanceFieldSequence(original), StringComparer.Ordinal))
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

        foreach (BaseTypeDeclarationSyntax decl in localDecls)
        {
            if (decl is not TypeDeclarationSyntax typeDecl)
                continue;

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
            if (IsDeclarationName(node) || InStrippedSpan(node))
                continue;

            ISymbol? symbol = model.GetSymbolInfo(node).Symbol;
            if (symbol is INamedTypeSymbol named && IsRenamedTypeSymbol(named, renamedSymbols))
                typeRefCandidates[node] = named;
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

            string fqn = pair.Value.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat);
            bool expressionContext =
                node.Parent is MemberAccessExpressionSyntax memberAccess && memberAccess.Expression == node;
            SyntaxNode replacement = expressionContext
                ? SyntaxFactory.ParseExpression(fqn)
                : SyntaxFactory.ParseTypeName(fqn);
            nodeReplacements[node] = replacement.WithTriviaFrom(node);
        }

        // Unqualified static field/property/event reads of renamed types →
        // qualified back to the original type (single static state source).
        foreach (IdentifierNameSyntax identifier in root.DescendantNodes().OfType<IdentifierNameSyntax>())
        {
            if (nodeReplacements.ContainsKey(identifier) || InStrippedSpan(identifier))
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
            if (!IsRenamedTypeSymbol(symbol.ContainingType, renamedSymbols))
                continue;

            string qualified =
                symbol.ContainingType.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat) +
                "." + symbol.Name;
            nodeReplacements[identifier] = SyntaxFactory.ParseExpression(qualified).WithTriviaFrom(identifier);
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

        SyntaxNode rewritten = root.ReplaceSyntax(
            nodeReplacements.Keys,
            (original, _) => nodeReplacements[original],
            tokenReplacements.Keys,
            (original, _) => tokenReplacements[original],
            null,
            null);

        result.Tree = CSharpSyntaxTree.Create(
            (CSharpSyntaxNode)rewritten,
            parseOptions,
            path: path,
            encoding: System.Text.Encoding.UTF8);

        // Detour map: changed (non-added) members, original → patch type.
        foreach (HotDiffMethod method in diff.ChangedMethods)
        {
            if (method.Added)
                continue;
            result.Methods.Add(new PatchMethodMap
            {
                DeclaringType = method.DeclaringType,
                PatchDeclaringType = PatchTypeName(method.DeclaringType),
                Name = method.Name,
                ParamTypeNames = method.ParamTypeNames,
                IsStatic = method.IsStatic,
                IsCtor = method.IsCtor,
            });
        }

        return result;
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

    private static INamedTypeSymbol? FindOriginalType(
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
