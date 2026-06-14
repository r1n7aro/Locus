using System.Globalization;
using System.Reflection;
using System.Reflection.Metadata;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace Locus.CompileServer;

/// <summary>
/// Project-level SerializedProperty schema built from Roslyn metadata. This
/// keeps static member/attribute/type work in the sidecar and lets the Unity
/// editor return dynamic values without walking AppDomain assemblies for every
/// inspector snapshot.
/// </summary>
public static class SerializedSchemaSource
{
    private const int SchemaVersion = 1;
    private const TypeAttributes SerializableTypeAttributeFlag = (TypeAttributes)0x00002000;

    public static JsonObject Build(IEnumerable<PortableExecutableReference> references)
    {
        var referenceArray = references.ToArray();
        var compilation = CSharpCompilation.Create(
            "__LocusSerializedSchema",
            references: referenceArray,
            options: new CSharpCompilationOptions(
                OutputKind.DynamicallyLinkedLibrary,
                assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default)
                .WithMetadataImportOptions(MetadataImportOptions.All));

        INamedTypeSymbol? unityObjectType = compilation.GetTypeByMetadataName("UnityEngine.Object");
        INamedTypeSymbol? listType = compilation.GetTypeByMetadataName("System.Collections.Generic.List`1");

        HashSet<string> serializableTypeKeys = SerializableTypeKeys(referenceArray);
        List<INamedTypeSymbol> candidateTypes = compilation.SourceModule.ReferencedAssemblySymbols
            .OrderBy(a => a.Identity.Name, StringComparer.Ordinal)
            .Where(assembly => ShouldIndexAssembly(assembly.Identity.Name))
            .SelectMany(assembly => EnumerateTypes(assembly.GlobalNamespace))
            .Where(type => type.TypeKind != TypeKind.Error)
            .ToList();

        var relevantTypeKeys = new HashSet<string>(StringComparer.Ordinal);
        foreach (INamedTypeSymbol type in candidateTypes)
        {
            string key = TypeKey(TypeFullName(type), type.ContainingAssembly.Identity.Name);
            if (IsSchemaRootType(type, unityObjectType, serializableTypeKeys.Contains(key)))
                AddTypeAndBaseClosure(type, relevantTypeKeys);
        }
        AddSerializedFieldTypeClosure(candidateTypes, listType, relevantTypeKeys);

        var types = new JsonArray();
        foreach (INamedTypeSymbol type in candidateTypes)
        {
            string key = TypeKey(TypeFullName(type), type.ContainingAssembly.Identity.Name);
            if (!relevantTypeKeys.Contains(key))
                continue;

            try
            {
                types.Add(TypeToJson(type, unityObjectType, listType, serializableTypeKeys.Contains(key)));
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine(
                    "[LocusCompileServer] serialized schema skipped type " +
                    TypeFullName(type) + ": " + ex.Message);
            }
        }

        return new JsonObject
        {
            ["schemaVersion"] = SchemaVersion,
            ["count"] = types.Count,
            ["types"] = types,
        };
    }

    private static bool ShouldIndexAssembly(string assemblyName)
    {
        if (TypeIndexSource.ShouldSkipAssembly(assemblyName))
            return false;

        // Core framework metadata is still referenced by every project, but
        // serialized field owners come from Unity/project/package assemblies.
        return !assemblyName.Equals("mscorlib", StringComparison.Ordinal)
            && !assemblyName.Equals("netstandard", StringComparison.Ordinal)
            && !assemblyName.Equals("System", StringComparison.Ordinal)
            && !assemblyName.StartsWith("System.", StringComparison.Ordinal)
            && !assemblyName.StartsWith("Microsoft.", StringComparison.Ordinal)
            && !assemblyName.StartsWith("Mono.", StringComparison.Ordinal)
            && !assemblyName.Equals("UnityEditor", StringComparison.Ordinal)
            && !assemblyName.StartsWith("UnityEditor.", StringComparison.Ordinal);
    }

