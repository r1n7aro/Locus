using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;

using Newtonsoft.Json;

namespace Locus.Json
{
    /// <summary>
    /// JSON bridge used inside the Unity Editor.
    ///
    /// Serialization projects stored state into a finite object graph. A BFS
    /// assigns each reference object to its shallowest first occurrence. Shared
    /// and cyclic edges emit $ref, while the owning occurrence emits $id. Values
    /// come from fields and compiler backing fields only; target property getters
    /// and deferred/custom enumerators are never executed.
    /// </summary>
    public static class LocusJson
    {
        private static readonly JsonSerializerSettings DeserializeSettings =
            new JsonSerializerSettings
            {
                ConstructorHandling = ConstructorHandling.AllowNonPublicDefaultConstructor,
                Culture = CultureInfo.InvariantCulture,
                DateParseHandling = DateParseHandling.None,
                MetadataPropertyHandling = MetadataPropertyHandling.Ignore,
                MissingMemberHandling = MissingMemberHandling.Ignore,
                NullValueHandling = NullValueHandling.Include,
                ObjectCreationHandling = ObjectCreationHandling.Replace,
                TypeNameHandling = TypeNameHandling.None
            };

        public static object Deserialize(string json, Type type)
        {
            if (type == null)
                throw new ArgumentNullException("type");

            string source = string.IsNullOrWhiteSpace(json) ? "{}" : json;
            return JsonConvert.DeserializeObject(source, type, DeserializeSettings);
        }

        public static T Deserialize<T>(string json)
        {
            object value = Deserialize(json, typeof(T));
            return value == null ? default(T) : (T)value;
        }

        public static string Serialize(object value)
        {
            GraphPlan plan = GraphPlan.Build(value);
            var output = new StringWriter(CultureInfo.InvariantCulture);
            using (var writer = new JsonTextWriter(output))
            {
                writer.CloseOutput = false;
                writer.Culture = CultureInfo.InvariantCulture;
                writer.Formatting = Formatting.None;
                WriteGraph(writer, value, plan);
                writer.Flush();
            }
            return output.ToString();
        }

        private enum NodeKind
        {
            Object,
            Dictionary,
            Sequence,
            UnityObject,
            DeferredEnumerable,
            Opaque
        }

        private sealed class PlannedEdge
        {
            internal string JsonName;
            internal object Value;
            internal bool ReadFailed;
            internal string ReadErrorType;
            internal PlannedNode TargetNode;
            internal bool IsReference;
            internal NodeKind InlineKind;
            internal List<PlannedEdge> InlineEdges;
        }

        private sealed class PlannedNode
        {
            internal int Id;
            internal object Value;
            internal Type Type;
            internal NodeKind Kind;
            internal List<PlannedEdge> Edges;
            internal bool Expanded;
            internal bool RequiresId;
        }

        private sealed class PendingLocation
        {
            internal object Value;
            internal PlannedNode Node;
            internal PlannedEdge InlineOwner;
            internal bool IsRoot;
        }

        private sealed class StoredMember
        {
            internal string Name;
            internal FieldInfo Field;

            internal bool TryRead(object owner, out object value, out string errorType)
            {
                value = null;
                errorType = "";
                try
                {
                    value = Field.GetValue(owner);
                    return true;
                }
                catch (Exception ex)
                {
                    errorType = ex.GetType().FullName ?? ex.GetType().Name;
                    return false;
                }
            }
        }

        private sealed class ReferenceIdentityComparer : IEqualityComparer<object>
        {
            internal static readonly ReferenceIdentityComparer Instance =
                new ReferenceIdentityComparer();

            public new bool Equals(object x, object y)
            {
                return ReferenceEquals(x, y);
            }

            public int GetHashCode(object value)
            {
                return RuntimeHelpers.GetHashCode(value);
            }
        }

        private sealed class GraphPlan
        {
            private readonly Dictionary<object, PlannedNode> _nodes =
                new Dictionary<object, PlannedNode>(ReferenceIdentityComparer.Instance);
            private readonly Dictionary<Type, StoredMember[]> _storedMembers =
                new Dictionary<Type, StoredMember[]>();
            private readonly Queue<PendingLocation> _pending =
                new Queue<PendingLocation>();
            private int _nextNodeId = 1;

            internal PlannedNode RootNode;
            internal NodeKind RootInlineKind;
            internal List<PlannedEdge> RootInlineEdges;

