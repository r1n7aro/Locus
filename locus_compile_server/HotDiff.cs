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
}

/// <summary>Hot/cold classification of one edited file.</summary>
public sealed class HotDiffFileResult
{
    public bool Hot;

    /// <summary>Cold reasons (empty when hot).</summary>
    public List<string> Reasons = new();

    /// <summary>Member surfaces that must pass the call-site scan (M3)
    /// before this file can be hot. Populated alongside a cold reason while
    /// the scan is not wired up.</summary>
    public List<CallerCheckMember> RequiresCallerCheck = new();

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

        Dictionary<string, BaseTypeDeclarationSyntax> oldTypes = CollectTypes(oldRoot, result.Reasons);
        Dictionary<string, BaseTypeDeclarationSyntax> newTypes = CollectTypes(newRoot, result.Reasons);
        if (result.Reasons.Count > 0)
            return result;

        foreach (string removed in oldTypes.Keys.Where(k => !newTypes.ContainsKey(k)))
            result.Reasons.Add("type removed: " + removed);
        if (result.Reasons.Count > 0)
            return result;

        foreach (var pair in newTypes)
        {
            if (!oldTypes.ContainsKey(pair.Key))
                result.NewTypes.Add(pair.Key);
        }

        foreach (var pair in newTypes)
        {
            if (!oldTypes.TryGetValue(pair.Key, out BaseTypeDeclarationSyntax? oldDecl))
                continue;

            DiffType(pair.Key, oldDecl, pair.Value, result);
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
                if (!oldTypes.ContainsKey(pair.Key) || pair.Value is not TypeDeclarationSyntax type)
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
                if (!oldTypes.ContainsKey(pair.Key) || pair.Value is not TypeDeclarationSyntax type)
                    continue;
                if (type is InterfaceDeclarationSyntax)
                    continue; // gate guarantees no default implementations
                AddRehookMembers(pair.Key, type, result);
            }
        }

        if (result.ChangedMethods.Count > 0 &&
            result.ChangedMethods.All(m => m.Added) &&
            result.NewTypes.Count == 0)
        {
            result.ChangedMethods.Clear();
            result.PatchedTypes.Clear();
            result.Reasons.Add("added helper methods require a changed original method");
            return result;
        }

        result.NewTypes.Sort(StringComparer.Ordinal);
        result.PatchedTypes = result.PatchedTypes.Distinct().OrderBy(t => t, StringComparer.Ordinal).ToList();
        result.Hot = true;
        return result;
    }

    // ── type collection ──────────────────────────────────────────────

    private static Dictionary<string, BaseTypeDeclarationSyntax> CollectTypes(
        CompilationUnitSyntax root,
        List<string> reasons)
    {
        var types = new Dictionary<string, BaseTypeDeclarationSyntax>(StringComparer.Ordinal);

        foreach (BaseTypeDeclarationSyntax decl in root.DescendantNodes().OfType<BaseTypeDeclarationSyntax>())
        {
            if (decl is RecordDeclarationSyntax)
            {
                reasons.Add("record types are not hot-reloadable: " + decl.Identifier.Text);
                return types;
            }

            // A patch source only contains this file's part: compiling it
            // would drop the other parts' members and fields.
            if (decl.Modifiers.Any(SyntaxKind.PartialKeyword))
            {
                reasons.Add("partial type in file: " + decl.Identifier.Text);
                return types;
            }

            types[MetadataName(decl)] = decl;
        }

        return types;
    }

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
        BaseTypeDeclarationSyntax oldDecl,
        BaseTypeDeclarationSyntax newDecl,
        HotDiffFileResult result)
    {
        if (oldDecl.RawKind != newDecl.RawKind)
        {
            result.Reasons.Add("type kind changed: " + metadataName);
            return;
        }

        // Enum bodies are inlined constants; delegates are pure signatures.
        if (oldDecl is EnumDeclarationSyntax || newDecl is EnumDeclarationSyntax)
        {
            if (TokenText(oldDecl) != TokenText(newDecl))
                result.Reasons.Add("enum changed: " + metadataName);
            return;
        }

        // Interfaces: default-implementation bodies dispatch through the
        // IMT, where Mono detour reliability is unverified; signature-only
        // members are pure type surface. Any interface change stays cold.
        if (oldDecl is InterfaceDeclarationSyntax)
        {
            if (TokenText(oldDecl) != TokenText(newDecl))
                result.Reasons.Add(
                    "interface changed: " + metadataName +
                    " (interface dispatch cannot be hot-patched)");
            return;
        }

        var oldType = (TypeDeclarationSyntax)oldDecl;
        var newType = (TypeDeclarationSyntax)newDecl;

        if (TypeHeaderText(oldType) != TypeHeaderText(newType))
        {
            result.Reasons.Add("type declaration changed: " + metadataName);
            return;
        }

        bool genericContext = IsGenericContext(newType);
        bool burstContext = HasBurstCompileAttribute(oldType) || HasBurstCompileAttribute(newType);

        if (!LayoutSequence(oldType).SequenceEqual(LayoutSequence(newType), StringComparer.Ordinal))
        {
            result.Reasons.Add("field layout changed: " + metadataName);
            return;
        }

        // Constants are inlined at every use site; a recompile is the only
        // way to update consumers. Static initializers ran with the original
        // domain's static constructor and would silently not re-run.
        if (ConstAndStaticInitText(oldType) != ConstAndStaticInitText(newType))
        {
            result.Reasons.Add("const or static initializer changed: " + metadataName);
            return;
        }

        bool instanceInitChanged = InstanceInitText(oldType) != InstanceInitText(newType);

        Dictionary<string, MemberDeclarationSyntax> oldMembers = ExecutableMembers(oldType, result.Reasons, metadataName);
        Dictionary<string, MemberDeclarationSyntax> newMembers = ExecutableMembers(newType, result.Reasons, metadataName);
        if (result.Reasons.Count > 0)
            return;

        foreach (var pair in oldMembers)
        {
            if (!newMembers.ContainsKey(pair.Key))
            {
                result.Reasons.Add("member removed: " + metadataName + "." + DisplayName(pair.Value));
                return;
            }
        }

        bool patched = false;

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

            int before = result.Reasons.Count;
            bool changed = DiffMember(metadataName, oldMember, newMember, genericContext, burstContext, result);
            if (result.Reasons.Count > before)
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

        if (patched)
            result.PatchedTypes.Add(metadataName);
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

    /// <summary>
    /// Ordered, layout-affecting member shapes: fields, field-like events,
    /// and auto-properties (whose backing fields live in the type's layout).
    /// Any difference in this sequence makes patched bodies read wrong
    /// offsets through original `this` pointers.
    /// </summary>
    private static List<string> LayoutSequence(TypeDeclarationSyntax type)
    {
        var sequence = new List<string>();

        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when !field.Modifiers.Any(SyntaxKind.ConstKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        sequence.Add(
                            "field|" + ModifiersText(field.Modifiers) + "|" +
                            AttributeListsText(field.AttributeLists) + "|" +
                            TokenText(field.Declaration.Type) + "|" + declarator.Identifier.Text);
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

    internal static bool IsAutoProperty(PropertyDeclarationSyntax property)
    {
        if (property.ExpressionBody != null || property.AccessorList == null)
            return false;
        return property.AccessorList.Accessors.All(a => a.Body == null && a.ExpressionBody == null);
    }

    private static string ConstAndStaticInitText(TypeDeclarationSyntax type)
    {
        var parts = new List<string>();
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when field.Modifiers.Any(SyntaxKind.ConstKeyword):
                    parts.Add("const|" + TokenText(field));
                    break;
                case FieldDeclarationSyntax field when field.Modifiers.Any(SyntaxKind.StaticKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                    {
                        if (declarator.Initializer != null)
                            parts.Add("sinit|" + declarator.Identifier.Text + "|" + TokenText(declarator.Initializer));
                    }
                    break;
                case PropertyDeclarationSyntax property when
                    IsAutoProperty(property) &&
                    property.Modifiers.Any(SyntaxKind.StaticKeyword) &&
                    property.Initializer != null:
                    parts.Add("sinit|" + property.Identifier.Text + "|" + TokenText(property.Initializer));
                    break;
            }
        }
        return string.Join("\n", parts);
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
                PropertyDeclarationSyntax property when !IsAutoProperty(property) =>
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
                // Same signature twice would not compile; treat as cold so a
                // real compile surfaces the error.
                reasons.Add("duplicate member signature: " + metadataName);
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
    /// false (with a reason) when it forces the cold path.</summary>
    private static bool ClassifyAddedMember(
        string metadataName,
        MemberDeclarationSyntax member,
        bool genericContext,
        bool burstContext,
        HotDiffFileResult result)
    {
        if (genericContext)
        {
            result.Reasons.Add("member added to generic type: " + metadataName);
            return false;
        }

        switch (member)
        {
            case ConstructorDeclarationSyntax:
                // `new Foo(...)` in patched bodies binds to the *original*
                // type, which does not have the new overload.
                result.Reasons.Add("constructor added: " + metadataName);
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
                if (!IsPrivateOrImplicitPrivate(method.Modifiers))
                {
                    result.Reasons.Add("non-private method added: " + metadataName + "." + method.Identifier.Text);
                    return false;
                }
                if (UnityMagicMethods.Contains(method.Identifier.Text))
                {
                    // Unity discovered the original type's message set at
                    // load; a new message method would never be called.
                    result.Reasons.Add("new Unity message method: " + metadataName + "." + method.Identifier.Text);
                    return false;
                }
                if ((method.TypeParameterList?.Parameters.Count ?? 0) > 0)
                {
                    result.Reasons.Add("generic method added: " + metadataName + "." + method.Identifier.Text);
                    return false;
                }
                AddMethod(
                    result, metadataName, method.Identifier.Text, ParamTypeNames(method.ParameterList),
                    method.Modifiers.Any(SyntaxKind.StaticKeyword), isCtor: false, added: true);
                return true;

            case PropertyDeclarationSyntax property:
                // Properties add metadata surface even when they are backed
                // only by methods; external call sites cannot bind to them
                // until the original assembly is recompiled.
                result.Reasons.Add("property added: " + metadataName + "." + property.Identifier.Text);
                return false;

            case IndexerDeclarationSyntax:
            case EventDeclarationSyntax:
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
            result.Reasons.Add("member declaration changed: " + metadataName + "." + DisplayName(newMember));
            return false;
        }

        // Accessibility sits outside the header: WIDENING changes nothing at
        // runtime until a real compile (original metadata keeps the old
        // accessibility; patched bodies bypass access checks anyway), so an
        // unchanged body is a noop and a changed body is a plain hot edit.
        // NARROWING can strand call sites outside this batch → caller check
        // (cold until the M3 scan is wired up).
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
            });
            result.Reasons.Add(detail + " (call-site verification is not yet enabled; use unity_recompile)");
            return false;
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
                if (genericContext || (newMethod.TypeParameterList?.Parameters.Count ?? 0) > 0)
                {
                    result.Reasons.Add("generic method body changed: " + metadataName + "." + newMethod.Identifier.Text);
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

    private static string AccessorName(AccessorDeclarationSyntax accessor, string propertyName)
    {
        // init-only accessors compile to set_X (with a modreq).
        string prefix = accessor.Keyword.Text == "get" ? "get_" : "set_";
        return prefix + propertyName;
    }

    private static string[] AccessorParams(
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
        bool added)
    {
        result.ChangedMethods.Add(new HotDiffMethod
        {
            DeclaringType = declaringType,
            Name = name,
            ParamTypeNames = paramTypeNames,
            IsStatic = isStatic,
            IsCtor = isCtor,
            Added = added,
        });
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

    private static bool HasBurstCompileAttribute(SyntaxNode node)
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

    private static string? OperatorMetadataName(string token, int paramCount)
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