    private static bool IsSchemaRootType(
        INamedTypeSymbol type,
        INamedTypeSymbol? unityObjectType,
        bool hasSerializableMetadataFlag)
    {
        return type.TypeKind == TypeKind.Enum
            || IsSerializableType(type, hasSerializableMetadataFlag)
            || (!IsUnityBuiltinAssembly(type.ContainingAssembly.Identity.Name)
                && IsOrDerivesFrom(type, unityObjectType));
    }

    private static bool AddTypeAndBaseClosure(INamedTypeSymbol type, HashSet<string> keys)
    {
        bool changed = false;
        for (INamedTypeSymbol? current = type; current != null; current = current.BaseType)
        {
            string assemblyName = current.ContainingAssembly.Identity.Name;
            if (!ShouldIndexAssembly(assemblyName))
                continue;
            changed |= keys.Add(TypeKey(TypeFullName(current), assemblyName));
        }
        return changed;
    }

    private static void AddSerializedFieldTypeClosure(
        List<INamedTypeSymbol> candidateTypes,
        INamedTypeSymbol? listType,
        HashSet<string> relevantTypeKeys)
    {
        var candidateByKey = candidateTypes
            .GroupBy(type => TypeKey(TypeFullName(type), type.ContainingAssembly.Identity.Name), StringComparer.Ordinal)
            .ToDictionary(group => group.Key, group => group.First(), StringComparer.Ordinal);

        bool changed;
        do
        {
            changed = false;
            foreach (INamedTypeSymbol type in candidateTypes)
            {
                string key = TypeKey(TypeFullName(type), type.ContainingAssembly.Identity.Name);
                if (!relevantTypeKeys.Contains(key))
                    continue;

                foreach (IFieldSymbol field in SerializableFields(type))
                {
                    changed |= AddFieldTypeAndBaseClosure(field.Type, listType, relevantTypeKeys, candidateByKey);
                }
            }
        }
        while (changed);
    }

    private static bool AddFieldTypeAndBaseClosure(
        ITypeSymbol fieldType,
        INamedTypeSymbol? listType,
        HashSet<string> relevantTypeKeys,
        Dictionary<string, INamedTypeSymbol> candidateByKey)
    {
        bool changed = AddTypeSymbolAndBaseClosure(fieldType, relevantTypeKeys, candidateByKey);
        if (TryGetElementType(fieldType, listType, out ITypeSymbol? elementType) && elementType != null)
            changed |= AddTypeSymbolAndBaseClosure(elementType, relevantTypeKeys, candidateByKey);
        return changed;
    }

    private static bool AddTypeSymbolAndBaseClosure(
        ITypeSymbol type,
        HashSet<string> relevantTypeKeys,
        Dictionary<string, INamedTypeSymbol> candidateByKey)
    {
        INamedTypeSymbol? named = SchemaNamedType(type);
        if (named == null)
            return false;

        string key = TypeKey(TypeFullName(named), named.ContainingAssembly.Identity.Name);
        return candidateByKey.TryGetValue(key, out INamedTypeSymbol? candidate)
            && AddTypeAndBaseClosure(candidate, relevantTypeKeys);
    }

    private static INamedTypeSymbol? SchemaNamedType(ITypeSymbol type)
    {
        if (type is IArrayTypeSymbol array)
            return SchemaNamedType(array.ElementType);
        if (type is INamedTypeSymbol named)
            return named.OriginalDefinition;
        return null;
    }

    private static IEnumerable<INamedTypeSymbol> EnumerateTypes(INamespaceSymbol ns)
    {
        foreach (INamespaceSymbol child in ns.GetNamespaceMembers())
        {
            foreach (INamedTypeSymbol type in EnumerateTypes(child))
                yield return type;
        }

        foreach (INamedTypeSymbol type in ns.GetTypeMembers())
        {
            foreach (INamedTypeSymbol nestedOrSelf in EnumerateTypes(type))
                yield return nestedOrSelf;
        }
    }

    private static IEnumerable<INamedTypeSymbol> EnumerateTypes(INamedTypeSymbol type)
    {
        yield return type;

        foreach (INamedTypeSymbol nested in type.GetTypeMembers())
        {
            foreach (INamedTypeSymbol nestedOrSelf in EnumerateTypes(nested))
                yield return nestedOrSelf;
        }
    }