            private GraphPlan()
            {
            }

            internal static GraphPlan Build(object root)
            {
                var plan = new GraphPlan();
                plan.BindRoot(root);
                while (plan._pending.Count > 0)
                    plan.Expand(plan._pending.Dequeue());
                return plan;
            }

            private void BindRoot(object value)
            {
                if (value == null)
                    return;

                Type type = value.GetType();
                if (IsInlineValue(value, type))
                    return;

                NodeKind kind = DetermineKind(value, type);
                if (!type.IsValueType)
                {
                    RootNode = CreateNode(value, type, kind);
                    _nodes.Add(value, RootNode);
                    if (IsExpandable(kind))
                    {
                        _pending.Enqueue(new PendingLocation
                        {
                            Value = value,
                            Node = RootNode
                        });
                    }
                    return;
                }

                RootInlineKind = kind;
                if (IsExpandable(kind))
                {
                    _pending.Enqueue(new PendingLocation
                    {
                        Value = value,
                        IsRoot = true
                    });
                }
            }

            private void BindEdge(PlannedEdge edge)
            {
                object value = edge.Value;
                if (value == null)
                    return;

                Type type = value.GetType();
                if (IsInlineValue(value, type))
                    return;

                NodeKind kind = DetermineKind(value, type);
                if (!type.IsValueType)
                {
                    PlannedNode existing;
                    if (_nodes.TryGetValue(value, out existing))
                    {
                        existing.RequiresId = true;
                        edge.TargetNode = existing;
                        edge.IsReference = true;
                        return;
                    }

                    PlannedNode node = CreateNode(value, type, kind);
                    _nodes.Add(value, node);
                    edge.TargetNode = node;
                    if (IsExpandable(kind))
                    {
                        _pending.Enqueue(new PendingLocation
                        {
                            Value = value,
                            Node = node
                        });
                    }
                    return;
                }

                edge.InlineKind = kind;
                if (IsExpandable(kind))
                {
                    _pending.Enqueue(new PendingLocation
                    {
                        Value = value,
                        InlineOwner = edge
                    });
                }
            }

            private PlannedNode CreateNode(object value, Type type, NodeKind kind)
            {
                return new PlannedNode
                {
                    Id = _nextNodeId++,
                    Value = value,
                    Type = type,
                    Kind = kind,
                    Edges = new List<PlannedEdge>()
                };
            }

            private void Expand(PendingLocation location)
            {
                NodeKind kind;
                if (location.Node != null)
                {
                    if (location.Node.Expanded)
                        return;
                    location.Node.Expanded = true;
                    kind = location.Node.Kind;
                }
                else
                {
                    kind = DetermineKind(location.Value, location.Value.GetType());
                }

                List<PlannedEdge> edges = SnapshotEdges(location.Value, kind);
                if (location.Node != null)
                    location.Node.Edges = edges;
                else if (location.InlineOwner != null)
                    location.InlineOwner.InlineEdges = edges;
                else if (location.IsRoot)
                    RootInlineEdges = edges;

                for (int i = 0; i < edges.Count; i++)
                {
                    if (!edges[i].ReadFailed)
                        BindEdge(edges[i]);
                }
            }

            private List<PlannedEdge> SnapshotEdges(object value, NodeKind kind)
            {
                switch (kind)
                {
                    case NodeKind.Dictionary:
                        return SnapshotDictionary((IDictionary)value);
                    case NodeKind.Sequence:
                        return SnapshotSequence((IEnumerable)value);
                    case NodeKind.Object:
                        return SnapshotObject(value);
                    default:
                        return new List<PlannedEdge>();
                }
            }

            private static List<PlannedEdge> SnapshotDictionary(IDictionary dictionary)
            {
                var edges = new List<PlannedEdge>();
                var usedNames = new HashSet<string>(StringComparer.Ordinal);
                IDictionaryEnumerator enumerator = dictionary.GetEnumerator();
                int index = 0;
                try
                {
                    while (enumerator.MoveNext())
                    {
                        DictionaryEntry entry = enumerator.Entry;
                        string name = UniqueDictionaryName(
                            StoredDictionaryKey(entry.Key, index),
                            usedNames);
                        edges.Add(new PlannedEdge
                        {
                            JsonName = name,
                            Value = entry.Value
                        });
                        index++;
                    }
                }
                finally
                {
                    IDisposable disposable = enumerator as IDisposable;
                    if (disposable != null)
                        disposable.Dispose();
                }
                return edges;
            }

