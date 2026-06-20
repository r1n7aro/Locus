// Locus private async primitives.
//
// Lightweight, allocation-light thread-hopping awaitables modeled on UniTask's
// SwitchToMainThread / SwitchToThreadPool — but deliberately built on Locus's
// OWN main-thread pump (PumpMainThreadQueue) instead of Unity's PlayerLoop.
//
// Why not UniTask itself:
//   * UniTask's editor pump (ForceEditorPlayerLoopUpdate) bails out while the
//     editor is compiling / importing / changing play mode — exactly when the
//     bridge must keep answering commands. Locus's pump drains unconditionally
//     every EditorApplication.update, so these primitives keep working through
//     domain reloads and recompiles.
//   * UniTask installs itself into the global PlayerLoop, which would collide
//     with a user project's own UniTask. These primitives have no global side
//     effects and never touch the PlayerLoop.
//
// Usage inside a request handler (which now runs off the main thread, see
// HandleNativeRequestAsync):
//
//     await LocusAsync.SwitchToMainThread();   // safe to call Unity API
//     var v = SomeUnityEditorCall();
//     await LocusAsync.SwitchToThreadPool();   // get back off the main thread
//     return OkResponse(id, v);
//
// The switch points themselves are allocation-free: the awaiters are structs
// and, when already on the desired thread, complete synchronously without a
// hop. Only the captured continuation (the compiler-generated state machine
// box) costs what a normal `async` method already costs.

using System;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        internal static class LocusAsync
        {
            private static int _mainThreadId = -1;

            /// <summary>
            /// Record the Unity main thread. Must be called once from the main
            /// thread (the static ctor / Start run on it).
            /// </summary>
            internal static void CaptureMainThread()
            {
                _mainThreadId = Thread.CurrentThread.ManagedThreadId;
            }

            internal static bool IsMainThread
            {
                get { return Thread.CurrentThread.ManagedThreadId == _mainThreadId; }
            }

            /// <summary>
            /// Resume the awaiting continuation on the Unity main thread, via the
            /// Locus pump. If already on the main thread, continues inline.
            /// </summary>
            public static SwitchToMainThreadAwaitable SwitchToMainThread()
            {
                return default(SwitchToMainThreadAwaitable);
            }

            /// <summary>
            /// Resume the awaiting continuation on a background thread-pool thread.
            /// If already off the main thread, continues inline (no needless hop).
            /// </summary>
            public static SwitchToThreadPoolAwaitable SwitchToThreadPool()
            {
                return default(SwitchToThreadPoolAwaitable);
            }

            /// <summary>
            /// TaskCompletionSource factory that always schedules continuations
            /// asynchronously. Completing it on the main thread therefore never
            /// inlines the awaiting handler's remainder onto the main thread.
            /// </summary>
            public static TaskCompletionSource<T> CreateTcs<T>()
            {
                return new TaskCompletionSource<T>(TaskCreationOptions.RunContinuationsAsynchronously);
            }

            /// <summary>
            /// Run <paramref name="work"/> on the Unity main thread and return its
            /// result, throwing <see cref="TimeoutException"/> if the main-thread
            /// pump does not run it within <paramref name="timeoutMs"/> (&lt;= 0
            /// means wait forever). Replaces the hand-rolled
            /// TCS + PostToMainThread + WhenAny(Task.Delay) boilerplate and frees
            /// the native active-request slot if the main thread is wedged.
            /// </summary>
            public static async Task<T> RunOnMainThreadAsync<T>(Func<T> work, int timeoutMs)
            {
                var tcs = CreateTcs<T>();
                PostToMainThread(delegate
                {
                    try { tcs.TrySetResult(work()); }
                    catch (Exception ex) { tcs.TrySetException(ex); }
                });

                if (timeoutMs > 0)
                {
                    Task completed = await Task.WhenAny(tcs.Task, Task.Delay(timeoutMs)).ConfigureAwait(false);
                    if (completed != tcs.Task)
                    {
                        ObserveException(tcs.Task);
                        throw new TimeoutException("main-thread work did not complete within " + timeoutMs + "ms");
                    }
                }

                return await tcs.Task.ConfigureAwait(false);
            }

            /// <summary>
            /// Await <paramref name="task"/> but give up after
            /// <paramref name="timeoutMs"/> (&lt;= 0 means wait forever),
            /// throwing <see cref="TimeoutException"/> tagged with
            /// <paramref name="what"/>.
            /// </summary>
            public static async Task<T> WithTimeout<T>(Task<T> task, int timeoutMs, string what)
            {
                if (timeoutMs <= 0)
                    return await task.ConfigureAwait(false);

                Task completed = await Task.WhenAny(task, Task.Delay(timeoutMs)).ConfigureAwait(false);
                if (completed != task)
                {
                    ObserveException(task);
                    throw new TimeoutException((what ?? "operation") + " timed out after " + timeoutMs + "ms");
                }

                return await task.ConfigureAwait(false);
            }

            /// <summary>Non-generic overload of <see cref="WithTimeout{T}"/>.</summary>
            public static async Task WithTimeout(Task task, int timeoutMs, string what)
            {
                if (timeoutMs <= 0)
                {
                    await task.ConfigureAwait(false);
                    return;
                }

                Task completed = await Task.WhenAny(task, Task.Delay(timeoutMs)).ConfigureAwait(false);
                if (completed != task)
                {
                    ObserveException(task);
                    throw new TimeoutException((what ?? "operation") + " timed out after " + timeoutMs + "ms");
                }

                await task.ConfigureAwait(false);
            }

            // Swallow a faulted result we are about to abandon (timeout path), so
            // it never surfaces as an UnobservedTaskException.
            private static void ObserveException(Task task)
            {
                task.ContinueWith(
                    delegate(Task t) { _ = t.Exception; },
                    CancellationToken.None,
                    TaskContinuationOptions.OnlyOnFaulted | TaskContinuationOptions.ExecuteSynchronously,
                    TaskScheduler.Default);
            }
        }

        // ───────────── Allocation-free switch awaiters ─────────────

        internal struct SwitchToMainThreadAwaitable
        {
            public Awaiter GetAwaiter() { return default(Awaiter); }

            public struct Awaiter : ICriticalNotifyCompletion
            {
                // Already on the main thread → run the continuation inline.
                public bool IsCompleted { get { return LocusAsync.IsMainThread; } }
                public void GetResult() { }
                public void OnCompleted(Action continuation) { PostToMainThread(continuation); }
                public void UnsafeOnCompleted(Action continuation) { PostToMainThread(continuation); }
            }
        }

        internal struct SwitchToThreadPoolAwaitable
        {
            public Awaiter GetAwaiter() { return default(Awaiter); }

            public struct Awaiter : ICriticalNotifyCompletion
            {
                private static readonly WaitCallback RunContinuation = state => ((Action)state)();

                // Already off the main thread → continue inline; only hop when on
                // the main thread, so a no-op switch on a worker costs nothing.
                public bool IsCompleted { get { return !LocusAsync.IsMainThread; } }
                public void GetResult() { }

                public void OnCompleted(Action continuation)
                {
                    ThreadPool.QueueUserWorkItem(RunContinuation, continuation);
                }

                public void UnsafeOnCompleted(Action continuation)
                {
                    ThreadPool.UnsafeQueueUserWorkItem(RunContinuation, continuation);
                }
            }
        }
    }
}
