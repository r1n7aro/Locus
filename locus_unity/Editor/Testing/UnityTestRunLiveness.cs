using System;
using System.Collections;
using System.Reflection;
using UnityEditor.TestTools.TestRunner.Api;

namespace Locus.UnityTesting
{
    internal enum UnityTestRunLiveness
    {
        Unknown,
        Running,
        NotRunning
    }

    /// <summary>
    /// Compatibility shim for UTF's exact per-run state. UTF 1.7 exposes an
    /// internal IsRunning(guid) helper, while 1.4 keeps the same information in
    /// its internal TestJobDataHolder. The public API exposes cancellation but
    /// no matching liveness query in these package versions.
    /// </summary>
    internal static class UnityTestRunLivenessProbe
    {
        private const BindingFlags StaticNonPublic = BindingFlags.Static | BindingFlags.NonPublic;
        private const BindingFlags InstancePublic = BindingFlags.Instance | BindingFlags.Public;

        private static readonly MethodInfo IsRunningMethod = typeof(TestRunnerApi).GetMethod(
            "IsRunning",
            StaticNonPublic,
            null,
            new[] { typeof(string) },
            null);

        private static readonly PropertyInfo JobDataHolderProperty = typeof(TestRunnerApi).GetProperty(
            "m_testJobDataHolder",
            StaticNonPublic);

        internal static UnityTestRunLiveness Query(string unityRunGuid)
        {
            if (string.IsNullOrEmpty(unityRunGuid))
                return UnityTestRunLiveness.Unknown;

            try
            {
                if (IsRunningMethod != null)
                {
                    object value = IsRunningMethod.Invoke(null, new object[] { unityRunGuid });
                    if (value is bool)
                        return (bool)value
                            ? UnityTestRunLiveness.Running
                            : UnityTestRunLiveness.NotRunning;
                }

                return QueryJobDataHolder(unityRunGuid);
            }
            catch (Exception)
            {
                return UnityTestRunLiveness.Unknown;
            }
        }

        private static UnityTestRunLiveness QueryJobDataHolder(string unityRunGuid)
        {
            if (JobDataHolderProperty == null)
                return UnityTestRunLiveness.Unknown;

            object holder = JobDataHolderProperty.GetValue(null, null);
            if (holder == null)
                return UnityTestRunLiveness.Unknown;

            FieldInfo testRunsField = holder.GetType().GetField("TestRuns", InstancePublic);
            IEnumerable testRuns = testRunsField != null
                ? testRunsField.GetValue(holder) as IEnumerable
                : null;
            if (testRuns == null)
                return UnityTestRunLiveness.Unknown;

            foreach (object run in testRuns)
            {
                if (run == null)
                    continue;
                Type runType = run.GetType();
                FieldInfo guidField = runType.GetField("guid", InstancePublic);
                FieldInfo runningField = runType.GetField("isRunning", InstancePublic);
                if (guidField == null || runningField == null)
                    return UnityTestRunLiveness.Unknown;

                string guid = guidField.GetValue(run) as string;
                if (!string.Equals(guid, unityRunGuid, StringComparison.Ordinal))
                    continue;

                object running = runningField.GetValue(run);
                return running is bool && (bool)running
                    ? UnityTestRunLiveness.Running
                    : UnityTestRunLiveness.NotRunning;
            }

            return UnityTestRunLiveness.NotRunning;
        }
    }
}
