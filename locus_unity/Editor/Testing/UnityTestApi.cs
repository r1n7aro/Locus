using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using UnityEngine;

namespace Locus
{
    /// <summary>Unity Test Framework modes used by <see cref="UnityTestApi"/>.</summary>
    [Flags]
    public enum UnityTestMode
    {
        Edit = 1 << 0,
        Play = 1 << 1,
        EditAndPlay = Edit | Play
    }

    /// <summary>Amount of per-test data retained by a Unity Test run.</summary>
    public enum UnityTestResultDetail
    {
        Summary,
        Failures,
        All
    }

    /// <summary>Filters used when discovering or running Unity tests.</summary>
    [Serializable]
    public class UnityTestQuery
    {
        public UnityTestMode Mode = UnityTestMode.EditAndPlay;
        public string[] Assemblies;
        public string[] Tests;
        public string[] Groups;
        public string[] Categories;
        public int MaxResults = 500;
    }

    /// <summary>Options used when starting a Unity Test Framework run.</summary>
    [Serializable]
    public sealed class UnityTestRunOptions : UnityTestQuery
    {
        public UnityTestResultDetail ResultDetail = UnityTestResultDetail.Failures;
    }

    /// <summary>A leaf test discovered by the Unity Test Framework.</summary>
    [Serializable]
    public sealed class UnityTestCase
    {
        public string Name;
        public string FullName;
        public string Assembly;
        public string Mode;
        public string[] Path;
        public string[] Categories;
    }

    /// <summary>Result of a Unity Test Framework discovery request.</summary>
    [Serializable]
    public sealed class UnityTestListResult
    {
        public string Mode;
        public int Matched;
        public bool Truncated;
        public UnityTestCase[] Tests;

        public int Count
        {
            get { return Tests == null ? 0 : Tests.Length; }
        }
    }

    /// <summary>Result data for one completed Unity test.</summary>
    [Serializable]
    public sealed class UnityTestResult
    {
        public string FullName;
        public string ResultState;
        public long DurationMs;
        public string Message;
        public string StackTrace;
        public string Output;
    }

    /// <summary>Persisted snapshot of a Unity Test Framework run.</summary>
    [Serializable]
    public sealed class UnityTestRunSnapshot
    {
        public string RunId;
        public string UnityRunGuid;
        public string Status;
        public string Mode;
        public string CurrentTest;
        public long StartedAtTicks;
        public long FinishedAtTicks;
        public long DurationMs;
        public int Total;
        public int Passed;
        public int Failed;
        public int Skipped;
        public int Inconclusive;
        public string Error;
        public UnityTestResult[] Failures;
        public UnityTestResult[] Results;

        public int Completed
        {
            get { return Passed + Failed + Skipped + Inconclusive; }
        }

        public bool IsComplete
        {
            get { return UnityTestApi.IsTerminalStatus(Status); }
        }

        public bool Succeeded
        {
            get { return string.Equals(Status, "passed", StringComparison.OrdinalIgnoreCase); }
        }
    }

    /// <summary>
    /// Typed facade over the project's official Unity Test Framework.
    /// Start returns a persisted run handle; use Status and Cancel from later
    /// unity_execute calls so Play Mode and domain reloads do not require a
    /// reflection-based TestRunner integration.
    /// </summary>
    public static class UnityTestApi
    {
        public static async Task<UnityTestListResult> ListAsync(UnityTestQuery query = null)
        {
            Locus.UnityTesting.UnityTestListDto result =
                await Locus.UnityTesting.LocusUnityTestService.ListAsync(
                    ToFilterRequest(query ?? new UnityTestQuery()));
            return ToListResult(result);
        }

        public static UnityTestRunSnapshot Start(UnityTestRunOptions options = null)
        {
            UnityTestRunOptions normalized = options ?? new UnityTestRunOptions();
            Locus.UnityTesting.UnityTestRunRequest request = ToRunRequest(normalized);
            return ToRunSnapshot(Locus.UnityTesting.LocusUnityTestService.Start(request));
        }

        public static UnityTestRunSnapshot Status(string runId = null)
        {
            return ToRunSnapshot(Locus.UnityTesting.LocusUnityTestService.Status(runId));
        }

        public static UnityTestRunSnapshot Cancel(string runId = null)
        {
            return ToRunSnapshot(Locus.UnityTesting.LocusUnityTestService.Cancel(runId));
        }

        public static bool IsTerminalStatus(string status)
        {
            return string.Equals(status, "passed", StringComparison.OrdinalIgnoreCase)
                || string.Equals(status, "failed", StringComparison.OrdinalIgnoreCase)
                || string.Equals(status, "error", StringComparison.OrdinalIgnoreCase)
                || string.Equals(status, "cancelled", StringComparison.OrdinalIgnoreCase);
        }

