using System;
using System.Collections.Generic;
using UnityEditor;
using UnityEngine;
using UnityEngine.LowLevel;
using UnityEngine.SceneManagement;

namespace Locus
{
    /// <summary>
    /// Drives Unity message methods ADDED by a hot patch after the engine had
    /// already fixed each type's message set at load (so it never dispatches them
    /// natively). Only the parameterless per-frame PlayerLoop callbacks are
    /// pumped — <c>Update</c>, <c>LateUpdate</c> and <c>FixedUpdate</c>. Messages
    /// that fire from an engine thread with engine-supplied arguments
    /// (physics/input/animation/audio) or run in their own loop (OnGUI) are out
    /// of scope and stay on the cold path; the sidecar never registers them here.
    ///
    /// The pump injects itself into the global PlayerLoop ONLY while at least one
    /// message is registered, and strips exactly its own systems again when the
    /// last one is removed (and on every domain reload, since the statics reset).
    /// A project that never hot-adds a message therefore never has its PlayerLoop
    /// touched, and because it only ever adds/removes its own marker systems it
    /// does not disturb anything else in the loop (e.g. a user project's UniTask).
    /// Driving is play-mode only — these callbacks do not fire in edit mode — so a
    /// registration made in edit mode is armed when play mode begins.
    ///
    /// Registrations are keyed by SOURCE FILE: <see cref="ClearSource"/> drops a
    /// file's drivers so the caller can replace-not-accumulate on every patch (a
    /// deleted/changed-away message stops; a re-edited one re-binds to the fresh
    /// shim). Per frame it reuses a cached instance list (invalidated on hierarchy
    /// / scene change, with a frame-bounded safety refresh) rather than scanning
    /// the scene, and calls a precompiled delegate rather than reflection.
    /// </summary>
    internal static class LocusMessagePump
    {
        // Empty marker types identify the systems this pump owns inside the loop.
        private struct LocusUpdate { }
        private struct LocusLateUpdate { }
        private struct LocusFixedUpdate { }

        private sealed class Entry
        {
            public Type DeclaringType;     // MonoBehaviour subtype whose live instances are driven
            public string SourcePath;      // edited file that registered it (replace-by-source key)
            public Action<object> Invoke;  // precompiled (object instance) => Shim((T)instance)
        }

        // phase name → live registrations. Touched only on the main thread
        // (hot-patch apply and the PlayerLoop tick), so no locking is needed.
        private static readonly Dictionary<string, List<Entry>> Entries =
            new Dictionary<string, List<Entry>>(StringComparer.Ordinal)
            {
                { "Update", new List<Entry>() },
                { "LateUpdate", new List<Entry>() },
                { "FixedUpdate", new List<Entry>() },
            };

        private static readonly HashSet<Type> Markers =
            new HashSet<Type> { typeof(LocusUpdate), typeof(LocusLateUpdate), typeof(LocusFixedUpdate) };

        // Per-frame instance cache (P2⑤): one scene scan per type fills it; it is
        // invalidated on hierarchy/scene changes and at least every TtlFrames so a
        // freshly spawned instance is picked up without scanning every frame.
        private static readonly Dictionary<Type, UnityEngine.Object[]> InstanceCache =
            new Dictionary<Type, UnityEngine.Object[]>();
        private static readonly Dictionary<Type, int> ExecutionOrder = new Dictionary<Type, int>();
        private static readonly HashSet<string> DirtyPhases = new HashSet<string>(StringComparer.Ordinal);
        private const int TtlFrames = 30;
        // Anchor at -TtlFrames (NOT int.MinValue): with int.MinValue the very first
        // `frame - _cacheFrame` overflows (unchecked) to a large negative, so the
        // `>= TtlFrames` refresh never fires and _cacheFrame stays pinned forever —
        // the TTL safety refresh is silently dead. -TtlFrames makes the first call
        // (frame >= 0) refresh and anchor cleanly.
        private static int _cacheFrame = -TtlFrames;

        private static bool _installed;
        private static bool _hooked;

        /// <summary>Register (or replace) the driver for one added message on one
        /// MonoBehaviour type, installing the pump lazily on the first
        /// registration. Returns false (and registers nothing) when the type is not
        /// a MonoBehaviour — then the added method is an ordinary method, not an
        /// engine message, and the caller counts it as skipped rather than driven.
        /// <paramref name="invoke"/> is a precompiled delegate that calls the shim
        /// with one instance.</summary>
        public static bool Register(string message, Type declaringType, string sourcePath, Action<object> invoke)
        {
            if (declaringType == null || invoke == null)
                return false;
            if (!Entries.TryGetValue(message, out List<Entry> list))
                return false;   // not a PlayerLoop-pumpable phase
            if (!typeof(MonoBehaviour).IsAssignableFrom(declaringType))
                return false;   // only MonoBehaviour subtypes receive engine messages

            HookOnce();

            // Replace any prior driver for the SAME type+message so a re-edited
            // message picks up the new shim instead of stacking duplicate calls.
            list.RemoveAll(e => e.DeclaringType == declaringType);
            list.Add(new Entry { DeclaringType = declaringType, SourcePath = sourcePath ?? "", Invoke = invoke });
            DirtyPhases.Add(message);

            if (Application.isPlaying)
                EnsureInstalled();
            return true;
        }

