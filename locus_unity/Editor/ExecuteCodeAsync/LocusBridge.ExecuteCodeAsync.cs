using UnityEngine;
using UnityEditor;

using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Reflection;
using System.Collections.Generic;
using System.Runtime.CompilerServices;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
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
        private static readonly object _executeCodeProgressLock = new object();

        private static int _executeAsyncEditorUpdateTick;
        private static int _activeAsyncExecuteCount;
        private static bool _hasSavedRunInBackground;
        private static bool _savedRunInBackground;
        private static double _lastAsyncExecutePumpRequestSeconds;
        private static ExecuteCodeProgressSnapshot _executeCodeProgress =
            new ExecuteCodeProgressSnapshot { active = false, title = "", info = "", progress = 0, revision = 0 };
        private static int _executeCodeProgressRevision;

        private sealed class CompiledAsyncSnippet
        {
            public readonly Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>> Executor;

            public CompiledAsyncSnippet(
                Func<ScriptGlobals, ExecuteCodeContext, CancellationToken, Task<object>> executor)
            {
                Executor = executor;
            }
        }

        private sealed class AsyncSnippetExecution : IDisposable
        {
            private long _lastActivityTimestamp;

            public readonly CancellationTokenSource Cancellation = new CancellationTokenSource();
            public readonly TaskCompletionSource<string> Completion = new TaskCompletionSource<string>();

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

        private sealed class ExecuteCodeRequestState : IDisposable
        {
            private readonly object _lock = new object();
            private AsyncSnippetExecution _execution;
            private long _lastClientHeartbeatTimestamp;
            private int _clientHeartbeatCount;
            private volatile bool _disposed;

            public readonly CancellationTokenSource Cancellation = new CancellationTokenSource();

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

            public void Dispose()
            {
                Cancel();
                _disposed = true;
                Cancellation.Dispose();
            }
        }

        private static readonly object _executeCodeRequestStateLock = new object();
        private static ExecuteCodeRequestState _activeExecuteCodeRequest;

        private static ExecuteCodeRequestState ActiveExecuteCodeRequest
        {
            get
            {
                lock (_executeCodeRequestStateLock)
                {
                    return _activeExecuteCodeRequest;
                }
            }
        }

        private static void TouchActiveExecuteCodeClientHeartbeat()
        {
            ExecuteCodeRequestState requestState = ActiveExecuteCodeRequest;
            if (requestState != null)
                requestState.TouchClientHeartbeat();
        }

        private static void CancelActiveExecuteCode(string reason)
        {
            ExecuteCodeRequestState requestState = ActiveExecuteCodeRequest;
            if (requestState == null)
                return;

            requestState.Cancel();
            ResetExecuteCodeProgress();

            if (!string.IsNullOrEmpty(reason))
                Debug.LogWarning("[Locus] execute_code canceled: " + reason);
        }

        private static void ResetExecuteCodeProgress()
        {
            lock (_executeCodeProgressLock)
            {
                _executeCodeProgressRevision++;
                _executeCodeProgress = new ExecuteCodeProgressSnapshot
                {
                    active = false,
                    title = "",
                    info = "",
                    progress = 0,
                    revision = _executeCodeProgressRevision,
                    source = ""
                };
            }
        }

        private static void SetExecuteCodeProgress(string title, string info, float progress)
        {
            SetExecuteCodeProgressSnapshot(title, info, progress, "api");
        }

        private static void SetExecuteCodeStage(string info)
        {
            SetExecuteCodeProgressSnapshot(info, "", 0, "stage");
        }

        private static void SetExecuteCodeProgressSnapshot(string title, string info, float progress, string source)
        {
            lock (_executeCodeProgressLock)
            {
                _executeCodeProgressRevision++;
                _executeCodeProgress = new ExecuteCodeProgressSnapshot
                {
                    active = true,
                    title = string.IsNullOrEmpty(title) ? "Locus" : title,
                    info = info ?? "",
                    progress = Mathf.Clamp01(progress),
                    revision = _executeCodeProgressRevision,
                    source = string.IsNullOrEmpty(source) ? "api" : source
                };
            }
        }

        private static string GetExecuteCodeProgressJson()
        {
            lock (_executeCodeProgressLock)
            {
                return JsonUtility.ToJson(_executeCodeProgress);
            }
        }

        private static PipeEnvelope HandleCancelExecuteCode(string requestId)
        {
            ExecuteCodeRequestState requestState;
            lock (_executeCodeRequestStateLock)
            {
                requestState = _activeExecuteCodeRequest;
            }

            if (requestState == null)
            {
                ResetExecuteCodeProgress();
                return OkResponse(requestId, "no active execute_code");
            }

            requestState.Cancel();
            ResetExecuteCodeProgress();
            return OkResponse(requestId, "execute_code cancellation requested");
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
                    ResetExecuteCodeProgress();
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

            if (ActiveExecuteCodeRequest == null)
                SetExecuteCodeStage("Waiting for Unity execute lock");

            bool lockTaken = false;
            try
            {
                if (!await _executeCodeLock.WaitAsync(ExecuteCodeLockWaitTimeoutMs))
                {
                    if (ActiveExecuteCodeRequest == null)
                        ResetExecuteCodeProgress();
                    return ErrorResponse(
                        requestId,
                        "execute_code lock wait timed out after " +
                        (ExecuteCodeLockWaitTimeoutMs / 1000) +
                        " seconds");
                }

                lockTaken = true;
            }
            catch (ObjectDisposedException ex)
            {
                if (ActiveExecuteCodeRequest == null)
                    ResetExecuteCodeProgress();
                return ErrorResponse(requestId, "execute_code lock unavailable: " + ex.Message);
            }

            ExecuteCodeRequestState requestState = null;
            try
            {
                requestState = new ExecuteCodeRequestState();
                lock (_executeCodeRequestStateLock)
                {
                    _activeExecuteCodeRequest = requestState;
                }
                _ = MonitorExecuteCodeClientHeartbeatAsync(requestState);

                ResetExecuteCodeProgress();
                SetExecuteCodeStage("Checking compiler cache");

                string prepareError = await EnsureExecuteCodeCompilationReadyAsync(
                    SetExecuteCodeStage,
                    requestState.Cancellation.Token);
                if (!string.IsNullOrEmpty(prepareError))
                {
                    requestState.ThrowIfCancellationRequested();
                    SetExecuteCodeStage("Compiler preparation failed");
                    return ErrorResponse(requestId, prepareError);
                }

                requestState.ThrowIfCancellationRequested();

                CompiledAsyncSnippet snippet;
                try
                {
                    SetExecuteCodeStage("Compiling snippet");
                    requestState.ThrowIfCancellationRequested();
                    snippet = CompileAsyncSnippet(code);
                    requestState.ThrowIfCancellationRequested();
                }
                catch (OperationCanceledException)
                {
                    throw;
                }
                catch (Exception ex)
                {
                    SetExecuteCodeStage("Compilation failed");
                    return ErrorResponse(requestId, "async snippet compilation exception: " + ex.Message);
                }

                SetExecuteCodeStage("Executing snippet");
                string resultText = await ExecuteAsyncSnippetOnMainThreadAsync(snippet, requestState);

                if (resultText.StartsWith("__ERROR__: ", StringComparison.Ordinal))
                {
                    requestState.ThrowIfCancellationRequested();
                    SetExecuteCodeStage("Execution failed");
                    return ErrorResponse(requestId, resultText.Substring("__ERROR__: ".Length));
                }

                requestState.ThrowIfCancellationRequested();
                SetExecuteCodeStage("Execution complete");
                return OkResponse(requestId, resultText);
            }
            catch (OperationCanceledException)
            {
                return ErrorResponse(requestId, "execute_code canceled");
            }
            finally
            {
                lock (_executeCodeRequestStateLock)
                {
                    if (ReferenceEquals(_activeExecuteCodeRequest, requestState))
                        _activeExecuteCodeRequest = null;
                }
                if (requestState != null)
                    requestState.Dispose();
                ResetExecuteCodeProgress();
                if (lockTaken)
                    _executeCodeLock.Release();
            }
        }

        private static CompiledAsyncSnippet CompileAsyncSnippet(string code)
        {
            string leadingUsings;
            string bodyCode;
            SplitLeadingUsings(code, out leadingUsings, out bodyCode);

            CompiledAsyncSnippet snippet;
            string primaryError;

            if (TryCompileAsyncSnippet(bodyCode, leadingUsings, false, out snippet, out primaryError))
                return snippet;

            string fallbackError;
            if (TryCompileAsyncSnippet(bodyCode, leadingUsings, true, out snippet, out fallbackError))
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
            out CompiledAsyncSnippet snippet,
            out string error)
        {
            snippet = null;
            error = null;

            const string hostTypeName = "__LocusAsyncSnippetHost";
            const string fullTypeName = "Locus.RuntimeSnippets.__LocusAsyncSnippetHost";

            string source = BuildAsyncSnippetSource(hostTypeName, leadingUsings, bodyCode, expressionMode);

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
                try
                {
                    emitResult = compilation.Emit(peStream);
                }
                catch (Exception ex)
                {
                    error = "emit failed: " + ex;
                    return false;
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

        private static string BuildAsyncSnippetSource(
            string hostTypeName,
            string leadingUsings,
            string bodyCode,
            bool expressionMode)
        {
            var sb = new StringBuilder(4096);

            sb.AppendLine("using System;");
            sb.AppendLine("using System.IO;");
            sb.AppendLine("using System.Text;");
            sb.AppendLine("using System.Linq;");
            sb.AppendLine("using System.Reflection;");
            sb.AppendLine("using System.Threading;");
            sb.AppendLine("using System.Threading.Tasks;");
            sb.AppendLine("using System.Collections;");
            sb.AppendLine("using System.Collections.Generic;");
            sb.AppendLine("using UnityEngine;");
            sb.AppendLine("using UnityEngine.SceneManagement;");
            sb.AppendLine("using UnityEngine.UI;");
            sb.AppendLine("using UnityEditor;");
            sb.AppendLine("using UnityEditor.SceneManagement;");
            sb.AppendLine("using UnityEditor.Animations;");
            sb.AppendLine("using static UnityEngine.Object;");
            sb.AppendLine("using Object = UnityEngine.Object;");

            if (!string.IsNullOrWhiteSpace(leadingUsings))
                sb.AppendLine(leadingUsings);

            sb.AppendLine("namespace Locus.RuntimeSnippets");
            sb.AppendLine("{");
            sb.Append("    public static class ").Append(hostTypeName).AppendLine();
            sb.AppendLine("    {");
            sb.AppendLine("        public static async global::System.Threading.Tasks.Task<object> ExecuteAsync(global::Locus.LocusBridge.ScriptGlobals globals, global::Locus.LocusBridge.ExecuteCodeContext ctx, global::System.Threading.CancellationToken cancellationToken)");
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

            try
            {
                if (requestState != null)
                    requestState.ThrowIfCancellationRequested();

                var globals = new ScriptGlobals(execution.TouchActivity);
                ctx = new ExecuteCodeContext(execution.Cancellation, execution.TouchActivity);

                object returnValue = await snippet.Executor(globals, ctx, execution.Cancellation.Token);

                if (returnValue != null)
                    globals.print(returnValue);

                execution.Completion.TrySetResult(globals.GetOutput());
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

        public sealed class ExecuteCodeContext
        {
            private readonly CancellationTokenSource _cancellation;
            private readonly Action _touchActivity;
            private Exception _waitException;

            internal ExecuteCodeContext(CancellationTokenSource cancellation, Action touchActivity)
            {
                _cancellation = cancellation;
                _touchActivity = touchActivity;
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
                get { return WaitFrame(); }
            }

            public ExecuteCodeFrameAwaitable WaitFrame()
            {
                return new ExecuteCodeFrameAwaitable(this, 1, 0, null);
            }

            public ExecuteCodeFrameAwaitable WaitFrames(int frames)
            {
                return new ExecuteCodeFrameAwaitable(this, Math.Max(1, frames), 0, null);
            }

            public ExecuteCodeFrameAwaitable WaitSeconds(float seconds)
            {
                double normalized = seconds < 0 ? 0 : seconds;
                return new ExecuteCodeFrameAwaitable(this, 1, normalized, null);
            }

            public ExecuteCodeFrameAwaitable WaitUntil(Func<bool> predicate)
            {
                if (predicate == null)
                    throw new ArgumentNullException("predicate");

                return new ExecuteCodeFrameAwaitable(this, 0, 0, predicate);
            }

            public bool Progress(string title, string info, float progress)
            {
                TouchActivity();
                ThrowIfCancellationRequested();

                string normalizedTitle = string.IsNullOrEmpty(title) ? "Locus" : title;
                string normalizedInfo = info ?? "";
                float normalizedProgress = Mathf.Clamp01(progress);

                SetExecuteCodeProgress(normalizedTitle, normalizedInfo, normalizedProgress);

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
                ResetExecuteCodeProgress();
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

            internal void ScheduleWait(Action continuation, int frames, double seconds, Func<bool> predicate)
            {
                if (continuation == null)
                    return;

                int targetTick = frames <= 0
                    ? -1
                    : _executeAsyncEditorUpdateTick + frames;
                double targetTime = seconds <= 0
                    ? 0
                    : EditorApplication.timeSinceStartup + seconds;

                ScheduleExecuteContinuation(new ExecuteCodeWaitState(
                    this,
                    continuation,
                    targetTick,
                    targetTime,
                    predicate));
            }
        }

        public struct ExecuteCodeFrameAwaitable
        {
            private readonly ExecuteCodeContext _context;
            private readonly int _frames;
            private readonly double _seconds;
            private readonly Func<bool> _predicate;

            internal ExecuteCodeFrameAwaitable(
                ExecuteCodeContext context,
                int frames,
                double seconds,
                Func<bool> predicate)
            {
                _context = context;
                _frames = frames;
                _seconds = seconds;
                _predicate = predicate;
            }

            public Awaiter GetAwaiter()
            {
                return new Awaiter(_context, _frames, _seconds, _predicate);
            }

            public struct Awaiter : ICriticalNotifyCompletion
            {
                private readonly ExecuteCodeContext _context;
                private readonly int _frames;
                private readonly double _seconds;
                private readonly Func<bool> _predicate;

                internal Awaiter(
                    ExecuteCodeContext context,
                    int frames,
                    double seconds,
                    Func<bool> predicate)
                {
                    _context = context;
                    _frames = frames;
                    _seconds = seconds;
                    _predicate = predicate;
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

                    _context.ScheduleWait(continuation, _frames, _seconds, _predicate);
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
                Continuation();
            }
        }
    }
}