    private static JsonObject TypeToJson(
        INamedTypeSymbol type,
        INamedTypeSymbol? unityObjectType,
        INamedTypeSymbol? listType,
        bool hasSerializableMetadataFlag)
    {
        var interfaces = new JsonArray();
        foreach (INamedTypeSymbol iface in type.AllInterfaces.OrderBy(t => TypeFullName(t), StringComparer.Ordinal))
            interfaces.Add(TypeRefToJson(iface));

        var fields = new JsonArray();
        foreach (IFieldSymbol field in SerializableFields(type))
        {
            try
            {
                fields.Add(FieldToJson(field, listType));
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine(
                    "[LocusCompileServer] serialized schema skipped field " +
                    TypeFullName(type) + "." + field.Name + ": " + ex.Message);
            }
        }

        return new JsonObject
        {
            ["fullName"] = TypeFullName(type),
            ["assembly"] = type.ContainingAssembly.Identity.Name,
            ["baseTypeFullName"] = TypeFullName(type.BaseType),
            ["baseTypeAssembly"] = TypeAssembly(type.BaseType),
            ["interfaces"] = interfaces,
            ["isSerializable"] = IsSerializableType(type, hasSerializableMetadataFlag),
            ["isAbstract"] = type.IsAbstract,
            ["isInterface"] = type.TypeKind == TypeKind.Interface,
            ["isGenericTypeDefinition"] = type.TypeParameters.Length > 0,
            ["containsGenericParameters"] = ContainsGenericParameters(type),
            ["isUnityObject"] = IsOrDerivesFrom(type, unityObjectType),
            ["isFlagsEnum"] = HasAttribute(type.GetAttributes(), "System.FlagsAttribute"),
            ["enumOptions"] = EnumOptionsToJson(type),
            ["fields"] = fields,
        };
    }

    private static IEnumerable<IFieldSymbol> SerializableFields(INamedTypeSymbol type)
    {
        return type.GetMembers()
            .OfType<IFieldSymbol>()
            .Where(field =>
                !field.IsStatic
                && !field.IsConst
                && field.Name != "value__"
                && IsUnitySerializedField(field))
            .OrderBy(field => field.MetadataName, StringComparer.Ordinal);
    }

    private static bool IsUnitySerializedField(IFieldSymbol field)
    {
        AttributeData[] attributes = field.GetAttributes().ToArray();
        if (HasAttribute(attributes, "System.NonSerializedAttribute"))
            return false;
        if (HasAttribute(attributes, "UnityEngine.SerializeField")
            || HasAttribute(attributes, "UnityEngine.SerializeReference"))
            return true;
        if (field.IsImplicitlyDeclared || field.Name.Contains("k__BackingField", StringComparison.Ordinal))
            return false;
        return field.DeclaredAccessibility == Accessibility.Public;
    }

    private static JsonObject FieldToJson(IFieldSymbol field, INamedTypeSymbol? listType)
    {
        ITypeSymbol fieldType = field.Type;
        bool hasElement = TryGetElementType(fieldType, listType, out ITypeSymbol? elementType);
        ITypeSymbol serializedType = elementType ?? fieldType;
        AttributeData[] attributes = field.GetAttributes().ToArray();

        var serializedAttributes = new JsonArray();
        for (int i = 0; i < Math.Min(attributes.Length, 32); i++)
            serializedAttributes.Add(AttributeToJson(attributes[i]));

        return new JsonObject
        {
            ["name"] = field.Name,
            ["fieldTypeFullName"] = TypeFullName(fieldType),
            ["fieldTypeAssembly"] = TypeAssembly(fieldType),
            ["elementTypeFullName"] = hasElement ? TypeFullName(serializedType) : "",
            ["elementTypeAssembly"] = hasElement ? TypeAssembly(serializedType) : "",
            ["isArray"] = fieldType is IArrayTypeSymbol,
            ["isList"] = IsListType(fieldType, listType),
            ["hasSerializeReference"] = HasAttribute(attributes, "UnityEngine.SerializeReference"),
            ["isFlagsEnum"] = IsFlagsEnum(serializedType),
            ["enumOptions"] = EnumOptionsToJson(serializedType),
            ["tooltip"] = FirstStringArgument(attributes, "UnityEngine.TooltipAttribute"),
            ["header"] = FirstStringArgument(attributes, "UnityEngine.HeaderAttribute"),
            ["hasRange"] = HasAttribute(attributes, "UnityEngine.RangeAttribute"),
            ["rangeMin"] = FirstNumericArgument(attributes, "UnityEngine.RangeAttribute", 0),
            ["rangeMax"] = FirstNumericArgument(attributes, "UnityEngine.RangeAttribute", 1),
            ["multiline"] = HasAttribute(attributes, "UnityEngine.TextAreaAttribute")
                || HasAttribute(attributes, "UnityEngine.MultilineAttribute"),
            ["minLines"] = MultilineMinLines(attributes),
            ["maxLines"] = MultilineMaxLines(attributes),
            ["attributes"] = serializedAttributes,
        };
    }

