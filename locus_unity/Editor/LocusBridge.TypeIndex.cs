using UnityEngine;

using System;
using System.IO;
using System.Reflection;
using System.Collections.Generic;

namespace Locus
{
    public static partial class LocusBridge
    {
        [Serializable]
        private sealed class TypeIndexExport
        {
            public string fingerprint;
            public TypeIndexEntry[] types;
        }

        [Serializable]
        private sealed class TypeIndexFingerprintExport
        {
            public string fingerprint;
        }

        [Serializable]
        private sealed class TypeIndexEntry
        {
            public string simpleName;
            public string ns;
            public string fullName;
            public string assembly;
        }

        private static string ExportTypeIndexJson()
        {
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            Array.Sort(assemblies, CompareAssembliesByName);

            var entries = new List<TypeIndexEntry>(16384);
            var seen = new HashSet<string>(StringComparer.Ordinal);
            ulong fingerprint = 1469598103934665603UL;

            for (int i = 0; i < assemblies.Length; i++)
            {
                Assembly asm = assemblies[i];
                if (asm == null || asm.IsDynamic)
                    continue;

                string assemblyName = SafeAssemblyName(asm);
                if (ShouldSkipTypeIndexAssembly(assemblyName))
                    continue;

                fingerprint = HashString(fingerprint, assemblyName);
                fingerprint = HashString(fingerprint, SafeAssemblyLocation(asm));
                fingerprint = HashString(fingerprint, SafeAssemblyMvid(asm));
                fingerprint = HashString(fingerprint, SafeAssemblyWriteStamp(asm));

                Type[] types = SafeGetAssemblyTypes(asm);
                if (types == null)
                    continue;

                for (int j = 0; j < types.Length; j++)
                {
                    Type type = types[j];
                    if (type == null || type.IsNested || !type.IsPublic)
                        continue;

                    string ns = type.Namespace ?? "";
                    string simpleName = StripGenericArity(type.Name);
                    if (string.IsNullOrEmpty(simpleName))
                        continue;

                    string fullName = string.IsNullOrEmpty(ns)
                        ? simpleName
                        : ns + "." + simpleName;

                    string key = fullName;
                    if (!seen.Add(key))
                        continue;

                    entries.Add(new TypeIndexEntry
                    {
                        simpleName = simpleName,
                        ns = ns,
                        fullName = fullName,
                        assembly = assemblyName
                    });
                }
            }

            entries.Sort(CompareTypeIndexEntries);

            var export = new TypeIndexExport
            {
                fingerprint = fingerprint.ToString("x16"),
                types = entries.ToArray()
            };
            return JsonUtility.ToJson(export);
        }

        private static string ExportTypeIndexFingerprintJson()
        {
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            Array.Sort(assemblies, CompareAssembliesByName);

            ulong fingerprint = 1469598103934665603UL;
            for (int i = 0; i < assemblies.Length; i++)
            {
                Assembly asm = assemblies[i];
                if (asm == null || asm.IsDynamic)
                    continue;

                string assemblyName = SafeAssemblyName(asm);
                if (ShouldSkipTypeIndexAssembly(assemblyName))
                    continue;

                fingerprint = HashString(fingerprint, assemblyName);
                fingerprint = HashString(fingerprint, SafeAssemblyLocation(asm));
                fingerprint = HashString(fingerprint, SafeAssemblyMvid(asm));
                fingerprint = HashString(fingerprint, SafeAssemblyWriteStamp(asm));
            }

            var export = new TypeIndexFingerprintExport
            {
                fingerprint = fingerprint.ToString("x16")
            };
            return JsonUtility.ToJson(export);
        }

        private static int CompareAssembliesByName(Assembly a, Assembly b)
        {
            return string.Compare(SafeAssemblyName(a), SafeAssemblyName(b), StringComparison.Ordinal);
        }

        private static int CompareTypeIndexEntries(TypeIndexEntry a, TypeIndexEntry b)
        {
            int byName = string.Compare(a.simpleName, b.simpleName, StringComparison.Ordinal);
            if (byName != 0)
                return byName;

            int byNamespace = string.Compare(a.ns, b.ns, StringComparison.Ordinal);
            if (byNamespace != 0)
                return byNamespace;

            return string.Compare(a.assembly, b.assembly, StringComparison.Ordinal);
        }

        private static bool ShouldSkipTypeIndexAssembly(string assemblyName)
        {
            if (string.IsNullOrEmpty(assemblyName))
                return true;

            return assemblyName.StartsWith("__LocusRuntimeAsync_", StringComparison.Ordinal)
                || assemblyName.StartsWith("__LocusRunStates_", StringComparison.Ordinal)
                || assemblyName == "Locus.Editor"
                || assemblyName.StartsWith("Microsoft.CodeAnalysis", StringComparison.Ordinal)
                || assemblyName == "System.Collections.Immutable"
                || assemblyName == "System.Reflection.Metadata";
        }

        private static string SafeAssemblyName(Assembly asm)
        {
            try
            {
                return asm == null ? "" : asm.GetName().Name ?? "";
            }
            catch
            {
                return "";
            }
        }

        private static string SafeAssemblyLocation(Assembly asm)
        {
            try
            {
                return asm == null || asm.IsDynamic ? "" : asm.Location ?? "";
            }
            catch
            {
                return "";
            }
        }

        private static string SafeAssemblyMvid(Assembly asm)
        {
            try
            {
                return asm == null ? "" : asm.ManifestModule.ModuleVersionId.ToString("N");
            }
            catch
            {
                return "";
            }
        }

        private static string SafeAssemblyWriteStamp(Assembly asm)
        {
            string location = SafeAssemblyLocation(asm);
            if (string.IsNullOrEmpty(location) || !File.Exists(location))
                return "";

            try
            {
                return File.GetLastWriteTimeUtc(location).Ticks.ToString();
            }
            catch
            {
                return "";
            }
        }

        private static Type[] SafeGetAssemblyTypes(Assembly asm)
        {
            try
            {
                return asm.GetTypes();
            }
            catch (ReflectionTypeLoadException ex)
            {
                return ex.Types;
            }
            catch
            {
                return null;
            }
        }

        private static string StripGenericArity(string name)
        {
            if (string.IsNullOrEmpty(name))
                return "";

            int tick = name.IndexOf('`');
            return tick >= 0 ? name.Substring(0, tick) : name;
        }

        private static ulong HashString(ulong hash, string value)
        {
            if (string.IsNullOrEmpty(value))
                return HashByte(hash, 0);

            for (int i = 0; i < value.Length; i++)
            {
                char c = value[i];
                hash = HashByte(hash, (byte)(c & 0xff));
                hash = HashByte(hash, (byte)(c >> 8));
            }
            return HashByte(hash, 0);
        }

        private static ulong HashByte(ulong hash, byte value)
        {
            hash ^= value;
            return hash * 1099511628211UL;
        }
    }
}
