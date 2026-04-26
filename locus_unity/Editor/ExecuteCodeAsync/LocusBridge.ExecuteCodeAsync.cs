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
                    revision = _executeCodeProgressRevision
                };
            }
        }

        private static void SetExecuteCodeProgress(string title, string info, float progress)
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
                    revision = _executeCodeProgressRevision
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

        private static async Task<PipeEnvelope> HandleExecuteCode(string requestId, string code)
        {
            if (string.IsNullOrWhiteSpace(code))
                return ErrorResponse(requestId, "empty code");

            await _executeCodeLock.WaitAsync();
            try
            {
                ResetExecuteCodeProgress();

                string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
                if (!string.IsNullOrEmpty(prepareError))
                    return ErrorResponse(requestId, prepareError);

                CompiledAsyncSnippet snippet;
                try
                {
                    snippet = CompileAsyncSnippet(code);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "async snippet compilation exception: " + ex.Message);
                }

                string resultText = await ExecuteAsyncSnippetOnMainThreadAsync(snippet);

                if (resultText.StartsWith("__ERROR__: ", StringComparison.Ordinal))
                    return ErrorResponse(requestId, resultText.Substring("__ERROR__: ".Length));

                return OkResponse(requestId, resultText);
            }
            finally
            {
                ResetExecuteCodeProgress();
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

        private static Task<string> ExecuteAsyncSnippetOnMainThreadAsync(CompiledAsyncSnippet snippet)
        {
            var execution = new AsyncSnippetExecution();

            PostToMainThread(delegate
            {
                RunAsyncSnippetOnMainThread(snippet, execution);
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
            AsyncSnippetExecution execution)
        {
            BeginAsyncExecuteRuntime();

            ExecuteCodeContext ctx = null;

            try
            {
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
