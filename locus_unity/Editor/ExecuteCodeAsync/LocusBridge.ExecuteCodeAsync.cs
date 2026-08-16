using UnityEngine;
using UnityEditor;

using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Reflection;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using Microsoft.CodeAnalysis.Emit;
using Assembly = System.Reflection.Assembly;

namespace Locus
{
    public static partial class LocusBridge
    {
        private const double AsyncExecutePumpRequestIntervalSeconds = 0.05;
        private const int AsyncExecuteInactivityPollMs = 250;
        private const int ExecuteCodeLockWaitTimeoutMs = 30000;
        private const int ExecuteClientHeartbeatTimeoutMs = 120000;

        private static readonly object _executeAsyncContinuationQueueLock = new object();
        private static readonly List<ExecuteCodeWaitState> _executeAsyncContinuationQueue =
            new List<ExecuteCodeWaitState>(64);
        private static int _executeAsyncEditorUpdateTick;
        private static int _activeAsyncExecuteCount;
        private static bool _hasSavedRunInBackground;
        private static bool _savedRunInBackground;
        private static double _lastAsyncExecutePumpRequestSeconds;
        private static readonly bool ExecuteCodeDebugLoggingEnabled = IsExecuteCodeDebugLoggingEnabled();
        private const string ExecuteCodeExecutionIdMarker = "//__LOCUS_EXECUTION_ID__:";

        private sealed class CompiledAsyncSnippet
        {
            public readonly Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>> Executor;

            public CompiledAsyncSnippet(
                Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>> executor)
            {
                Executor = executor;
            }
        }

        internal sealed class AsyncSnippetExecution : IDisposable
        {
            private long _lastActivityTimestamp;

            public readonly CancellationTokenSource Cancellation = new CancellationTokenSource();
            public readonly TaskCompletionSource<string> Completion = LocusAsync.CreateTcs<string>();

            public AsyncSnippetExecution()
            {
                TouchActivity();
            }

            public void TouchActivity()
            {
                Interlocked.Exchange(
                    ref _lastActivityTimestamp,
                    System.Diagnostics.Stopwatch.GetTimestamp());
            }

            public double IdleSeconds
            {
                get
                {
                    long last = Interlocked.Read(ref _lastActivityTimestamp);
                    long now = System.Diagnostics.Stopwatch.GetTimestamp();
                    long elapsed = now - last;
                    if (elapsed <= 0)
                        return 0;

                    return elapsed / (double)System.Diagnostics.Stopwatch.Frequency;
                }
            }

            public void Cancel()
            {
                try
                {
                    Cancellation.Cancel();
                }
                catch
                {
                }
            }

            public void Dispose()
            {
                Cancellation.Dispose();
            }
        }

        internal sealed class ExecuteCodeRequestState : IDisposable
        {
            private readonly object _lock = new object();
            private AsyncSnippetExecution _execution;
            private long _lastClientHeartbeatTimestamp;
            private int _clientHeartbeatCount;
            private volatile bool _disposed;
            private ExecuteCodeProgressSnapshot _progress = new ExecuteCodeProgressSnapshot
            {
                active = false,
                title = "",
                info = "",
                progress = 0,
                revision = 0,
                source = ""
            };
            private int _progressRevision;
            private string _progressJsonCache;
            private int _progressJsonCacheRevision = int.MinValue;
            private ExecuteCodeProgressSnapshot _pendingProgressSnapshot;
            private readonly string[] _sourceLines;
            private long _waitStartedTimestamp;

            public readonly string ExecutionId;
            public readonly CancellationTokenSource Cancellation = new CancellationTokenSource();

            public ExecuteCodeRequestState(string executionId, string sourceCode)
            {
                ExecutionId = executionId;
                string leadingUsings;
                string bodyCode;
                SplitLeadingUsings(sourceCode ?? "", out leadingUsings, out bodyCode);
                _sourceLines = bodyCode.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n');
            }

            public bool IsCancellationRequested
            {
                get
                {
                    if (_disposed)
                        return true;

                    try
                    {
                        return Cancellation.IsCancellationRequested;
                    }
                    catch (ObjectDisposedException)
                    {
                        return true;
                    }
                }
            }

            public void SetExecution(AsyncSnippetExecution execution)
            {
                if (execution == null)
                    return;

                bool shouldCancel;
                lock (_lock)
                {
                    _execution = execution;
                    shouldCancel = Cancellation.IsCancellationRequested;
                }

                if (shouldCancel)
                    execution.Cancel();
            }

            public void TouchClientHeartbeat()
            {
                if (_disposed)
                    return;

                Interlocked.Exchange(
                    ref _lastClientHeartbeatTimestamp,
                    System.Diagnostics.Stopwatch.GetTimestamp());
                Interlocked.Increment(ref _clientHeartbeatCount);
            }

            public int ClientHeartbeatCount
            {
                get { return Interlocked.CompareExchange(ref _clientHeartbeatCount, 0, 0); }
            }

            public double ClientHeartbeatIdleSeconds
            {
                get
                {
                    long last = Interlocked.Read(ref _lastClientHeartbeatTimestamp);
                    if (last <= 0)
                        return 0;

                    long now = System.Diagnostics.Stopwatch.GetTimestamp();
                    long elapsed = now - last;
                    if (elapsed <= 0)
                        return 0;

                    return elapsed / (double)System.Diagnostics.Stopwatch.Frequency;
                }
            }

            public void ClearExecution(AsyncSnippetExecution execution)
            {
                lock (_lock)
                {
                    if (ReferenceEquals(_execution, execution))
                        _execution = null;
                }
            }

            public void Cancel()
            {
                AsyncSnippetExecution execution;
                try
                {
                    if (!_disposed)
                        Cancellation.Cancel();
                }
                catch
                {
                }

                lock (_lock)
                {
                    execution = _execution;
                }

                if (execution != null)
                    execution.Cancel();
            }

            public void ThrowIfCancellationRequested()
            {
                if (_disposed)
                    throw new OperationCanceledException();

                Cancellation.Token.ThrowIfCancellationRequested();
            }

            public void SetStage(string info)
            {
                SetProgress(info, "", 0, "stage");
            }

            public void SetApiProgress(string title, string info, float progress)
            {
                SetProgress(title, info, progress, "api");
            }

            public void SetAwaiting(
                string kind,
                string target,
                string condition,
                int sourceLine)
            {
                lock (_lock)
                {
                    if (_progress != null && _progress.source == "api")
                        _pendingProgressSnapshot = _progress;
                    _waitStartedTimestamp = System.Diagnostics.Stopwatch.GetTimestamp();
                    _progressRevision++;
                    _progress = new ExecuteCodeProgressSnapshot
                    {
                        active = true,
                        title = "Awaiting Unity",
                        info = "",
                        progress = 0,
                        revision = _progressRevision,
                        source = "await",
                        waitKind = kind ?? "",
                        waitTarget = target ?? "",
                        waitCondition = condition ?? "",
                        sourceLine = Math.Max(0, sourceLine),
                        sourceText = SourceLineText(sourceLine),
                        waitedMs = 0
                    };
                    _progressJsonCache = null;
                }
            }

            public void ClearAwaiting()
            {
                lock (_lock)
                {
                    if (_progress == null || _progress.source != "await")
                        return;
                    _waitStartedTimestamp = 0;
                    _progressRevision++;
                    _progress = new ExecuteCodeProgressSnapshot
                    {
                        active = true,
                        title = "Executing snippet",
                        info = "",
                        progress = 0,
                        revision = _progressRevision,
                        source = "stage"
                    };
                    _progressJsonCache = null;
                }
            }

            public void ResetProgress()
            {
                lock (_lock)
                {
                    _pendingProgressSnapshot = null;
                    _progressRevision++;
                    _progress = new ExecuteCodeProgressSnapshot
                    {
                        active = false,
                        title = "",
                        info = "",
                        progress = 0,
                        revision = _progressRevision,
                        source = ""
                    };
                    _progressJsonCache = null;
                }
            }

