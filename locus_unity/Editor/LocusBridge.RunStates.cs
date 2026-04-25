using UnityEngine;
using UnityEditor;

using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Reflection;
using System.Collections.Generic;
using System.Diagnostics;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;
using Assembly = System.Reflection.Assembly;

namespace Locus
{
    public static partial class LocusBridge
    {
        private const int RunStatesMaxFrames = 3600;
        private const int RunStatesPrintTokenByteRatio = 4;
        private const int RunStatesPrintHardLimitTokens = 1000000;
        private const long RunStatesPrintHardLimitBytes =
            (long)RunStatesPrintHardLimitTokens * RunStatesPrintTokenByteRatio;

        private static RuntimeStateMachineSession _activeRunStatesSession;

        [Serializable]
        private sealed class RunStatesRequest
        {
            public string request_editor_status;
            public string initial_state;
            public RunStatesStateRequest[] states;
        }

        [Serializable]
        private sealed class RunStatesStateRequest
        {
            public string name;
            public string variables;
            public string start;
            public string update;
            public string end;
        }

        private sealed class CompiledRunStates
        {
            public readonly Func<RuntimeStateMachineDefinition> Builder;

            public CompiledRunStates(Func<RuntimeStateMachineDefinition> builder)
            {
                Builder = builder;
            }
        }

        internal sealed class RunStatesCompletion
        {
            public readonly bool Ok;
            public readonly string Message;

            public RunStatesCompletion(bool ok, string message)
            {
                Ok = ok;
                Message = message ?? "";
            }
        }

        private enum RunStatesControlKind
        {
            Sleep,
            Goto,
            Done,
            Fail
        }

        private sealed class RunStatesControlException : Exception
        {
            public readonly RunStatesControlKind Kind;
            public readonly string Target;
            public readonly string MessageText;
            public readonly int SleepFrames;

            public RunStatesControlException(RunStatesControlKind kind, string target, string message, int sleepFrames)
                : base(kind.ToString())
            {
                Kind = kind;
                Target = target;
                MessageText = message;
                SleepFrames = sleepFrames;
            }
        }

        public sealed class RuntimeStateMachineDefinition
        {
            private readonly Dictionary<string, RuntimeStateDefinition> _states =
                new Dictionary<string, RuntimeStateDefinition>(StringComparer.Ordinal);

            public void AddState(string name, Action<RuntimeCtx> start, Action<RuntimeCtx> update, Action<RuntimeCtx> end)
            {
                string normalizedName = (name ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedName))
                    throw new ArgumentException("State name is required.");
                if (update == null)
                    throw new ArgumentException("State '" + normalizedName + "' requires an update handler.");
                if (_states.ContainsKey(normalizedName))
                    throw new ArgumentException("Duplicate state name: " + normalizedName);

                _states.Add(normalizedName, new RuntimeStateDefinition(normalizedName, start, update, end));
            }

            internal bool ContainsState(string name)
            {
                return !string.IsNullOrEmpty(name) && _states.ContainsKey(name);
            }

            internal RuntimeStateDefinition GetState(string name)
            {
                RuntimeStateDefinition state;
                if (!_states.TryGetValue(name, out state))
                    throw new InvalidOperationException("Unknown state: " + name);
                return state;
            }
        }

        internal sealed class RuntimeStateDefinition
        {
            public readonly string Name;
            public readonly Action<RuntimeCtx> Start;
            public readonly Action<RuntimeCtx> Update;
            public readonly Action<RuntimeCtx> End;

            public RuntimeStateDefinition(string name, Action<RuntimeCtx> start, Action<RuntimeCtx> update, Action<RuntimeCtx> end)
            {
                Name = name;
                Start = start;
                Update = update;
                End = end;
            }
        }

        public sealed class RuntimeVar<T>
        {
            private readonly RuntimeStateMachineSession _session;
            private readonly string _key;

