using System;
using System.Collections.Generic;
using UnityEditor;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace Locus
{
    /// <summary>
    /// Base for the forwarding proxies the <see cref="LocusMessageProxyHub"/>
    /// attaches to target GameObjects so an engine message ADDED after load (the
    /// engine's message set is fixed at load) still reaches the patch shim. The
    /// proxy sits on the same object as the target component, so Unity delivers
    /// exactly the events that component would have received natively, and forwards
    /// each through a compiled delegate. Concrete subclasses exist per family so a
    /// proxy attached for one message does not pull in another family's cost or
    /// side effect (e.g. declaring OnAnimatorMove disables auto root motion;
    /// OnGUI adds an IMGUI pass; mouse-hover needs per-frame raycasts).
    ///
    /// Hidden and never serialized; the hub owns the lifetime.
    /// </summary>
    internal abstract class LocusProxyBase : MonoBehaviour
    {
        public struct Forwarder
        {
            public UnityEngine.Object Target;        // the original component instance
            public Action<object, object> Invoke;    // (target, engineArg) => Shim((T)target [, (A)engineArg])
        }

        private Dictionary<string, List<Forwarder>> _forwarders;
        private bool _suppressDestroyForward;

        public void SetForwarders(Dictionary<string, List<Forwarder>> forwarders) => _forwarders = forwarders;

        // Set by the hub before it tears a proxy down, so the resulting OnDestroy
        // is not mistaken for the target object actually being destroyed.
        public void SuppressDestroyForward() => _suppressDestroyForward = true;
        protected bool DestroyForwardSuppressed => _suppressDestroyForward;

        protected void Dispatch(string message, object engineArg)
        {
            if (_forwarders == null || !_forwarders.TryGetValue(message, out List<Forwarder> list))
                return;
            for (int i = 0; i < list.Count; i++)
            {
                Forwarder forwarder = list[i];
                if (forwarder.Target == null)   // destroyed component (Unity-null)
                    continue;
                try
                {
                    forwarder.Invoke(forwarder.Target, engineArg);
                }
                catch (Exception ex)
                {
                    Debug.LogException(ex);   // one faulting target must not stop the rest
                }
            }
        }
    }

    /// <summary>Cost-free, event-driven messages: physics/trigger (3D + 2D),
    /// particle, CharacterController hit, Animator IK, and OnDestroy. Declaring
    /// these has no presence side effect, so they share one proxy.</summary>
    [AddComponentMenu("")]
    internal sealed class LocusEventProxy : LocusProxyBase
    {
        private void OnTriggerEnter(Collider other) => Dispatch("OnTriggerEnter", other);
        private void OnTriggerStay(Collider other) => Dispatch("OnTriggerStay", other);
        private void OnTriggerExit(Collider other) => Dispatch("OnTriggerExit", other);
        private void OnCollisionEnter(Collision collision) => Dispatch("OnCollisionEnter", collision);
        private void OnCollisionStay(Collision collision) => Dispatch("OnCollisionStay", collision);
        private void OnCollisionExit(Collision collision) => Dispatch("OnCollisionExit", collision);

        private void OnTriggerEnter2D(Collider2D other) => Dispatch("OnTriggerEnter2D", other);
        private void OnTriggerStay2D(Collider2D other) => Dispatch("OnTriggerStay2D", other);
        private void OnTriggerExit2D(Collider2D other) => Dispatch("OnTriggerExit2D", other);
        private void OnCollisionEnter2D(Collision2D collision) => Dispatch("OnCollisionEnter2D", collision);
        private void OnCollisionStay2D(Collision2D collision) => Dispatch("OnCollisionStay2D", collision);
        private void OnCollisionExit2D(Collision2D collision) => Dispatch("OnCollisionExit2D", collision);

        private void OnParticleCollision(GameObject other) => Dispatch("OnParticleCollision", other);
        private void OnParticleTrigger() => Dispatch("OnParticleTrigger", null);
        private void OnParticleSystemStopped() => Dispatch("OnParticleSystemStopped", null);

        private void OnControllerColliderHit(ControllerColliderHit hit) => Dispatch("OnControllerColliderHit", hit);

        private void OnAnimatorIK(int layerIndex) => Dispatch("OnAnimatorIK", layerIndex);

        private void OnDestroy()
        {
            if (!DestroyForwardSuppressed)
                Dispatch("OnDestroy", null);
        }
    }

    /// <summary>Mouse messages. Hover variants (Enter/Exit/Over) cost a per-frame
    /// raycast, so all mouse messages live on their own proxy — attached only to
    /// objects that hot-add a mouse message.</summary>
    [AddComponentMenu("")]
    internal sealed class LocusMouseProxy : LocusProxyBase
    {
        private void OnMouseDown() => Dispatch("OnMouseDown", null);
        private void OnMouseUp() => Dispatch("OnMouseUp", null);
        private void OnMouseUpAsButton() => Dispatch("OnMouseUpAsButton", null);
        private void OnMouseDrag() => Dispatch("OnMouseDrag", null);
        private void OnMouseEnter() => Dispatch("OnMouseEnter", null);
        private void OnMouseExit() => Dispatch("OnMouseExit", null);
        private void OnMouseOver() => Dispatch("OnMouseOver", null);
    }

    /// <summary>OnGUI. Declaring it adds an IMGUI pass for the object, so it is
    /// isolated to objects that hot-add OnGUI. The forwarded shim runs inside this
    /// OnGUI call, so Event.current / GUILayout work as usual.</summary>
    [AddComponentMenu("")]
    internal sealed class LocusGuiProxy : LocusProxyBase
    {
        private void OnGUI() => Dispatch("OnGUI", null);
    }

    /// <summary>OnAnimatorMove. Declaring it makes the Animator stop applying
    /// root motion automatically and expect this callback to handle it — the same
    /// native behavior — so it is isolated to objects that hot-add OnAnimatorMove
    /// (a shared proxy would silently disable root motion everywhere).</summary>
    [AddComponentMenu("")]
    internal sealed class LocusAnimatorMoveProxy : LocusProxyBase
    {
        private void OnAnimatorMove() => Dispatch("OnAnimatorMove", null);
    }

    /// <summary>
    /// Owns the component-proxy drivers and keeps the right proxy component on
    /// every GameObject that hosts a target component, with forwarders in sync.
    /// It does NOT run per frame: the engine delivers events to the proxies
    /// natively, so the only periodic work is a throttled reconcile that attaches
    /// proxies to newly spawned objects and removes them from gone ones.
    /// Registrations are keyed by source file (replace-by-source), so a patch
    /// replaces rather than accumulates and a deleted/changed-away message tears
    /// its proxies down.
    /// </summary>
    internal static class LocusMessageProxyHub
    {
        // Which proxy family handles each component_proxy message.
        private static readonly Dictionary<string, Type> MessageProxyType = new Dictionary<string, Type>(StringComparer.Ordinal)
        {
            { "OnTriggerEnter", typeof(LocusEventProxy) },
            { "OnTriggerStay", typeof(LocusEventProxy) },
            { "OnTriggerExit", typeof(LocusEventProxy) },
            { "OnCollisionEnter", typeof(LocusEventProxy) },
            { "OnCollisionStay", typeof(LocusEventProxy) },
            { "OnCollisionExit", typeof(LocusEventProxy) },
            { "OnTriggerEnter2D", typeof(LocusEventProxy) },
            { "OnTriggerStay2D", typeof(LocusEventProxy) },
            { "OnTriggerExit2D", typeof(LocusEventProxy) },
            { "OnCollisionEnter2D", typeof(LocusEventProxy) },
            { "OnCollisionStay2D", typeof(LocusEventProxy) },
            { "OnCollisionExit2D", typeof(LocusEventProxy) },
            { "OnParticleCollision", typeof(LocusEventProxy) },
            { "OnParticleTrigger", typeof(LocusEventProxy) },
            { "OnParticleSystemStopped", typeof(LocusEventProxy) },
            { "OnControllerColliderHit", typeof(LocusEventProxy) },
            { "OnAnimatorIK", typeof(LocusEventProxy) },
            { "OnDestroy", typeof(LocusEventProxy) },
            { "OnMouseDown", typeof(LocusMouseProxy) },
            { "OnMouseUp", typeof(LocusMouseProxy) },
            { "OnMouseUpAsButton", typeof(LocusMouseProxy) },
            { "OnMouseDrag", typeof(LocusMouseProxy) },
            { "OnMouseEnter", typeof(LocusMouseProxy) },
            { "OnMouseExit", typeof(LocusMouseProxy) },
            { "OnMouseOver", typeof(LocusMouseProxy) },
            { "OnGUI", typeof(LocusGuiProxy) },
            { "OnAnimatorMove", typeof(LocusAnimatorMoveProxy) },
        };

        private sealed class Driver
        {
            public Type DeclaringType;
            public string Message;
            public string SourcePath;
            public Action<object, object> Invoke;
            public Type ProxyType;
        }

        private static readonly List<Driver> Drivers = new List<Driver>();

        // Live proxies this hub created this domain session. We MUST track them
        // ourselves: proxies are HideAndDontSave, and Object.FindObjectsByType does
        // NOT return DontSave objects, so it can never rediscover them (that would
        // leak/duplicate proxies on every reconcile). Pruned of Unity-null (destroyed)
        // entries on each reconcile; reset on domain reload, where a one-time
        // Resources sweep catches any orphan that outlived it.
        private static readonly List<LocusProxyBase> _proxies = new List<LocusProxyBase>();
        private static bool _hooked;
        private static double _lastReconcile;
        private const double ReconcileInterval = 0.5;   // seconds; safety net for spawns the event hooks miss

        /// <summary>Register (or replace) a component-proxy driver for one message
        /// on one MonoBehaviour type. Returns false (registers nothing) when the
        /// message is not proxy-able or the type is not a MonoBehaviour.</summary>
        public static bool Register(string message, Type declaringType, string sourcePath, Action<object, object> invoke)
        {
            if (declaringType == null || invoke == null || string.IsNullOrEmpty(message))
                return false;
            if (!MessageProxyType.TryGetValue(message, out Type proxyType))
                return false;
            if (!typeof(MonoBehaviour).IsAssignableFrom(declaringType))
                return false;

            HookOnce();
            Drivers.RemoveAll(d => d.DeclaringType == declaringType && d.Message == message);
            Drivers.Add(new Driver
            {
                DeclaringType = declaringType,
                Message = message,
                SourcePath = sourcePath ?? "",
                Invoke = invoke,
                ProxyType = proxyType,
            });
            Reconcile();
            return true;
        }

        /// <summary>Drop every driver from <paramref name="sourcePath"/> and
        /// reconcile, so a deleted/changed-away message stops being forwarded and
        /// its now-empty proxies are removed.</summary>
        public static void ClearSource(string sourcePath)
        {
            if (string.IsNullOrEmpty(sourcePath))
                return;
            int before = Drivers.Count;
            Drivers.RemoveAll(d => d.SourcePath == sourcePath);
            if (Drivers.Count != before)
                Reconcile();
        }

        public static void Clear()
        {
            Drivers.Clear();
            Reconcile();
        }

        // After a domain reload the hub (and its registry) is empty, but
        // HideAndDontSave proxies can outlive the reload. They are stale (the message
        // is native again after a recompile, or a fresh patch re-creates them) and no
        // longer owned by our registry — sweep and destroy them once the scene is
        // ready, then reconcile.
        [InitializeOnLoadMethod]
        private static void OnDomainLoad()
        {
            EditorApplication.delayCall += () =>
            {
                CleanupOrphanProxies();
                if (Application.isPlaying)
                    Reconcile();
            };
        }

        // Destroy every proxy NOT in our registry — orphans that outlived a domain
        // reload (which reset the registry). Resources.FindObjectsOfTypeAll is the API
        // that returns HideAndDontSave objects; Object.FindObjectsByType cannot, which
        // is exactly why the hub keeps its own registry. Editor-only, runs on load.
        private static void CleanupOrphanProxies()
        {
            LocusProxyBase[] all = Resources.FindObjectsOfTypeAll<LocusProxyBase>();
            for (int i = 0; i < all.Length; i++)
            {
                LocusProxyBase proxy = all[i];
                // Skip null, ones we still own, and any persistent (asset/prefab)
                // instance — FindObjectsOfTypeAll also returns those, and
                // DestroyImmediate throws on an asset.
                if (proxy == null || _proxies.Contains(proxy) || EditorUtility.IsPersistent(proxy))
                    continue;
                proxy.SuppressDestroyForward();
                UnityEngine.Object.DestroyImmediate(proxy);
            }
        }

        private static void HookOnce()
        {
            if (_hooked)
                return;
            _hooked = true;
            EditorApplication.playModeStateChanged += OnPlayModeStateChanged;
            EditorApplication.hierarchyChanged += OnHierarchyChanged;
            EditorApplication.update += OnEditorUpdate;
            SceneManager.sceneLoaded += OnSceneLoaded;
            SceneManager.sceneUnloaded += OnSceneUnloaded;
        }

        private static void OnHierarchyChanged()
        {
            if (Application.isPlaying)
                Reconcile();
        }

        private static void OnSceneLoaded(Scene scene, LoadSceneMode mode)
        {
            if (Application.isPlaying)
                Reconcile();
        }

        private static void OnSceneUnloaded(Scene scene)
        {
            if (Application.isPlaying)
                Reconcile();
        }

        private static void OnEditorUpdate()
        {
            // Nothing registered → nothing to maintain (the last ClearSource already
            // reconciled away any proxies). Don't scan the scene every interval.
            if (!Application.isPlaying || Drivers.Count == 0)
                return;
            double now = EditorApplication.timeSinceStartup;
            if (now - _lastReconcile < ReconcileInterval)
                return;
            _lastReconcile = now;
            Reconcile();
        }

        private static void OnPlayModeStateChanged(PlayModeStateChange change)
        {
            if (change == PlayModeStateChange.EnteredPlayMode && Drivers.Count > 0)
                Reconcile();
        }

        private static void Reconcile()
        {
            if (!Application.isPlaying)
                return;

            // desired: GameObject -> proxyType -> message -> forwarders.
            var desired = new Dictionary<GameObject, Dictionary<Type, Dictionary<string, List<LocusProxyBase.Forwarder>>>>();
            // One scene scan per DECLARING TYPE, shared across drivers on that type.
            var scanCache = new Dictionary<Type, UnityEngine.Object[]>();
            for (int i = 0; i < Drivers.Count; i++)
            {
                Driver driver = Drivers[i];
                if (!scanCache.TryGetValue(driver.DeclaringType, out UnityEngine.Object[] instances))
                {
#if UNITY_2022_2_OR_NEWER
                    instances = UnityEngine.Object.FindObjectsByType(
                        driver.DeclaringType, FindObjectsInactive.Include, FindObjectsSortMode.InstanceID);
#else
                    // Unity 2021 and earlier: FindObjectsOfType(Type, true) — includes
                    // inactive and is InstanceID-sorted, matching the newer call above.
                    instances = UnityEngine.Object.FindObjectsOfType(driver.DeclaringType, true);
#endif
                    scanCache[driver.DeclaringType] = instances;
                }
                for (int j = 0; j < instances.Length; j++)
                {
                    if (!(instances[j] is Component component) || component == null)
                        continue;
                    GameObject go = component.gameObject;
                    if (!desired.TryGetValue(go, out var byType))
                    {
                        byType = new Dictionary<Type, Dictionary<string, List<LocusProxyBase.Forwarder>>>();
                        desired[go] = byType;
                    }
                    if (!byType.TryGetValue(driver.ProxyType, out var byMessage))
                    {
                        byMessage = new Dictionary<string, List<LocusProxyBase.Forwarder>>(StringComparer.Ordinal);
                        byType[driver.ProxyType] = byMessage;
                    }
                    if (!byMessage.TryGetValue(driver.Message, out var list))
                    {
                        list = new List<LocusProxyBase.Forwarder>();
                        byMessage[driver.Message] = list;
                    }
                    list.Add(new LocusProxyBase.Forwarder { Target = component, Invoke = driver.Invoke });
                }
            }

            // Apply: refresh wanted proxies, tear down the rest, add missing ones.
            // Existing proxies come from our OWN registry, not FindObjectsByType:
            // they are HideAndDontSave, which Unity excludes from FindObjectsByType,
            // so re-finding them that way would return nothing and we would re-add a
            // duplicate proxy every reconcile (leak + duplicate dispatch). Iterate
            // backwards so destroyed/torn-down entries can be removed in place.
            var handled = new HashSet<(GameObject, Type)>();
            for (int i = _proxies.Count - 1; i >= 0; i--)
            {
                LocusProxyBase proxy = _proxies[i];
                if (proxy == null)   // the GameObject (and its proxy) was destroyed
                {
                    _proxies.RemoveAt(i);
                    continue;
                }
                GameObject go = proxy.gameObject;
                Type pType = proxy.GetType();
                if (desired.TryGetValue(go, out var byType) && byType.TryGetValue(pType, out var byMessage))
                {
                    proxy.SetForwarders(byMessage);
                    handled.Add((go, pType));
                }
                else
                {
                    proxy.SuppressDestroyForward();
                    DestroyProxy(proxy);
                    _proxies.RemoveAt(i);
                }
            }
            foreach (var goEntry in desired)
            {
                foreach (var typeEntry in goEntry.Value)
                {
                    if (handled.Contains((goEntry.Key, typeEntry.Key)))
                        continue;
                    var proxy = (LocusProxyBase)goEntry.Key.AddComponent(typeEntry.Key);
                    proxy.hideFlags = HideFlags.HideAndDontSave;
                    proxy.SetForwarders(typeEntry.Value);
                    _proxies.Add(proxy);
                }
            }
        }

        private static void DestroyProxy(LocusProxyBase proxy)
        {
            if (proxy == null)
                return;
            // Immediate (the caller already suppressed the OnDestroy forward): the
            // proxy is gone synchronously, so a later Reconcile in the same frame
            // can't re-find and mis-bind a dying proxy, and a real GameObject
            // destruction can't race a pending deferred teardown. Reconcile only runs
            // from editor/main-thread callbacks (never a physics/animation phase), so
            // DestroyImmediate is safe here; we iterate a snapshot array and null-check.
            UnityEngine.Object.DestroyImmediate(proxy);
        }
    }

    /// <summary>How a catch-up message must be gated so the one-shot run matches
    /// native lifecycle timing. The caller picks the policy per message name.</summary>
    internal enum CatchUpGate
    {
        Always,          // OnValidate: an editor-time callback — runs in edit AND play
        PlayingActive,   // Awake: play-mode only, on components of active GameObjects
        PlayingEnabled,  // Start: play-mode only, on enabled components (isActiveAndEnabled)
    }

    /// <summary>One-shot driver for lifecycle messages whose native timing has
    /// already passed (Awake/Start at load, OnValidate on edit): runs the shim once
    /// on each ELIGIBLE existing instance now, gated to match the message's native
    /// timing. New instances are not covered — the hot-reload result tells the agent
    /// that (see the message's note).</summary>
    internal static class LocusMessageCatchUp
    {
        /// <summary>Run the shim once on each eligible existing instance and return the
        /// NUMBER it dispatched to, or -1 when the declaring type is not a MonoBehaviour
        /// (then the added method is an ordinary method, not an engine message). A
        /// return of 0 means it IS a real message but nothing was eligible — no live
        /// instance, or (for a play-mode lifecycle message) we are in edit mode — so
        /// the caller reports that honestly instead of as "driven", since nothing ran
        /// and future instances need a recompile. <paramref name="gate"/> keeps timing
        /// faithful: Awake/Start never fire in edit mode (native does not run them
        /// there, and running gameplay lifecycle in the editor has real side effects),
        /// and Start only runs on enabled components — matching native. A faulting user
        /// body still counts (it WAS dispatched) — the user's bug, logged, not a wiring
        /// failure.</summary>
        public static int RunOnce(Type declaringType, Action<object> invoke, CatchUpGate gate)
        {
            if (declaringType == null || invoke == null)
                return -1;
            if (!typeof(MonoBehaviour).IsAssignableFrom(declaringType))
                return -1;
            // Play-mode lifecycle (Awake/Start) must not run in edit mode.
            if (gate != CatchUpGate.Always && !Application.isPlaying)
                return 0;
#if UNITY_2022_2_OR_NEWER
            UnityEngine.Object[] instances = UnityEngine.Object.FindObjectsByType(
                declaringType, FindObjectsInactive.Include, FindObjectsSortMode.InstanceID);
#else
            // Unity 2021 and earlier: FindObjectsOfType(Type, true) — includes inactive
            // and is InstanceID-sorted, matching the newer call above.
            UnityEngine.Object[] instances = UnityEngine.Object.FindObjectsOfType(declaringType, true);
#endif
            int ran = 0;
            for (int i = 0; i < instances.Length; i++)
            {
                UnityEngine.Object obj = instances[i];
                if (obj == null)
                    continue;
                // Awake runs for components of active GameObjects; Start only for
                // enabled components — skip the rest so the catch-up matches native.
                if (gate == CatchUpGate.PlayingActive
                    && obj is Component component && !component.gameObject.activeInHierarchy)
                    continue;
                if (gate == CatchUpGate.PlayingEnabled
                    && obj is Behaviour behaviour && !behaviour.isActiveAndEnabled)
                    continue;
                ran++;
                try
                {
                    invoke(obj);
                }
                catch (Exception ex)
                {
                    Debug.LogException(ex);
                }
            }
            return ran;
        }
    }
}