            public string GetProgressJson()
            {
                lock (_lock)
                {
                    if (_pendingProgressSnapshot != null)
                    {
                        ExecuteCodeProgressSnapshot pending = _pendingProgressSnapshot;
                        _pendingProgressSnapshot = null;
                        return JsonUtility.ToJson(pending);
                    }
                    if (_progress != null && _progress.source == "await" && _waitStartedTimestamp > 0)
                    {
                        long elapsed = System.Diagnostics.Stopwatch.GetTimestamp() - _waitStartedTimestamp;
                        _progress.waitedMs = elapsed <= 0
                            ? 0
                            : (int)Math.Min(int.MaxValue,
                                elapsed * 1000L / System.Diagnostics.Stopwatch.Frequency);
                        _progressRevision++;
                        _progress.revision = _progressRevision;
                        _progressJsonCache = null;
                    }
                    if (_progressJsonCache == null || _progressJsonCacheRevision != _progressRevision)
                    {
                        _progressJsonCache = JsonUtility.ToJson(_progress);
                        _progressJsonCacheRevision = _progressRevision;
                    }
                    return _progressJsonCache;
                }
            }

            private string SourceLineText(int sourceLine)
            {
                int index = sourceLine - 1;
                if (index < 0 || index >= _sourceLines.Length)
                    return "";
                return (_sourceLines[index] ?? "").Trim();
            }

            private void SetProgress(string title, string info, float progress, string source)
            {
                lock (_lock)
                {
                    if (source == "api")
                        _pendingProgressSnapshot = null;
                    _progressRevision++;
                    _progress = new ExecuteCodeProgressSnapshot
                    {
                        active = true,
                        title = string.IsNullOrEmpty(title) ? "Locus" : title,
                        info = info ?? "",
                        progress = Mathf.Clamp01(progress),
                        revision = _progressRevision,
                        source = string.IsNullOrEmpty(source) ? "api" : source
                    };
                    _progressJsonCache = null;
                }
            }

            public void Dispose()
            {
                Cancel();
                _disposed = true;
                Cancellation.Dispose();
            }
        }

        private static readonly object _executeCodeRequestStateLock = new object();
        private static readonly Dictionary<string, ExecuteCodeRequestState> _executeCodeRequestStates =
            new Dictionary<string, ExecuteCodeRequestState>(StringComparer.Ordinal);

        private const int CompletedExecuteCodeEntryLimit = 256;
        private static readonly long CompletedExecuteCodeEntryLifetimeTicks = TimeSpan.FromMinutes(10).Ticks;
        private static readonly object _executeCodeIdempotencyLock = new object();
        private static readonly Dictionary<string, ExecuteCodeIdempotencyEntry> _activeExecuteCodeEntries =
            new Dictionary<string, ExecuteCodeIdempotencyEntry>(StringComparer.Ordinal);
        private static readonly Dictionary<string, ExecuteCodeIdempotencyEntry> _completedExecuteCodeEntries =
            new Dictionary<string, ExecuteCodeIdempotencyEntry>(StringComparer.Ordinal);

        private sealed class ExecuteCodeIdempotencyEntry
        {
            public readonly string ExecutionId;
            public readonly string RequestSignature;
            public readonly TaskCompletionSource<PipeEnvelope> Completion =
                LocusAsync.CreateTcs<PipeEnvelope>();
            public PipeEnvelope CompletedResponse;
            public long CompletedAtUtcTicks;

            public ExecuteCodeIdempotencyEntry(string executionId, string requestSignature)
            {
                ExecutionId = executionId;
                RequestSignature = requestSignature;
            }
        }

        private static string NormalizeExecuteCodeExecutionId(string executionId, string requestId)
        {
            string normalized = (executionId ?? "").Trim();
            return string.IsNullOrEmpty(normalized) ? (requestId ?? Guid.NewGuid().ToString("N")) : normalized;
        }

        private static string HashExecuteCodePayload(byte[] payload)
        {
            using (SHA256 sha256 = SHA256.Create())
                return Convert.ToBase64String(sha256.ComputeHash(payload ?? new byte[0]));
        }

        private static string HashExecuteCodePayload(string payload)
        {
            return HashExecuteCodePayload(Encoding.UTF8.GetBytes(payload ?? ""));
        }

        private static string BuildExecuteCodeRequestSignature(string kind, string sourceCode)
        {
            return (kind ?? "execute_code") + "|" + HashExecuteCodePayload(sourceCode);
        }

        private static string BuildExecuteLoadedRequestSignature(
            string entryTypeName,
            string sourceCode,
            byte[] assemblyBytes)
        {
            return "execute_loaded|" +
                (entryTypeName ?? "") + "|" +
                HashExecuteCodePayload(sourceCode) + "|" +
                HashExecuteCodePayload(assemblyBytes);
        }

        private static PipeEnvelope RebindExecuteCodeResponse(PipeEnvelope response, string requestId)
        {
            if (response == null)
                return ErrorResponse(requestId, "execute_code completed without a response");

            return new PipeEnvelope
            {
                id = response.id,
                reply_to = requestId,
                type = response.type,
                ok = response.ok,
                message = response.message,
                error = response.error,
                processId = response.processId,
                processPath = response.processPath
            };
        }

        private static void CleanupCompletedExecuteCodeEntriesLocked(long nowUtcTicks)
        {
            string[] expired = _completedExecuteCodeEntries
                .Where(pair => nowUtcTicks - pair.Value.CompletedAtUtcTicks > CompletedExecuteCodeEntryLifetimeTicks)
                .Select(pair => pair.Key)
                .ToArray();
            for (int i = 0; i < expired.Length; i++)
                _completedExecuteCodeEntries.Remove(expired[i]);

            int removeCount = _completedExecuteCodeEntries.Count - CompletedExecuteCodeEntryLimit;
            if (removeCount <= 0)
                return;

            string[] oldest = _completedExecuteCodeEntries
                .OrderBy(pair => pair.Value.CompletedAtUtcTicks)
                .Take(removeCount)
                .Select(pair => pair.Key)
                .ToArray();
            for (int i = 0; i < oldest.Length; i++)
                _completedExecuteCodeEntries.Remove(oldest[i]);
        }

        private static ExecuteCodeIdempotencyEntry AcquireExecuteCodeEntry(
            string executionId,
            string requestSignature,
            out bool isOwner,
            out string error)
        {
            lock (_executeCodeIdempotencyLock)
            {
                CleanupCompletedExecuteCodeEntriesLocked(DateTime.UtcNow.Ticks);

                ExecuteCodeIdempotencyEntry entry;
                if (_activeExecuteCodeEntries.TryGetValue(executionId, out entry)
                    || _completedExecuteCodeEntries.TryGetValue(executionId, out entry))
                {
                    isOwner = false;
                    error = string.Equals(entry.RequestSignature, requestSignature, StringComparison.Ordinal)
                        ? null
                        : "execution_id is already bound to a different request: " + executionId;
                    return entry;
                }

                entry = new ExecuteCodeIdempotencyEntry(executionId, requestSignature);
                _activeExecuteCodeEntries.Add(executionId, entry);
                isOwner = true;
                error = null;
                return entry;
            }
        }

        private static ExecuteCodeIdempotencyEntry FindExecuteCodeEntry(string executionId)
        {
            lock (_executeCodeIdempotencyLock)
            {
                CleanupCompletedExecuteCodeEntriesLocked(DateTime.UtcNow.Ticks);
                ExecuteCodeIdempotencyEntry entry;
                if (_activeExecuteCodeEntries.TryGetValue(executionId, out entry)
                    || _completedExecuteCodeEntries.TryGetValue(executionId, out entry))
                    return entry;
                return null;
            }
        }