    private static JsonObject TypeRefToJson(INamedTypeSymbol type)
    {
        return new JsonObject
        {
            ["fullName"] = TypeFullName(type),
            ["assembly"] = type.ContainingAssembly.Identity.Name,
        };
    }

    private static JsonObject AttributeToJson(AttributeData attr)
    {
        string typeName = TypeFullName(attr.AttributeClass);
        string displayName = attr.AttributeClass?.Name ?? "";
        return new JsonObject
        {
            ["type"] = typeName,
            ["displayName"] = displayName,
            ["value"] = AttributeValue(attr, typeName),
        };
    }

    private static JsonArray EnumOptionsToJson(ITypeSymbol type)
    {
        var options = new JsonArray();
        if (type is not INamedTypeSymbol named || named.TypeKind != TypeKind.Enum)
            return options;

        int index = 0;
        foreach (IFieldSymbol field in EnumFieldsInRuntimeOrder(named))
        {
            int optionIndex = index++;
            options.Add(new JsonObject
            {
                ["index"] = optionIndex,
                ["name"] = field.Name,
                ["label"] = ObjectNamesNicifyVariableName(field.Name),
                ["value"] = optionIndex.ToString(CultureInfo.InvariantCulture),
                ["numericValue"] = EnumNumericValueOrFallback(field.ConstantValue, optionIndex),
            });
        }
        return options;
    }

    private static IEnumerable<IFieldSymbol> EnumFieldsInRuntimeOrder(INamedTypeSymbol type)
    {
        return type.GetMembers()
            .OfType<IFieldSymbol>()
            .Where(field => field.HasConstantValue && field.Name != "value__")
            .Select((field, declarationIndex) => new
            {
                Field = field,
                DeclarationIndex = declarationIndex,
                SortKey = EnumUnsignedSortKey(field.ConstantValue, declarationIndex),
            })
            .OrderBy(entry => entry.SortKey)
            .ThenBy(entry => entry.DeclarationIndex)
            .Select(entry => entry.Field);
    }

    private static ulong EnumUnsignedSortKey(object? value, int fallback)
    {
        try
        {
            return value switch
            {
                sbyte typed => unchecked((byte)typed),
                byte typed => typed,
                short typed => unchecked((ushort)typed),
                ushort typed => typed,
                int typed => unchecked((uint)typed),
                uint typed => typed,
                long typed => unchecked((ulong)typed),
                ulong typed => typed,
                _ => Convert.ToUInt64(value, CultureInfo.InvariantCulture),
            };
        }
        catch
        {
            return (ulong)Math.Max(fallback, 0);
        }
    }

    private static long EnumNumericValueOrFallback(object? value, int fallback)
    {
        try
        {
            return Convert.ToInt64(value, CultureInfo.InvariantCulture);
        }
        catch
        {
            return fallback;
        }
    }