            private static List<PlannedEdge> SnapshotSequence(IEnumerable sequence)
            {
                var edges = new List<PlannedEdge>();
                IEnumerator enumerator = sequence.GetEnumerator();
                try
                {
                    while (enumerator.MoveNext())
                        edges.Add(new PlannedEdge { Value = enumerator.Current });
                }
                finally
                {
                    IDisposable disposable = enumerator as IDisposable;
                    if (disposable != null)
                        disposable.Dispose();
                }
                return edges;
            }

            private List<PlannedEdge> SnapshotObject(object value)
            {
                StoredMember[] members = MembersFor(value.GetType());
                var edges = new List<PlannedEdge>(members.Length);
                for (int i = 0; i < members.Length; i++)
                {
                    StoredMember member = members[i];
                    object memberValue;
                    string errorType;
                    bool read = member.TryRead(value, out memberValue, out errorType);
                    edges.Add(new PlannedEdge
                    {
                        JsonName = member.Name,
                        Value = memberValue,
                        ReadFailed = !read,
                        ReadErrorType = errorType
                    });
                }
                return edges;
            }

            private StoredMember[] MembersFor(Type type)
            {
                StoredMember[] cached;
                if (_storedMembers.TryGetValue(type, out cached))
                    return cached;

                var members = new List<StoredMember>();
                var names = new HashSet<string>(StringComparer.Ordinal);
                FieldInfo[] fields = type.GetFields(BindingFlags.Instance | BindingFlags.Public);
                Array.Sort(fields, CompareMetadataOrder);
                for (int i = 0; i < fields.Length; i++)
                {
                    FieldInfo field = fields[i];
                    if (field.IsStatic || field.IsNotSerialized || !names.Add(field.Name))
                        continue;
                    members.Add(new StoredMember { Name = field.Name, Field = field });
                }

                PropertyInfo[] properties = type.GetProperties(
                    BindingFlags.Instance | BindingFlags.Public);
                Array.Sort(properties, CompareMetadataOrder);
                for (int i = 0; i < properties.Length; i++)
                {
                    PropertyInfo property = properties[i];
                    if (property.GetIndexParameters().Length != 0 || names.Contains(property.Name))
                        continue;

                    FieldInfo backingField = AutoPropertyBackingField(property);
                    if (backingField == null || backingField.IsStatic)
                        continue;
                    names.Add(property.Name);
                    members.Add(new StoredMember
                    {
                        Name = property.Name,
                        Field = backingField
                    });
                }

                cached = members.ToArray();
                _storedMembers[type] = cached;
                return cached;
            }

            private static FieldInfo AutoPropertyBackingField(PropertyInfo property)
            {
                Type owner = property.DeclaringType;
                if (owner == null)
                    return null;

                const BindingFlags flags = BindingFlags.Instance
                    | BindingFlags.Public
                    | BindingFlags.NonPublic;
                FieldInfo field = owner.GetField(
                    "<" + property.Name + ">k__BackingField",
                    flags);
                if (field != null)
                    return field;

                // Roslyn anonymous types use i__Field.
                return owner.GetField("<" + property.Name + ">i__Field", flags);
            }

            private static int CompareMetadataOrder(MemberInfo left, MemberInfo right)
            {
                int moduleOrder = string.CompareOrdinal(
                    left.Module.ScopeName,
                    right.Module.ScopeName);
                if (moduleOrder != 0)
                    return moduleOrder;
                int tokenOrder = left.MetadataToken.CompareTo(right.MetadataToken);
                return tokenOrder != 0
                    ? tokenOrder
                    : string.CompareOrdinal(left.Name, right.Name);
            }
        }

        private sealed class WriteFrame
        {
            internal bool IsContainer;
            internal bool IsSequence;
            internal bool WrappedSequence;
            internal object Value;
            internal PlannedNode Node;
            internal PlannedEdge Edge;
            internal NodeKind InlineKind;
            internal List<PlannedEdge> InlineEdges;
            internal List<PlannedEdge> Edges;
            internal int NextIndex;
        }