        private static void CompleteExecuteCodeEntry(
            ExecuteCodeIdempotencyEntry entry,
            PipeEnvelope response)
        {
            PipeEnvelope completedResponse = RebindExecuteCodeResponse(response, null);
            lock (_executeCodeIdempotencyLock)
            {
                ExecuteCodeIdempotencyEntry current;
                if (_activeExecuteCodeEntries.TryGetValue(entry.ExecutionId, out current)
                    && ReferenceEquals(current, entry))
                    _activeExecuteCodeEntries.Remove(entry.ExecutionId);

                entry.CompletedResponse = completedResponse;
                entry.CompletedAtUtcTicks = DateTime.UtcNow.Ticks;
                _completedExecuteCodeEntries[entry.ExecutionId] = entry;
                CleanupCompletedExecuteCodeEntriesLocked(entry.CompletedAtUtcTicks);
            }
            entry.Completion.TrySetResult(completedResponse);
        }

        private static async Task<PipeEnvelope> HandleWaitExecuteCode(string requestId, string executionId)
        {
            string normalized = (executionId ?? "").Trim();
            if (string.IsNullOrEmpty(normalized))
                return ErrorResponse(requestId, "execute_code_wait requires execution_id");

            ExecuteCodeIdempotencyEntry entry = FindExecuteCodeEntry(normalized);
            if (entry == null)
                return ErrorResponse(requestId, "unknown execution_id: " + normalized);

            PipeEnvelope response = await entry.Completion.Task.ConfigureAwait(false);
            return RebindExecuteCodeResponse(response, requestId);
        }

        private static ExecuteCodeRequestState FindExecuteCodeRequestState(string executionId)
        {
            string normalized = (executionId ?? "").Trim();
            lock (_executeCodeRequestStateLock)
            {
                ExecuteCodeRequestState state;
                return !string.IsNullOrEmpty(normalized)
                    && _executeCodeRequestStates.TryGetValue(normalized, out state)
                    ? state
                    : null;
            }
        }

        private static void RegisterExecuteCodeRequestState(ExecuteCodeRequestState requestState)
        {
            lock (_executeCodeRequestStateLock)
            {
                _executeCodeRequestStates[requestState.ExecutionId] = requestState;
            }
        }

        private static void UnregisterExecuteCodeRequestState(ExecuteCodeRequestState requestState)
        {
            if (requestState == null)
                return;
            lock (_executeCodeRequestStateLock)
            {
                ExecuteCodeRequestState current;
                if (_executeCodeRequestStates.TryGetValue(requestState.ExecutionId, out current)
                    && ReferenceEquals(current, requestState))
                    _executeCodeRequestStates.Remove(requestState.ExecutionId);
            }
        }

        private static void TouchExecuteCodeClientHeartbeat(string executionId)
        {
            ExecuteCodeRequestState requestState = FindExecuteCodeRequestState(executionId);
            if (requestState != null)
                requestState.TouchClientHeartbeat();
        }

        private static void CancelActiveExecuteCode(string reason)
        {
            ExecuteCodeRequestState[] states;
            lock (_executeCodeRequestStateLock)
                states = _executeCodeRequestStates.Values.ToArray();
            for (int i = 0; i < states.Length; i++)
                states[i].Cancel();

            if (states.Length > 0 && !string.IsNullOrEmpty(reason))
                Debug.LogWarning("[Locus] execute_code canceled: " + reason);
        }

        private static string InactiveExecuteCodeProgressJson()
        {
            return "{\"active\":false,\"title\":\"\",\"info\":\"\",\"progress\":0,\"revision\":0,\"source\":\"\"}";
        }

        private static string GetExecuteCodeProgressJson(string executionId)
        {
            ExecuteCodeRequestState requestState = FindExecuteCodeRequestState(executionId);
            return requestState != null ? requestState.GetProgressJson() : InactiveExecuteCodeProgressJson();
        }

        private static void LogExecuteCodeDebug(string requestId, string message)
        {
            if (!ExecuteCodeDebugLoggingEnabled)
                return;

            Debug.Log("[Locus] execute_code[" + (requestId ?? "?") + "] " + message);
        }

        private static void LogExecuteLoadedDebug(string requestId, string message)
        {
            if (!ExecuteCodeDebugLoggingEnabled)
                return;

            Debug.Log("[Locus] execute_loaded[" + (requestId ?? "?") + "] " + message);
        }

        private static bool IsExecuteCodeDebugLoggingEnabled()
        {
            return IsTruthyEnvironmentValue(Environment.GetEnvironmentVariable("LOCUS_EXECUTE_CODE_DEBUG"))
                || IsTruthyEnvironmentValue(Environment.GetEnvironmentVariable("LOCUS_UNITY_EXECUTE_DEBUG"));
        }

        private static bool IsTruthyEnvironmentValue(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
                return false;

            string normalized = value.Trim();
            return string.Equals(normalized, "1", StringComparison.OrdinalIgnoreCase)
                || string.Equals(normalized, "true", StringComparison.OrdinalIgnoreCase)
                || string.Equals(normalized, "yes", StringComparison.OrdinalIgnoreCase)
                || string.Equals(normalized, "on", StringComparison.OrdinalIgnoreCase);
        }

        private static long DebugElapsedMilliseconds(System.Diagnostics.Stopwatch stopwatch)
        {
            return stopwatch != null ? stopwatch.ElapsedMilliseconds : 0;
        }

        private static PipeEnvelope HandleCancelExecuteCode(string requestId, string executionId)
        {
            string normalized = (executionId ?? "").Trim();
            if (!string.IsNullOrEmpty(normalized))
            {
                ExecuteCodeRequestState requestState = FindExecuteCodeRequestState(normalized);
                if (requestState == null)
                    return OkResponse(requestId, "no active execute_code for " + normalized);
                requestState.Cancel();
                requestState.ResetProgress();
                return OkResponse(requestId, "execute_code cancellation requested for " + normalized);
            }

            ExecuteCodeRequestState[] states;
            lock (_executeCodeRequestStateLock)
                states = _executeCodeRequestStates.Values.ToArray();
            if (states.Length == 0)
                return OkResponse(requestId, "no active execute_code");
            for (int i = 0; i < states.Length; i++)
            {
                states[i].Cancel();
                states[i].ResetProgress();
            }
            return OkResponse(requestId, "execute_code cancellation requested for " + states.Length + " executions");
        }

        private static async Task MonitorExecuteCodeClientHeartbeatAsync(ExecuteCodeRequestState requestState)
        {
            if (requestState == null)
                return;

            try
            {
                while (!requestState.IsCancellationRequested)
                {
                    await Task.Delay(AsyncExecuteInactivityPollMs).ConfigureAwait(false);

                    if (requestState.IsCancellationRequested)
                        return;

                    if (requestState.ClientHeartbeatCount <= 0)
                        continue;

                    if (requestState.ClientHeartbeatIdleSeconds < ExecuteClientHeartbeatTimeoutMs / 1000.0)
                        continue;

                    requestState.Cancel();
                    requestState.ResetProgress();
                    Debug.LogWarning(
                        "[Locus] execute_code canceled: client heartbeat timed out after " +
                        (ExecuteClientHeartbeatTimeoutMs / 1000) +
                        " seconds");
                    return;
                }
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] execute_code heartbeat monitor failed: " + ex);
            }
        }

        private static async Task<PipeEnvelope> HandleExecuteCode(string requestId, string code)
        {
            if (string.IsNullOrWhiteSpace(code))
                return ErrorResponse(requestId, "empty code");

            string executionId = ExtractExecuteCodeExecutionId(code, requestId);

            return await ExecuteSnippetRequestAsync(
                requestId,
                executionId,
                code,
                BuildExecuteCodeRequestSignature("execute_code", code),
                prepareCompiler: true,
                compileStage: "Compiling snippet",
                compile: delegate { return CompileAsyncSnippet(code); });
        }

        // ───────────────── execute_loaded (compile-server sidecar) ─────────────────

        [Serializable]
        private sealed class ExecuteLoadedRequest
        {
            public string assembly_b64;
            public string assembly_path;
            public string entry_type;
            public string execution_id;
            public string source_code;
        }