    private static string AttributeValue(AttributeData attr, string typeName)
    {
        if (AttributeNameMatches(typeName, "UnityEngine.RangeAttribute") && attr.ConstructorArguments.Length >= 2)
            return FormatConstant(attr.ConstructorArguments[0].Value)
                + ".."
                + FormatConstant(attr.ConstructorArguments[1].Value);
        if ((AttributeNameMatches(typeName, "UnityEngine.TooltipAttribute")
                || AttributeNameMatches(typeName, "UnityEngine.HeaderAttribute"))
            && attr.ConstructorArguments.Length >= 1)
            return FormatConstant(attr.ConstructorArguments[0].Value);
        if (AttributeNameMatches(typeName, "UnityEngine.TextAreaAttribute") && attr.ConstructorArguments.Length >= 2)
            return FormatConstant(attr.ConstructorArguments[0].Value)
                + ".."
                + FormatConstant(attr.ConstructorArguments[1].Value);
        if (AttributeNameMatches(typeName, "UnityEngine.MultilineAttribute") && attr.ConstructorArguments.Length >= 1)
            return FormatConstant(attr.ConstructorArguments[0].Value);
        if (AttributeNameMatches(typeName, "UnityEngine.MinAttribute") && attr.ConstructorArguments.Length >= 1)
            return FormatConstant(attr.ConstructorArguments[0].Value);

        return "";
    }

    private static string FormatConstant(object? value)
    {
        return value == null
            ? ""
            : Convert.ToString(value, CultureInfo.InvariantCulture) ?? "";
    }

    private static string FirstStringArgument(AttributeData[] attributes, string typeName)
    {
        AttributeData? attr = attributes.FirstOrDefault(attr => AttributeNameMatches(AttributeTypeFullName(attr), typeName));
        return attr != null && attr.ConstructorArguments.Length >= 1
            ? FormatConstant(attr.ConstructorArguments[0].Value)
            : "";
    }

    private static double FirstNumericArgument(AttributeData[] attributes, string typeName, int index)
    {
        AttributeData? attr = attributes.FirstOrDefault(attr => AttributeNameMatches(AttributeTypeFullName(attr), typeName));
        if (attr == null || attr.ConstructorArguments.Length <= index || attr.ConstructorArguments[index].Value == null)
            return 0d;
        return Convert.ToDouble(attr.ConstructorArguments[index].Value, CultureInfo.InvariantCulture);
    }

    private static int MultilineMinLines(AttributeData[] attributes)
    {
        AttributeData? textArea = attributes.FirstOrDefault(attr => AttributeNameMatches(AttributeTypeFullName(attr), "UnityEngine.TextAreaAttribute"));
        if (textArea != null && textArea.ConstructorArguments.Length >= 1)
            return Convert.ToInt32(textArea.ConstructorArguments[0].Value, CultureInfo.InvariantCulture);

        AttributeData? multiline = attributes.FirstOrDefault(attr => AttributeNameMatches(AttributeTypeFullName(attr), "UnityEngine.MultilineAttribute"));
        if (multiline != null && multiline.ConstructorArguments.Length >= 1)
            return Convert.ToInt32(multiline.ConstructorArguments[0].Value, CultureInfo.InvariantCulture);

        return 0;
    }

    private static int MultilineMaxLines(AttributeData[] attributes)
    {
        AttributeData? textArea = attributes.FirstOrDefault(attr => AttributeNameMatches(AttributeTypeFullName(attr), "UnityEngine.TextAreaAttribute"));
        return textArea != null && textArea.ConstructorArguments.Length >= 2
            ? Convert.ToInt32(textArea.ConstructorArguments[1].Value, CultureInfo.InvariantCulture)
            : 0;
    }

    private static bool TryGetElementType(ITypeSymbol type, INamedTypeSymbol? listType, out ITypeSymbol? elementType)
    {
        if (type is IArrayTypeSymbol array)
        {
            elementType = array.ElementType;
            return true;
        }

        if (type is INamedTypeSymbol named
            && named.TypeArguments.Length == 1
            && IsListType(type, listType))
        {
            elementType = named.TypeArguments[0];
            return true;
        }

        elementType = null;
        return false;
    }

    private static bool IsListType(ITypeSymbol type, INamedTypeSymbol? listType)
    {
        return type is INamedTypeSymbol named
            && named.TypeArguments.Length == 1
            && IsListDefinition(named, listType);
    }

