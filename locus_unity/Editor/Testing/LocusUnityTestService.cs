using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using Locus;
using UnityEditor;
using UnityEditor.TestTools.TestRunner.Api;
using UnityEngine;

namespace Locus.UnityTesting
{
    [Serializable]
    internal class UnityTestFilterRequest
    {
        public string mode;
        public string[] assemblies;
        public string[] tests;
        public string[] groups;
        public string[] categories;
        public int max_results = 500;
    }

    [Serializable]
    internal sealed class UnityTestRunRequest : UnityTestFilterRequest
    {
        public string result_detail = "failures";
    }

    [Serializable]
    internal sealed class UnityTestStatusRequest
    {
        public string run_id;
    }

    [Serializable]
    internal sealed class UnityTestCaseDto
    {
        public string name;
        public string full_name;
        public string assembly;
        public string mode;
        public string[] path;
        public string[] categories;
    }

    [Serializable]
    internal sealed class UnityTestListDto
    {
        public string mode;
        public int matched;
        public bool truncated;
        public List<UnityTestCaseDto> tests = new List<UnityTestCaseDto>();
    }

    [Serializable]
    internal sealed class UnityTestResultDto
    {
        public string full_name;
        public string result_state;
        public long duration_ms;
        public string message;
        public string stack_trace;
        public string output;
    }

    [Serializable]
    internal sealed class UnityTestRunSnapshotDto
    {
        public string run_id;
        public string unity_run_guid;
        public string status;
        public string mode;
        public string current_test;
        public long started_at_ticks;
        public long finished_at_ticks;
        public long duration_ms;
        public int total;
        public int passed;
        public int failed;
        public int skipped;
        public int inconclusive;
        public string error;
        public List<UnityTestResultDto> failures;
        public List<UnityTestResultDto> results;
    }

    [FilePath("Library/Locus/UnityTestRunState.asset", FilePathAttribute.Location.ProjectFolder)]
    internal sealed class LocusUnityTestRunState : ScriptableSingleton<LocusUnityTestRunState>
    {
        public bool active;
        public string editor_session_id;
        public string run_id;
        public string unity_run_guid;
        public string status;
        public string mode;
        public string current_test;
        public string result_detail;
        public bool cancellation_requested;
        public string error;
        public long started_at_ticks;
        public long finished_at_ticks;
        public long duration_ms;
        public int total;
        public int passed;
        public int failed;
        public int skipped;
        public int inconclusive;
        public List<UnityTestResultDto> failures = new List<UnityTestResultDto>();
        public List<UnityTestResultDto> results = new List<UnityTestResultDto>();

        public void Persist()
        {
            Save(true);
        }

        public void Begin(string editorSessionId, string modeValue, string resultDetail)
        {
            active = true;
            editor_session_id = editorSessionId;
            run_id = Guid.NewGuid().ToString("N");
            unity_run_guid = "";
            status = "starting";
            mode = modeValue;
            current_test = "";
            result_detail = resultDetail;
            cancellation_requested = false;
            error = "";
            started_at_ticks = DateTime.UtcNow.Ticks;
            finished_at_ticks = 0;
            duration_ms = 0;
            total = 0;
            passed = 0;
            failed = 0;
            skipped = 0;
            inconclusive = 0;
            failures.Clear();
            results.Clear();
            Persist();
        }
    }

    internal sealed class LocusUnityTestCallbacks : ICallbacks
    {
        public void RunStarted(ITestAdaptor testsToRun)
        {
            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            if (!state.active)
                return;
            state.status = state.cancellation_requested ? "cancelling" : "running";
            state.total = testsToRun != null ? testsToRun.TestCaseCount : 0;
            state.Persist();
        }