        private static void WriteGraph(JsonTextWriter writer, object root, GraphPlan plan)
        {
            var frames = new Stack<WriteFrame>();
            frames.Push(new WriteFrame
            {
                Value = root,
                Node = plan.RootNode,
                InlineKind = plan.RootInlineKind,
                InlineEdges = plan.RootInlineEdges
            });

            while (frames.Count > 0)
            {
                WriteFrame frame = frames.Pop();
                if (frame.IsContainer)
                    ContinueContainer(writer, frame, frames);
                else
                    WriteOneValue(writer, frame, frames);
            }
        }

        private static void WriteOneValue(
            JsonTextWriter writer,
            WriteFrame frame,
            Stack<WriteFrame> frames)
        {
            PlannedEdge edge = frame.Edge;
            if (edge != null && edge.ReadFailed)
            {
                writer.WriteStartObject();
                writer.WritePropertyName("$readError");
                writer.WriteValue(edge.ReadErrorType ?? "member read failed");
                writer.WriteEndObject();
                return;
            }

            if (edge != null && edge.IsReference)
            {
                WriteReference(writer, edge.TargetNode.Id);
                return;
            }

            PlannedNode node = edge != null && edge.TargetNode != null
                ? edge.TargetNode
                : frame.Node;
            object value = node != null
                ? node.Value
                : edge != null ? edge.Value : frame.Value;
            if (value == null)
            {
                writer.WriteNull();
                return;
            }

            Type type = value.GetType();
            if (TryWriteInline(writer, value, type))
                return;

            NodeKind kind = node != null
                ? node.Kind
                : edge != null ? edge.InlineKind : frame.InlineKind;
            switch (kind)
            {
                case NodeKind.UnityObject:
                    WriteUnityObject(writer, value, type, node);
                    return;
                case NodeKind.DeferredEnumerable:
                    WriteDescriptor(writer, "$deferredEnumerable", type, node);
                    return;
                case NodeKind.Opaque:
                    WriteDescriptor(writer, "$opaque", type, node);
                    return;
            }

            List<PlannedEdge> edges = node != null
                ? node.Edges
                : edge != null ? edge.InlineEdges : frame.InlineEdges;
            bool isSequence = kind == NodeKind.Sequence;
            bool wrappedSequence = isSequence && node != null && node.RequiresId;
            if (isSequence)
            {
                if (wrappedSequence)
                {
                    writer.WriteStartObject();
                    WriteNodeId(writer, node.Id);
                    writer.WritePropertyName("$values");
                }
                writer.WriteStartArray();
            }
            else
            {
                writer.WriteStartObject();
                if (node != null && node.RequiresId)
                    WriteNodeId(writer, node.Id);
            }

            frames.Push(new WriteFrame
            {
                IsContainer = true,
                IsSequence = isSequence,
                WrappedSequence = wrappedSequence,
                Edges = edges ?? new List<PlannedEdge>(),
                NextIndex = 0
            });
        }

        private static void ContinueContainer(
            JsonTextWriter writer,
            WriteFrame frame,
            Stack<WriteFrame> frames)
        {
            if (frame.NextIndex >= frame.Edges.Count)
            {
                if (frame.IsSequence)
                {
                    writer.WriteEndArray();
                    if (frame.WrappedSequence)
                        writer.WriteEndObject();
                }
                else
                {
                    writer.WriteEndObject();
                }
                return;
            }

            PlannedEdge edge = frame.Edges[frame.NextIndex++];
            frames.Push(frame);
            if (!frame.IsSequence)
                writer.WritePropertyName(edge.JsonName);
            frames.Push(new WriteFrame { Edge = edge });
        }

        private static void WriteNodeId(JsonTextWriter writer, int id)
        {
            writer.WritePropertyName("$id");
            writer.WriteValue(id);
        }

        private static void WriteReference(JsonTextWriter writer, int id)
        {
            writer.WriteStartObject();
            writer.WritePropertyName("$ref");
            writer.WriteValue(id);
            writer.WriteEndObject();
        }