            internal RuntimeVar(RuntimeStateMachineSession session, string key)
            {
                _session = session;
                _key = key;
            }

            public string Key { get { return _key; } }

            public T Value
            {
                get { return _session.GetMemory<T>(_key); }
                set { _session.SetMemory(_key, value); }
            }

            public override string ToString()
            {
                object value = Value;
                return value != null ? value.ToString() : "null";
            }
        }

        public sealed class RuntimeCtx
        {
            private readonly RuntimeStateMachineSession _session;

            internal RuntimeCtx(RuntimeStateMachineSession session)
            {
                _session = session;
            }

            public string StateName { get { return _session.CurrentStateName; } }
            public int TotalFrames { get { return _session.TotalFrames; } }
            public int ElapsedFramesInState { get { return _session.ElapsedFramesInState; } }
            public float TotalSeconds { get { return (float)_session.TotalSeconds; } }
            public float ElapsedSecondsInState { get { return (float)_session.ElapsedSecondsInState; } }

            public void Print(object value)
            {
                _session.Print(value);
            }

            public void Sleep(int frames)
            {
                int normalized = Math.Max(0, frames);
                if (normalized <= 0)
                    return;
                throw new RunStatesControlException(RunStatesControlKind.Sleep, null, null, normalized);
            }

            public void Goto(string stateName)
            {
                throw new RunStatesControlException(RunStatesControlKind.Goto, stateName, null, 0);
            }

            public void Done(string message)
            {
                throw new RunStatesControlException(RunStatesControlKind.Done, null, message, 0);
            }

            public void Done()
            {
                Done(null);
            }

            public void Fail(string message)
            {
                throw new RunStatesControlException(RunStatesControlKind.Fail, null, message, 0);
            }

            public void PromptUser(string token, string message)
            {
                _session.PromptUser(token, message);
            }

            public void ClearPrompt(string token)
            {
                _session.ClearPrompt(token);
            }

            public void Set(string key, object value)
            {
                _session.SetMemory(key, value);
            }

            public void SetGlobal(string key, object value)
            {
                _session.SetMemory(key, value);
            }

            public bool Has(string key)
            {
                return _session.HasMemory(key);
            }

            public bool HasGlobal(string key)
            {
                return _session.HasMemory(key);
            }

            public T Get<T>(string key)
            {
                return _session.GetMemory<T>(key);
            }

            public T GetGlobal<T>(string key)
            {
                return _session.GetMemory<T>(key);
            }

            public RuntimeVar<T> Global<T>(string key)
            {
                return _session.GetGlobal<T>(key, default(T));
            }

            public RuntimeVar<T> Global<T>(string key, T initialValue)
            {
                return _session.GetGlobal<T>(key, initialValue);
            }

            public bool Remove(string key)
            {
                return _session.RemoveMemory(key);
            }

            public bool RemoveGlobal(string key)
            {
                return _session.RemoveMemory(key);
            }
        }

        internal sealed class RuntimeStateMachineSession
        {
            private readonly RuntimeStateMachineDefinition _definition;
            private readonly TaskCompletionSource<RunStatesCompletion> _completion;
            private readonly List<string> _prints = new List<string>(64);
            private readonly Dictionary<string, object> _memory = new Dictionary<string, object>(StringComparer.Ordinal);
            private readonly Stopwatch _stopwatch = Stopwatch.StartNew();

            private string _currentStateName;
            private bool _needsStart = true;
            private bool _completed;
            private bool _runningEnd;
            private int _sleepFrames;
            private int _stateStartFrame;
            private double _stateStartSeconds;
            private int _printLineCount;
            private long _printBytes;
            private bool _printHardLimitReached;

            internal RuntimeStateMachineSession(
                RuntimeStateMachineDefinition definition,
                string initialState,
                TaskCompletionSource<RunStatesCompletion> completion)
            {
                _definition = definition;
                _currentStateName = initialState;
                _completion = completion;
                _stateStartFrame = 0;
                _stateStartSeconds = 0;
            }