        public void RunFinished(ITestResultAdaptor result)
        {
            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            if (!state.active)
                return;

            bool cancelled = state.cancellation_requested;
            state.active = false;
            state.current_test = "";
            state.finished_at_ticks = DateTime.UtcNow.Ticks;
            if (result != null)
            {
                state.duration_ms = SecondsToMilliseconds(result.Duration);
                state.passed = result.PassCount;
                state.failed = result.FailCount;
                state.skipped = result.SkipCount;
                state.inconclusive = result.InconclusiveCount;
                state.total = state.passed + state.failed + state.skipped + state.inconclusive;
            }
            state.status = cancelled ? "cancelled" : (state.failed > 0 ? "failed" : "passed");
            state.Persist();
        }

        public void TestStarted(ITestAdaptor test)
        {
            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            if (!state.active || test == null || test.HasChildren)
                return;
            state.current_test = test.FullName ?? test.Name ?? "";
            state.Persist();
        }

        public void TestFinished(ITestResultAdaptor result)
        {
            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            if (!state.active || result == null || result.HasChildren)
                return;

            UnityTestResultDto dto = ResultDto(result);
            string resultState = result.ResultState ?? "";
            if (resultState.StartsWith("Passed", StringComparison.OrdinalIgnoreCase))
                state.passed++;
            else if (resultState.StartsWith("Failed", StringComparison.OrdinalIgnoreCase))
            {
                state.failed++;
                state.failures.Add(dto);
            }
            else if (resultState.StartsWith("Skipped", StringComparison.OrdinalIgnoreCase))
                state.skipped++;
            else
                state.inconclusive++;

            if (string.Equals(state.result_detail, "all", StringComparison.OrdinalIgnoreCase))
                state.results.Add(dto);
            state.current_test = "";
            state.Persist();
        }

        private static UnityTestResultDto ResultDto(ITestResultAdaptor result)
        {
            return new UnityTestResultDto
            {
                full_name = result.FullName ?? result.Name ?? "",
                result_state = result.ResultState ?? "",
                duration_ms = SecondsToMilliseconds(result.Duration),
                message = result.Message ?? "",
                stack_trace = result.StackTrace ?? "",
                output = result.Output ?? ""
            };
        }

        private static long SecondsToMilliseconds(double seconds)
        {
            return (long)Math.Round(Math.Max(0d, seconds) * 1000d);
        }
    }

    [InitializeOnLoad]
    internal static class LocusUnityTestService
    {
        private const string EditorSessionStateKey = "Locus_UnityTestEditorSessionId";
        private static readonly TestRunnerApi Api;
        private static readonly LocusUnityTestCallbacks Callbacks;

        static LocusUnityTestService()
        {
            Api = ScriptableObject.CreateInstance<TestRunnerApi>();
            Api.hideFlags = HideFlags.HideAndDontSave;
            Callbacks = new LocusUnityTestCallbacks();
            Api.RegisterCallbacks(Callbacks, 1000);

            RecoverEditorSession();
            LocusBridge.RegisterExtensionMessageHandler("unity_test_list", HandleListAsync);
            LocusBridge.RegisterExtensionMessageHandler("unity_test_start", HandleStartAsync);
            LocusBridge.RegisterExtensionMessageHandler("unity_test_status", HandleStatusAsync);
            LocusBridge.RegisterExtensionMessageHandler("unity_test_cancel", HandleCancelAsync);
        }

        private static string EditorSessionId()
        {
            string value = SessionState.GetString(EditorSessionStateKey, "");
            if (!string.IsNullOrEmpty(value))
                return value;
            value = Guid.NewGuid().ToString("N");
            SessionState.SetString(EditorSessionStateKey, value);
            return value;
        }

        private static void RecoverEditorSession()
        {
            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            string currentSessionId = EditorSessionId();
            if (!state.active || string.IsNullOrEmpty(state.editor_session_id) ||
                state.editor_session_id == currentSessionId)
                return;

            state.active = false;
            state.status = "error";
            state.error = "Unity Editor restarted before the test run completed.";
            state.finished_at_ticks = DateTime.UtcNow.Ticks;
            state.Persist();
        }