        private static bool TryWriteInline(JsonTextWriter writer, object value, Type type)
        {
            if (value is string)
            {
                writer.WriteValue((string)value);
                return true;
            }
            if (value is byte[])
            {
                writer.WriteValue((byte[])value);
                return true;
            }
            if (type.IsEnum)
            {
                writer.WriteValue(value.ToString());
                return true;
            }
            if (value is Guid)
            {
                writer.WriteValue((Guid)value);
                return true;
            }
            if (value is TimeSpan)
            {
                writer.WriteValue((TimeSpan)value);
                return true;
            }
            if (value is DateTimeOffset)
            {
                writer.WriteValue((DateTimeOffset)value);
                return true;
            }
            if (value is Uri)
            {
                writer.WriteValue(((Uri)value).OriginalString);
                return true;
            }

            switch (Type.GetTypeCode(type))
            {
                case TypeCode.Boolean: writer.WriteValue((bool)value); return true;
                case TypeCode.Char: writer.WriteValue((char)value); return true;
                case TypeCode.SByte: writer.WriteValue((sbyte)value); return true;
                case TypeCode.Byte: writer.WriteValue((byte)value); return true;
                case TypeCode.Int16: writer.WriteValue((short)value); return true;
                case TypeCode.UInt16: writer.WriteValue((ushort)value); return true;
                case TypeCode.Int32: writer.WriteValue((int)value); return true;
                case TypeCode.UInt32: writer.WriteValue((uint)value); return true;
                case TypeCode.Int64: writer.WriteValue((long)value); return true;
                case TypeCode.UInt64: writer.WriteValue((ulong)value); return true;
                case TypeCode.Single: writer.WriteValue((float)value); return true;
                case TypeCode.Double: writer.WriteValue((double)value); return true;
                case TypeCode.Decimal: writer.WriteValue((decimal)value); return true;
                case TypeCode.DateTime: writer.WriteValue((DateTime)value); return true;
                default: return false;
            }
        }

        private static bool IsInlineValue(object value, Type type)
        {
            return value is string
                || value is byte[]
                || value is Guid
                || value is TimeSpan
                || value is DateTimeOffset
                || value is Uri
                || type.IsEnum
                || Type.GetTypeCode(type) != TypeCode.Object;
        }

        private static NodeKind DetermineKind(object value, Type type)
        {
            if (IsUnityObjectType(type))
                return NodeKind.UnityObject;
            if (IsOpaqueType(type))
                return NodeKind.Opaque;
            if (type.IsArray)
                return NodeKind.Sequence;
            if (value is IDictionary && IsMaterializedCollectionType(type))
                return NodeKind.Dictionary;
            if (value is IEnumerable)
            {
                return IsMaterializedCollectionType(type)
                    ? NodeKind.Sequence
                    : NodeKind.DeferredEnumerable;
            }
            return NodeKind.Object;
        }

        private static bool IsExpandable(NodeKind kind)
        {
            return kind == NodeKind.Object
                || kind == NodeKind.Dictionary
                || kind == NodeKind.Sequence;
        }

        private static bool IsMaterializedCollectionType(Type type)
        {
            if (type.IsArray)
                return true;

            string name = type.IsGenericType
                ? type.GetGenericTypeDefinition().FullName
                : type.FullName;
            switch (name)
            {
                case "System.Collections.ArrayList":
                case "System.Collections.Hashtable":
                case "System.Collections.Queue":
                case "System.Collections.SortedList":
                case "System.Collections.Stack":
                case "System.Collections.Generic.Dictionary`2":
                case "System.Collections.Generic.HashSet`1":
                case "System.Collections.Generic.LinkedList`1":
                case "System.Collections.Generic.List`1":
                case "System.Collections.Generic.Queue`1":
                case "System.Collections.Generic.SortedDictionary`2":
                case "System.Collections.Generic.SortedList`2":
                case "System.Collections.Generic.SortedSet`1":
                case "System.Collections.Generic.Stack`1":
                case "System.Collections.ObjectModel.Collection`1":
                case "System.Collections.ObjectModel.ReadOnlyCollection`1":
                case "System.Collections.ObjectModel.ReadOnlyDictionary`2":
                case "System.Collections.Concurrent.ConcurrentBag`1":
                case "System.Collections.Concurrent.ConcurrentDictionary`2":
                case "System.Collections.Concurrent.ConcurrentQueue`1":
                case "System.Collections.Concurrent.ConcurrentStack`1":
                    return true;
                default:
                    return false;
            }
        }

        private static bool IsOpaqueType(Type type)
        {
            return typeof(Delegate).IsAssignableFrom(type)
                || typeof(Exception).IsAssignableFrom(type)
                || typeof(Stream).IsAssignableFrom(type)
                || typeof(MemberInfo).IsAssignableFrom(type)
                || typeof(System.Threading.Tasks.Task).IsAssignableFrom(type)
                || type == typeof(Type)
                || type == typeof(IntPtr)
                || type == typeof(UIntPtr);
        }