        /// <summary>Drop every driver that was registered from <paramref name="sourcePath"/>.
        /// The caller clears each file a patch touched before re-adding, so a
        /// deleted or changed-away message stops being driven.</summary>
        public static void ClearSource(string sourcePath)
        {
            if (string.IsNullOrEmpty(sourcePath))
                return;
            foreach (List<Entry> list in Entries.Values)
                list.RemoveAll(e => e.SourcePath == sourcePath);
            if (TotalEntries() == 0)
                Uninstall();
        }

        /// <summary>Drop every registration and uninstall. Used on teardown.</summary>
        public static void Clear()
        {
            foreach (List<Entry> list in Entries.Values)
                list.Clear();
            Uninstall();
        }

        private static int TotalEntries()
        {
            int n = 0;
            foreach (List<Entry> list in Entries.Values)
                n += list.Count;
            return n;
        }

        private static void HookOnce()
        {
            if (_hooked)
                return;
            _hooked = true;
            EditorApplication.playModeStateChanged += OnPlayModeStateChanged;
            // Cache invalidation: anything that adds/removes/reparents objects.
            EditorApplication.hierarchyChanged += InvalidateInstanceCache;
            SceneManager.sceneLoaded += OnSceneLoaded;
            SceneManager.sceneUnloaded += OnSceneUnloaded;
        }

        private static void OnSceneLoaded(Scene scene, LoadSceneMode mode) => InvalidateInstanceCache();
        private static void OnSceneUnloaded(Scene scene) => InvalidateInstanceCache();
        private static void InvalidateInstanceCache() => InstanceCache.Clear();

        private static void OnPlayModeStateChanged(PlayModeStateChange change)
        {
            if (change == PlayModeStateChange.EnteredPlayMode)
            {
                // Entering play mode rebuilds the loop from scratch; re-arm if any
                // message is still pending from edit mode or a prior session.
                _installed = false;
                InvalidateInstanceCache();
                if (TotalEntries() > 0)
                    EnsureInstalled();
            }
            else if (change == PlayModeStateChange.ExitingPlayMode)
            {
                // Unity resets the PlayerLoop on exit; forget our injection and the
                // now-stale instance cache.
                _installed = false;
                InvalidateInstanceCache();
            }
        }

        private static void EnsureInstalled()
        {
            if (_installed || TotalEntries() == 0)
                return;
            PlayerLoopSystem loop = PlayerLoop.GetCurrentPlayerLoop();
            loop = WithDriver(loop, typeof(UnityEngine.PlayerLoop.Update), TickUpdate, typeof(LocusUpdate));
            loop = WithDriver(loop, typeof(UnityEngine.PlayerLoop.PreLateUpdate), TickLateUpdate, typeof(LocusLateUpdate));
            loop = WithDriver(loop, typeof(UnityEngine.PlayerLoop.FixedUpdate), TickFixedUpdate, typeof(LocusFixedUpdate));
            PlayerLoop.SetPlayerLoop(loop);
            _installed = true;
        }

        private static void Uninstall()
        {
            if (!_installed)
                return;
            PlayerLoopSystem loop = WithoutMarkers(PlayerLoop.GetCurrentPlayerLoop());
            PlayerLoop.SetPlayerLoop(loop);
            _installed = false;
        }

        /// <summary>Return a copy of <paramref name="root"/> with one driver system
        /// appended under the given phase (idempotent — a no-op if already present).
        /// Touched arrays are cloned so no other system in the loop is disturbed.</summary>
        private static PlayerLoopSystem WithDriver(
            PlayerLoopSystem root, Type phaseType, PlayerLoopSystem.UpdateFunction driver, Type marker)
        {
            if (root.subSystemList == null)
                return root;
            PlayerLoopSystem[] top = (PlayerLoopSystem[])root.subSystemList.Clone();
            for (int i = 0; i < top.Length; i++)
            {
                if (top[i].type != phaseType)
                    continue;
                PlayerLoopSystem[] children = top[i].subSystemList ?? Array.Empty<PlayerLoopSystem>();
                for (int k = 0; k < children.Length; k++)
                {
                    if (children[k].type == marker)
                    {
                        root.subSystemList = top;   // already installed
                        return root;
                    }
                }
                var grown = new PlayerLoopSystem[children.Length + 1];
                Array.Copy(children, grown, children.Length);
                grown[children.Length] = new PlayerLoopSystem { type = marker, updateDelegate = driver };
                top[i].subSystemList = grown;
                break;
            }
            root.subSystemList = top;
            return root;
        }