        private static Task<string> HandleStartAsync(string json)
        {
            UnityTestRunRequest request = ParseRequest<UnityTestRunRequest>(json);
            return Task.FromResult(JsonUtility.ToJson(Start(request), true));
        }

        internal static UnityTestRunSnapshotDto Start(UnityTestRunRequest request)
        {
            if (request == null)
                request = new UnityTestRunRequest();
            TestMode mode = ParseMode(request.mode);
            string modeName = ModeName(mode);
            string resultDetail = NormalizeResultDetail(request.result_detail);
            ValidateRegexes(request.groups);

            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            if (state.active)
                throw new InvalidOperationException("A Unity Test run is already active.");

            state.Begin(EditorSessionId(), modeName, resultDetail);
            try
            {
                Filter filter = BuildFilter(request, mode);
                state.unity_run_guid = Api.Execute(new ExecutionSettings(filter));
                state.Persist();
                return Snapshot(state);
            }
            catch (Exception ex)
            {
                state.active = false;
                state.status = "error";
                state.error = ex.Message;
                state.finished_at_ticks = DateTime.UtcNow.Ticks;
                state.Persist();
                throw;
            }
        }

        private static Task<string> HandleStatusAsync(string json)
        {
            UnityTestStatusRequest request = string.IsNullOrWhiteSpace(json)
                ? new UnityTestStatusRequest()
                : ParseRequest<UnityTestStatusRequest>(json);
            return Task.FromResult(JsonUtility.ToJson(Status(request.run_id), true));
        }

        internal static UnityTestRunSnapshotDto Status(string runId = null)
        {
            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            if (!string.IsNullOrEmpty(runId) && runId != state.run_id)
                throw new InvalidOperationException("Unity Test run was not found: " + runId);
            return Snapshot(state);
        }

        private static Task<string> HandleCancelAsync(string json)
        {
            UnityTestStatusRequest request = string.IsNullOrWhiteSpace(json)
                ? new UnityTestStatusRequest()
                : ParseRequest<UnityTestStatusRequest>(json);
            return Task.FromResult(JsonUtility.ToJson(Cancel(request.run_id), true));
        }

        internal static UnityTestRunSnapshotDto Cancel(string runId = null)
        {
            LocusUnityTestRunState state = LocusUnityTestRunState.instance;
            if (!string.IsNullOrEmpty(runId) && runId != state.run_id)
                throw new InvalidOperationException("Unity Test run was not found: " + runId);
            if (!state.active)
                return Snapshot(state);

            string previousStatus = state.status;
            state.status = "cancelling";
            state.Persist();
            bool accepted = false;
#if LOCUS_HAS_UNITY_TEST_CANCEL
            accepted = TestRunnerApi.CancelTestRun(state.unity_run_guid);
#endif
            if (!accepted && string.Equals(state.mode, "play", StringComparison.OrdinalIgnoreCase))
            {
                EditorApplication.isPlaying = false;
                accepted = true;
            }
            if (!accepted)
            {
                state.status = previousStatus;
                state.error = "The installed Unity Test Framework does not expose test cancellation.";
            }
            else
            {
                state.cancellation_requested = true;
            }
            state.Persist();
            return Snapshot(state);
        }

        private static Task<string> HandleListAsync(string json)
        {
            UnityTestFilterRequest request = ParseRequest<UnityTestFilterRequest>(json);
            return SerializeListAsync(request);
        }

        private static async Task<string> SerializeListAsync(UnityTestFilterRequest request)
        {
            UnityTestListDto response = await ListAsync(request);
            return JsonUtility.ToJson(response, true);
        }

        internal static async Task<UnityTestListDto> ListAsync(UnityTestFilterRequest request)
        {
            if (request == null)
                request = new UnityTestFilterRequest();
            TestMode mode = ParseMode(request.mode);
            ValidateRegexes(request.groups);
            int maxResults = request.max_results <= 0 ? 500 : Math.Min(request.max_results, 5000);
            UnityTestListDto response = new UnityTestListDto { mode = ModeName(mode) };

            if ((mode & TestMode.EditMode) == TestMode.EditMode)
                await CollectModeAsync(
                    TestMode.EditMode,
                    "edit",
                    request,
                    response,
                    maxResults);
            if ((mode & TestMode.PlayMode) == TestMode.PlayMode)
                await CollectModeAsync(
                    TestMode.PlayMode,
                    "play",
                    request,
                    response,
                    maxResults);

            return response;
        }

