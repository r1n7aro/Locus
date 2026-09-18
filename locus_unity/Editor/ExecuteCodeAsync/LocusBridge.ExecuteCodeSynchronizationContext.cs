using System;
using System.Runtime.ExceptionServices;
using System.Threading;

namespace Locus
{
    public static partial class LocusBridge
    {
        // Installed only while invoking a Locus snippet or one of its continuations.
        // Never pump UnitySynchronizationContext: its queue also owns gameplay tasks.
        private sealed class ExecuteCodeSynchronizationContext : SynchronizationContext
        {
            public override SynchronizationContext CreateCopy() { return this; }

            public override void Post(SendOrPostCallback callback, object state)
            {
                if (callback == null) throw new ArgumentNullException(nameof(callback));
                PostToMainThread(() => Run(() => callback(state)));
            }

            public override void Send(SendOrPostCallback callback, object state)
            {
                if (callback == null) throw new ArgumentNullException(nameof(callback));
                if (LocusAsync.IsMainThread)
                {
                    Run(() => callback(state));
                    return;
                }

                ExceptionDispatchInfo error = null;
                using (var completed = new ManualResetEventSlim())
                {
                    PostToMainThread(() =>
                    {
                        try { Run(() => callback(state)); }
                        catch (Exception exception) { error = ExceptionDispatchInfo.Capture(exception); }
                        finally { completed.Set(); }
                    });
                    completed.Wait();
                }
                if (error != null) error.Throw();
            }

            internal void Run(Action continuation)
            {
                SynchronizationContext previous = Current;
                try
                {
                    SetSynchronizationContext(this);
                    continuation();
                }
                finally { SetSynchronizationContext(previous); }
            }
        }
    }
}