        private static bool IsUnityObjectType(Type type)
        {
            for (Type current = type; current != null; current = current.BaseType)
            {
                if (string.Equals(current.FullName, "UnityEngine.Object", StringComparison.Ordinal))
                    return true;
            }
            return false;
        }

        private static void WriteDescriptor(
            JsonTextWriter writer,
            string property,
            Type type,
            PlannedNode node)
        {
            writer.WriteStartObject();
            if (node != null && node.RequiresId)
                WriteNodeId(writer, node.Id);
            writer.WritePropertyName(property);
            writer.WriteValue(type.FullName ?? type.Name);
            writer.WriteEndObject();
        }

        private static void WriteUnityObject(
            JsonTextWriter writer,
            object value,
            Type type,
            PlannedNode node)
        {
            writer.WriteStartObject();
            if (node != null && node.RequiresId)
                WriteNodeId(writer, node.Id);
            writer.WritePropertyName("$unityObject");
            writer.WriteValue(true);
            writer.WritePropertyName("type");
            writer.WriteValue(type.FullName ?? type.Name);

            int instanceId;
            if (TryReadUnityInstanceId(value, type, out instanceId))
            {
                writer.WritePropertyName("instanceId");
                writer.WriteValue(instanceId);
            }

            string assetPath = TryReadUnityAssetPath(value);
            if (!string.IsNullOrEmpty(assetPath))
            {
                writer.WritePropertyName("assetPath");
                writer.WriteValue(assetPath);
            }
            writer.WriteEndObject();
        }

        private static bool TryReadUnityInstanceId(object value, Type type, out int instanceId)
        {
            instanceId = 0;
            try
            {
                MethodInfo method = type.GetMethod(
                    "GetInstanceID",
                    BindingFlags.Instance | BindingFlags.Public,
                    null,
                    Type.EmptyTypes,
                    null);
                if (method == null)
                    return false;
                object result = method.Invoke(value, null);
                if (!(result is int))
                    return false;
                instanceId = (int)result;
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static string TryReadUnityAssetPath(object value)
        {
            try
            {
                MethodInfo method = FindAssetDatabaseGetAssetPath();
                return method == null ? null : method.Invoke(null, new[] { value }) as string;
            }
            catch
            {
                return null;
            }
        }

        private static MethodInfo FindAssetDatabaseGetAssetPath()
        {
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            for (int i = 0; i < assemblies.Length; i++)
            {
                Type assetDatabaseType = assemblies[i].GetType(
                    "UnityEditor.AssetDatabase",
                    false);
                if (assetDatabaseType == null)
                    continue;

                MethodInfo[] methods = assetDatabaseType.GetMethods(
                    BindingFlags.Static | BindingFlags.Public);
                for (int j = 0; j < methods.Length; j++)
                {
                    MethodInfo method = methods[j];
                    if (!string.Equals(method.Name, "GetAssetPath", StringComparison.Ordinal))
                        continue;
                    ParameterInfo[] parameters = method.GetParameters();
                    if (parameters.Length == 1
                        && string.Equals(
                            parameters[0].ParameterType.FullName,
                            "UnityEngine.Object",
                            StringComparison.Ordinal))
                    {
                        return method;
                    }
                }
            }
            return null;
        }

        private static string StoredDictionaryKey(object key, int index)
        {
            if (key == null)
                return "null";
            if (key is string)
                return (string)key;

            Type type = key.GetType();
            if (type.IsEnum
                || key is Guid
                || key is TimeSpan
                || key is DateTimeOffset
                || Type.GetTypeCode(type) != TypeCode.Object)
            {
                return Convert.ToString(key, CultureInfo.InvariantCulture);
            }

            return "$key" + index.ToString(CultureInfo.InvariantCulture)
                + ":" + (type.FullName ?? type.Name);
        }

        private static string UniqueDictionaryName(string name, HashSet<string> used)
        {
            string baseName = name ?? "null";
            if (used.Add(baseName))
                return baseName;

            int suffix = 2;
            string candidate;
            do
            {
                candidate = baseName + "#" + suffix.ToString(CultureInfo.InvariantCulture);
                suffix++;
            }
            while (!used.Add(candidate));
            return candidate;
        }
    }
}