        private static Task CollectModeAsync(
            TestMode mode,
            string modeName,
            UnityTestFilterRequest request,
            UnityTestListDto response,
            int maxResults)
        {
            TaskCompletionSource<bool> completion = new TaskCompletionSource<bool>();
            Api.RetrieveTestList(mode, delegate(ITestAdaptor root)
            {
                try
                {
                    CollectTests(
                        root,
                        "",
                        new List<string>(),
                        true,
                        modeName,
                        request,
                        response,
                        maxResults);
                    completion.TrySetResult(true);
                }
                catch (Exception ex)
                {
                    completion.TrySetException(ex);
                }
            });
            return completion.Task;
        }

        private static void CollectTests(
            ITestAdaptor node,
            string assemblyName,
            List<string> suitePath,
            bool isRoot,
            string modeName,
            UnityTestFilterRequest request,
            UnityTestListDto response,
            int maxResults)
        {
            if (node == null)
                return;
            if (node.IsTestAssembly)
            {
                assemblyName = TrimAssemblyExtension(node.Name);
                suitePath = new List<string>();
            }
            else if (!isRoot && node.IsSuite && !string.IsNullOrWhiteSpace(node.Name))
            {
                suitePath = new List<string>(suitePath) { node.Name };
            }

            if (!node.IsSuite && Matches(node, assemblyName, request))
            {
                response.matched++;
                if (response.tests.Count < maxResults)
                {
                    List<string> testPath = new List<string>(suitePath);
                    if (!string.IsNullOrWhiteSpace(node.Name))
                        testPath.Add(node.Name);
                    response.tests.Add(new UnityTestCaseDto
                    {
                        name = node.Name ?? "",
                        full_name = node.FullName ?? node.Name ?? "",
                        assembly = assemblyName ?? "",
                        mode = modeName,
                        path = testPath.ToArray(),
                        categories = node.Categories ?? new string[0]
                    });
                }
                else
                {
                    response.truncated = true;
                }
            }

            if (!node.HasChildren)
                return;
            foreach (ITestAdaptor child in node.Children)
                CollectTests(
                    child,
                    assemblyName,
                    suitePath,
                    false,
                    modeName,
                    request,
                    response,
                    maxResults);
        }

        private static bool Matches(
            ITestAdaptor test,
            string assemblyName,
            UnityTestFilterRequest request)
        {
            string fullName = test.FullName ?? test.Name ?? "";
            if (HasValues(request.assemblies) &&
                !request.assemblies.Any(value => string.Equals(
                    TrimAssemblyExtension(value), assemblyName, StringComparison.OrdinalIgnoreCase)))
                return false;
            if (HasValues(request.tests) &&
                !request.tests.Any(value => string.Equals(value, fullName, StringComparison.Ordinal)))
                return false;
            if (HasValues(request.groups) &&
                !request.groups.Any(pattern => Regex.IsMatch(fullName, pattern)))
                return false;
            if (HasValues(request.categories))
            {
                string[] categories = test.Categories ?? new string[0];
                if (!request.categories.Any(expected => categories.Any(actual =>
                    string.Equals(expected, actual, StringComparison.OrdinalIgnoreCase))))
                    return false;
            }
            return true;
        }

        private static Filter BuildFilter(UnityTestFilterRequest request, TestMode mode)
        {
            return new Filter
            {
                testMode = mode,
                assemblyNames = NormalizeValues(request.assemblies),
                testNames = NormalizeValues(request.tests),
                groupNames = NormalizeValues(request.groups),
                categoryNames = NormalizeValues(request.categories)
            };
        }