    private static bool IsListDefinition(INamedTypeSymbol named, INamedTypeSymbol? listType)
    {
        if (listType != null && SymbolEqualityComparer.Default.Equals(named.OriginalDefinition, listType))
            return true;

        INamedTypeSymbol definition = named.OriginalDefinition;
        return definition.MetadataName == "List`1"
            && string.Equals(
                definition.ContainingNamespace?.ToDisplayString(),
                "System.Collections.Generic",
                StringComparison.Ordinal);
    }

    private static bool IsSerializableType(INamedTypeSymbol type, bool hasSerializableMetadataFlag)
    {
        return type.TypeKind == TypeKind.Enum
            || hasSerializableMetadataFlag
            || HasAttribute(type.GetAttributes(), "System.SerializableAttribute");
    }

    private static bool IsFlagsEnum(ITypeSymbol type)
    {
        return type is INamedTypeSymbol named
            && named.TypeKind == TypeKind.Enum
            && HasAttribute(named.GetAttributes(), "System.FlagsAttribute");
    }

    private static bool IsOrDerivesFrom(INamedTypeSymbol type, INamedTypeSymbol? baseType)
    {
        if (baseType == null)
            return false;

        for (INamedTypeSymbol? current = type; current != null; current = current.BaseType)
        {
            if (SymbolEqualityComparer.Default.Equals(current.OriginalDefinition, baseType))
                return true;
        }
        return false;
    }

    private static bool IsUnityBuiltinAssembly(string assemblyName)
    {
        return assemblyName.Equals("UnityEngine", StringComparison.Ordinal)
            || assemblyName.StartsWith("UnityEngine.", StringComparison.Ordinal)
            || assemblyName.Equals("UnityEditor", StringComparison.Ordinal)
            || assemblyName.StartsWith("UnityEditor.", StringComparison.Ordinal);
    }

    private static bool ContainsGenericParameters(INamedTypeSymbol type)
    {
        if (type.TypeParameters.Length > 0)
            return true;
        return type.TypeArguments.Any(arg => arg is INamedTypeSymbol named && ContainsGenericParameters(named));
    }

    private static bool HasAttribute(IEnumerable<AttributeData> attributes, string fullName)
    {
        return attributes.Any(attr => AttributeNameMatches(AttributeTypeFullName(attr), fullName));
    }

    private static bool AttributeNameMatches(string actual, string expected)
    {
        if (string.Equals(actual, expected, StringComparison.Ordinal))
            return true;
        const string suffix = "Attribute";
        if (expected.EndsWith(suffix, StringComparison.Ordinal))
        {
            string expectedWithoutSuffix = expected.Substring(0, expected.Length - suffix.Length);
            return string.Equals(actual, expectedWithoutSuffix, StringComparison.Ordinal);
        }
        return string.Equals(actual, expected + suffix, StringComparison.Ordinal);
    }

    private static string AttributeTypeFullName(AttributeData attr)
    {
        return TypeFullName(attr.AttributeClass);
    }

    private static string TypeAssembly(ITypeSymbol? type)
    {
        if (type is IArrayTypeSymbol array)
            return TypeAssembly(array.ElementType);
        return type?.ContainingAssembly?.Identity.Name ?? "";
    }

    private static HashSet<string> SerializableTypeKeys(IEnumerable<PortableExecutableReference> references)
    {
        var serializable = new HashSet<string>(StringComparer.Ordinal);
        foreach (PortableExecutableReference reference in references)
        {
            try
            {
                if (reference.GetMetadata() is not AssemblyMetadata assembly)
                    continue;
                MetadataReader reader = assembly.GetModules()[0].GetMetadataReader();
                if (!reader.IsAssembly)
                    continue;

                string assemblyName = reader.GetString(reader.GetAssemblyDefinition().Name);
                foreach (TypeDefinitionHandle handle in reader.TypeDefinitions)
                {
                    TypeDefinition typeDef = reader.GetTypeDefinition(handle);
                    if ((typeDef.Attributes & SerializableTypeAttributeFlag) == 0)
                        continue;
                    serializable.Add(TypeKey(MetadataTypeFullName(reader, handle), assemblyName));
                }
            }
            catch
            {
            }
        }
        return serializable;
    }

