using System;

namespace Locus
{
    public static partial class LocusBridge
    {
        public sealed partial class ExecuteCodeContext
        {
            private ProfilerApi _profiler;
            private Action<object> _profilerOutput;
            private bool _profilerDisposed;

            public ProfilerApi Profiler
            {
                get
                {
                    ThrowIfCancellationRequested();
                    if (_profilerDisposed) throw new ObjectDisposedException(nameof(ExecuteCodeContext));
                    return _profiler ?? (_profiler = new ProfilerApi(
                        () => _executeAsyncEditorUpdateTick, _profilerOutput, "unity_execute_editor_update", true, CancellationToken));
                }
            }

            internal void InitializeProfilerOutput(Action<object> output) { _profilerOutput = output; }
            internal void DisposeProfiler()
            {
                _profilerDisposed = true;
                _profiler?.Dispose();
            }
        }
    }
}