        private static UnityTestRunSnapshotDto Snapshot(LocusUnityTestRunState state)
        {
            bool includeFailures = !string.Equals(
                state.result_detail, "summary", StringComparison.OrdinalIgnoreCase);
            bool includeResults = string.Equals(
                state.result_detail, "all", StringComparison.OrdinalIgnoreCase);
            return new UnityTestRunSnapshotDto
            {
                run_id = state.run_id ?? "",
                unity_run_guid = state.unity_run_guid ?? "",
                status = state.status ?? "idle",
                mode = state.mode ?? "",
                current_test = state.current_test ?? "",
                started_at_ticks = state.started_at_ticks,
                finished_at_ticks = state.finished_at_ticks,
                duration_ms = state.duration_ms,
                total = state.total,
                passed = state.passed,
                failed = state.failed,
                skipped = state.skipped,
                inconclusive = state.inconclusive,
                error = state.error ?? "",
                failures = includeFailures ? new List<UnityTestResultDto>(state.failures) : null,
                results = includeResults ? new List<UnityTestResultDto>(state.results) : null
            };
        }

        private static T ParseRequest<T>(string json) where T : new()
        {
            if (string.IsNullOrWhiteSpace(json))
                return new T();
            T value = JsonUtility.FromJson<T>(json);
            if (value == null)
                throw new InvalidOperationException("Unity Test request is invalid.");
            return value;
        }

        private static TestMode ParseMode(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
                return TestMode.EditMode | TestMode.PlayMode;

            TestMode mode = (TestMode)0;
            foreach (string rawPart in value.Split('|'))
            {
                string part = rawPart.Trim();
                if (string.Equals(part, "edit", StringComparison.OrdinalIgnoreCase))
                    mode |= TestMode.EditMode;
                else if (string.Equals(part, "play", StringComparison.OrdinalIgnoreCase))
                    mode |= TestMode.PlayMode;
                else
                    throw new InvalidOperationException(
                        "Unity Test mode must be 'edit', 'play', or 'edit|play'.");
            }
            if (mode == 0)
                throw new InvalidOperationException(
                    "Unity Test mode must be 'edit', 'play', or 'edit|play'.");
            return mode;
        }

        private static string ModeName(TestMode mode)
        {
            bool includesEdit = (mode & TestMode.EditMode) == TestMode.EditMode;
            bool includesPlay = (mode & TestMode.PlayMode) == TestMode.PlayMode;
            if (includesEdit && includesPlay)
                return "edit|play";
            if (includesPlay)
                return "play";
            if (includesEdit)
                return "edit";
            throw new InvalidOperationException("Unity Test mode did not include edit or play.");
        }

        private static string NormalizeResultDetail(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
                return "failures";
            string normalized = value.Trim().ToLowerInvariant();
            if (normalized == "summary" || normalized == "failures" || normalized == "all")
                return normalized;
            throw new InvalidOperationException(
                "Unity Test result_detail must be 'summary', 'failures', or 'all'.");
        }

        private static string[] NormalizeValues(string[] values)
        {
            if (values == null)
                return null;
            string[] normalized = values
                .Where(value => !string.IsNullOrWhiteSpace(value))
                .Select(value => value.Trim())
                .Distinct()
                .ToArray();
            return normalized.Length == 0 ? null : normalized;
        }

        private static bool HasValues(string[] values)
        {
            return values != null && values.Any(value => !string.IsNullOrWhiteSpace(value));
        }

        private static void ValidateRegexes(string[] patterns)
        {
            if (!HasValues(patterns))
                return;
            foreach (string pattern in patterns)
            {
                if (!string.IsNullOrWhiteSpace(pattern))
                    _ = new Regex(pattern);
            }
        }

        private static string TrimAssemblyExtension(string value)
        {
            string normalized = (value ?? "").Trim();
            return normalized.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)
                ? normalized.Substring(0, normalized.Length - 4)
                : normalized;
        }
    }
}
