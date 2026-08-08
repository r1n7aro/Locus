using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        private static readonly object ExtensionMessageHandlersLock = new object();
        private static readonly Dictionary<string, Func<string, Task<string>>> ExtensionMessageHandlers =
            new Dictionary<string, Func<string, Task<string>>>(StringComparer.Ordinal);

        /// <summary>
        /// Register an optional package integration without adding its assembly
        /// dependencies to the core Locus bridge. Registrations are rebuilt by
        /// the extension assembly after every domain reload.
        /// </summary>
        public static void RegisterExtensionMessageHandler(
            string messageType,
            Func<string, Task<string>> handler)
        {
            if (string.IsNullOrWhiteSpace(messageType))
                throw new ArgumentException("Extension message type is required.", nameof(messageType));
            if (handler == null)
                throw new ArgumentNullException(nameof(handler));

            lock (ExtensionMessageHandlersLock)
                ExtensionMessageHandlers[messageType.Trim()] = handler;
        }

        private static async Task<PipeEnvelope> HandleExtensionMessageAsync(
            string requestId,
            string messageType,
            string message)
        {
            Func<string, Task<string>> handler;
            lock (ExtensionMessageHandlersLock)
            {
                if (!ExtensionMessageHandlers.TryGetValue(messageType ?? "", out handler))
                    return ErrorResponse(requestId, "unknown message type: " + (messageType ?? ""));
            }

            try
            {
                await LocusAsync.SwitchToMainThread();
                string result = await handler(message ?? "");
                return OkResponse(requestId, result ?? "");
            }
            catch (Exception ex)
            {
                Exception error = ex.InnerException ?? ex;
                return ErrorResponse(requestId, error.Message);
            }
        }
    }
}