        /// <summary>
        /// Sidecar variant of execute_code: the snippet was already compiled
        /// by the Locus compile server; load the assembly bytes and run the
        /// entry point through the same execution pipeline (progress,
        /// heartbeat, cancellation) as the in-Unity compile path.
        /// </summary>
        private static async Task<PipeEnvelope> HandleExecuteLoaded(string requestId, string requestJson)
        {
            if (string.IsNullOrWhiteSpace(requestJson))
            {
                LogExecuteLoadedDebug(requestId, "empty request payload");
                return ErrorResponse(requestId, "empty execute_loaded request");
            }

            if (ExecuteCodeDebugLoggingEnabled)
                LogExecuteLoadedDebug(requestId, "received payload chars=" + requestJson.Length);

            ExecuteLoadedRequest request;
            try
            {
                request = JsonUtility.FromJson<ExecuteLoadedRequest>(requestJson);
            }
            catch (Exception ex)
            {
                if (ExecuteCodeDebugLoggingEnabled)
                    LogExecuteLoadedDebug(requestId, "parse failed: " + ex.Message);
                return ErrorResponse(requestId, "execute_loaded request parse failed: " + ex.Message);
            }

            if (request == null ||
                (string.IsNullOrEmpty(request.assembly_b64) &&
                 string.IsNullOrEmpty(request.assembly_path)))
            {
                LogExecuteLoadedDebug(requestId, "missing assembly bytes");
                return ErrorResponse(requestId, "execute_loaded request missing assembly bytes");
            }

            byte[] assemblyBytes;
            try
            {
                assemblyBytes = ReadAssemblyPayload(request.assembly_b64, request.assembly_path);
            }
            catch (Exception ex)
            {
                if (ExecuteCodeDebugLoggingEnabled)
                    LogExecuteLoadedDebug(requestId, "assembly load failed: " + ex.Message);
                return ErrorResponse(requestId, "execute_loaded assembly load failed: " + ex.Message);
            }

            string entryTypeName = string.IsNullOrEmpty(request.entry_type)
                ? "Locus.RuntimeSnippets.__LocusAsyncSnippetHost"
                : request.entry_type;
            if (ExecuteCodeDebugLoggingEnabled)
            {
                LogExecuteLoadedDebug(
                    requestId,
                    "decoded assembly bytes=" + assemblyBytes.Length + ", entry=" + entryTypeName);
            }

            return await ExecuteSnippetRequestAsync(
                requestId,
                NormalizeExecuteCodeExecutionId(request.execution_id, requestId),
                request.source_code ?? "",
                BuildExecuteLoadedRequestSignature(entryTypeName, request.source_code, assemblyBytes),
                prepareCompiler: false,
                compileStage: "Loading compiled snippet",
                compile: delegate { return LoadCompiledSnippet(requestId, assemblyBytes, entryTypeName); });
        }