            public string CurrentStateName { get { return _currentStateName; } }
            public bool IsCompleted { get { return _completed; } }
            public int TotalFrames { get; private set; }
            public int ElapsedFramesInState { get { return Math.Max(0, TotalFrames - _stateStartFrame); } }
            public double TotalSeconds { get { return _stopwatch.Elapsed.TotalSeconds; } }
            public double ElapsedSecondsInState { get { return Math.Max(0, TotalSeconds - _stateStartSeconds); } }

            public void Tick()
            {
                if (_completed)
                    return;

                TotalFrames++;

                if (TotalFrames > RunStatesMaxFrames)
                {
                    Complete(false, BuildResult("error", "max frame limit reached: " + RunStatesMaxFrames));
                    return;
                }

                if (_sleepFrames > 0)
                {
                    _sleepFrames--;
                    return;
                }

                RuntimeStateDefinition state;
                try
                {
                    state = _definition.GetState(_currentStateName);
                }
                catch (Exception ex)
                {
                    Complete(false, BuildResult("error", ex.Message));
                    return;
                }

                if (_needsStart)
                {
                    _needsStart = false;
                    if (!RunHandler(state.Start, "start"))
                        return;
                }

                RunHandler(state.Update, "update");
            }

            public void Print(object value)
            {
                string text = value != null ? (value.ToString() ?? "null") : "null";
                long nextBytes = _printBytes + EstimatePrintBytes(text);
                _printLineCount += CountPrintLines(text);

                if (nextBytes > RunStatesPrintHardLimitBytes)
                {
                    _printBytes = nextBytes;
                    _printHardLimitReached = true;
                    Complete(false, BuildResult(
                        "error",
                        "too large: print output exceeded hard limit of "
                            + RunStatesPrintHardLimitTokens
                            + " estimated tokens; result was not saved."));
                    throw new RunStatesControlException(RunStatesControlKind.Fail, null, "too large", 0);
                }

                _printBytes = nextBytes;
                _prints.Add(text);
            }

            public void PromptUser(string token, string message)
            {
                string normalizedToken = (token ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedToken))
                    return;

                string normalizedMessage = (message ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedMessage))
                    return;