        /// <summary>Serialize API DTOs for print(...) output in unity_execute.</summary>
        public static string ToJson(object value, bool pretty = true)
        {
            return JsonUtility.ToJson(value, pretty);
        }

        private static Locus.UnityTesting.UnityTestFilterRequest ToFilterRequest(
            UnityTestQuery query)
        {
            return new Locus.UnityTesting.UnityTestFilterRequest
            {
                mode = ModeName(query.Mode),
                assemblies = query.Assemblies,
                tests = query.Tests,
                groups = query.Groups,
                categories = query.Categories,
                max_results = query.MaxResults
            };
        }

        private static Locus.UnityTesting.UnityTestRunRequest ToRunRequest(
            UnityTestRunOptions options)
        {
            return new Locus.UnityTesting.UnityTestRunRequest
            {
                mode = ModeName(options.Mode),
                assemblies = options.Assemblies,
                tests = options.Tests,
                groups = options.Groups,
                categories = options.Categories,
                max_results = options.MaxResults,
                result_detail = ResultDetailName(options.ResultDetail)
            };
        }

        private static UnityTestListResult ToListResult(
            Locus.UnityTesting.UnityTestListDto source)
        {
            List<UnityTestCase> tests = new List<UnityTestCase>();
            if (source != null && source.tests != null)
            {
                foreach (Locus.UnityTesting.UnityTestCaseDto test in source.tests)
                {
                    if (test == null)
                        continue;
                    tests.Add(new UnityTestCase
                    {
                        Name = test.name ?? "",
                        FullName = test.full_name ?? test.name ?? "",
                        Assembly = test.assembly ?? "",
                        Mode = test.mode ?? "",
                        Path = test.path ?? new string[0],
                        Categories = test.categories ?? new string[0]
                    });
                }
            }

            return new UnityTestListResult
            {
                Mode = source != null ? source.mode ?? "" : "",
                Matched = source != null ? source.matched : 0,
                Truncated = source != null && source.truncated,
                Tests = tests.ToArray()
            };
        }

        private static UnityTestRunSnapshot ToRunSnapshot(
            Locus.UnityTesting.UnityTestRunSnapshotDto source)
        {
            if (source == null)
                return new UnityTestRunSnapshot { Status = "idle" };

            return new UnityTestRunSnapshot
            {
                RunId = source.run_id ?? "",
                UnityRunGuid = source.unity_run_guid ?? "",
                Status = source.status ?? "idle",
                Mode = source.mode ?? "",
                CurrentTest = source.current_test ?? "",
                StartedAtTicks = source.started_at_ticks,
                FinishedAtTicks = source.finished_at_ticks,
                DurationMs = source.duration_ms,
                Total = source.total,
                Passed = source.passed,
                Failed = source.failed,
                Skipped = source.skipped,
                Inconclusive = source.inconclusive,
                Error = source.error ?? "",
                Failures = ToTestResults(source.failures),
                Results = ToTestResults(source.results)
            };
        }

        private static UnityTestResult[] ToTestResults(
            IList<Locus.UnityTesting.UnityTestResultDto> source)
        {
            if (source == null)
                return null;

            List<UnityTestResult> results = new List<UnityTestResult>(source.Count);
            foreach (Locus.UnityTesting.UnityTestResultDto result in source)
            {
                if (result == null)
                    continue;
                results.Add(new UnityTestResult
                {
                    FullName = result.full_name ?? "",
                    ResultState = result.result_state ?? "",
                    DurationMs = result.duration_ms,
                    Message = result.message ?? "",
                    StackTrace = result.stack_trace ?? "",
                    Output = result.output ?? ""
                });
            }
            return results.ToArray();
        }

        private static string ModeName(UnityTestMode mode)
        {
            bool includesEdit = (mode & UnityTestMode.Edit) == UnityTestMode.Edit;
            bool includesPlay = (mode & UnityTestMode.Play) == UnityTestMode.Play;
            if (includesEdit && includesPlay)
                return "edit|play";
            if (includesPlay)
                return "play";
            if (includesEdit)
                return "edit";
            throw new ArgumentOutOfRangeException("mode", mode, "Select edit, play, or both modes.");
        }

        private static string ResultDetailName(UnityTestResultDetail detail)
        {
            switch (detail)
            {
                case UnityTestResultDetail.Summary:
                    return "summary";
                case UnityTestResultDetail.All:
                    return "all";
                default:
                    return "failures";
            }
        }
    }
}