        /// <summary>
        /// Load a sidecar-compiled snippet assembly and bind its entry point.
        /// Error wording mirrors TryCompileAsyncSnippet so the agent-facing
        /// error shape stays identical across both compile paths.
        /// </summary>
        private static CompiledAsyncSnippet LoadCompiledSnippet(
            string requestId,
            byte[] assemblyBytes,
            string entryTypeName)
        {
            Type hostType;
            MethodInfo executeMethod;
            System.Diagnostics.Stopwatch loadStarted = ExecuteCodeDebugLoggingEnabled
                ? System.Diagnostics.Stopwatch.StartNew()
                : null;
            try
            {
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteLoadedDebug(
                        requestId,
                        "Assembly.Load begin, bytes=" + (assemblyBytes == null ? 0 : assemblyBytes.Length));
                }
                Assembly assembly = Assembly.Load(assemblyBytes);
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteLoadedDebug(
                        requestId,
                        "Assembly.Load complete in " + DebugElapsedMilliseconds(loadStarted) + "ms: " +
                        assembly.FullName);
                }
                hostType = assembly.GetType(entryTypeName, true);
                if (ExecuteCodeDebugLoggingEnabled)
                    LogExecuteLoadedDebug(requestId, "entry type resolved: " + hostType.FullName);
                executeMethod = hostType.GetMethod(
                    "ExecuteAsync",
                    BindingFlags.Public | BindingFlags.Static
                );
            }
            catch (Exception ex)
            {
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteLoadedDebug(
                        requestId,
                        "assembly load/bootstrap failed after " +
                        DebugElapsedMilliseconds(loadStarted) +
                        "ms: " +
                        ex.Message);
                }
                throw new Exception("assembly load/bootstrap failed: " + ex);
            }

            if (executeMethod == null)
            {
                if (ExecuteCodeDebugLoggingEnabled)
                    LogExecuteLoadedDebug(requestId, "missing ExecuteAsync method on " + entryTypeName);
                throw new Exception("compiled async snippet missing ExecuteAsync method");
            }

            try
            {
                var executor =
                    (Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>>)
                        Delegate.CreateDelegate(
                            typeof(Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>>),
                            executeMethod
                        );

                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteLoadedDebug(
                        requestId,
                        "ExecuteAsync delegate bound in " + DebugElapsedMilliseconds(loadStarted) + "ms");
                }
                return new CompiledAsyncSnippet(executor);
            }
            catch (Exception ex)
            {
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteLoadedDebug(
                        requestId,
                        "delegate bind failed after " +
                        DebugElapsedMilliseconds(loadStarted) +
                        "ms: " +
                        ex.Message);
                }
                throw new Exception("assembly load/bootstrap failed: " + ex);
            }
        }

        /// <summary>
        /// Shared execute_code / execute_loaded pipeline: lock, request
        /// state, heartbeat monitor, snippet acquisition via `compile`,
        /// main-thread execution, progress and error mapping.
        /// </summary>
        private static async Task<PipeEnvelope> ExecuteSnippetRequestAsync(
            string requestId,
            string executionId,
            string sourceCode,
            string requestSignature,
            bool prepareCompiler,
            string compileStage,
            Func<CompiledAsyncSnippet> compile)
        {
            executionId = NormalizeExecuteCodeExecutionId(executionId, requestId);
            bool isOwner;
            string idempotencyError;
            ExecuteCodeIdempotencyEntry entry = AcquireExecuteCodeEntry(
                executionId,
                requestSignature,
                out isOwner,
                out idempotencyError);
            if (!string.IsNullOrEmpty(idempotencyError))
                return ErrorResponse(requestId, idempotencyError);

            if (!isOwner)
            {
                LogExecuteCodeDebug(requestId, "joining existing execution_id=" + executionId);
                PipeEnvelope existingResponse = await entry.Completion.Task.ConfigureAwait(false);
                return RebindExecuteCodeResponse(existingResponse, requestId);
            }

            PipeEnvelope ownerResponse;
            try
            {
                ownerResponse = await ExecuteSnippetRequestCoreAsync(
                    requestId,
                    executionId,
                    sourceCode,
                    prepareCompiler,
                    compileStage,
                    compile).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                ownerResponse = ErrorResponse(requestId, "execute_code exception: " + ex);
            }

            CompleteExecuteCodeEntry(entry, ownerResponse);
            return RebindExecuteCodeResponse(ownerResponse, requestId);
        }

        private static async Task<PipeEnvelope> ExecuteSnippetRequestCoreAsync(
            string requestId,
            string executionId,
            string sourceCode,
            bool prepareCompiler,
            string compileStage,
            Func<CompiledAsyncSnippet> compile)
        {
            System.Diagnostics.Stopwatch requestStarted = ExecuteCodeDebugLoggingEnabled
                ? System.Diagnostics.Stopwatch.StartNew()
                : null;
            if (ExecuteCodeDebugLoggingEnabled)
            {
                LogExecuteCodeDebug(
                    requestId,
                    "begin, prepareCompiler=" + prepareCompiler + ", acquireStage=" + compileStage);
            }

            bool lockTaken = false;
            try
            {
                LogExecuteCodeDebug(requestId, "waiting for execute lock");
                if (!await _executeCodeLock.WaitAsync(ExecuteCodeLockWaitTimeoutMs))
                {
                    if (ExecuteCodeDebugLoggingEnabled)
                    {
                        LogExecuteCodeDebug(
                            requestId,
                            "execute lock wait timed out after " +
                            (ExecuteCodeLockWaitTimeoutMs / 1000) +
                            "s");
                    }
                    return ErrorResponse(
                        requestId,
                        "execute_code lock wait timed out after " +
                        (ExecuteCodeLockWaitTimeoutMs / 1000) +
                        " seconds");
                }

                lockTaken = true;
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteCodeDebug(
                        requestId,
                        "execute lock acquired after " + DebugElapsedMilliseconds(requestStarted) + "ms");
                }
            }
            catch (ObjectDisposedException ex)
            {
                if (ExecuteCodeDebugLoggingEnabled)
                    LogExecuteCodeDebug(requestId, "execute lock unavailable: " + ex.Message);
                return ErrorResponse(requestId, "execute_code lock unavailable: " + ex.Message);
            }

            ExecuteCodeRequestState requestState = null;
            try
            {
                requestState = new ExecuteCodeRequestState(executionId, sourceCode);
                RegisterExecuteCodeRequestState(requestState);
                _ = MonitorExecuteCodeClientHeartbeatAsync(requestState);
                LogExecuteCodeDebug(requestId, "request state registered");

                requestState.ResetProgress();

                if (prepareCompiler)
                {
                    requestState.SetStage("Checking compiler cache");
                    LogExecuteCodeDebug(requestId, "checking Unity compiler cache");

                    string prepareError = await EnsureExecuteCodeCompilationReadyAsync(
                        requestState.SetStage,
                        requestState.Cancellation.Token);
                    if (!string.IsNullOrEmpty(prepareError))
                    {
                        requestState.ThrowIfCancellationRequested();
                        requestState.SetStage("Compiler preparation failed");
                        if (ExecuteCodeDebugLoggingEnabled)
                            LogExecuteCodeDebug(requestId, "compiler preparation failed: " + prepareError);
                        return ErrorResponse(requestId, prepareError);
                    }

                    requestState.ThrowIfCancellationRequested();
                    if (ExecuteCodeDebugLoggingEnabled)
                    {
                        LogExecuteCodeDebug(
                            requestId,
                            "Unity compiler cache ready after " + DebugElapsedMilliseconds(requestStarted) + "ms");
                    }
                }

                CompiledAsyncSnippet snippet;
                try
                {
                    requestState.SetStage(compileStage);
                    requestState.ThrowIfCancellationRequested();
                    if (ExecuteCodeDebugLoggingEnabled)
                        LogExecuteCodeDebug(requestId, "acquiring snippet: " + compileStage);
                    long acquireStartedMs = DebugElapsedMilliseconds(requestStarted);
                    snippet = compile();
                    requestState.ThrowIfCancellationRequested();
                    if (ExecuteCodeDebugLoggingEnabled)
                    {
                        LogExecuteCodeDebug(
                            requestId,
                            "snippet ready in " +
                            (DebugElapsedMilliseconds(requestStarted) - acquireStartedMs) +
                            "ms");
                    }
                }
                catch (OperationCanceledException)
                {
                    LogExecuteCodeDebug(requestId, "canceled while acquiring snippet");
                    throw;
                }
                catch (Exception ex)
                {
                    requestState.SetStage("Compilation failed");
                    if (ExecuteCodeDebugLoggingEnabled)
                        LogExecuteCodeDebug(requestId, "snippet acquisition failed: " + ex.Message);
                    return ErrorResponse(requestId, "async snippet compilation exception: " + ex.Message);
                }

                requestState.SetStage("Executing snippet");
                LogExecuteCodeDebug(requestId, "queueing snippet on Unity main thread");
                long executeStartedMs = DebugElapsedMilliseconds(requestStarted);
                Task<string> executionTask = ExecuteAsyncSnippetOnMainThreadAsync(snippet, requestState);
                if (lockTaken)
                {
                    _executeCodeLock.Release();
                    lockTaken = false;
                }
                string resultText = await executionTask;
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteCodeDebug(
                        requestId,
                        "main-thread execution returned in " +
                        (DebugElapsedMilliseconds(requestStarted) - executeStartedMs) +
                        "ms, output chars=" +
                        (resultText == null ? 0 : resultText.Length));
                }

                if (resultText.StartsWith("__ERROR__: ", StringComparison.Ordinal))
                {
                    requestState.ThrowIfCancellationRequested();
                    requestState.SetStage("Execution failed");
                    if (ExecuteCodeDebugLoggingEnabled)
                        LogExecuteCodeDebug(requestId, "execution failed: " + resultText);
                    return ErrorResponse(requestId, resultText.Substring("__ERROR__: ".Length));
                }

                requestState.ThrowIfCancellationRequested();
                requestState.SetStage("Execution complete");
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteCodeDebug(
                        requestId,
                        "complete after " + DebugElapsedMilliseconds(requestStarted) + "ms");
                }
                return OkResponse(requestId, resultText);
            }
            catch (OperationCanceledException)
            {
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteCodeDebug(
                        requestId,
                        "canceled after " + DebugElapsedMilliseconds(requestStarted) + "ms");
                }
                return ErrorResponse(requestId, "execute_code canceled");
            }
            finally
            {
                UnregisterExecuteCodeRequestState(requestState);
                if (requestState != null)
                    requestState.Dispose();
                if (lockTaken)
                    _executeCodeLock.Release();
                if (ExecuteCodeDebugLoggingEnabled)
                {
                    LogExecuteCodeDebug(
                        requestId,
                        "finished cleanup after " + DebugElapsedMilliseconds(requestStarted) + "ms");
                }
            }
        }

        private static CompiledAsyncSnippet CompileAsyncSnippet(string code)
        {
            string leadingUsings;
            string bodyCode;
            SplitLeadingUsings(code, out leadingUsings, out bodyCode);
            bool useAsyncWrapper = RequiresAsyncWrapper(bodyCode, SnippetParseOptions);

            CompiledAsyncSnippet snippet;
            string primaryError;

            if (TryCompileAsyncSnippet(
                    bodyCode,
                    leadingUsings,
                    false,
                    useAsyncWrapper,
                    out snippet,
                    out primaryError))
                return snippet;

            string fallbackError;
            if (TryCompileAsyncSnippet(
                    bodyCode,
                    leadingUsings,
                    true,
                    useAsyncWrapper,
                    out snippet,
                    out fallbackError))
                return snippet;

            var sb = new StringBuilder();

            if (!string.IsNullOrEmpty(primaryError))
                sb.Append(primaryError);

            if (!string.IsNullOrEmpty(fallbackError) &&
                !string.Equals(primaryError, fallbackError, StringComparison.Ordinal))
            {
                if (sb.Length > 0)
                    sb.Append("\n\nexpression fallback:\n");

                sb.Append(fallbackError);
            }

            throw new Exception(sb.Length > 0 ? sb.ToString() : "unknown async compilation failure");
        }

        private static bool TryCompileAsyncSnippet(
            string bodyCode,
            string leadingUsings,
            bool expressionMode,
            bool useAsyncWrapper,
            out CompiledAsyncSnippet snippet,
            out string error)
        {
            snippet = null;
            error = null;

            const string hostTypeName = "__LocusAsyncSnippetHost";
            const string fullTypeName = "Locus.RuntimeSnippets.__LocusAsyncSnippetHost";

            string source = BuildAsyncSnippetSource(
                hostTypeName,
                leadingUsings,
                bodyCode,
                expressionMode,
                useAsyncWrapper);

            SyntaxTree syntaxTree;
            try
            {
                syntaxTree = CSharpSyntaxTree.ParseText(
                    source,
                    SnippetParseOptions,
                    path: "LocusRuntimeAsyncSnippet.cs",
                    encoding: Utf8NoBom
                );
            }
            catch (Exception ex)
            {
                error = "parse failed: " + ex;
                return false;
            }

            string assemblyName =
                "__LocusRuntimeAsync_" + Interlocked.Increment(ref _snippetAssemblyCounter).ToString("X8");

            CSharpCompilation compilation = CSharpCompilation.Create(
                assemblyName: assemblyName,
                syntaxTrees: new[] { syntaxTree },
                references: EnsureMetadataReferences(),
                options: SnippetCompilationOptions
            );

            using (var peStream = new MemoryStream(16 * 1024))
            {
                EmitResult emitResult;
                using (EnterInProcessCompile())
                {
                    try
                    {
                        emitResult = compilation.Emit(peStream, cancellationToken: InProcessCompileReloadToken);
                    }
                    catch (Exception ex)
                    {
                        error = "emit failed: " + ex;
                        return false;
                    }
                }

                if (!emitResult.Success)
                {
                    error = BuildDiagnosticErrorText(emitResult.Diagnostics);
                    return false;
                }

                try
                {
                    byte[] assemblyBytes = peStream.ToArray();
                    Assembly assembly = Assembly.Load(assemblyBytes);

                    Type hostType = assembly.GetType(fullTypeName, true);
                    MethodInfo executeMethod = hostType.GetMethod(
                        "ExecuteAsync",
                        BindingFlags.Public | BindingFlags.Static
                    );

                    if (executeMethod == null)
                    {
                        error = "compiled async snippet missing ExecuteAsync method";
                        return false;
                    }

                    var executor =
                        (Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>>)
                            Delegate.CreateDelegate(
                                typeof(Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>>),
                                executeMethod
                            );

                    snippet = new CompiledAsyncSnippet(executor);
                    return true;
                }
                catch (Exception ex)
                {
                    error = "assembly load/bootstrap failed: " + ex;
                    return false;
                }
            }
        }

        private static bool RequiresAsyncWrapper(
            string bodyCode,
            CSharpParseOptions parseOptions)
        {
            if (string.IsNullOrWhiteSpace(bodyCode))
                return false;

            StatementSyntax body = SyntaxFactory.ParseStatement(
                "{\n" + bodyCode + "\n}",
                options: parseOptions,
                consumeFullText: true);
            return ContainsTopLevelAwait(body);
        }

        private static string ExtractExecuteCodeExecutionId(string code, string requestId)
        {
            int marker = (code ?? "").LastIndexOf(ExecuteCodeExecutionIdMarker, StringComparison.Ordinal);
            if (marker < 0)
                return NormalizeExecuteCodeExecutionId(null, requestId);
            int start = marker + ExecuteCodeExecutionIdMarker.Length;
            int end = code.IndexOfAny(new[] { '\r', '\n' }, start);
            string value = end >= 0 ? code.Substring(start, end - start) : code.Substring(start);
            return NormalizeExecuteCodeExecutionId(value, requestId);
        }

        private static bool ContainsTopLevelAwait(SyntaxNode node)
        {
            if (node is AnonymousFunctionExpressionSyntax || node is LocalFunctionStatementSyntax)
                return false;

            foreach (SyntaxToken token in node.ChildTokens())
            {
                if (token.IsKind(SyntaxKind.AwaitKeyword))
                    return true;
            }

            foreach (SyntaxNode child in node.ChildNodes())
            {
                if (ContainsTopLevelAwait(child))
                    return true;
            }

            return false;
        }

        private static string BuildAsyncSnippetSource(
            string hostTypeName,
            string leadingUsings,
            string bodyCode,
            bool expressionMode,
            bool useAsyncWrapper)
        {
            var sb = new StringBuilder(4096);

            sb.AppendLine("using System;");
            AppendCommonIoAliases(sb);
            sb.AppendLine("using System.Text;");
            sb.AppendLine("using System.Linq;");
            sb.AppendLine("using System.Reflection;");
            sb.AppendLine("using System.Threading;");
            sb.AppendLine("using System.Threading.Tasks;");
            sb.AppendLine("using System.Collections;");
            sb.AppendLine("using System.Collections.Generic;");
            sb.AppendLine("using UnityEngine;");
            sb.AppendLine("using UnityEngine.SceneManagement;");
            sb.AppendLine("using UnityEditor;");
            sb.AppendLine("using UnityEditor.SceneManagement;");
            sb.AppendLine("using UnityEditor.Animations;");
            sb.AppendLine("using Locus;");
            sb.AppendLine("using static UnityEngine.Object;");
            sb.AppendLine("using Object = UnityEngine.Object;");

            if (!string.IsNullOrWhiteSpace(leadingUsings))
                sb.AppendLine(leadingUsings);

            sb.AppendLine("namespace Locus.RuntimeSnippets");
            sb.AppendLine("{");
            sb.Append("    public static class ").Append(hostTypeName).AppendLine();
            sb.AppendLine("    {");

            if (useAsyncWrapper)
            {
                sb.AppendLine("        public static async global::System.Threading.Tasks.Task<object> ExecuteAsync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
            }
            else
            {
                sb.AppendLine("        public static global::System.Threading.Tasks.Task<object> ExecuteAsync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
                sb.AppendLine("        {");
                sb.AppendLine("            return global::System.Threading.Tasks.Task.FromResult<object>(ExecuteSync(globals, ctx, cancellationToken));");
                sb.AppendLine("        }");
                sb.AppendLine();
                sb.AppendLine("        private static object ExecuteSync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
            }

            sb.AppendLine("        {");
            sb.AppendLine("            var print = new global::System.Action<object>(globals.print);");
            sb.AppendLine("            var printJson = new global::System.Action<object>(globals.printJson);");
            sb.AppendLine("            var clear = new global::System.Action(globals.clear);");
            sb.AppendLine("            var ct = cancellationToken;");
            sb.AppendLine("            ctx.ThrowIfCancellationRequested();");
            sb.AppendLine("            #line 1");

            if (expressionMode)
            {
                if (string.IsNullOrWhiteSpace(bodyCode))
                {
                    sb.AppendLine("            return null;");
                }
                else
                {
                    sb.Append("            return (object)(");
                    sb.Append(bodyCode);
                    sb.AppendLine(");");
                }
            }
            else
            {
                if (!string.IsNullOrWhiteSpace(bodyCode))
                    sb.AppendLine(bodyCode);

                sb.AppendLine("            return null;");
            }

            sb.AppendLine("            #line default");
            sb.AppendLine("        }");
            sb.AppendLine("    }");
            sb.AppendLine("}");

            return sb.ToString();
        }

        private static Task<string> ExecuteAsyncSnippetOnMainThreadAsync(
            CompiledAsyncSnippet snippet,
            ExecuteCodeRequestState requestState)
        {
            var execution = new AsyncSnippetExecution();
            if (requestState != null)
                requestState.SetExecution(execution);

            if (requestState != null && requestState.IsCancellationRequested)
            {
                execution.Cancel();
                execution.Completion.TrySetResult("__ERROR__: execution canceled");
                return execution.Completion.Task;
            }

            PostToMainThread(delegate
            {
                if (requestState != null && requestState.IsCancellationRequested)
                {
                    execution.Cancel();
                    execution.Completion.TrySetResult("__ERROR__: execution canceled");
                    return;
                }

                RunAsyncSnippetOnMainThread(snippet, execution, requestState);
            });

            _ = MonitorAsyncSnippetInactivityAsync(execution);

            return execution.Completion.Task;
        }

        private static async Task MonitorAsyncSnippetInactivityAsync(AsyncSnippetExecution execution)
        {
            try
            {
                while (!execution.Completion.Task.IsCompleted)
                {
                    await Task.Delay(AsyncExecuteInactivityPollMs).ConfigureAwait(false);

                    if (execution.Completion.Task.IsCompleted)
                        return;

                    if (execution.IdleSeconds < ExecuteTimeoutMs / 1000.0)
                        continue;

                    execution.Cancel();
                    execution.Completion.TrySetResult(
                        "__ERROR__: execution timed out after " +
                        (ExecuteTimeoutMs / 1000) +
                        " seconds without print/progress output");
                    return;
                }
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Async execute timeout monitor failed: " + ex);
            }
        }

        private static async void RunAsyncSnippetOnMainThread(
            CompiledAsyncSnippet snippet,
            AsyncSnippetExecution execution,
            ExecuteCodeRequestState requestState)
        {
            BeginAsyncExecuteRuntime();

            ExecuteCodeContext ctx = null;
            ScriptGlobals globals = null;

            try
            {
                if (requestState != null)
                    requestState.ThrowIfCancellationRequested();

                globals = new ScriptGlobals(execution.TouchActivity);
                ctx = new ExecuteCodeContext(execution.Cancellation, execution.TouchActivity, requestState);

                object returnValue = await snippet.Executor(globals, ctx, execution.Cancellation.Token);

                if (returnValue != null)
                    globals.print(returnValue);

                execution.Completion.TrySetResult(globals.GetOutput());
            }
            catch (ExecuteCodeBreakpointReachedException breakpoint)
            {
                string output = globals != null ? globals.GetOutput() : "";
                string result = breakpoint.Result != null
                    ? breakpoint.Result.ToResultText()
                    : "status: breakpoint\neditor_status: playing_paused";
                if (!string.IsNullOrEmpty(output))
                    result = output.TrimEnd() + "\n" + result;
                execution.Completion.TrySetResult(result);
            }
            catch (OperationCanceledException)
            {
                execution.Completion.TrySetResult("__ERROR__: execution canceled");
            }
            catch (Exception ex)
            {
                execution.Completion.TrySetResult("__ERROR__: runtime error: " + ex);
            }
            finally
            {
                if (ctx != null)
                    ctx.ClearProgress();

                if (requestState != null)
                    requestState.ClearExecution(execution);

                execution.Dispose();
                EndAsyncExecuteRuntime();
            }
        }

        private static void PumpExecuteCodeAsyncRuntime()
        {
            _executeAsyncEditorUpdateTick++;
            PumpExecuteCodeContinuations();
            RequestAsyncExecuteEditorPump();
        }

        private static void BeginAsyncExecuteRuntime()
        {
            if (_activeAsyncExecuteCount == 0)
            {
                try
                {
                    _savedRunInBackground = Application.runInBackground;
                    _hasSavedRunInBackground = true;
                    Application.runInBackground = true;
                }
                catch
                {
                    _hasSavedRunInBackground = false;
                }
            }

            _activeAsyncExecuteCount++;
            RequestAsyncExecuteEditorPump();
        }

        private static void EndAsyncExecuteRuntime()
        {
            if (_activeAsyncExecuteCount > 0)
                _activeAsyncExecuteCount--;

            if (_activeAsyncExecuteCount != 0)
                return;

            try
            {
                EditorUtility.ClearProgressBar();
            }
            catch
            {
            }

            if (_hasSavedRunInBackground)
            {
                try
                {
                    Application.runInBackground = _savedRunInBackground;
                }
                catch
                {
                }
            }

            _hasSavedRunInBackground = false;
            RequestTickSchedulerClear();
        }

        private static void ScheduleExecuteContinuation(ExecuteCodeWaitState state)
        {
            if (state == null || state.Continuation == null)
                return;

            lock (_executeAsyncContinuationQueueLock)
            {
                _executeAsyncContinuationQueue.Add(state);
            }

            RequestAsyncExecuteEditorPump();
        }

        private static void RequestAsyncExecuteEditorPump()
        {
            if (_activeAsyncExecuteCount <= 0)
                return;

            try
            {
                double now = EditorApplication.timeSinceStartup;
                if (now - _lastAsyncExecutePumpRequestSeconds < AsyncExecutePumpRequestIntervalSeconds)
                    return;

                _lastAsyncExecutePumpRequestSeconds = now;
                EditorApplication.QueuePlayerLoopUpdate();
            }
            catch
            {
            }
        }

        private static void PumpExecuteCodeContinuations()
        {
            List<ExecuteCodeWaitState> ready = null;
            double now = EditorApplication.timeSinceStartup;

            lock (_executeAsyncContinuationQueueLock)
            {
                if (_executeAsyncContinuationQueue.Count == 0)
                    return;

                for (int i = _executeAsyncContinuationQueue.Count - 1; i >= 0; i--)
                {
                    ExecuteCodeWaitState state = _executeAsyncContinuationQueue[i];
                    if (state == null || state.IsReady(_executeAsyncEditorUpdateTick, now))
                    {
                        _executeAsyncContinuationQueue.RemoveAt(i);
                        if (state != null)
                        {
                            if (ready == null)
                                ready = new List<ExecuteCodeWaitState>();
                            ready.Add(state);
                        }
                    }
                }
            }

            if (ready == null)
                return;

            for (int i = ready.Count - 1; i >= 0; i--)
            {
                ExecuteCodeWaitState state = ready[i];
                try
                {
                    state.InvokeContinuation();
                }
                catch (Exception ex)
                {
                    Debug.LogError("[Locus] Async execute continuation failed: " + ex);
                }
            }
        }

        public sealed partial class ExecuteCodeContext
        {
            private readonly CancellationTokenSource _cancellation;
            private readonly Action _touchActivity;
            private readonly ExecuteCodeRequestState _requestState;
            private Exception _waitException;

            internal ExecuteCodeContext(
                CancellationTokenSource cancellation,
                Action touchActivity,
                ExecuteCodeRequestState requestState)
            {
                _cancellation = cancellation;
                _touchActivity = touchActivity;
                _requestState = requestState;
            }

            public CancellationToken CancellationToken
            {
                get { return _cancellation.Token; }
            }

            public CancellationToken cancellationToken
            {
                get { return _cancellation.Token; }
            }

            public bool IsCancellationRequested
            {
                get { return _cancellation.IsCancellationRequested; }
            }

            public ExecuteCodeFrameAwaitable wait
            {
                get { return new ExecuteCodeFrameAwaitable(this, 1, 0, null, "editor_update", "next EditorApplication.update", "", 0); }
            }

            public ExecuteCodeFrameAwaitable WaitFrame([CallerLineNumber] int sourceLine = 0)
            {
                return new ExecuteCodeFrameAwaitable(
                    this, 1, 0, null, "editor_update", "next EditorApplication.update", "", sourceLine);
            }

            public ExecuteCodeFrameAwaitable WaitFrames(
                int frames,
                [CallerLineNumber] int sourceLine = 0)
            {
                int normalized = Math.Max(1, frames);
                return new ExecuteCodeFrameAwaitable(
                    this, normalized, 0, null, "editor_updates",
                    normalized + " EditorApplication.update ticks", "", sourceLine);
            }

            public ExecuteCodeFrameAwaitable WaitSeconds(
                float seconds,
                [CallerLineNumber] int sourceLine = 0)
            {
                double normalized = seconds < 0 ? 0 : seconds;
                return new ExecuteCodeFrameAwaitable(
                    this, 1, normalized, null, "editor_time",
                    normalized.ToString("R", System.Globalization.CultureInfo.InvariantCulture)
                        + " seconds and a later EditorApplication.update",
                    "", sourceLine);
            }

            public ExecuteCodeFrameAwaitable WaitUntil(
                Func<bool> predicate,
                string condition = null,
                [CallerLineNumber] int sourceLine = 0)
            {
                if (predicate == null)
                    throw new ArgumentNullException("predicate");

                return new ExecuteCodeFrameAwaitable(
                    this, 0, 0, predicate, "condition", "EditorApplication.update",
                    string.IsNullOrWhiteSpace(condition) ? PredicateDescription(predicate) : condition.Trim(),
                    sourceLine);
            }

            public ConsoleLogResult GetConsoleLog(string level = null, int limit = 50)
            {
                TouchActivity();
                ThrowIfCancellationRequested();
                ConsoleLogResult result = BuildConsoleLogResult(level, limit);
                TouchActivity();
                return result;
            }

            public ConsoleLogResult GetConsoleLog(string[] levels, int limit = 50)
            {
                TouchActivity();
                ThrowIfCancellationRequested();
                ConsoleLogResult result = BuildConsoleLogResult(null, levels, limit);
                TouchActivity();
                return result;
            }

            public string PropertyTree(
                UnityEngine.Object target,
                int depth = 2,
                int maxArrayItems = 4)
            {
                TouchActivity();
                ThrowIfCancellationRequested();
                string result = LocusPropertyTree.Format(target, depth, maxArrayItems);
                TouchActivity();
                return result;
            }

            public bool Progress(string title, string info, float progress)
            {
                TouchActivity();
                ThrowIfCancellationRequested();

                string normalizedTitle = string.IsNullOrEmpty(title) ? "Locus" : title;
                string normalizedInfo = info ?? "";
                float normalizedProgress = Mathf.Clamp01(progress);

                if (_requestState != null)
                    _requestState.SetApiProgress(normalizedTitle, normalizedInfo, normalizedProgress);

                TouchActivity();
                return _cancellation.IsCancellationRequested;
            }

            public bool Progress(string info, float progress)
            {
                return Progress("Locus", info, progress);
            }

            public bool Progress(float progress)
            {
                return Progress("Locus", "", progress);
            }

            public void ClearProgress()
            {
                if (_requestState != null)
                    _requestState.ResetProgress();
                try
                {
                    EditorUtility.ClearProgressBar();
                }
                catch
                {
                }
            }

            public void ThrowIfCancellationRequested()
            {
                _cancellation.Token.ThrowIfCancellationRequested();

                if (_waitException != null)
                {
                    Exception ex = _waitException;
                    _waitException = null;
                    throw ex;
                }
            }

            internal bool ShouldResumeImmediately
            {
                get { return _cancellation.IsCancellationRequested || _waitException != null; }
            }

            private void TouchActivity()
            {
                try
                {
                    if (_touchActivity != null)
                        _touchActivity();
                }
                catch
                {
                }
            }

            internal bool IsWaitReady(int targetTick, double targetTime, Func<bool> predicate)
            {
                if (_cancellation.IsCancellationRequested)
                    return true;

                if (_waitException != null)
                    return true;

                if (targetTick >= 0 && _executeAsyncEditorUpdateTick < targetTick)
                    return false;

                if (targetTime > 0 && EditorApplication.timeSinceStartup < targetTime)
                    return false;

                if (predicate == null)
                    return true;

                try
                {
                    return predicate();
                }
                catch (Exception ex)
                {
                    _waitException = ex;
                    return true;
                }
            }

            internal void ScheduleWait(
                Action continuation,
                int frames,
                double seconds,
                Func<bool> predicate,
                string waitKind,
                string waitTarget,
                string waitCondition,
                int sourceLine)
            {
                if (continuation == null)
                    return;

                int targetTick = frames <= 0
                    ? -1
                    : _executeAsyncEditorUpdateTick + frames;
                double targetTime = seconds <= 0
                    ? 0
                    : EditorApplication.timeSinceStartup + seconds;

                SetAwaiting(waitKind, waitTarget, waitCondition, sourceLine);

                ScheduleExecuteContinuation(new ExecuteCodeWaitState(
                    this,
                    continuation,
                    targetTick,
                    targetTime,
                    predicate));
            }

            internal void SetAwaiting(
                string kind,
                string target,
                string condition,
                int sourceLine)
            {
                if (_requestState != null)
                    _requestState.SetAwaiting(kind, target, condition, sourceLine);
                TouchActivity();
            }

            internal void ClearAwaiting()
            {
                if (_requestState != null)
                    _requestState.ClearAwaiting();
                TouchActivity();
            }

            internal static string PredicateDescription(Func<bool> predicate)
            {
                if (predicate == null || predicate.Method == null)
                    return "predicate";
                Type declaring = predicate.Method.DeclaringType;
                return (declaring != null ? declaring.FullName + "." : "") + predicate.Method.Name;
            }
        }

        public struct ExecuteCodeFrameAwaitable
        {
            private readonly ExecuteCodeContext _context;
            private readonly int _frames;
            private readonly double _seconds;
            private readonly Func<bool> _predicate;
            private readonly string _waitKind;
            private readonly string _waitTarget;
            private readonly string _waitCondition;
            private readonly int _sourceLine;

            internal ExecuteCodeFrameAwaitable(
                ExecuteCodeContext context,
                int frames,
                double seconds,
                Func<bool> predicate,
                string waitKind,
                string waitTarget,
                string waitCondition,
                int sourceLine)
            {
                _context = context;
                _frames = frames;
                _seconds = seconds;
                _predicate = predicate;
                _waitKind = waitKind;
                _waitTarget = waitTarget;
                _waitCondition = waitCondition;
                _sourceLine = sourceLine;
            }

            public Awaiter GetAwaiter()
            {
                return new Awaiter(
                    _context, _frames, _seconds, _predicate,
                    _waitKind, _waitTarget, _waitCondition, _sourceLine);
            }

            public struct Awaiter : ICriticalNotifyCompletion
            {
                private readonly ExecuteCodeContext _context;
                private readonly int _frames;
                private readonly double _seconds;
                private readonly Func<bool> _predicate;
                private readonly string _waitKind;
                private readonly string _waitTarget;
                private readonly string _waitCondition;
                private readonly int _sourceLine;

                internal Awaiter(
                    ExecuteCodeContext context,
                    int frames,
                    double seconds,
                    Func<bool> predicate,
                    string waitKind,
                    string waitTarget,
                    string waitCondition,
                    int sourceLine)
                {
                    _context = context;
                    _frames = frames;
                    _seconds = seconds;
                    _predicate = predicate;
                    _waitKind = waitKind;
                    _waitTarget = waitTarget;
                    _waitCondition = waitCondition;
                    _sourceLine = sourceLine;
                }

                public bool IsCompleted
                {
                    get
                    {
                        if (_context == null)
                            return true;

                        if (_frames > 0 || _seconds > 0)
                            return false;

                        return _context.IsWaitReady(-1, 0, _predicate);
                    }
                }

                public void GetResult()
                {
                    if (_context != null)
                        _context.ThrowIfCancellationRequested();
                }

                public void OnCompleted(Action continuation)
                {
                    if (_context == null)
                    {
                        continuation();
                        return;
                    }

                    _context.ScheduleWait(
                        continuation, _frames, _seconds, _predicate,
                        _waitKind, _waitTarget, _waitCondition, _sourceLine);
                }

                public void UnsafeOnCompleted(Action continuation)
                {
                    OnCompleted(continuation);
                }
            }
        }

        private sealed class ExecuteCodeWaitState
        {
            private readonly ExecuteCodeContext _context;
            private readonly int _targetTick;
            private readonly double _targetTime;
            private readonly Func<bool> _predicate;

            public readonly Action Continuation;

            public ExecuteCodeWaitState(
                ExecuteCodeContext context,
                Action continuation,
                int targetTick,
                double targetTime,
                Func<bool> predicate)
            {
                _context = context;
                Continuation = continuation;
                _targetTick = targetTick;
                _targetTime = targetTime;
                _predicate = predicate;
            }

            public bool IsReady(int currentTick, double currentTime)
            {
                if (_context == null)
                    return true;

                if (_context.ShouldResumeImmediately)
                    return true;

                if (_targetTick >= 0 && currentTick < _targetTick)
                    return false;

                if (_targetTime > 0 && currentTime < _targetTime)
                    return false;

                return _context.IsWaitReady(-1, 0, _predicate);
            }

            public void InvokeContinuation()
            {
                if (_context != null)
                    _context.ClearAwaiting();
                Continuation();
            }
        }
    }
}
