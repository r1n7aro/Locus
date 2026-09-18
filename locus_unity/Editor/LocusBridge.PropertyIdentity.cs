using System;
using System.Collections.Generic;
using UnityEditor;
using UnityEngine;

namespace Locus
{
    public static partial class LocusBridge
    {
        private static readonly string PropertyIdentitySession = Guid.NewGuid().ToString("N");
        private static readonly Dictionary<string, WeakReference> PropertyRuntimeIdentities = new Dictionary<string, WeakReference>();

        private static string PropertyTreeIdentity(UnityEngine.Object obj)
        {
            if (obj == null) return "";
            GlobalObjectId id = GlobalObjectId.GetGlobalObjectIdSlow(obj);
            if (id.targetObjectId != 0) return id.ToString();
            string key = "runtime:" + PropertyIdentitySession + ":" + LocusObjectIdentity.InstanceId(obj);
            PropertyRuntimeIdentities[key] = new WeakReference(obj);
            return key;
        }

        private static UnityEngine.Object ResolvePropertyTreeIdentity(string value)
        {
            UnityEngine.Object obj = null;
            if (value.StartsWith("runtime:", StringComparison.Ordinal))
            {
                WeakReference reference;
                if (PropertyRuntimeIdentities.TryGetValue(value, out reference)) obj = reference.Target as UnityEngine.Object;
            }
            else
            {
                GlobalObjectId id;
                if (GlobalObjectId.TryParse(value, out id)) obj = GlobalObjectId.GlobalObjectIdentifierToObjectSlow(id);
            }
            if (obj == null) throw new InvalidOperationException("Unity property target no longer exists: " + value);
            return obj;
        }
    }
}
