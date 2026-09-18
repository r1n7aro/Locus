using System;
using System.Collections.Generic;
using System.Reflection;
using UnityEditorInternal;
using UnityEditor;
using UnityEngine;
using UnityEngine.Profiling;

namespace Locus
{
    public static partial class LocusBridge
    {
        public sealed class ProfilerCaptureOptions
        {
            public int WarmupFrames = 0;
            public int SampleEveryFrames = 1;
            public int MaxSamples = 3600;
            // Auto uses distinct Time.frameCount in Play Mode, editor ticks in Edit Mode.
            public string Clock = "auto";
            public bool CaptureHierarchy = false;
            public bool CaptureGpu = false;

            internal ProfilerCaptureOptions CopyValidated()
            {
                if (WarmupFrames < 0 || WarmupFrames > 60000) throw new ArgumentOutOfRangeException(nameof(WarmupFrames));
                if (SampleEveryFrames < 1 || SampleEveryFrames > 60000) throw new ArgumentOutOfRangeException(nameof(SampleEveryFrames));
                if (MaxSamples < 1 || MaxSamples > 60000) throw new ArgumentOutOfRangeException(nameof(MaxSamples));
                if (Clock != "auto" && Clock != "unity_frame" && Clock != "editor_update")
                    throw new ArgumentException("Clock must be auto, unity_frame, or editor_update.");
                return (ProfilerCaptureOptions)MemberwiseClone();
            }
        }

        // Multiple overlapping captures share one lease; never stop a pre-existing user capture.
        private static int _profilerRecordingLeases;
        private static bool _profilerRecordingWasEnabled;
        private static bool _profilerCpuWasEnabled, _profilerEditorWasEnabled;
        private static int _profilerGpuLeases;
        private static bool _profilerGpuWasEnabled;
        private static int _profilerRecordingTarget, _profilerGpuTarget;
        private static readonly Func<bool> _isProfilerEditorTarget = ResolveProfilerEditorTargetGetter();

        private static Func<bool> ResolveProfilerEditorTargetGetter()
        {
            // Unity exposes the target test internally; bind once and fail explicitly if it changes.
            try
            {
                var method = typeof(ProfilerDriver).GetMethod("IsConnectionEditor", BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic,
                    null, Type.EmptyTypes, null);
                return method == null || method.ReturnType != typeof(bool) ? null : (Func<bool>)Delegate.CreateDelegate(typeof(Func<bool>), method);
            }
            catch { return null; }
        }

        private static void RequireProfilerEditorTarget()
        {
            if (_isProfilerEditorTarget == null)
                throw new NotSupportedException("This Unity version does not expose a supported Profiler target check.");
            if (!_isProfilerEditorTarget())
                throw new InvalidOperationException("Select the local Editor target in the Unity Profiler before capturing frames or memory snapshots. Recorder metrics observe the local Editor.");
        }
        private sealed class ProfilerRecordingLease : IDisposable
        {
            private bool _disposed;
            private readonly bool _gpu;
            internal ProfilerRecordingLease(bool gpu = false)
            {
                RequireProfilerEditorTarget();
                _gpu = gpu;
                if (_profilerRecordingLeases == 0)
                {
                    _profilerRecordingWasEnabled = ProfilerDriver.enabled;
                    _profilerRecordingTarget = ProfilerDriver.connectedProfiler;
                    _profilerCpuWasEnabled = ProfilerDriver.IsAreaEnabled(ProfilerArea.CPU);
                    _profilerEditorWasEnabled = ProfilerDriver.profileEditor;
                    if (!_profilerCpuWasEnabled) ProfilerDriver.SetAreaEnabled(ProfilerArea.CPU, true);
                    if (!EditorApplication.isPlaying && !_profilerEditorWasEnabled) ProfilerDriver.profileEditor = true;
                    if (!_profilerRecordingWasEnabled) ProfilerDriver.enabled = true;
                }
                _profilerRecordingLeases++;
                if (_gpu && _profilerGpuLeases++ == 0)
                {
                    _profilerGpuWasEnabled = ProfilerDriver.profileGPU;
                    _profilerGpuTarget = ProfilerDriver.connectedProfiler;
                    if (!_profilerGpuWasEnabled) ProfilerDriver.profileGPU = true;
                }
            }
            public void Dispose()
            {
                if (_disposed) return;
                _disposed = true;
                if (_gpu && --_profilerGpuLeases == 0 && !_profilerGpuWasEnabled && ProfilerDriver.connectedProfiler == _profilerGpuTarget)
                    ProfilerDriver.profileGPU = false;
                // Do not apply the old target's settings to a target the user selected during capture.
                if (--_profilerRecordingLeases == 0 && ProfilerDriver.connectedProfiler == _profilerRecordingTarget)
                {
                    if (!_profilerRecordingWasEnabled && ProfilerDriver.enabled) ProfilerDriver.enabled = false;
                    if (!_profilerCpuWasEnabled) ProfilerDriver.SetAreaEnabled(ProfilerArea.CPU, false);
                    if (!_profilerEditorWasEnabled) ProfilerDriver.profileEditor = false;
                }
            }
        }

        public sealed class ProfilerObservation
        {
            public int SessionFrame, UnityTimeFrameCount, ProfilerFrameIndex;
            public long ElapsedMs;
            public double? Value;
        }

        public sealed class ProfilerComparison
        {
            public string Metric, Unit, Error;
            public int BaselineSamples, CandidateSamples;
            public double? AverageDelta, AveragePercent, P95Delta, P95Percent, MaxDelta;
        }

        public sealed class ProfilerBudget
        {
            public string Metric, Unit;
            public double Threshold;
            public int SampleCount, ExceededCount;
            public double? ExceededPercent;
        }
    }
}
