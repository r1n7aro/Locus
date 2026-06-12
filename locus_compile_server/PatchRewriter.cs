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
                        CollectNewType(model, typeDecl, metadataName, result);
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
                    continue;
                if (model.GetDeclaredSymbol(declarator) is not IFieldSymbol fieldSymbol)
                    continue;

                AddedFieldInfo info = BuildAddedFieldInfo(pair.Key, typeDecl, change, fieldSymbol, declarator, batch);
                addedFieldSymbols[fieldSymbol] = info;
                if (!addedFieldsByType.TryGetValue(pair.Key, out List<AddedFieldInfo>? list))
                    addedFieldsByType[pair.Key] = list = new List<AddedFieldInfo>();
                list.Add(info);
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

        // ── locate ADDED members (M2) in this file ───────────────────

        var addedDecls = new Dictionary<MethodDeclarationSyntax, ShimTarget>();
        foreach (HotDiffMethod added in diff.ChangedMethods.Where(m => m.Added))
        {
            MethodDeclarationSyntax? decl = PatchBatchContext.FindAddedMethodDeclaration(root, added);
            if (decl == null)
                continue;
            if (model.GetDeclaredSymbol(decl) is not IMethodSymbol symbol)
                continue;
            if (batch.AddedMembers.TryGetValue(symbol.OriginalDefinition, out ShimTarget? target))
                addedDecls[decl] = target;
        }

        bool InAddedMember(SyntaxNode node)
        {
            foreach (MethodDeclarationSyntax decl in addedDecls.Keys)
            {
                if (decl.FullSpan.Contains(node.Span))
                    return true;
            }
            return false;
        }

        ShimTarget? EnclosingAddedTarget(SyntaxNode node)
        {
            foreach (var pair in addedDecls)
            {
                if (pair.Key.FullSpan.Contains(node.Span))
                    return pair.Value;
            }
            return null;
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
        // field/property/event of renamed types).
        if (addedDecls.Count > 0)
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
                    batch.AddedMembers.ContainsKey(candidateMethod.OriginalDefinition))
                {
                    continue; // the shim-call rewrite owns these
                }
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

        // ── calls to ADDED members → direct shim calls (M2) ──────────
        foreach (InvocationExpressionSyntax invocation in root.DescendantNodes().OfType<InvocationExpressionSyntax>())
        {
            if (InStrippedSpan(invocation))
                continue;
            if (model.GetSymbolInfo(invocation).Symbol is not IMethodSymbol invoked)
                continue;
            if (!batch.AddedMembers.TryGetValue(invoked.OriginalDefinition, out ShimTarget? target))
                continue;

            if (invocation.Expression is MemberBindingExpressionSyntax)
            {
                result.ColdReason =
                    "added member called through ?. (conditional access): " + target.DeclaringTypeMetadataName +
                    "." + target.MethodName + " — rewrite the call without ?. or use unity_recompile";
                return result;
            }

            ShimTarget? enclosingAdded = EnclosingAddedTarget(invocation);
            ShimTarget capturedTarget = target;
            dynamicReplacements[invocation] = rewrittenNode =>
                BuildShimInvocation((InvocationExpressionSyntax)rewrittenNode, capturedTarget, enclosingAdded);
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

            SymbolInfo info = model.GetSymbolInfo(node);
            IMethodSymbol? groupSymbol = info.Symbol as IMethodSymbol;
            if (groupSymbol == null)
            {
                foreach (ISymbol candidate in info.CandidateSymbols)
                {
                    if (candidate is IMethodSymbol candidateMethod &&
                        batch.AddedMembers.ContainsKey(candidateMethod.OriginalDefinition))
                    {
                        groupSymbol = candidateMethod;
                        break;
                    }
                }
            }
            if (groupSymbol == null ||
                !batch.AddedMembers.TryGetValue(groupSymbol.OriginalDefinition, out ShimTarget? groupTarget))
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
            dynamicReplacements[node] = rewrittenNode =>
                BuildShimLambda(rewrittenNode, capturedGroupTarget, invoke, enclosingAdded);
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

        // Record index paths of added members and added-field initializers
        // BEFORE the rewrite (replace operations preserve node counts and
        // order), so the rewritten nodes can be located afterwards.
        var addedPaths = new List<(List<int> Path, ShimTarget Target)>();
        foreach (var pair in addedDecls)
            addedPaths.Add((IndexPath(pair.Key), pair.Value));
        var initializerPaths = new List<(List<int> Path, AddedFieldInfo Field)>();
        foreach (AddedFieldInfo info in addedFieldSymbols.Values)
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

        // ── extract added members into shim classes (M2) ─────────────
        if (addedPaths.Count > 0)
        {
            var extracted = new List<(MethodDeclarationSyntax Decl, ShimTarget Target)>();
            foreach (var (pathIndices, target) in addedPaths)
            {
                if (NodeAtPath(rewritten, pathIndices) is MethodDeclarationSyntax method)
                    extracted.Add((method, target));
            }

            rewritten = rewritten.RemoveNodes(
                extracted.Select(e => (SyntaxNode)e.Decl),
                SyntaxRemoveOptions.KeepNoTrivia)!;

            // One shim class per top-level type, grouped by namespace.
            foreach (var group in extracted.GroupBy(e => e.Target.ShimTypeMetadataName, StringComparer.Ordinal))
            {
                ShimTarget first = group.First().Target;
                var shimMethods = new List<MemberDeclarationSyntax>();
                foreach (var (decl, target) in group)
                    shimMethods.Add(BuildShimMethod(decl, target, root));

                string shimSimpleName = first.ShimTypeMetadataName.Contains('.')
                    ? first.ShimTypeMetadataName[(first.ShimTypeMetadataName.LastIndexOf('.') + 1)..]
                    : first.ShimTypeMetadataName;

                ClassDeclarationSyntax shimClass = SyntaxFactory.ClassDeclaration(shimSimpleName)
                    .WithModifiers(SyntaxFactory.TokenList(
                        SyntaxFactory.Token(SyntaxKind.PublicKeyword),
                        SyntaxFactory.Token(SyntaxKind.StaticKeyword)))
                    .WithMembers(SyntaxFactory.List(shimMethods))
                    .NormalizeWhitespace()
                    .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);

                rewritten = AppendTypeToNamespace((CompilationUnitSyntax)rewritten, shimClass, first.ShimNamespace);

                foreach (var (_, target) in group)
                {
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

        // ── deletions (M5) ───────────────────────────────────────────
        // Removed members stay untouched in the loaded original (in-flight
        // delegates/coroutines are legitimate callers) and tombstone in the
        // registry. Removed UNITY MESSAGE methods additionally get an
        // empty-body stub re-materialized in the patch type + a normal
        // detour mapping: the engine calls the original every frame, and
        // the stub is what makes the deletion observable immediately.
        foreach (HotDiffRemovedMember removed in diff.RemovedMembers)
        {
            result.ShimRegistrations.Add(new ShimRegistration
            {
                MemberKey = MemberSurfaceRegistry.MemberKey(
                    removed.DeclaringType, removed.Name, removed.ParamTypeNames, removed.IsStatic),
                Entry = new MemberSurfaceRegistry.ShimEntry { Kind = "tombstone" },
            });

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
            });
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

        /// <summary>False when an earlier batch already declared the store —
        /// accesses bind to it and no new declaration is generated.</summary>
        public bool IsNewStore = true;

        public ExpressionSyntax? InitializerValue;
        public string? RewrittenInitializerText;
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

        // "Ns.Outer+Inner" → holder "__LocusFields_Outer_Inner" in Ns. The
        // name stays inside the __LocusHotPatch_ assembly, so the type-index
        // skip list covers it without a new prefix entry.
        int dot = declaringType.LastIndexOf('.');
        string ns = dot < 0 ? "" : declaringType[..dot];
        string chain = (dot < 0 ? declaringType : declaringType[(dot + 1)..]).Replace('+', '_');
        string storeName = "__LocusFields_" + chain;
        info.StoreNamespace = ns;
        info.StoreMetadataName = ns.Length == 0 ? storeName : ns + "." + storeName;
        info.StoreFqn = "global::" + info.StoreMetadataName;
        _ = typeDecl;
        return info;
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
                    FieldKey = FieldStoreRegistry.FieldKey(info.DeclaringType, info.Name),
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

            parameters.Add(SyntaxFactory.Parameter(SyntaxFactory.Identifier("self"))
                .WithModifiers(SyntaxFactory.TokenList(selfModifiers))
                .WithType(SyntaxFactory.ParseTypeName(target.DeclaringTypeFqn).WithTrailingTrivia(SyntaxFactory.Space)));
        }
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

        if (target.GenericShim && target.TypeParameters.Length > 0)
        {
            shim = shim.WithTypeParameterList(SyntaxFactory.TypeParameterList(
                SyntaxFactory.SeparatedList(target.TypeParameters.Select(SyntaxFactory.TypeParameter))));

            // Carry the declaring chain's constraints so `A<T> self` stays
            // well-formed (e.g. `where T : class`).
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
            if (constraints.Count > 0)
                shim = shim.WithConstraintClauses(SyntaxFactory.List(constraints));
        }

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

    /// <summary>`expr.M(args)` / `M(args)` → `global::Ns.Foo__LocusShims.M(self, args)`.</summary>
    private static SyntaxNode BuildShimInvocation(
        InvocationExpressionSyntax rewrittenInvocation,
        ShimTarget target,
        ShimTarget? enclosingAdded)
    {
        ExpressionSyntax? receiver = rewrittenInvocation.Expression switch
        {
            MemberAccessExpressionSyntax memberAccess => memberAccess.Expression,
            _ => null,
        };

        var arguments = new List<ArgumentSyntax>();
        if (target.HasSelf)
        {
            ExpressionSyntax selfExpression;
            if (receiver == null || receiver is ThisExpressionSyntax)
            {
                selfExpression = enclosingAdded != null
                    ? SyntaxFactory.IdentifierName("self")
                    : SyntaxFactory.ParseExpression("((" + target.DeclaringTypeFqn + ")(object)this)");
            }
            else
            {
                selfExpression = receiver.WithoutTrivia();
            }

            ArgumentSyntax selfArgument = SyntaxFactory.Argument(selfExpression);
            if (target.SelfIsValueType && !target.SelfIsRefLike)
                selfArgument = selfArgument.WithRefKindKeyword(SyntaxFactory.Token(SyntaxKind.RefKeyword));
            arguments.Add(selfArgument);
        }
        arguments.AddRange(rewrittenInvocation.ArgumentList.Arguments);

        return SyntaxFactory.InvocationExpression(
                SyntaxFactory.ParseExpression(target.ShimTypeFqn + "." + target.MethodName),
                SyntaxFactory.ArgumentList(SyntaxFactory.SeparatedList(arguments)))
            .WithTriviaFrom(rewrittenInvocation);
    }

    /// <summary>Method group `foo.M` → `(a0, ...) => Shims.M(foo, a0, ...)`.
    /// The receiver evaluates at INVOCATION time instead of delegate
    /// creation, and delegate equality differs — both documented.</summary>
    private static SyntaxNode BuildShimLambda(
        SyntaxNode rewrittenGroup,
        ShimTarget target,
        IMethodSymbol invoke,
        ShimTarget? enclosingAdded)
    {
        ExpressionSyntax? receiver = rewrittenGroup switch
        {
            MemberAccessExpressionSyntax memberAccess => memberAccess.Expression,
            _ => null,
        };

        var lambdaParams = new List<ParameterSyntax>();
        var callArgs = new List<ArgumentSyntax>();

        if (target.HasSelf)
        {
            ExpressionSyntax selfExpression;
            if (receiver == null || receiver is ThisExpressionSyntax)
            {
                selfExpression = enclosingAdded != null
                    ? SyntaxFactory.IdentifierName("self")
                    : SyntaxFactory.ParseExpression("((" + target.DeclaringTypeFqn + ")(object)this)");
            }
            else
            {
                selfExpression = receiver.WithoutTrivia();
            }
            ArgumentSyntax selfArgument = SyntaxFactory.Argument(selfExpression);
            if (target.SelfIsValueType && !target.SelfIsRefLike)
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
            SyntaxFactory.ParseExpression(target.ShimTypeFqn + "." + target.MethodName),
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