                UpdateRunStatesPrompt(normalizedToken, normalizedMessage, _currentStateName, TotalFrames);
            }

            public void ClearPrompt(string token)
            {
                string normalizedToken = (token ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedToken))
                    return;

                ClearRunStatesPrompt(normalizedToken);
            }

            public void SetMemory(string key, object value)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                _memory[normalizedKey] = value;
            }

            public bool HasMemory(string key)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                return _memory.ContainsKey(normalizedKey);
            }

            public T GetMemory<T>(string key)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                object value;
                if (!_memory.TryGetValue(normalizedKey, out value))
                    throw new KeyNotFoundException("Runtime memory key not found: " + normalizedKey);

                if (value == null)
                    return default(T);

                if (!(value is T))
                    throw new InvalidCastException("Runtime memory key '" + normalizedKey + "' contains " + value.GetType().FullName);

                return (T)value;
            }

            public RuntimeVar<T> GetGlobal<T>(string key, T initialValue)
            {
                string normalizedKey = EnsureMemory(key, initialValue);
                return new RuntimeVar<T>(this, normalizedKey);
            }

            public bool RemoveMemory(string key)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                return _memory.Remove(normalizedKey);
            }

            private string EnsureMemory<T>(string key, T initialValue)
            {
                string normalizedKey = NormalizeMemoryKey(key);
                object value;
                if (_memory.TryGetValue(normalizedKey, out value))
                {
                    if (value != null && !(value is T))
                        throw new InvalidCastException("Runtime memory key '" + normalizedKey + "' contains " + value.GetType().FullName);
                    return normalizedKey;
                }

                _memory[normalizedKey] = initialValue;
                return normalizedKey;
            }

            private string NormalizeMemoryKey(string key)
            {
                string normalizedKey = (key ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedKey))
                    throw new ArgumentException("Runtime memory key is required.");
                return normalizedKey;
            }

            private bool RunHandler(Action<RuntimeCtx> handler, string phase)
            {
                if (handler == null || _completed)
                    return !_completed;

                try
                {
                    handler(new RuntimeCtx(this));
                    return !_completed;
                }
                catch (RunStatesControlException control)
                {
                    HandleControl(control, phase);
                    return false;
                }
                catch (Exception ex)
                {
                    CompleteWithEnd(false, "runtime error in state '" + _currentStateName + "' " + phase + ": " + ex);
                    return false;
                }
            }

            private void HandleControl(RunStatesControlException control, string phase)
            {
                if (_completed)
                    return;

                if (_runningEnd && control.Kind != RunStatesControlKind.Fail)
                {
                    Complete(false, BuildResult("error", "State end handler cannot call " + control.Kind + "."));
                    return;
                }

                switch (control.Kind)
                {
                    case RunStatesControlKind.Sleep:
                        if (string.Equals(phase, "end", StringComparison.Ordinal))
                        {
                            Complete(false, BuildResult("error", "State end handler cannot call Sleep."));
                            return;
                        }
                        _sleepFrames = Math.Max(0, control.SleepFrames);
                        break;

                    case RunStatesControlKind.Goto:
                        TransitionTo(control.Target);
                        break;

                    case RunStatesControlKind.Done:
                        CompleteWithEnd(true, control.MessageText);
                        break;

                    case RunStatesControlKind.Fail:
                        CompleteWithEnd(false, string.IsNullOrEmpty(control.MessageText) ? "state machine failed" : control.MessageText);
                        break;
                }
            }

            private void TransitionTo(string targetState)
            {
                string normalizedTarget = (targetState ?? "").Trim();
                if (string.IsNullOrEmpty(normalizedTarget))
                {
                    CompleteWithEnd(false, "Goto requires a state name.");
                    return;
                }

                if (!_definition.ContainsState(normalizedTarget))
                {
                    CompleteWithEnd(false, "Unknown target state: " + normalizedTarget);
                    return;
                }

                string previousState = _currentStateName;
                RunEnd(previousState);
                if (_completed)
                    return;

                ClearPromptsForState(previousState);
                _currentStateName = normalizedTarget;
                _needsStart = true;
                _sleepFrames = 0;
                _stateStartFrame = TotalFrames;
                _stateStartSeconds = TotalSeconds;
            }

            private void CompleteWithEnd(bool ok, string message)
            {
                string stateName = _currentStateName;
                RunEnd(stateName);
                if (_completed)
                    return;

                ClearPromptsForState(stateName);
                string status = ok ? "ok" : "error";
                Complete(ok, BuildResult(status, message));
            }

            private void RunEnd(string stateName)
            {
                if (_runningEnd || _completed)
                    return;

                RuntimeStateDefinition state;
                try
                {
                    state = _definition.GetState(stateName);
                }
                catch (Exception ex)
                {
                    Complete(false, BuildResult("error", ex.Message));
                    return;
                }

                if (state.End == null)
                    return;

                _runningEnd = true;
                try
                {
                    state.End(new RuntimeCtx(this));
                }
                catch (RunStatesControlException control)
                {
                    if (control.Kind == RunStatesControlKind.Fail)
                        Complete(false, BuildResult("error", string.IsNullOrEmpty(control.MessageText) ? "state end failed" : control.MessageText));
                    else
                        Complete(false, BuildResult("error", "State end handler cannot call " + control.Kind + "."));
                }
                catch (Exception ex)
                {
                    Complete(false, BuildResult("error", "runtime error in state '" + stateName + "' end: " + ex));
                }
                finally
                {
                    _runningEnd = false;
                }
            }

            private string BuildResult(string status, string message)
            {
                var sb = new StringBuilder(1024);
                sb.Append("status: ").AppendLine(status);
                sb.Append("final_state: ").AppendLine(_currentStateName ?? "");
                sb.Append("frames: ").AppendLine(TotalFrames.ToString());
                sb.Append("duration_ms: ").AppendLine(_stopwatch.ElapsedMilliseconds.ToString());
                if (!string.IsNullOrEmpty(message))
                    sb.Append("message: ").AppendLine(message);
                sb.Append("print_lines: ").AppendLine(_printLineCount.ToString());
                sb.Append("print_tokens_estimate: ").AppendLine(EstimatePrintTokens(_printBytes).ToString());
                if (_printHardLimitReached)
                {
                    sb.AppendLine("print_output: too large");
                    return sb.ToString();
                }
                sb.AppendLine("prints:");
                for (int i = 0; i < _prints.Count; i++)
                    sb.AppendLine(_prints[i]);
                return sb.ToString();
            }

            private static long EstimatePrintBytes(string text)
            {
                long bytes = Utf8NoBom.GetByteCount(text ?? "");
                return bytes + 1;
            }

            private static int CountPrintLines(string text)
            {
                if (string.IsNullOrEmpty(text))
                    return 1;

                int lines = 1;
                for (int i = 0; i < text.Length; i++)
                {
                    if (text[i] == '\n')
                        lines++;
                }
                return lines;
            }

            private static long EstimatePrintTokens(long byteCount)
            {
                if (byteCount <= 0)
                    return 0;
                return (byteCount + RunStatesPrintTokenByteRatio - 1) / RunStatesPrintTokenByteRatio;
            }

            private void Complete(bool ok, string message)
            {
                if (_completed)
                    return;

                _completed = true;
                ClearAllRunStatesPrompts();
                _completion.TrySetResult(new RunStatesCompletion(ok, message));
            }
        }

        private static async Task<PipeEnvelope> HandleSetEditorStatus(string requestId, string desiredStatus)
        {
            string normalized = (desiredStatus ?? "").Trim();
            if (string.IsNullOrEmpty(normalized))
                return ErrorResponse(requestId, "empty requested editor status");

            var tcs = new TaskCompletionSource<PipeEnvelope>();
            PostToMainThread(delegate
            {
                try
                {
                    switch (normalized)
                    {
                        case "editing":
                            EditorApplication.isPaused = false;
                            EditorApplication.isPlaying = false;
                            tcs.TrySetResult(OkResponse(requestId, "editing_requested"));
                            break;

                        case "playing":
                            EditorApplication.isPaused = false;
                            EditorApplication.isPlaying = true;
                            tcs.TrySetResult(OkResponse(requestId, "playing_requested"));
                            break;

                        case "playing_paused":
                            EditorApplication.isPaused = true;
                            EditorApplication.isPlaying = true;
                            tcs.TrySetResult(OkResponse(requestId, "playing_paused_requested"));
                            break;

                        default:
                            tcs.TrySetResult(ErrorResponse(requestId, "unsupported editor status: " + normalized));
                            break;
                    }
                }
                catch (Exception ex)
                {
                    tcs.TrySetResult(ErrorResponse(requestId, ex.ToString()));
                }
            });
            return await tcs.Task;
        }

        private static async Task<PipeEnvelope> HandleRunStates(string requestId, string requestJson)
        {
            if (string.IsNullOrWhiteSpace(requestJson))
                return ErrorResponse(requestId, "empty run_states request");

            await _runStatesLock.WaitAsync();
            try
            {
                string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
                if (!string.IsNullOrEmpty(prepareError))
                    return ErrorResponse(requestId, prepareError);

                RunStatesRequest request;
                try
                {
                    request = JsonUtility.FromJson<RunStatesRequest>(requestJson);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "run_states request parse failed: " + ex.Message);
                }

                string validationError = ValidateRunStatesRequest(request);
                if (!string.IsNullOrEmpty(validationError))
                    return ErrorResponse(requestId, validationError);

                string initialState = request.initial_state.Trim();

                CompiledRunStates compiled;
                try
                {
                    compiled = CompileRunStates(request);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "run_states compilation exception: " + ex.Message);
                }

                var completion = new TaskCompletionSource<RunStatesCompletion>();
                PostToMainThread(delegate
                {
                    try
                    {
                        if (_activeRunStatesSession != null)
                        {
                            completion.TrySetResult(new RunStatesCompletion(false, "A unity_run_states session is already running."));
                            return;
                        }

                        RuntimeStateMachineDefinition definition = compiled.Builder();
                        if (!definition.ContainsState(initialState))
                        {
                            completion.TrySetResult(new RunStatesCompletion(false, "Initial state not found: " + initialState));
                            return;
                        }

                        _activeRunStatesSession = new RuntimeStateMachineSession(definition, initialState, completion);
                    }
                    catch (Exception ex)
                    {
                        completion.TrySetResult(new RunStatesCompletion(false, "run_states bootstrap failed: " + ex));
                    }
                });

                RunStatesCompletion result = await completion.Task;
                if (result.Ok)
                    return OkResponse(requestId, result.Message);
                return ErrorResponse(requestId, result.Message);
            }
            finally
            {
                _runStatesLock.Release();
            }
        }

        private static string ValidateRunStatesRequest(RunStatesRequest request)
        {
            if (request == null)
                return "run_states request is empty";
            if (string.IsNullOrWhiteSpace(request.request_editor_status))
                return "request_editor_status is required";
            if (string.IsNullOrWhiteSpace(request.initial_state))
                return "initial_state is required";
            if (request.states == null || request.states.Length == 0)
                return "states must contain at least one state";

            var names = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < request.states.Length; i++)
            {
                RunStatesStateRequest state = request.states[i];
                if (state == null)
                    return "states[" + i + "] is empty";

                string name = (state.name ?? "").Trim();
                if (string.IsNullOrEmpty(name))
                    return "states[" + i + "].name is required";
                if (!names.Add(name))
                    return "duplicate state name: " + name;
                if (string.IsNullOrWhiteSpace(state.update))
                    return "state '" + name + "' requires update code";
            }

            if (!names.Contains(request.initial_state.Trim()))
                return "initial_state not found in states: " + request.initial_state;

            return null;
        }

        private static CompiledRunStates CompileRunStates(RunStatesRequest request)
        {
            string source = BuildRunStatesSource(request);

            SyntaxTree syntaxTree;
            try
            {
                syntaxTree = CSharpSyntaxTree.ParseText(
                    source,
                    SnippetParseOptions,
                    path: "LocusRunStates.cs",
                    encoding: Utf8NoBom
                );
            }
            catch (Exception ex)
            {
                throw new Exception("parse failed: " + ex);
            }

            string assemblyName =
                "__LocusRunStates_" + Interlocked.Increment(ref _snippetAssemblyCounter).ToString("X8");

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
                    throw new Exception("emit failed: " + ex);
                }

                if (!emitResult.Success)
                    throw new Exception(BuildDiagnosticErrorText(emitResult.Diagnostics));

                try
                {
                    byte[] assemblyBytes = peStream.ToArray();
                    Assembly assembly = Assembly.Load(assemblyBytes);

                    Type hostType = assembly.GetType("Locus.RuntimeStateMachines.__LocusRunStatesHost", true);
                    MethodInfo buildMethod = hostType.GetMethod(
                        "Build",
                        BindingFlags.Public | BindingFlags.Static
                    );

                    if (buildMethod == null)
                        throw new Exception("compiled state machine missing Build method");

                    Func<RuntimeStateMachineDefinition> builder =
                        (Func<RuntimeStateMachineDefinition>)Delegate.CreateDelegate(
                            typeof(Func<RuntimeStateMachineDefinition>),
                            buildMethod
                        );

                    return new CompiledRunStates(builder);
                }
                catch (Exception ex)
                {
                    throw new Exception("assembly load/bootstrap failed: " + ex);
                }
            }
        }

        private static string BuildRunStatesSource(RunStatesRequest request)
        {
            var sb = new StringBuilder(8192);
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
            sb.AppendLine();
            sb.AppendLine("namespace Locus.RuntimeStateMachines");
            sb.AppendLine("{");
            sb.AppendLine("    public static class __LocusRunStatesHost");
            sb.AppendLine("    {");
            sb.AppendLine("        public static global::Locus.LocusBridge.RuntimeStateMachineDefinition Build()");
            sb.AppendLine("        {");
            sb.AppendLine("            var machine = new global::Locus.LocusBridge.RuntimeStateMachineDefinition();");

            for (int i = 0; i < request.states.Length; i++)
            {
                RunStatesStateRequest state = request.states[i];
                string name = (state.name ?? "").Trim();
                sb.AppendLine("            {");
                AppendRunStatesVariables(sb, name, state.variables, "                ");
                sb.Append("                machine.AddState(").Append(ToCSharpStringLiteral(name)).AppendLine(",");
                AppendRunStatesHandler(sb, name, "start", state.start, "                    ");
                sb.AppendLine(",");
                AppendRunStatesHandler(sb, name, "update", state.update, "                    ");
                sb.AppendLine(",");
                AppendRunStatesHandler(sb, name, "end", state.end, "                    ");
                sb.AppendLine("                );");
                sb.AppendLine("            }");
            }

            sb.AppendLine("            return machine;");
            sb.AppendLine("        }");
            sb.AppendLine("    }");
            sb.AppendLine("}");
            return sb.ToString();
        }

        private static void AppendRunStatesVariables(StringBuilder sb, string stateName, string code, string indent)
        {
            if (string.IsNullOrWhiteSpace(code))
                return;

            sb.Append(indent).Append("    #line 1 ").AppendLine(ToCSharpStringLiteral("unity_run_states:" + stateName + ":variables"));
            sb.AppendLine(code);
            sb.Append(indent).AppendLine("    #line default");
        }

        private static void AppendRunStatesHandler(StringBuilder sb, string stateName, string phase, string code, string indent)
        {
            if (string.IsNullOrWhiteSpace(code))
            {
                sb.Append(indent).Append("null");
                return;
            }

            sb.Append(indent).AppendLine("new global::System.Action<global::Locus.LocusBridge.RuntimeCtx>(ctx =>");
            sb.Append(indent).AppendLine("{");
            sb.Append(indent).Append("    #line 1 ").AppendLine(ToCSharpStringLiteral("unity_run_states:" + stateName + ":" + phase));
            sb.AppendLine(code);
            sb.Append(indent).AppendLine("    #line default");
            sb.Append(indent).Append("})");
        }

        private static string ToCSharpStringLiteral(string value)
        {
            if (value == null)
                return "null";

            var sb = new StringBuilder(value.Length + 2);
            sb.Append('"');
            for (int i = 0; i < value.Length; i++)
            {
                char ch = value[i];
                switch (ch)
                {
                    case '\\': sb.Append("\\\\"); break;
                    case '"': sb.Append("\\\""); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\t': sb.Append("\\t"); break;
                    default: sb.Append(ch); break;
                }
            }
            sb.Append('"');
            return sb.ToString();
        }

        private static void PumpRunStates()
        {
            RuntimeStateMachineSession session = _activeRunStatesSession;
            if (session == null)
                return;

            session.Tick();

            if (session == _activeRunStatesSession && session.IsCompleted)
                _activeRunStatesSession = null;
        }

        private static void UpdateRunStatesPrompt(string token, string message, string stateName, int frame)
        {
        }

        private static void ClearRunStatesPrompt(string token)
        {
        }

        private static void ClearPromptsForState(string stateName)
        {
        }

        private static void ClearAllRunStatesPrompts()
        {
        }
    }
}
