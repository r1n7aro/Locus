using UnityEngine;
using UnityEngine.LowLevel;
using UnityEditor;

using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Locus
{
    public enum UnityTickBoundary
    {
        Before,
        After
    }

    public enum UnityLoopPoint
    {
        EditorUpdate,
        BeforeFixedUpdate,
        AfterFixedUpdate,
        BeforeUpdate,
        AfterUpdate,
        BeforeLateUpdate,
        AfterLateUpdate,
        EndOfFrame
    }

    [Serializable]
    public sealed class UnityThreadInfo
    {
        public int ManagedThreadId;
        public string Name;
        public bool IsMainThread;
        public bool IsThreadPoolThread;
        public string SynchronizationContext;

        public override string ToString()
        {
            return "thread=" + ManagedThreadId
                + " name=" + (Name ?? "")
                + " main=" + IsMainThread
                + " thread_pool=" + IsThreadPoolThread
                + " synchronization_context=" + (SynchronizationContext ?? "");
        }
    }

    [Serializable]
    public sealed class UnityTickSystemInfo
    {
        public string Id;
        public string TypeName;
        public string TypeFullName;
        public string AssemblyName;
        public string Path;
        public int Depth;
        public int SiblingIndex;
        public bool HasManagedDelegate;
        public bool HasNativeFunction;
        public string ManagedMethod;
        public bool CanAnchor;
        public string SnapshotFingerprint;

        public override string ToString()
        {
            return Id ?? TypeFullName ?? TypeName ?? "<unknown tick system>";
        }
    }

    [Serializable]
    public sealed class UnityTickSystemSnapshot
    {
        public string Fingerprint;
        public int Count;
        public UnityTickSystemInfo[] Nodes;
    }

    [Serializable]
    public sealed class UnityTickStamp
    {
        public string NodeId;
        public string NodeType;
        public string Boundary;
        public long Sequence;
        public int FrameCount;
        public int RenderedFrameCount;
        public double EditorTime;
        public int ManagedThreadId;

        public override string ToString()
        {
            return "node=" + (NodeId ?? "")
                + " boundary=" + (Boundary ?? "")
                + " sequence=" + Sequence
                + " frame=" + FrameCount
                + " rendered_frame=" + RenderedFrameCount
                + " thread=" + ManagedThreadId;
        }
    }

    [Serializable]
    public sealed class UnityBreakpointResult
    {
        public string Status;
        public string Label;
        public string EditorStatus;
        public UnityTickStamp Hit;
        public int PauseConfirmedFrameCount;
        public double PauseConfirmedEditorTime;

        public string ToResultText()
        {
            var sb = new StringBuilder(256);
            sb.AppendLine("status: breakpoint");
            sb.AppendLine("editor_status: " + (EditorStatus ?? "playing_paused"));
            if (!string.IsNullOrEmpty(Label))
                sb.AppendLine("label: " + Label.Replace("\r", " ").Replace("\n", " "));
            if (Hit != null)
            {
                sb.AppendLine("tick_node: " + (Hit.NodeId ?? ""));
                sb.AppendLine("tick_type: " + (Hit.NodeType ?? ""));
                sb.AppendLine("tick_boundary: " + (Hit.Boundary ?? ""));
                sb.AppendLine("tick_sequence: " + Hit.Sequence);
                sb.AppendLine("hit_frame: " + Hit.FrameCount);
                sb.AppendLine("hit_rendered_frame: " + Hit.RenderedFrameCount);
                sb.AppendLine("hit_thread: " + Hit.ManagedThreadId);
            }
            sb.AppendLine("pause_confirmed_frame: " + PauseConfirmedFrameCount);
            sb.Append("pause_confirmed_editor_time: ")
                .Append(PauseConfirmedEditorTime.ToString("R", System.Globalization.CultureInfo.InvariantCulture));
            return sb.ToString();
        }
    }

    internal sealed class ExecuteCodeBreakpointReachedException : Exception
    {
        public readonly UnityBreakpointResult Result;

        public ExecuteCodeBreakpointReachedException(UnityBreakpointResult result)
            : base("locus_breakpoint_reached")
        {
            Result = result;
        }
    }

    public static partial class LocusBridge
    {
        private struct LocusTickBeforeMarker { }
        private struct LocusTickAfterMarker { }

        private sealed class TickAnchor
        {
            public string Key;
            public string NodeId;
            public string NodeType;
            public UnityTickBoundary Boundary;
            public PlayerLoopSystem.UpdateFunction Driver;
            public readonly List<ExecuteCodeTickWaitRegistration> Waits =
                new List<ExecuteCodeTickWaitRegistration>();
            public long Sequence;
        }

        internal sealed class ExecuteCodeTickWaitRegistration
        {
            public ExecuteCodeContext Context;
            public string NodeId;
            public string NodeType;
            public string SnapshotFingerprint;
            public UnityTickBoundary Boundary;
            public Func<bool> Predicate;
            public bool BreakOnMatch;
            public string BreakLabel;
            public Action Continuation;
            public UnityTickStamp Result;
            public Exception Error;
            public bool Completed;
            public bool Scheduled;
            public int SourceLine;
            public string Condition;
        }

        private static readonly Dictionary<string, TickAnchor> _tickAnchors =
            new Dictionary<string, TickAnchor>(StringComparer.Ordinal);
        private static readonly List<ExecuteCodeTickWaitRegistration> _editorTickWaits =
            new List<ExecuteCodeTickWaitRegistration>();
        private static readonly List<ExecuteCodeTickWaitRegistration> _pendingBreakpointConfirms =
            new List<ExecuteCodeTickWaitRegistration>();
        private static string _installedTickLogicalFingerprint;
        private static bool _tickSchedulerInstalled;
        private static bool _tickSchedulerClearRequested;
        private static long _editorTickSequence;
        private static RuntimeStateMachineSession _runStatesTickSession;
        private static string _runStatesTickAnchorKey;

        private static bool IsLocusTickMarker(Type type)
        {
            return type == typeof(LocusTickBeforeMarker) || type == typeof(LocusTickAfterMarker);
        }

        private static string TickTypeName(Type type)
        {
            return type != null ? (type.FullName ?? type.Name ?? "<unnamed>") : "<null>";
        }

        private static string TickSegment(Type type, int occurrence)
        {
            return TickTypeName(type).Replace("/", "%2F") + "[" + occurrence + "]";
        }

        private static PlayerLoopSystem StripLocusTickMarkers(PlayerLoopSystem system)
        {
            PlayerLoopSystem[] children = system.subSystemList;
            if (children == null || children.Length == 0)
                return system;

            var kept = new List<PlayerLoopSystem>(children.Length);
            for (int i = 0; i < children.Length; i++)
            {
                if (IsLocusTickMarker(children[i].type))
                    continue;
                kept.Add(StripLocusTickMarkers(children[i]));
            }
            system.subSystemList = kept.ToArray();
            return system;
        }

        private static UnityTickSystemSnapshot BuildTickSystemSnapshot(PlayerLoopSystem logicalLoop)
        {
            var nodes = new List<UnityTickSystemInfo>(256);
            string rootId = TickSegment(logicalLoop.type, 0);
            AppendTickSystemSnapshot(logicalLoop, rootId, 0, 0, false, nodes);

            ulong hash = 1469598103934665603UL;
            unchecked
            {
                for (int i = 0; i < nodes.Count; i++)
                {
                    string value = nodes[i].Id + "\n" + nodes[i].ManagedMethod + "\n";
                    for (int j = 0; j < value.Length; j++)
                    {
                        hash ^= value[j];
                        hash *= 1099511628211UL;
                    }
                }
            }
            string fingerprint = hash.ToString("x16");
            for (int i = 0; i < nodes.Count; i++)
                nodes[i].SnapshotFingerprint = fingerprint;

            return new UnityTickSystemSnapshot
            {
                Fingerprint = fingerprint,
                Count = nodes.Count,
                Nodes = nodes.ToArray()
            };
        }

        private static void AppendTickSystemSnapshot(
            PlayerLoopSystem system,
            string id,
            int depth,
            int siblingIndex,
            bool canAnchor,
            List<UnityTickSystemInfo> nodes)
        {
            Delegate managed = system.updateDelegate;
            Type type = system.type;
            string method = "";
            if (managed != null && managed.Method != null)
            {
                Type declaring = managed.Method.DeclaringType;
                method = (declaring != null ? declaring.FullName + "." : "") + managed.Method.Name;
            }

            nodes.Add(new UnityTickSystemInfo
            {
                Id = id,
                TypeName = type != null ? type.Name : "<null>",
                TypeFullName = TickTypeName(type),
                AssemblyName = type != null && type.Assembly != null
                    ? type.Assembly.GetName().Name
                    : "",
                Path = id,
                Depth = depth,
                SiblingIndex = siblingIndex,
                HasManagedDelegate = managed != null,
                HasNativeFunction = system.updateFunction != IntPtr.Zero,
                ManagedMethod = method,
                CanAnchor = canAnchor && !IsLocusTickMarker(type),
                SnapshotFingerprint = ""
            });

            PlayerLoopSystem[] children = system.subSystemList;
            if (children == null)
                return;

            var occurrences = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < children.Length; i++)
            {
                string typeName = TickTypeName(children[i].type);
                int occurrence;
                occurrences.TryGetValue(typeName, out occurrence);
                occurrences[typeName] = occurrence + 1;
                string childId = id + "/" + TickSegment(children[i].type, occurrence);
                AppendTickSystemSnapshot(children[i], childId, depth + 1, i, true, nodes);
            }
        }

        private static UnityTickSystemSnapshot CurrentTickSystemSnapshot()
        {
            PlayerLoopSystem logical = StripLocusTickMarkers(PlayerLoop.GetCurrentPlayerLoop());
            return BuildTickSystemSnapshot(logical);
        }

        private static UnityTickSystemInfo ResolveTickSystem(
            string nodeId,
            string snapshotFingerprint,
            bool requireFingerprint)
        {
            UnityTickSystemSnapshot snapshot = CurrentTickSystemSnapshot();
            if (requireFingerprint && !string.IsNullOrEmpty(snapshotFingerprint)
                && !string.Equals(snapshotFingerprint, snapshot.Fingerprint, StringComparison.Ordinal))
            {
                throw new InvalidOperationException(
                    "PlayerLoop changed after the tick system was listed. listed="
                    + snapshotFingerprint + ", current=" + snapshot.Fingerprint
                    + ". List tick systems again before waiting.");
            }

            UnityTickSystemInfo node = snapshot.Nodes.FirstOrDefault(
                value => string.Equals(value.Id, nodeId, StringComparison.Ordinal));
            if (node == null)
                throw new KeyNotFoundException("PlayerLoop tick system not found: " + (nodeId ?? ""));
            if (!node.CanAnchor)
                throw new InvalidOperationException("PlayerLoop tick system cannot be used as an anchor: " + node.Id);
            return node;
        }

        private static string TickAnchorKey(string nodeId, UnityTickBoundary boundary)
        {
            return (boundary == UnityTickBoundary.Before ? "before:" : "after:") + nodeId;
        }

        private static void ScheduleTickWait(ExecuteCodeTickWaitRegistration registration)
        {
            if (registration == null || registration.Context == null)
                return;

            Action schedule = delegate
            {
                if (registration.Completed || registration.Scheduled)
                    return;
                if (registration.Context.IsCancellationRequested)
                {
                    CompleteTickWait(registration, null, new OperationCanceledException());
                    return;
                }

                registration.Scheduled = true;
                if (registration.NodeId == "__editor_update__")
                {
                    registration.Context.SetAwaiting(
                        "editor_update",
                        "next UnityEditor.EditorApplication.update",
                        registration.Condition,
                        registration.SourceLine);
                    _editorTickWaits.Add(registration);
                    return;
                }

                UnityTickSystemInfo node = ResolveTickSystem(
                    registration.NodeId,
                    registration.SnapshotFingerprint,
                    !string.IsNullOrEmpty(registration.SnapshotFingerprint));
                registration.NodeType = node.TypeFullName;
                string key = TickAnchorKey(node.Id, registration.Boundary);
                TickAnchor anchor;
                if (!_tickAnchors.TryGetValue(key, out anchor))
                {
                    anchor = new TickAnchor
                    {
                        Key = key,
                        NodeId = node.Id,
                        NodeType = node.TypeFullName,
                        Boundary = registration.Boundary
                    };
                    string capturedKey = key;
                    anchor.Driver = delegate { PumpTickAnchor(capturedKey); };
                    _tickAnchors.Add(key, anchor);
                    InstallTickScheduler();
                }
                registration.Context.SetAwaiting(
                    registration.BreakOnMatch ? "breakpoint_condition" :
                        (registration.Predicate != null ? "tick_condition" : "player_loop"),
                    registration.Boundary + " " + node.Id,
                    registration.Condition,
                    registration.SourceLine);
                anchor.Waits.Add(registration);
            };

            if (LocusAsync.IsMainThread)
                schedule();
            else
                PostToMainThread(schedule);
        }

        private static bool InsertTickAnchors(
            ref PlayerLoopSystem system,
            string id,
            Dictionary<string, TickAnchor> anchors)
        {
            PlayerLoopSystem[] children = system.subSystemList;
            if (children == null || children.Length == 0)
                return false;

            bool changed = false;
            var output = new List<PlayerLoopSystem>(children.Length + 4);
            var occurrences = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < children.Length; i++)
            {
                PlayerLoopSystem child = children[i];
                string typeName = TickTypeName(child.type);
                int occurrence;
                occurrences.TryGetValue(typeName, out occurrence);
                occurrences[typeName] = occurrence + 1;
                string childId = id + "/" + TickSegment(child.type, occurrence);

                TickAnchor before;
                if (anchors.TryGetValue(TickAnchorKey(childId, UnityTickBoundary.Before), out before))
                {
                    output.Add(new PlayerLoopSystem
                    {
                        type = typeof(LocusTickBeforeMarker),
                        updateDelegate = before.Driver
                    });
                    changed = true;
                }

                if (InsertTickAnchors(ref child, childId, anchors))
                    changed = true;
                output.Add(child);

                TickAnchor after;
                if (anchors.TryGetValue(TickAnchorKey(childId, UnityTickBoundary.After), out after))
                {
                    output.Add(new PlayerLoopSystem
                    {
                        type = typeof(LocusTickAfterMarker),
                        updateDelegate = after.Driver
                    });
                    changed = true;
                }
            }

            if (changed)
                system.subSystemList = output.ToArray();
            return changed;
        }

        private static void InstallTickScheduler()
        {
            if (_tickAnchors.Count == 0)
                return;
            if (!EditorApplication.isPlaying || EditorApplication.isPaused)
                throw new InvalidOperationException(
                    "PlayerLoop tick waits require running Play Mode. Current status is "
                    + (EditorApplication.isPlaying ? "playing_paused" : "editing") + ".");

            PlayerLoopSystem loop = StripLocusTickMarkers(PlayerLoop.GetCurrentPlayerLoop());
            UnityTickSystemSnapshot snapshot = BuildTickSystemSnapshot(loop);
            foreach (TickAnchor anchor in _tickAnchors.Values)
            {
                if (!snapshot.Nodes.Any(node => node.Id == anchor.NodeId && node.CanAnchor))
                    throw new InvalidOperationException("PlayerLoop anchor disappeared: " + anchor.NodeId);
            }

            string rootId = TickSegment(loop.type, 0);
            InsertTickAnchors(ref loop, rootId, _tickAnchors);
            PlayerLoop.SetPlayerLoop(loop);
            _installedTickLogicalFingerprint = snapshot.Fingerprint;
            _tickSchedulerInstalled = true;
            _tickSchedulerClearRequested = false;
        }

        private static bool CurrentLoopContainsOwnedTickMarkers()
        {
            int expected = _tickAnchors.Count;
            int actual = 0;
            var stack = new Stack<PlayerLoopSystem>();
            stack.Push(PlayerLoop.GetCurrentPlayerLoop());
            while (stack.Count > 0)
            {
                PlayerLoopSystem item = stack.Pop();
                if (IsLocusTickMarker(item.type))
                    actual++;
                PlayerLoopSystem[] children = item.subSystemList;
                if (children == null)
                    continue;
                for (int i = 0; i < children.Length; i++)
                    stack.Push(children[i]);
            }
            return actual == expected;
        }

        private static void PumpTickAnchor(string key)
        {
            TickAnchor anchor;
            if (!_tickAnchors.TryGetValue(key, out anchor))
                return;

            anchor.Sequence++;
            var stamp = new UnityTickStamp
            {
                NodeId = anchor.NodeId,
                NodeType = anchor.NodeType,
                Boundary = anchor.Boundary.ToString(),
                Sequence = anchor.Sequence,
                FrameCount = Time.frameCount,
                RenderedFrameCount = Time.renderedFrameCount,
                EditorTime = EditorApplication.timeSinceStartup,
                ManagedThreadId = Thread.CurrentThread.ManagedThreadId
            };

            PumpRunStatesAtTickAnchor(key);

            if (anchor.Waits.Count == 0)
                return;

            var waits = anchor.Waits.ToArray();
            for (int i = 0; i < waits.Length; i++)
            {
                ExecuteCodeTickWaitRegistration wait = waits[i];
                if (wait.Completed)
                {
                    anchor.Waits.Remove(wait);
                    continue;
                }
                if (wait.Context.IsCancellationRequested)
                {
                    anchor.Waits.Remove(wait);
                    CompleteTickWait(wait, null, new OperationCanceledException());
                    continue;
                }

                bool matched = true;
                if (wait.Predicate != null)
                {
                    try
                    {
                        matched = wait.Predicate();
                    }
                    catch (Exception ex)
                    {
                        anchor.Waits.Remove(wait);
                        CompleteTickWait(wait, null, ex);
                        continue;
                    }
                }

                wait.Context.NotifyActivity();
                if (!matched)
                    continue;

                anchor.Waits.Remove(wait);
                if (wait.BreakOnMatch)
                {
                    wait.Result = stamp;
                    _pendingBreakpointConfirms.Add(wait);
                    EditorApplication.isPaused = true;
                }
                else
                {
                    CompleteTickWait(wait, stamp, null);
                }
            }
        }

        private static void CompleteTickWait(
            ExecuteCodeTickWaitRegistration registration,
            UnityTickStamp result,
            Exception error)
        {
            if (registration == null || registration.Completed)
                return;
            registration.Completed = true;
            registration.Result = result;
            registration.Error = error;
            if (registration.Context != null)
                registration.Context.ClearAwaiting();
            Action continuation = registration.Continuation;
            if (continuation == null)
                return;
            try
            {
                continuation();
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Tick continuation failed: " + ex);
            }
        }

        private static void PumpTickSchedulerEditorUpdate()
        {
            _editorTickSequence++;
            var editorStamp = new UnityTickStamp
            {
                NodeId = "__editor_update__",
                NodeType = "UnityEditor.EditorApplication.update",
                Boundary = "After",
                Sequence = _editorTickSequence,
                FrameCount = Time.frameCount,
                RenderedFrameCount = Time.renderedFrameCount,
                EditorTime = EditorApplication.timeSinceStartup,
                ManagedThreadId = Thread.CurrentThread.ManagedThreadId
            };

            if (_editorTickWaits.Count > 0)
            {
                ExecuteCodeTickWaitRegistration[] waits = _editorTickWaits.ToArray();
                _editorTickWaits.Clear();
                for (int i = 0; i < waits.Length; i++)
                {
                    ExecuteCodeTickWaitRegistration wait = waits[i];
                    if (wait.Context.IsCancellationRequested)
                        CompleteTickWait(wait, null, new OperationCanceledException());
                    else
                        CompleteTickWait(wait, editorStamp, null);
                }
            }

            if (_pendingBreakpointConfirms.Count > 0)
            {
                ExecuteCodeTickWaitRegistration[] pending = _pendingBreakpointConfirms.ToArray();
                for (int i = 0; i < pending.Length; i++)
                {
                    ExecuteCodeTickWaitRegistration wait = pending[i];
                    if (wait.Context.IsCancellationRequested)
                    {
                        _pendingBreakpointConfirms.Remove(wait);
                        CompleteTickWait(wait, null, new OperationCanceledException());
                        continue;
                    }
                    if (!EditorApplication.isPlaying)
                    {
                        _pendingBreakpointConfirms.Remove(wait);
                        CompleteTickWait(wait, null,
                            new InvalidOperationException("Play Mode exited while confirming breakpoint."));
                        continue;
                    }
                    if (!EditorApplication.isPaused)
                        continue;

                    _pendingBreakpointConfirms.Remove(wait);
                    var result = new UnityBreakpointResult
                    {
                        Status = "breakpoint",
                        Label = wait.BreakLabel ?? "",
                        EditorStatus = "playing_paused",
                        Hit = wait.Result,
                        PauseConfirmedFrameCount = Time.frameCount,
                        PauseConfirmedEditorTime = EditorApplication.timeSinceStartup
                    };
                    CompleteTickWait(wait, wait.Result, new ExecuteCodeBreakpointReachedException(result));
                }
            }

            if (_tickSchedulerClearRequested)
            {
                ClearTickSchedulerNow();
                return;
            }

            if (_tickSchedulerInstalled && _tickAnchors.Count > 0 && EditorApplication.isPlaying
                && !EditorApplication.isPaused)
            {
                UnityTickSystemSnapshot logical = CurrentTickSystemSnapshot();
                if (!string.Equals(logical.Fingerprint, _installedTickLogicalFingerprint, StringComparison.Ordinal)
                    || !CurrentLoopContainsOwnedTickMarkers())
                {
                    InstallTickScheduler();
                }
            }
        }

        private static void RequestTickSchedulerClear()
        {
            if (_runStatesTickSession != null && !_runStatesTickSession.IsCompleted)
                return;
            _tickSchedulerClearRequested = true;
        }

        private static void ConfigureRunStatesTickAnchor(
            RuntimeStateMachineSession session,
            UnityTickSystemInfo system,
            UnityTickBoundary boundary)
        {
            if (session == null || system == null)
                throw new ArgumentNullException(session == null ? "session" : "system");
            if (!ReferenceEquals(session, _activeRunStatesSession))
                throw new InvalidOperationException("The unity_run_states session is no longer active.");

            UnityTickSystemInfo node = ResolveTickSystem(
                system.Id,
                system.SnapshotFingerprint,
                !string.IsNullOrEmpty(system.SnapshotFingerprint));
            string key = TickAnchorKey(node.Id, boundary);
            if (string.Equals(_runStatesTickAnchorKey, key, StringComparison.Ordinal)
                && ReferenceEquals(_runStatesTickSession, session))
                return;

            ReleaseRunStatesTickAnchor(session);
            TickAnchor anchor;
            if (!_tickAnchors.TryGetValue(key, out anchor))
            {
                anchor = new TickAnchor
                {
                    Key = key,
                    NodeId = node.Id,
                    NodeType = node.TypeFullName,
                    Boundary = boundary
                };
                string capturedKey = key;
                anchor.Driver = delegate { PumpTickAnchor(capturedKey); };
                _tickAnchors.Add(key, anchor);
            }

            _runStatesTickSession = session;
            _runStatesTickAnchorKey = key;
            InstallTickScheduler();
        }

        private static void ReleaseRunStatesTickAnchor(RuntimeStateMachineSession session)
        {
            if (_runStatesTickSession == null
                || (session != null && !ReferenceEquals(session, _runStatesTickSession)))
                return;

            string key = _runStatesTickAnchorKey;
            _runStatesTickSession = null;
            _runStatesTickAnchorKey = null;
            TickAnchor anchor;
            if (!string.IsNullOrEmpty(key) && _tickAnchors.TryGetValue(key, out anchor)
                && anchor.Waits.Count == 0)
                _tickAnchors.Remove(key);

            if (_tickAnchors.Count == 0)
                _tickSchedulerClearRequested = true;
        }

        private static bool RunStatesUsesTickAnchor(RuntimeStateMachineSession session)
        {
            return session != null
                && ReferenceEquals(session, _runStatesTickSession)
                && !string.IsNullOrEmpty(_runStatesTickAnchorKey);
        }

        private static void PumpRunStatesAtTickAnchor(string key)
        {
            RuntimeStateMachineSession session = _runStatesTickSession;
            if (session == null || session.IsCompleted
                || !string.Equals(key, _runStatesTickAnchorKey, StringComparison.Ordinal))
                return;

            session.Tick();
            if (session.IsCompleted)
            {
                if (ReferenceEquals(session, _activeRunStatesSession))
                    _activeRunStatesSession = null;
                ReleaseRunStatesTickAnchor(session);
            }
        }

        private static void ClearTickSchedulerNow()
        {
            if (_tickSchedulerInstalled)
            {
                try
                {
                    PlayerLoop.SetPlayerLoop(StripLocusTickMarkers(PlayerLoop.GetCurrentPlayerLoop()));
                }
                catch (Exception ex)
                {
                    Debug.LogWarning("[Locus] Failed to remove tick scheduler: " + ex.Message);
                }
            }

            var pending = new List<ExecuteCodeTickWaitRegistration>();
            foreach (TickAnchor anchor in _tickAnchors.Values)
                pending.AddRange(anchor.Waits);
            pending.AddRange(_editorTickWaits);
            pending.AddRange(_pendingBreakpointConfirms);

            _tickAnchors.Clear();
            _editorTickWaits.Clear();
            _pendingBreakpointConfirms.Clear();
            _runStatesTickSession = null;
            _runStatesTickAnchorKey = null;
            _tickSchedulerInstalled = false;
            _tickSchedulerClearRequested = false;
            _installedTickLogicalFingerprint = null;

            for (int i = 0; i < pending.Count; i++)
                CompleteTickWait(pending[i], null, new OperationCanceledException("Tick scheduler stopped."));
        }

        private static void TickSchedulerOnPlayModeStateChanged(PlayModeStateChange state)
        {
            if (state == PlayModeStateChange.EnteredPlayMode)
            {
                _tickSchedulerInstalled = false;
                if (_tickAnchors.Count > 0)
                    InstallTickScheduler();
            }
            else if (state == PlayModeStateChange.ExitingPlayMode)
            {
                RequestTickSchedulerClear();
            }
        }

        public sealed partial class ExecuteCodeContext
        {
            public UnityThreadInfo Thread
            {
                get { return GetCurrentThread(); }
            }

            public bool IsMainThread
            {
                get { return LocusAsync.IsMainThread; }
            }

            public UnityThreadInfo GetCurrentThread()
            {
                System.Threading.Thread thread = System.Threading.Thread.CurrentThread;
                SynchronizationContext sync = SynchronizationContext.Current;
                return new UnityThreadInfo
                {
                    ManagedThreadId = thread.ManagedThreadId,
                    Name = thread.Name ?? "",
                    IsMainThread = LocusAsync.IsMainThread,
                    IsThreadPoolThread = thread.IsThreadPoolThread,
                    SynchronizationContext = sync != null ? sync.GetType().FullName : ""
                };
            }

            public SwitchToMainThreadAwaitable SwitchToMainThread()
            {
                NotifyActivity();
                return LocusAsync.SwitchToMainThread();
            }

            public SwitchToThreadPoolAwaitable SwitchToThreadPool()
            {
                NotifyActivity();
                return LocusAsync.SwitchToThreadPool();
            }

            public UnityTickSystemSnapshot ListTickSystems()
            {
                ThrowIfCancellationRequested();
                if (!LocusAsync.IsMainThread)
                    throw new InvalidOperationException("ListTickSystems must run on the Unity main thread. Use await ctx.SwitchToMainThread().");
                NotifyActivity();
                return CurrentTickSystemSnapshot();
            }

            public UnityTickSystemInfo[] FindTickSystems(string typeName)
            {
                string normalized = (typeName ?? "").Trim();
                if (string.IsNullOrEmpty(normalized))
                    throw new ArgumentException("Tick system type name is required.", "typeName");
                UnityTickSystemSnapshot snapshot = ListTickSystems();
                return snapshot.Nodes.Where(node =>
                    string.Equals(node.TypeFullName, normalized, StringComparison.Ordinal)
                    || string.Equals(node.TypeName, normalized, StringComparison.Ordinal)).ToArray();
            }

            public UnityTickSystemInfo FindTickSystem(string typeName, int occurrence = 0)
            {
                UnityTickSystemInfo[] matches = FindTickSystems(typeName);
                if (occurrence < 0 || occurrence >= matches.Length)
                    throw new KeyNotFoundException(
                        "Tick system '" + typeName + "' occurrence " + occurrence
                        + " was not found. matches=" + matches.Length);
                return matches[occurrence];
            }

            public ExecuteCodeTickAwaitable WaitBefore(
                UnityTickSystemInfo system,
                [CallerLineNumber] int sourceLine = 0)
            {
                return WaitAt(system, UnityTickBoundary.Before, sourceLine);
            }

            public ExecuteCodeTickAwaitable WaitAfter(
                UnityTickSystemInfo system,
                [CallerLineNumber] int sourceLine = 0)
            {
                return WaitAt(system, UnityTickBoundary.After, sourceLine);
            }

            public ExecuteCodeTickAwaitable WaitAt(
                UnityTickSystemInfo system,
                UnityTickBoundary boundary,
                [CallerLineNumber] int sourceLine = 0)
            {
                if (system == null)
                    throw new ArgumentNullException("system");
                return CreateTickAwaitable(
                    system.Id,
                    system.TypeFullName,
                    system.SnapshotFingerprint,
                    boundary,
                    null,
                    false,
                    null,
                    "",
                    sourceLine);
            }

            public ExecuteCodeTickAwaitable WaitAt(
                string nodeId,
                UnityTickBoundary boundary,
                [CallerLineNumber] int sourceLine = 0)
            {
                return CreateTickAwaitable(
                    nodeId, null, null, boundary, null, false, null, "", sourceLine);
            }

            public ExecuteCodeTickAwaitable Next(
                UnityLoopPoint point,
                [CallerLineNumber] int sourceLine = 0)
            {
                if (point == UnityLoopPoint.EditorUpdate)
                {
                    return CreateTickAwaitable(
                        "__editor_update__", "UnityEditor.EditorApplication.update", null,
                        UnityTickBoundary.After, null, false, null, "", sourceLine);
                }

                Type type;
                UnityTickBoundary boundary;
                ResolveLoopPoint(point, out type, out boundary);
                UnityTickSystemInfo system = FindTickSystem(type.FullName, 0);
                return WaitAt(system, boundary, sourceLine);
            }

            public ExecuteCodeTickAwaitable WaitUntil(
                UnityTickSystemInfo system,
                UnityTickBoundary boundary,
                Func<bool> predicate,
                string condition = null,
                [CallerLineNumber] int sourceLine = 0)
            {
                if (system == null)
                    throw new ArgumentNullException("system");
                if (predicate == null)
                    throw new ArgumentNullException("predicate");
                return CreateTickAwaitable(
                    system.Id, system.TypeFullName, system.SnapshotFingerprint,
                    boundary, predicate, false, null,
                    string.IsNullOrWhiteSpace(condition) ? PredicateDescription(predicate) : condition.Trim(),
                    sourceLine);
            }

            public ExecuteCodeTickAwaitable BreakWhen(
                UnityTickSystemInfo system,
                UnityTickBoundary boundary,
                Func<bool> predicate,
                string label = null,
                string condition = null,
                [CallerLineNumber] int sourceLine = 0)
            {
                if (system == null)
                    throw new ArgumentNullException("system");
                if (predicate == null)
                    throw new ArgumentNullException("predicate");
                return CreateTickAwaitable(
                    system.Id, system.TypeFullName, system.SnapshotFingerprint,
                    boundary, predicate, true, label,
                    string.IsNullOrWhiteSpace(condition) ? PredicateDescription(predicate) : condition.Trim(),
                    sourceLine);
            }

            public ExecuteCodeTickAwaitable BreakWhen(
                UnityLoopPoint point,
                Func<bool> predicate,
                string label = null,
                string condition = null,
                [CallerLineNumber] int sourceLine = 0)
            {
                if (point == UnityLoopPoint.EditorUpdate)
                    throw new ArgumentException("BreakWhen requires a PlayerLoop point.", "point");
                Type type;
                UnityTickBoundary boundary;
                ResolveLoopPoint(point, out type, out boundary);
                return BreakWhen(
                    FindTickSystem(type.FullName, 0), boundary, predicate,
                    label, condition, sourceLine);
            }

            public async Task<UnityThreadInfo> ResumeGame()
            {
                await SwitchToMainThread();
                ThrowIfCancellationRequested();
                if (!EditorApplication.isPlaying)
                    throw new InvalidOperationException("ResumeGame requires Play Mode.");
                EditorApplication.isPaused = false;
                while (EditorApplication.isPaused)
                    await Next(UnityLoopPoint.EditorUpdate, 0);
                NotifyActivity();
                return GetCurrentThread();
            }

            public async Task<UnityTickStamp> StepFrame()
            {
                await SwitchToMainThread();
                ThrowIfCancellationRequested();
                if (!EditorApplication.isPlaying || !EditorApplication.isPaused)
                    throw new InvalidOperationException("StepFrame requires paused Play Mode.");
                int startFrame = Time.frameCount;
                EditorApplication.Step();
                UnityTickStamp stamp = null;
                do
                {
                    stamp = await Next(UnityLoopPoint.EditorUpdate, 0);
                }
                while (EditorApplication.isPlaying && Time.frameCount <= startFrame);
                if (!EditorApplication.isPaused)
                    EditorApplication.isPaused = true;
                NotifyActivity();
                return stamp;
            }

            internal void NotifyActivity()
            {
                TouchActivity();
            }

            private ExecuteCodeTickAwaitable CreateTickAwaitable(
                string nodeId,
                string nodeType,
                string snapshotFingerprint,
                UnityTickBoundary boundary,
                Func<bool> predicate,
                bool breakOnMatch,
                string breakLabel,
                string condition,
                int sourceLine)
            {
                ThrowIfCancellationRequested();
                if (string.IsNullOrWhiteSpace(nodeId))
                    throw new ArgumentException("Tick system node id is required.", "nodeId");
                var registration = new ExecuteCodeTickWaitRegistration
                {
                    Context = this,
                    NodeId = nodeId.Trim(),
                    NodeType = nodeType ?? "",
                    SnapshotFingerprint = snapshotFingerprint ?? "",
                    Boundary = boundary,
                    Predicate = predicate,
                    BreakOnMatch = breakOnMatch,
                    BreakLabel = breakLabel ?? "",
                    Condition = condition ?? "",
                    SourceLine = Math.Max(0, sourceLine)
                };
                return new ExecuteCodeTickAwaitable(registration);
            }
        }

        private static void ResolveLoopPoint(
            UnityLoopPoint point,
            out Type type,
            out UnityTickBoundary boundary)
        {
            switch (point)
            {
                case UnityLoopPoint.BeforeFixedUpdate:
                    type = typeof(UnityEngine.PlayerLoop.FixedUpdate.ScriptRunBehaviourFixedUpdate);
                    boundary = UnityTickBoundary.Before;
                    return;
                case UnityLoopPoint.AfterFixedUpdate:
                    type = typeof(UnityEngine.PlayerLoop.FixedUpdate.ScriptRunBehaviourFixedUpdate);
                    boundary = UnityTickBoundary.After;
                    return;
                case UnityLoopPoint.BeforeUpdate:
                    type = typeof(UnityEngine.PlayerLoop.Update.ScriptRunBehaviourUpdate);
                    boundary = UnityTickBoundary.Before;
                    return;
                case UnityLoopPoint.AfterUpdate:
                    type = typeof(UnityEngine.PlayerLoop.Update.ScriptRunBehaviourUpdate);
                    boundary = UnityTickBoundary.After;
                    return;
                case UnityLoopPoint.BeforeLateUpdate:
                    type = typeof(UnityEngine.PlayerLoop.PreLateUpdate.ScriptRunBehaviourLateUpdate);
                    boundary = UnityTickBoundary.Before;
                    return;
                case UnityLoopPoint.AfterLateUpdate:
                    type = typeof(UnityEngine.PlayerLoop.PreLateUpdate.ScriptRunBehaviourLateUpdate);
                    boundary = UnityTickBoundary.After;
                    return;
                case UnityLoopPoint.EndOfFrame:
                    type = typeof(UnityEngine.PlayerLoop.PostLateUpdate.TriggerEndOfFrameCallbacks);
                    boundary = UnityTickBoundary.After;
                    return;
                default:
                    throw new ArgumentOutOfRangeException("point", point, "Unsupported PlayerLoop point.");
            }
        }

        public struct ExecuteCodeTickAwaitable
        {
            private readonly ExecuteCodeTickWaitRegistration _registration;

            internal ExecuteCodeTickAwaitable(ExecuteCodeTickWaitRegistration registration)
            {
                _registration = registration;
            }

            public Awaiter GetAwaiter()
            {
                return new Awaiter(_registration);
            }

            public struct Awaiter : System.Runtime.CompilerServices.ICriticalNotifyCompletion
            {
                private readonly ExecuteCodeTickWaitRegistration _registration;

                internal Awaiter(ExecuteCodeTickWaitRegistration registration)
                {
                    _registration = registration;
                }

                public bool IsCompleted
                {
                    get { return _registration == null || _registration.Completed; }
                }

                public UnityTickStamp GetResult()
                {
                    if (_registration == null)
                        return null;
                    if (_registration.Error != null)
                        throw _registration.Error;
                    _registration.Context.ThrowIfCancellationRequested();
                    return _registration.Result;
                }

                public void OnCompleted(Action continuation)
                {
                    if (_registration == null)
                    {
                        continuation();
                        return;
                    }
                    _registration.Continuation = continuation;
                    ScheduleTickWait(_registration);
                }

                public void UnsafeOnCompleted(Action continuation)
                {
                    OnCompleted(continuation);
                }
            }
        }
    }
}
