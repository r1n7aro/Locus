using UnityEngine;
using UnityEditor;

using System;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        private static readonly object _testRunnerLock = new object();
        private static bool _testRunActive;
        private static TestRunProgressPayload _testRunProgress = new TestRunProgressPayload();
        private static TestRunCallbacksProxy _activeTestCallbacks;

        [Serializable]
        private class TestFindRequest
        {
            public string testMode;
            public string assemblyName;
            public string fixtureName;
            public string testName;
            public string search;
        }

        [Serializable]
        private sealed class TestRunRequest : TestFindRequest
        {
            public string runId;
            public TestTarget[] tests;
        }

        [Serializable]
        private sealed class TestTarget
        {
            public string assemblyName;
            public string fixtureName;
            public string testName;
        }

        [Serializable]
        private sealed class TestDiscoveryResponse
        {
            public TestAssemblyNode[] assemblies = new TestAssemblyNode[0];
        }

        [Serializable]
        private sealed class TestAssemblyNode
        {
            public string name;
            public string testMode;
            public TestFixtureNode[] fixtures = new TestFixtureNode[0];
        }

        [Serializable]
        private sealed class TestFixtureNode
        {
            public string name;
            public TestMethodNode[] tests = new TestMethodNode[0];
        }

        [Serializable]
        private sealed class TestMethodNode
        {
            public string name;
            public string fullName;
            public string[] attributes = new string[0];
            public string sourcePath;
            public int line;
        }

        [Serializable]
        private sealed class TestRunPhaseResponse
        {
            public string runId;
            public string testMode;
            public string status;
            public int total;
            public int passed;
            public int failed;
            public int skipped;
            public double duration;
            public TestResultNode[] results = new TestResultNode[0];
            public string errorCode;
            public string errorMessage;
        }

        [Serializable]
        private sealed class TestResultNode
        {
            public string assemblyName;
            public string fixtureName;
            public string testName;
            public string fullName;
            public string outcome;
            public double duration;
            public string message;
            public string stackTrace;
            public string sourcePath;
            public int line;
        }

        [Serializable]
        private sealed class TestRunProgressPayload
        {
            public bool active;
            public string runId;
            public string phase;
            public string currentTest;
            public int completed;
            public int total;
            public int failed;
            public int revision;
        }

        [Serializable]
        private sealed class TestSourceOpenRequest
        {
            public string path;
            public int line;
        }

        [Serializable]
        private sealed class TestSourceOpenResponse
        {
            public bool opened;
            public bool positioned;
        }

        private static bool HasUnityTestFramework()
        {
            return TestFrameworkApi.IsAvailable();
        }

        private static async Task<PipeEnvelope> HandleFindTests(string requestId, string json)
        {
            TestFrameworkApi api;
            try
            {
                api = await TestFrameworkApi.CreateAsync().ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "test_framework_missing: " + FormatReflectionException(ex));
            }

            var request = JsonUtility.FromJson<TestFindRequest>(json ?? "{}") ?? new TestFindRequest();
            var modes = RequestedModes(request.testMode);
            var assemblies = new List<TestAssemblyNode>();

            foreach (var mode in modes)
            {
                var root = await api.RetrieveTestTree(mode).ConfigureAwait(false);
                var modeAssemblies = await LocusAsync.RunOnMainThreadAsync(delegate
                {
                    return BuildDiscoveryTree(root, mode, request);
                }, ExecuteTimeoutMs).ConfigureAwait(false);
                assemblies.AddRange(modeAssemblies);
            }

            return OkResponse(requestId, JsonUtility.ToJson(new TestDiscoveryResponse
            {
                assemblies = assemblies.ToArray()
            }));
        }

        private static async Task<PipeEnvelope> HandleRunTests(string requestId, string json)
        {
            TestFrameworkApi api;
            try
            {
                api = await TestFrameworkApi.CreateAsync().ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "test_framework_missing: " + FormatReflectionException(ex));
            }

            var request = JsonUtility.FromJson<TestRunRequest>(json ?? "{}") ?? new TestRunRequest();
            var mode = RequestedModes(request.testMode).FirstOrDefault();
            if (string.IsNullOrEmpty(mode))
                return ErrorResponse(requestId, "unsupported test_mode");

            lock (_testRunnerLock)
            {
                if (_testRunActive)
                    return ErrorResponse(requestId, "busy");
                _testRunActive = true;
                _testRunProgress = new TestRunProgressPayload
                {
                    active = true,
                    runId = request.runId ?? "",
                    phase = mode,
                    currentTest = "",
                    completed = 0,
                    total = 0,
                    failed = 0,
                    revision = 1,
                };
            }

            try
            {
                var response = await RunTestPhase(api, request, mode).ConfigureAwait(false);
                return OkResponse(requestId, JsonUtility.ToJson(response));
            }
            finally
            {
                lock (_testRunnerLock)
                {
                    _testRunActive = false;
                    _activeTestCallbacks = null;
                    _testRunProgress.active = false;
                    _testRunProgress.revision++;
                }
            }
        }

        private static async Task<PipeEnvelope> HandleCancelTests(string requestId)
        {
            lock (_testRunnerLock)
            {
                if (_activeTestCallbacks == null)
                    return OkResponse(requestId, "cancel_tests requested");
            }

            try
            {
                var api = await TestFrameworkApi.CreateAsync().ConfigureAwait(false);
                await LocusAsync.RunOnMainThreadAsync(delegate
                {
                    api.CancelTestRun();
                    return true;
                }, ExecuteTimeoutMs).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "cancel_tests failed: " + FormatReflectionException(ex));
            }
            return OkResponse(requestId, "cancel_tests requested");
        }

        private static PipeEnvelope HandleTestRunProgress(string requestId)
        {
            lock (_testRunnerLock)
                return OkResponse(requestId, JsonUtility.ToJson(_testRunProgress));
        }

        private static async Task<PipeEnvelope> HandleOpenTestSource(string requestId, string json)
        {
            var request = JsonUtility.FromJson<TestSourceOpenRequest>(json ?? "{}") ?? new TestSourceOpenRequest();
            string assetPath = (request.path ?? "").Replace('\\', '/').TrimStart('/');
            if (!assetPath.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase)
                && !assetPath.StartsWith("Packages/", StringComparison.OrdinalIgnoreCase))
                return ErrorResponse(requestId, "source_outside_workspace");

            return await LocusAsync.RunOnMainThreadAsync(delegate
            {
                var asset = AssetDatabase.LoadMainAssetAtPath(assetPath);
                if (asset == null)
                    return ErrorResponse(requestId, "source_not_found");
                bool positioned = request.line > 0;
                bool opened = positioned
                    ? AssetDatabase.OpenAsset(asset, Math.Max(1, request.line))
                    : AssetDatabase.OpenAsset(asset);
                return OkResponse(requestId, JsonUtility.ToJson(new TestSourceOpenResponse
                {
                    opened = opened,
                    positioned = opened && positioned,
                }));
            }, ExecuteTimeoutMs).ConfigureAwait(false);
        }

        private static string[] RequestedModes(string raw)
        {
            switch ((raw ?? "all").Trim().ToLowerInvariant())
            {
                case "editmode":
                case "edit":
                    return new[] { "editmode" };
                case "playmode":
                case "play":
                    return new[] { "playmode" };
                case "all":
                case "":
                    return new[] { "editmode", "playmode" };
                default:
                    return new string[0];
            }
        }

        private static Task<TestRunPhaseResponse> RunTestPhase(TestFrameworkApi api, TestRunRequest request, string mode)
        {
            var tcs = LocusAsync.CreateTcs<TestRunPhaseResponse>();
            PostToMainThread(delegate
            {
                try
                {
                    var callbacks = TestRunCallbacksProxy.Create(api, request, mode, tcs, delegate(TestRunProgressPayload progress)
                    {
                        lock (_testRunnerLock)
                        {
                            progress.revision = _testRunProgress.revision + 1;
                            _testRunProgress = progress;
                        }
                    });
                    _activeTestCallbacks = callbacks;
                    api.RegisterCallbacks(callbacks.CallbackObject);
                    api.Execute(BuildExecutionFilter(api, request, mode));
                }
                catch (Exception ex)
                {
                    tcs.TrySetResult(new TestRunPhaseResponse
                    {
                        runId = request.runId ?? "",
                        testMode = mode,
                        status = "runtime_error",
                        errorCode = "unknown",
                        errorMessage = ex.ToString(),
                    });
                }
            });
            return tcs.Task;
        }

        private static object BuildExecutionFilter(TestFrameworkApi api, TestRunRequest request, string mode)
        {
            var names = new List<string>();
            if (request.tests != null)
            {
                foreach (var target in request.tests)
                {
                    string full = JoinFullName(target.fixtureName, target.testName);
                    if (!string.IsNullOrWhiteSpace(full))
                        names.Add(full);
                }
            }
            else
            {
                string full = JoinFullName(request.fixtureName, request.testName);
                if (!string.IsNullOrWhiteSpace(full))
                    names.Add(full);
            }

            var filter = api.CreateFilter(mode);
            var assemblies = new List<string>();
            if (!string.IsNullOrWhiteSpace(request.assemblyName))
                assemblies.Add(request.assemblyName.Trim());
            // Explicit targets already carry exact full names. Adding their discovery assembly
            // labels here can select no tests when an older bridge reported EditMode/PlayMode
            // instead of the real compiled assembly name.
            if (assemblies.Count > 0)
                TestFrameworkApi.SetMember(filter, "assemblyNames", assemblies.Distinct(StringComparer.OrdinalIgnoreCase).ToArray());
            if (names.Count > 0)
                TestFrameworkApi.SetMember(filter, "testNames", names.ToArray());
            return api.CreateExecutionSettings(filter);
        }

        private static string JoinFullName(string fixtureName, string testName)
        {
            string fixture = (fixtureName ?? "").Trim();
            string test = (testName ?? "").Trim();
            if (fixture.Length > 0 && test.StartsWith(fixture + ".", StringComparison.Ordinal))
                return test;
            var parts = new[] { fixtureName, testName }
                .Where(part => !string.IsNullOrWhiteSpace(part))
                .Select(part => part.Trim())
                .ToArray();
            return parts.Length == 0 ? "" : string.Join(".", parts);
        }

        private static List<TestAssemblyNode> BuildDiscoveryTree(object root, string mode, TestFindRequest request)
        {
            var byAssembly = new Dictionary<string, Dictionary<string, List<TestMethodNode>>>(StringComparer.OrdinalIgnoreCase);
            foreach (var test in EnumerateLeafTests(root))
            {
                var info = TestInfo.FromAdaptor(test, mode);
                if (!MatchesRequest(info, request))
                    continue;
                if (!byAssembly.TryGetValue(info.AssemblyName, out var fixtures))
                {
                    fixtures = new Dictionary<string, List<TestMethodNode>>(StringComparer.OrdinalIgnoreCase);
                    byAssembly[info.AssemblyName] = fixtures;
                }
                if (!fixtures.TryGetValue(info.FixtureName, out var tests))
                {
                    tests = new List<TestMethodNode>();
                    fixtures[info.FixtureName] = tests;
                }
                tests.Add(new TestMethodNode
                {
                    name = info.TestName,
                    fullName = info.FullName,
                    attributes = info.Attributes.ToArray(),
                    sourcePath = info.SourcePath,
                    line = info.Line,
                });
            }

            return byAssembly
                .OrderBy(pair => pair.Key, StringComparer.OrdinalIgnoreCase)
                .Select(assembly => new TestAssemblyNode
                {
                    name = assembly.Key,
                    testMode = mode,
                    fixtures = assembly.Value
                        .OrderBy(pair => pair.Key, StringComparer.OrdinalIgnoreCase)
                        .Select(fixture => new TestFixtureNode
                        {
                            name = fixture.Key,
                            tests = fixture.Value.OrderBy(test => test.name, StringComparer.OrdinalIgnoreCase).ToArray(),
                        })
                        .ToArray(),
                })
                .ToList();
        }

        private static IEnumerable<object> EnumerateLeafTests(object root)
        {
            if (root == null)
                yield break;

            var children = TestFrameworkApi.GetMember(root, "Children") as System.Collections.IEnumerable;
            var childList = children != null ? children.Cast<object>().ToArray() : new object[0];
            if (childList.Length == 0)
            {
                yield return root;
                yield break;
            }

            foreach (var child in childList)
            {
                foreach (var leaf in EnumerateLeafTests(child))
                    yield return leaf;
            }
        }

        private static IEnumerable<object> EnumerateLeafResults(object root)
        {
            if (root == null)
                yield break;

            var children = TestFrameworkApi.GetMember(root, "Children") as System.Collections.IEnumerable;
            var childList = children != null ? children.Cast<object>().ToArray() : new object[0];
            if (childList.Length == 0)
            {
                yield return root;
                yield break;
            }

            foreach (var child in childList)
            {
                foreach (var leaf in EnumerateLeafResults(child))
                    yield return leaf;
            }
        }

        private static bool MatchesRequest(TestInfo info, TestFindRequest request)
        {
            if (!MatchesExact(info.AssemblyName, request.assemblyName))
                return false;
            if (!MatchesFixture(info.FixtureName, request.fixtureName))
                return false;
            if (!MatchesExact(info.TestName, request.testName))
                return false;

            string search = (request.search ?? "").Trim();
            if (search.Length == 0)
                return true;

            return Contains(info.AssemblyName, search)
                || Contains(info.FixtureName, search)
                || Contains(info.TestName, search)
                || Contains(info.FullName, search)
                || Contains(info.SourcePath, search);
        }

        private static bool MatchesExact(string actual, string expected)
        {
            return string.IsNullOrWhiteSpace(expected)
                || string.Equals(actual ?? "", expected.Trim(), StringComparison.OrdinalIgnoreCase);
        }

        private static bool MatchesFixture(string actual, string expected)
        {
            if (string.IsNullOrWhiteSpace(expected))
                return true;
            string expectedTrimmed = expected.Trim();
            string actualValue = actual ?? "";
            if (string.Equals(actualValue, expectedTrimmed, StringComparison.OrdinalIgnoreCase))
                return true;
            int dot = actualValue.LastIndexOf('.');
            string shortName = dot >= 0 ? actualValue.Substring(dot + 1) : actualValue;
            return string.Equals(shortName, expectedTrimmed, StringComparison.OrdinalIgnoreCase);
        }

        private static bool Contains(string actual, string needle)
        {
            return (actual ?? "").IndexOf(needle, StringComparison.OrdinalIgnoreCase) >= 0;
        }

        private sealed class TestRunCallbacksProxy
        {
            private readonly TestFrameworkApi _api;
            private readonly TestRunRequest _request;
            private readonly string _mode;
            private readonly TaskCompletionSource<TestRunPhaseResponse> _completion;
            private readonly Action<TestRunProgressPayload> _progress;
            private readonly List<TestResultNode> _results = new List<TestResultNode>();
            private int _total;
            private int _completed;
            private int _failed;
            private int _skipped;

            public object CallbackObject { get; private set; }

            private TestRunCallbacksProxy(
                TestFrameworkApi api,
                TestRunRequest request,
                string mode,
                TaskCompletionSource<TestRunPhaseResponse> completion,
                Action<TestRunProgressPayload> progress)
            {
                _api = api;
                _request = request;
                _mode = mode;
                _completion = completion;
                _progress = progress;
            }

            public static TestRunCallbacksProxy Create(
                TestFrameworkApi api,
                TestRunRequest request,
                string mode,
                TaskCompletionSource<TestRunPhaseResponse> completion,
                Action<TestRunProgressPayload> progress)
            {
                var callback = new TestRunCallbacksProxy(api, request, mode, completion, progress);
                callback.CallbackObject = CreateTestFrameworkDispatchProxy(api.CallbacksType, callback);
                return callback;
            }

            public object Invoke(string methodName, object[] args)
            {
                switch (methodName)
                {
                    case "RunStarted":
                        RunStarted(args != null && args.Length > 0 ? args[0] : null);
                        break;
                    case "TestStarted":
                        TestStarted(args != null && args.Length > 0 ? args[0] : null);
                        break;
                    case "TestFinished":
                        TestFinished(args != null && args.Length > 0 ? args[0] : null);
                        break;
                    case "RunFinished":
                        RunFinished(args != null && args.Length > 0 ? args[0] : null);
                        break;
                }
                return null;
            }

            private void RunStarted(object testsToRun)
            {
                _total = EnumerateLeafTests(testsToRun).Count();
                Publish("");
            }

            private void TestStarted(object test)
            {
                Publish(TestFrameworkApi.GetString(test, "FullName"));
            }

            private void TestFinished(object result)
            {
                var node = ResultNode(result);
                if (node == null)
                    return;
                _results.Add(node);
                _completed++;
                if (node.outcome == "failed")
                    _failed++;
                if (node.outcome == "skipped")
                    _skipped++;
                Publish(node.fullName);
            }

            private void RunFinished(object result)
            {
                var finalResults = EnumerateLeafResults(result)
                    .Select(ResultNode)
                    .Where(node => node != null)
                    .ToList();
                _results.Clear();
                _results.AddRange(finalResults);
                _completed = _results.Count;
                _failed = _results.Count(item => item.outcome == "failed");
                _skipped = _results.Count(item => item.outcome == "skipped");
                var duration = _results.Sum(item => item.duration);
                bool noTestsExecuted = _results.Count == 0;
                var response = new TestRunPhaseResponse
                {
                    runId = _request.runId ?? "",
                    testMode = _mode,
                    status = noTestsExecuted ? "runtime_error" : (_failed > 0 ? "failed" : "passed"),
                    total = _results.Count,
                    passed = _results.Count - _failed - _skipped,
                    failed = _failed,
                    skipped = _skipped,
                    duration = duration,
                    results = _results.ToArray(),
                    errorCode = noTestsExecuted ? "no_tests_executed" : "",
                    errorMessage = noTestsExecuted ? "Unity selected no leaf tests for the requested scope" : "",
                };
                _completion.TrySetResult(response);
                try { _api.UnregisterCallbacks(CallbackObject); } catch { }
            }

            private void Publish(string current)
            {
                _progress(new TestRunProgressPayload
                {
                    active = true,
                    runId = _request.runId ?? "",
                    phase = _mode,
                    currentTest = current ?? "",
                    completed = _completed,
                    total = _total,
                    failed = _failed,
                });
            }

            private TestResultNode ResultNode(object result)
            {
                if (result == null || TestFrameworkApi.GetBool(result, "HasChildren"))
                    return null;
                var fullName = TestFrameworkApi.GetString(result, "FullName");
                var test = TestFrameworkApi.GetMember(result, "Test");
                if (TestFrameworkApi.GetBool(test, "IsSuite"))
                    return null;
                var info = test != null
                    ? TestInfo.FromAdaptor(test, _mode)
                    : TestInfo.FromFullName(fullName, _mode);
                if (string.IsNullOrEmpty(info.FullName))
                    info.FullName = fullName;
                return new TestResultNode
                {
                    assemblyName = info.AssemblyName,
                    fixtureName = info.FixtureName,
                    testName = info.TestName,
                    fullName = fullName,
                    outcome = NormalizeOutcome(TestFrameworkApi.GetMember(result, "TestStatus")?.ToString()),
                    duration = TestFrameworkApi.GetDouble(result, "Duration"),
                    message = TestFrameworkApi.GetString(result, "Message"),
                    stackTrace = TestFrameworkApi.GetString(result, "StackTrace"),
                    sourcePath = info.SourcePath,
                    line = info.Line,
                };
            }
        }

        private sealed class TestInfo
        {
            public string AssemblyName;
            public string FixtureName;
            public string TestName;
            public string FullName;
            public string SourcePath;
            public int Line;
            public List<string> Attributes = new List<string>();

            public static TestInfo FromAdaptor(object test, string mode)
            {
                var fullName = TestFrameworkApi.GetString(test, "FullName");
                var info = FromFullName(fullName, mode);
                info.AssemblyName = ResolveAssemblyName(test, info.AssemblyName);
                var method = TestFrameworkApi.GetMember(test, "Method") as MethodInfo;
                if (method != null)
                {
                    info.TestName = method.Name;
                    info.FixtureName = method.DeclaringType != null ? method.DeclaringType.FullName : info.FixtureName;
                    info.AssemblyName = method.DeclaringType != null ? method.DeclaringType.Assembly.GetName().Name : info.AssemblyName;
                    info.FullName = string.IsNullOrEmpty(fullName) ? info.FixtureName + "." + info.TestName : fullName;
                    info.Attributes = method.GetCustomAttributes(false).Select(attr => attr.GetType().Name.Replace("Attribute", "")).ToList();
                    ResolveSource(info, method);
                }
                return info;
            }

            private static string ResolveAssemblyName(object test, string fallback)
            {
                var current = test;
                while (current != null)
                {
                    if (TestFrameworkApi.GetBool(current, "IsTestAssembly"))
                    {
                        var name = TestFrameworkApi.GetString(current, "Name");
                        if (!string.IsNullOrWhiteSpace(name))
                            return Path.GetFileNameWithoutExtension(name.Trim());
                    }
                    current = TestFrameworkApi.GetMember(current, "Parent");
                }
                return fallback;
            }

            public static TestInfo FromFullName(string fullName, string mode)
            {
                string safe = fullName ?? "";
                var parts = safe.Split('.');
                string test = parts.Length > 0 ? parts[parts.Length - 1] : safe;
                string fixture = parts.Length > 1 ? string.Join(".", parts.Take(parts.Length - 1).ToArray()) : "";
                return new TestInfo
                {
                    AssemblyName = mode == "playmode" ? "PlayMode" : "EditMode",
                    FixtureName = fixture,
                    TestName = test,
                    FullName = safe,
                    SourcePath = "",
                    Line = 0,
                };
            }

            private static void ResolveSource(TestInfo info, MethodInfo method)
            {
                try
                {
                    string typeName = method.DeclaringType != null ? method.DeclaringType.Name : "";
                    string[] guids = AssetDatabase.FindAssets(typeName + " t:Script");
                    foreach (string guid in guids)
                    {
                        string path = AssetDatabase.GUIDToAssetPath(guid);
                        if (string.IsNullOrEmpty(path) || !File.Exists(path))
                            continue;
                        string text = File.ReadAllText(path);
                        if (text.IndexOf("class " + typeName, StringComparison.Ordinal) < 0)
                            continue;
                        info.SourcePath = path.Replace('\\', '/');
                        info.Line = FindLine(text, method.Name);
                        return;
                    }
                }
                catch
                {
                }
            }

            private static int FindLine(string text, string needle)
            {
                var lines = text.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n');
                for (int i = 0; i < lines.Length; i++)
                {
                    if (lines[i].IndexOf(needle, StringComparison.Ordinal) >= 0)
                        return i + 1;
                }
                return 0;
            }
        }

        private sealed class TestFrameworkApi
        {
            private readonly object _api;
            private readonly Type _testModeType;
            private readonly Type _filterType;
            private readonly Type _executionSettingsType;

            public Type CallbacksType { get; private set; }

            private TestFrameworkApi(object api, Type testModeType, Type filterType, Type executionSettingsType, Type callbacksType)
            {
                _api = api;
                _testModeType = testModeType;
                _filterType = filterType;
                _executionSettingsType = executionSettingsType;
                CallbacksType = callbacksType;
            }

            public static bool IsAvailable()
            {
                Type apiType;
                Type testModeType;
                Type filterType;
                Type executionSettingsType;
                Type callbacksType;
                string error;
                return TryResolveTypes(out apiType, out testModeType, out filterType, out executionSettingsType, out callbacksType, out error);
            }

            public static async Task<TestFrameworkApi> CreateAsync()
            {
                Type apiType;
                Type testModeType;
                Type filterType;
                Type executionSettingsType;
                Type callbacksType;
                string error;
                if (!TryResolveTypes(out apiType, out testModeType, out filterType, out executionSettingsType, out callbacksType, out error))
                    throw new InvalidOperationException(error);

                return await LocusAsync.RunOnMainThreadAsync(delegate
                {
                    return new TestFrameworkApi(
                        ScriptableObject.CreateInstance(apiType),
                        testModeType,
                        filterType,
                        executionSettingsType,
                        callbacksType);
                }, ExecuteTimeoutMs).ConfigureAwait(false);
            }

            private static bool TryResolveTypes(
                out Type apiType,
                out Type testModeType,
                out Type filterType,
                out Type executionSettingsType,
                out Type callbacksType,
                out string error)
            {
                error = null;
                apiType = Type.GetType("UnityEditor.TestTools.TestRunner.Api.TestRunnerApi, UnityEditor.TestRunner");
                testModeType = Type.GetType("UnityEditor.TestTools.TestRunner.Api.TestMode, UnityEditor.TestRunner");
                filterType = Type.GetType("UnityEditor.TestTools.TestRunner.Api.Filter, UnityEditor.TestRunner");
                executionSettingsType = Type.GetType("UnityEditor.TestTools.TestRunner.Api.ExecutionSettings, UnityEditor.TestRunner");
                callbacksType = Type.GetType("UnityEditor.TestTools.TestRunner.Api.ICallbacks, UnityEditor.TestRunner");
                if (apiType == null || testModeType == null || filterType == null || executionSettingsType == null || callbacksType == null)
                {
                    error = "test_framework_missing";
                    return false;
                }
                return true;
            }

            public Task<object> RetrieveTestTree(string mode)
            {
                var tcs = LocusAsync.CreateTcs<object>();
                PostToMainThread(delegate
                {
                    try
                    {
                        var method = _api.GetType().GetMethods()
                            .FirstOrDefault(item => item.Name == "RetrieveTestList" && item.GetParameters().Length == 2);
                        if (method == null)
                            throw new MissingMethodException("RetrieveTestList");
                        var callbackType = method.GetParameters()[1].ParameterType;
                        var callback = Delegate.CreateDelegate(
                            callbackType,
                            new TestListCallback(tcs),
                            typeof(TestListCallback).GetMethod(nameof(TestListCallback.OnRetrieved), BindingFlags.Instance | BindingFlags.Public));
                        method.Invoke(_api, new[] { ParseMode(mode), callback });
                    }
                    catch (Exception ex)
                    {
                        tcs.TrySetException(new Exception(FormatReflectionException(ex), ex));
                    }
                });
                return tcs.Task;
            }

            public object CreateFilter(string mode)
            {
                var filter = Activator.CreateInstance(_filterType);
                SetMember(filter, "testMode", ParseMode(mode));
                return filter;
            }

            public object CreateExecutionSettings(object filter)
            {
                return Activator.CreateInstance(_executionSettingsType, filter);
            }

            public void Execute(object executionSettings)
            {
                InvokeApi(_api.GetType().GetMethod("Execute", new[] { _executionSettingsType }), new[] { executionSettings });
            }

            public void RegisterCallbacks(object callbacks)
            {
                var method = _api.GetType().GetMethod("RegisterCallbacks", new[] { CallbacksType });
                if (method != null)
                {
                    InvokeApi(method, new[] { callbacks });
                    return;
                }

                method = _api.GetType().GetMethods()
                    .FirstOrDefault(item => item.Name == "RegisterCallbacks" && item.IsGenericMethodDefinition);
                if (method == null)
                    throw new MissingMethodException("RegisterCallbacks");
                method = method.MakeGenericMethod(CallbacksType);
                var parameters = method.GetParameters();
                InvokeApi(method, parameters.Length > 1 ? new[] { callbacks, (object)0 } : new[] { callbacks });
            }

            public void UnregisterCallbacks(object callbacks)
            {
                var method = _api.GetType().GetMethod("UnregisterCallbacks", new[] { CallbacksType });
                if (method != null)
                {
                    InvokeApi(method, new[] { callbacks });
                    return;
                }

                method = _api.GetType().GetMethods()
                    .FirstOrDefault(item => item.Name == "UnregisterCallbacks" && item.IsGenericMethodDefinition);
                if (method == null)
                    throw new MissingMethodException("UnregisterCallbacks");
                InvokeApi(method.MakeGenericMethod(CallbacksType), new[] { callbacks });
            }

            public void CancelTestRun()
            {
                var method = _api.GetType().GetMethod("CancelTestRun", Type.EmptyTypes);
                if (method != null)
                    InvokeApi(method, null);
            }

            private void InvokeApi(MethodInfo method, object[] args)
            {
                if (method == null)
                    throw new MissingMethodException("Unity Test Framework API method not found");
                try
                {
                    method.Invoke(_api, args);
                }
                catch (Exception ex)
                {
                    throw new Exception(FormatReflectionException(ex), ex);
                }
            }

            private object ParseMode(string mode)
            {
                var value = mode == "playmode" ? "PlayMode" : "EditMode";
                return Enum.Parse(_testModeType, value);
            }

            public static object GetMember(object target, string name)
            {
                if (target == null)
                    return null;
                var flags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
                var type = target.GetType();
                var prop = type.GetProperty(name, flags);
                if (prop != null)
                    return prop.GetValue(target, null);
                var field = type.GetField(name, flags);
                return field != null ? field.GetValue(target) : null;
            }

            public static void SetMember(object target, string name, object value)
            {
                if (target == null)
                    return;
                var flags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
                var type = target.GetType();
                var prop = type.GetProperty(name, flags);
                if (prop != null)
                {
                    prop.SetValue(target, value, null);
                    return;
                }
                var field = type.GetField(name, flags);
                if (field != null)
                    field.SetValue(target, value);
            }

            public static string GetString(object target, string name)
            {
                return GetMember(target, name)?.ToString() ?? "";
            }

            public static bool GetBool(object target, string name)
            {
                var value = GetMember(target, name);
                return value is bool flag && flag;
            }

            public static double GetDouble(object target, string name)
            {
                var value = GetMember(target, name);
                if (value == null)
                    return 0;
                try { return Convert.ToDouble(value); } catch { return 0; }
            }

            private sealed class TestListCallback
            {
                private readonly TaskCompletionSource<object> _completion;

                public TestListCallback(TaskCompletionSource<object> completion)
                {
                    _completion = completion;
                }

                public void OnRetrieved(object root)
                {
                    _completion.TrySetResult(root);
                }
            }
        }

        private static object CreateTestFrameworkDispatchProxy(Type interfaceType, TestRunCallbacksProxy target)
        {
            var create = typeof(DispatchProxy).GetMethods(BindingFlags.Public | BindingFlags.Static)
                .FirstOrDefault(m => m.Name == "Create" && m.IsGenericMethodDefinition && m.GetParameters().Length == 0);
            if (create == null)
                throw new InvalidOperationException("DispatchProxy.Create<T, TProxy>() is unavailable");

            var proxyObject = create
                .MakeGenericMethod(interfaceType, typeof(TestFrameworkDispatchProxy))
                .Invoke(null, null);
            var proxy = proxyObject as TestFrameworkDispatchProxy;
            if (proxy == null)
                throw new InvalidOperationException("Failed to create test framework callbacks proxy");
            proxy._target = target;
            return proxyObject;
        }

        public class TestFrameworkDispatchProxy : DispatchProxy
        {
            internal object _target;

            public TestFrameworkDispatchProxy()
            {
            }

            protected override object Invoke(MethodInfo targetMethod, object[] args)
            {
                var target = _target as TestRunCallbacksProxy;
                if (target == null)
                    throw new InvalidOperationException("Test framework callback target is not initialized");
                return target.Invoke(targetMethod.Name, args);
            }
        }

        private static string NormalizeOutcome(string status)
        {
            switch ((status ?? "").Trim().ToLowerInvariant())
            {
                case "passed":
                    return "passed";
                case "skipped":
                case "inconclusive":
                    return "skipped";
                case "failed":
                default:
                    return "failed";
            }
        }

        private static string FormatReflectionException(Exception ex)
        {
            var target = ex as TargetInvocationException;
            if (target != null && target.InnerException != null)
                return target.InnerException.GetType().Name + ": " + target.InnerException.Message + "\n" + target.InnerException.StackTrace;
            return ex.GetType().Name + ": " + ex.Message + "\n" + ex.StackTrace;
        }
    }
}
