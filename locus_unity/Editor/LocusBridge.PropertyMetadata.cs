using System;
using System.Collections.Concurrent;
using System.Reflection;
using UnityEditor;

namespace Locus
{
    public static partial class LocusBridge
    {
        private static readonly ConcurrentDictionary<string, Type> PropertyManagedTypes = new ConcurrentDictionary<string, Type>();
        private static readonly ConcurrentDictionary<string, SerializedManagedReferenceTypeOption[]> PropertyManagedChoices = new ConcurrentDictionary<string, SerializedManagedReferenceTypeOption[]>();
        private static readonly ConcurrentDictionary<Tuple<Type, string>, Lazy<FieldInfo>> PropertyMemberFields = new ConcurrentDictionary<Tuple<Type, string>, Lazy<FieldInfo>>();
        private static readonly bool PropertyMetadataReloadHook = WatchPropertyMetadataAssemblies();

        private static bool WatchPropertyMetadataAssemblies()
        {
            AppDomain.CurrentDomain.AssemblyLoad += (sender, args) => {
                PropertyManagedTypes.Clear(); PropertyManagedChoices.Clear(); PropertyMemberFields.Clear();
            };
            return true;
        }

        private static Type ResolveManagedReferenceTypeName(string name)
        {
            name = (name ?? "").Trim();
            if (name.Length == 0) return null;
            Type type;
            if (PropertyManagedTypes.TryGetValue(name, out type)) return type;
            type = ResolveManagedReferenceTypeNameUncached(name);
            if (type != null) PropertyManagedTypes[name] = type;
            return type;
        }

        private static SerializedManagedReferenceTypeOption[] ManagedReferenceTypeOptions(SerializedProperty prop)
        {
            string key = prop.managedReferenceFieldTypename + "|" + prop.managedReferenceFullTypename;
            return PropertyManagedChoices.GetOrAdd(key, ignored => ManagedReferenceTypeOptionsUncached(prop));
        }

        private static FieldInfo SerializedMemberField(Type owner, string name)
        {
            if (owner == null) return null;
            return PropertyMemberFields.GetOrAdd(Tuple.Create(owner, name), key => new Lazy<FieldInfo>(() => SerializedMemberFieldUncached(key.Item1, key.Item2))).Value;
        }
    }
}