    private static string MetadataTypeFullName(MetadataReader reader, TypeDefinitionHandle handle)
    {
        TypeDefinition typeDef = reader.GetTypeDefinition(handle);
        string name = reader.GetString(typeDef.Name);
        TypeDefinitionHandle declaring = typeDef.GetDeclaringType();
        if (!declaring.IsNil)
            return MetadataTypeFullName(reader, declaring) + "+" + name;

        string ns = typeDef.Namespace.IsNil ? "" : reader.GetString(typeDef.Namespace);
        return ns.Length == 0 ? name : ns + "." + name;
    }

    private static string TypeKey(string fullName, string assembly)
    {
        return assembly.Trim() + "|" + fullName.Trim();
    }

    private static string TypeFullName(ITypeSymbol? type)
    {
        if (type == null)
            return "";
        if (type is IArrayTypeSymbol array)
            return TypeFullName(array.ElementType) + "[]";
        if (type is not INamedTypeSymbol named)
            return type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat).Replace("global::", "", StringComparison.Ordinal);

        INamedTypeSymbol definition = named.OriginalDefinition;
        var names = new Stack<string>();
        for (INamedTypeSymbol? current = definition; current != null; current = current.ContainingType)
            names.Push(current.MetadataName);
        string nested = string.Join("+", names);
        string ns = definition.ContainingNamespace.IsGlobalNamespace
            ? ""
            : definition.ContainingNamespace.ToDisplayString();
        string baseName = ns.Length == 0 ? nested : ns + "." + nested;
        if (named.TypeArguments.Length == 0
            || named.TypeArguments.All(arg => arg.TypeKind == TypeKind.TypeParameter))
            return baseName;

        string args = string.Join(
            ",",
            named.TypeArguments.Select(arg => "["
                + TypeFullName(arg)
                + ", "
                + AssemblyQualifiedName(arg.ContainingAssembly.Identity)
                + "]"));
        return baseName + "[" + args + "]";
    }

    private static string AssemblyQualifiedName(AssemblyIdentity identity)
    {
        string version = identity.Version.ToString();
        string culture = string.IsNullOrEmpty(identity.CultureName)
            ? "neutral"
            : identity.CultureName;
        string publicKeyToken = identity.PublicKeyToken.IsDefaultOrEmpty
            ? "null"
            : string.Concat(identity.PublicKeyToken.Select(b => b.ToString("x2", CultureInfo.InvariantCulture)));
        return identity.Name
            + ", Version=" + version
            + ", Culture=" + culture
            + ", PublicKeyToken=" + publicKeyToken;
    }

    private static string ObjectNamesNicifyVariableName(string value)
    {
        if (string.IsNullOrEmpty(value))
            return "";

        value = value.Trim();
        while (value.StartsWith("_", StringComparison.Ordinal))
            value = value.Substring(1);
        if (value.StartsWith("m_", StringComparison.Ordinal) && value.Length > 2)
            value = value.Substring(2);
        if (string.IsNullOrEmpty(value))
            return "";

        var chars = new List<char>(value.Length + 8);
        for (int i = 0; i < value.Length; i++)
        {
            char c = value[i];
            char previous = i > 0 ? value[i - 1] : '\0';
            bool wordBreak = i > 0
                && c != '_'
                && previous != '_'
                && char.IsUpper(c)
                && (char.IsLower(previous) || (i + 1 < value.Length && char.IsLower(value[i + 1])));
            if (wordBreak)
                chars.Add(' ');
            chars.Add(c == '_' ? ' ' : c);
        }

        string result = new string(chars.ToArray()).Trim();
        if (result.Length == 0)
            return "";
        char[] resultChars = result.ToCharArray();
        bool capitalizeNext = true;
        for (int i = 0; i < resultChars.Length; i++)
        {
            if (resultChars[i] == ' ')
            {
                capitalizeNext = true;
                continue;
            }
            if (capitalizeNext)
            {
                resultChars[i] = char.ToUpperInvariant(resultChars[i]);
                capitalizeNext = false;
            }
        }
        return new string(resultChars);
    }
}