        /// <summary>Return a copy of <paramref name="root"/> with every system this
        /// pump owns stripped out, leaving all other systems exactly in place.</summary>
        private static PlayerLoopSystem WithoutMarkers(PlayerLoopSystem root)
        {
            if (root.subSystemList == null)
                return root;
            PlayerLoopSystem[] top = (PlayerLoopSystem[])root.subSystemList.Clone();
            for (int i = 0; i < top.Length; i++)
            {
                PlayerLoopSystem[] children = top[i].subSystemList;
                if (children == null)
                    continue;
                bool owns = false;
                for (int k = 0; k < children.Length; k++)
                {
                    if (children[k].type != null && Markers.Contains(children[k].type))
                    {
                        owns = true;
                        break;
                    }
                }
                if (!owns)
                    continue;
                var kept = new List<PlayerLoopSystem>(children.Length);
                for (int k = 0; k < children.Length; k++)
                {
                    if (children[k].type == null || !Markers.Contains(children[k].type))
                        kept.Add(children[k]);
                }
                top[i].subSystemList = kept.ToArray();
            }
            root.subSystemList = top;
            return root;
        }

        private static void TickUpdate() => Drive("Update");
        private static void TickLateUpdate() => Drive("LateUpdate");
        private static void TickFixedUpdate() => Drive("FixedUpdate");

        private static void Drive(string phase)
        {
            if (!Entries.TryGetValue(phase, out List<Entry> list) || list.Count == 0)
                return;

            // Approximate Script Execution Order: drive lower-order types first.
            // Re-sort only when the set or a learned order changed (P2④).
            if (DirtyPhases.Remove(phase))
                list.Sort(CompareByExecutionOrder);

            for (int i = 0; i < list.Count; i++)
            {
                Entry entry = list[i];
                UnityEngine.Object[] instances = InstancesOf(entry.DeclaringType);
                for (int j = 0; j < instances.Length; j++)
                {
                    UnityEngine.Object obj = instances[j];
                    // Skip destroyed (Unity-null) and disabled instances, matching
                    // how the engine gates the real message callbacks.
                    if (obj == null)
                        continue;
                    if (obj is Behaviour behaviour && !behaviour.isActiveAndEnabled)
                        continue;
                    try
                    {
                        entry.Invoke(obj);
                    }
                    catch (Exception ex)
                    {
                        // One faulting instance must not stop the loop or the
                        // other instances — surface it and keep driving.
                        Debug.LogException(ex);
                    }
                }
            }
        }

        private static int CompareByExecutionOrder(Entry a, Entry b)
        {
            int oa = ExecutionOrder.TryGetValue(a.DeclaringType, out int va) ? va : 0;
            int ob = ExecutionOrder.TryGetValue(b.DeclaringType, out int vb) ? vb : 0;
            return oa.CompareTo(ob);
        }

        /// <summary>Live instances of <paramref name="type"/>, served from a cache
        /// that is refreshed on hierarchy/scene change and at least every
        /// <see cref="TtlFrames"/> frames. Instances come back in a stable
        /// (InstanceID) order rather than arbitrary.</summary>
        private static UnityEngine.Object[] InstancesOf(Type type)
        {
            int frame = Time.frameCount;
            if (frame - _cacheFrame >= TtlFrames)
            {
                InstanceCache.Clear();
                _cacheFrame = frame;
            }
            if (!InstanceCache.TryGetValue(type, out UnityEngine.Object[] instances))
            {
                // Cache INCLUDING inactive: the per-instance isActiveAndEnabled gate
                // in Drive decides each frame, so an object enabled at runtime is
                // picked up immediately instead of waiting out the cache TTL.
#if UNITY_2022_2_OR_NEWER
                instances = UnityEngine.Object.FindObjectsByType(
                    type, FindObjectsInactive.Include, FindObjectsSortMode.InstanceID);
#else
                // Unity 2021 and earlier lack FindObjectsByType / FindObjectsInactive /
                // FindObjectsSortMode. FindObjectsOfType(Type, true) is the behavior
                // equivalent: includes inactive and is likewise sorted by InstanceID.
                instances = UnityEngine.Object.FindObjectsOfType(type, true);
#endif
                InstanceCache[type] = instances;
                LearnExecutionOrder(type, instances);
            }
            return instances;
        }

        private static void LearnExecutionOrder(Type type, UnityEngine.Object[] instances)
        {
            if (ExecutionOrder.ContainsKey(type))
                return;
            for (int i = 0; i < instances.Length; i++)
            {
                if (instances[i] is MonoBehaviour behaviour && behaviour != null)
                {
                    MonoScript script = MonoScript.FromMonoBehaviour(behaviour);
                    ExecutionOrder[type] = script != null ? MonoImporter.GetExecutionOrder(script) : 0;
                    // A newly learned order changes the sort for any phase holding
                    // this type — mark all phases to re-sort on their next tick.
                    foreach (string phase in Entries.Keys)
                        DirtyPhases.Add(phase);
                    return;
                }
            }
        }
    }
}
