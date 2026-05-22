using UnityEngine;

using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;
using Assembly = System.Reflection.Assembly;

namespace Locus
{
    public static partial class LocusBridge
    {
        private static readonly object _viewScriptCacheLock = new object();
        private static readonly Dictionary<string, CompiledViewScript> _viewScriptCache =
            new Dictionary<string, CompiledViewScript>(StringComparer.Ordinal);
        private static readonly string _viewScriptDomainFingerprint = Guid.NewGuid().ToString("N");
        private static int _viewScriptAssemblyCounter;

        [Serializable]
        private class ViewCompileNamedRequest
        {
            public string viewId;
            public string scriptName;
            public string entryType;
            public string source;
            public string sourceHash;
            public string path;
        }

        [Serializable]
        private class ViewInvokeNamedRequest : ViewCompileNamedRequest
        {
            public string method;
            public string argsJson;
        }

        private sealed class CompiledViewScript
        {
            public string Name;
            public string Hash;
            public string EntryTypeName;
            public string AssemblyId;
            public string Path;
            public Assembly Assembly;
            public Type EntryType;
        }

        private static void InvalidateViewScriptCache()
        {
            lock (_viewScriptCacheLock)
            {
                _viewScriptCache.Clear();
            }
        }

        private static async Task<PipeEnvelope> HandleCompileNamed(string requestId, string message)
        {
            ViewCompileNamedRequest request;
            try
            {
                request = ParseCompileNamedRequest(message);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
            if (!string.IsNullOrEmpty(prepareError))
                return ErrorResponse(requestId, prepareError);

            bool cacheHit;
            CompiledViewScript compiled;
            try
            {
                compiled = CompileOrGetViewScript(request, out cacheHit);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return OkResponse(requestId, BuildCompileNamedResponse(compiled, cacheHit));
        }

        private static async Task<PipeEnvelope> HandleInvokeNamed(string requestId, string message)
        {
            ViewInvokeNamedRequest request;
            try
            {
                request = ParseInvokeNamedRequest(message);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            bool cacheHit;
            CompiledViewScript compiled;
            if (TryGetViewScriptFromCache(request, out compiled))
                return await InvokeViewScriptOnMainThread(requestId, compiled, true, request);

            string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
            if (!string.IsNullOrEmpty(prepareError))
                return ErrorResponse(requestId, prepareError);

            try
            {
                compiled = CompileOrGetViewScript(request, out cacheHit);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return await InvokeViewScriptOnMainThread(requestId, compiled, cacheHit, request);
        }

        private static async Task<PipeEnvelope> HandleInvokeNamedCached(string requestId, string message)
        {
            ViewInvokeNamedRequest request;
            try
            {
                request = ParseInvokeNamedCachedRequest(message);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            CompiledViewScript compiled;
            if (!TryGetViewScriptFromCache(request, out compiled))
            {
                return ErrorResponse(
                    requestId,
                    "compile_required: View Script cache miss for " +
                    (request.scriptName ?? "") +
                    "@" +
                    (request.sourceHash ?? "")
                );
            }

            return await InvokeViewScriptOnMainThread(requestId, compiled, true, request);
        }

        private static async Task<PipeEnvelope> InvokeViewScriptOnMainThread(
            string requestId,
            CompiledViewScript compiled,
            bool cacheHit,
            ViewInvokeNamedRequest request)
        {
            var tcs = new TaskCompletionSource<string>();
            PostToMainThread(delegate
            {
                try
                {
                    object result = InvokeCompiledViewScript(compiled, request);
                    tcs.TrySetResult(BuildInvokeNamedResponse(compiled, cacheHit, request.method, result));
                }
                catch (TargetInvocationException ex)
                {
                    tcs.TrySetException(ex.InnerException ?? ex);
                }
                catch (Exception ex)
                {
                    tcs.TrySetException(ex);
                }
            });

            Task completed = await Task.WhenAny(tcs.Task, Task.Delay(ExecuteTimeoutMs));
            if (completed != tcs.Task)
                return ErrorResponse(requestId, "invoke_named timed out");

            try
            {
                return OkResponse(requestId, tcs.Task.Result);
            }
            catch (AggregateException ex)
            {
                Exception inner = ex.InnerException ?? ex;
                return ErrorResponse(requestId, inner.Message);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }
        }

        private static ViewCompileNamedRequest ParseCompileNamedRequest(string message)
        {
            ViewCompileNamedRequest request = JsonUtility.FromJson<ViewCompileNamedRequest>(message ?? "{}");
            ValidateCompileNamedRequest(request);
            return request;
        }

        private static ViewInvokeNamedRequest ParseInvokeNamedRequest(string message)
        {
            ViewInvokeNamedRequest request = JsonUtility.FromJson<ViewInvokeNamedRequest>(message ?? "{}");
            ValidateCompileNamedRequest(request, true);
            ValidateInvokeNamedRequest(request);
            return request;
        }

        private static ViewInvokeNamedRequest ParseInvokeNamedCachedRequest(string message)
        {
            ViewInvokeNamedRequest request = JsonUtility.FromJson<ViewInvokeNamedRequest>(message ?? "{}");
            ValidateCompileNamedRequest(request, false);
            ValidateInvokeNamedRequest(request);
            return request;
        }

        private static void ValidateInvokeNamedRequest(ViewInvokeNamedRequest request)
        {
            if (string.IsNullOrWhiteSpace(request.method))
                throw new Exception("compile_named request missing method");
        }

        private static void ValidateCompileNamedRequest(ViewCompileNamedRequest request, bool requireSource = true)
        {
            if (request == null)
                throw new Exception("compile_named request is empty");
            if (string.IsNullOrWhiteSpace(request.viewId))
                throw new Exception("compile_named request missing viewId");
            if (string.IsNullOrWhiteSpace(request.scriptName))
                throw new Exception("compile_named request missing scriptName");
            if (string.IsNullOrWhiteSpace(request.entryType))
                throw new Exception("compile_named request missing entryType");
            if (string.IsNullOrWhiteSpace(request.sourceHash))
                throw new Exception("compile_named request missing sourceHash");
            if (requireSource && string.IsNullOrWhiteSpace(request.source))
                throw new Exception("compile_named request missing source");
            if (string.IsNullOrWhiteSpace(request.path))
                request.path = "ViewScript.cs";
        }

        private static CompiledViewScript CompileOrGetViewScript(
            ViewCompileNamedRequest request,
            out bool cacheHit)
        {
            string cacheKey = BuildViewScriptCacheKey(request);

            lock (_viewScriptCacheLock)
            {
                CompiledViewScript cached;
                if (_viewScriptCache.TryGetValue(cacheKey, out cached))
                {
                    cacheHit = true;
                    return cached;
                }

                CompiledViewScript compiled = CompileViewScript(request);
                _viewScriptCache[cacheKey] = compiled;
                cacheHit = false;
                return compiled;
            }
        }

        private static bool TryGetViewScriptFromCache(
            ViewCompileNamedRequest request,
            out CompiledViewScript compiled)
        {
            string cacheKey = BuildViewScriptCacheKey(request);
            lock (_viewScriptCacheLock)
            {
                return _viewScriptCache.TryGetValue(cacheKey, out compiled);
            }
        }

        private static string BuildViewScriptCacheKey(ViewCompileNamedRequest request)
        {
            return (request.viewId ?? "") + "|" +
                   (request.scriptName ?? "") + "|" +
                   (request.entryType ?? "") + "|" +
                   (request.sourceHash ?? "") + "|" +
                   _viewScriptDomainFingerprint;
        }

        private static CompiledViewScript CompileViewScript(ViewCompileNamedRequest request)
        {
            SyntaxTree syntaxTree;
            try
            {
                syntaxTree = CSharpSyntaxTree.ParseText(
                    request.source,
                    SnippetParseOptions,
                    path: request.path,
                    encoding: Utf8NoBom
                );
            }
            catch (Exception ex)
            {
                throw new Exception("parse failed: " + ex.Message);
            }

            string assemblyId =
                "__LocusView_" +
                SanitizeAssemblyNamePart(request.scriptName) +
                "_" +
                Interlocked.Increment(ref _viewScriptAssemblyCounter).ToString("X8");

            CSharpCompilation compilation = CSharpCompilation.Create(
                assemblyName: assemblyId,
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
                    throw new Exception("emit failed: " + ex.Message);
                }

                if (!emitResult.Success)
                    throw new Exception(BuildViewScriptDiagnosticErrorText(emitResult.Diagnostics));

                try
                {
                    Assembly assembly = Assembly.Load(peStream.ToArray());
                    Type entryType = ResolveEntryType(assembly, request.entryType);
                    return new CompiledViewScript
                    {
                        Name = request.scriptName,
                        Hash = request.sourceHash,
                        EntryTypeName = request.entryType,
                        AssemblyId = assemblyId,
                        Path = request.path,
                        Assembly = assembly,
                        EntryType = entryType
                    };
                }
                catch (Exception ex)
                {
                    throw new Exception("assembly load/bootstrap failed: " + ex.Message);
                }
            }
        }

        private static Type ResolveEntryType(Assembly assembly, string entryTypeName)
        {
            Type type = assembly.GetType(entryTypeName, false);
            if (type != null)
                return type;

            type = assembly
                .GetTypes()
                .FirstOrDefault(candidate =>
                    string.Equals(candidate.FullName, entryTypeName, StringComparison.Ordinal) ||
                    string.Equals(candidate.Name, entryTypeName, StringComparison.Ordinal));

            if (type == null)
                throw new Exception("compiled View Script entryType not found: " + entryTypeName);

            return type;
        }

        private static object InvokeCompiledViewScript(
            CompiledViewScript compiled,
            ViewInvokeNamedRequest request)
        {
            MethodInfo method = compiled.EntryType.GetMethod(
                request.method,
                BindingFlags.Public | BindingFlags.Static
            );
            if (method == null)
                throw new Exception("View Script method not found: " + request.method);

            ParameterInfo[] parameters = method.GetParameters();
            object[] args;
            if (parameters.Length == 0)
            {
                args = null;
            }
            else if (parameters.Length == 1)
            {
                args = new[] { ConvertViewScriptArgument(parameters[0].ParameterType, request.argsJson) };
            }
            else
            {
                throw new Exception("View Script methods may accept zero parameters or one JSON argument");
            }

            return method.Invoke(null, args);
        }

        private static object ConvertViewScriptArgument(Type parameterType, string argsJson)
        {
            if (parameterType == typeof(string) || parameterType == typeof(object))
                return argsJson ?? "{}";

            string json = string.IsNullOrWhiteSpace(argsJson) ? "{}" : argsJson;
            try
            {
                return JsonUtility.FromJson(json, parameterType);
            }
            catch (Exception ex)
            {
                throw new Exception("failed to parse View Script args as " + parameterType.FullName + ": " + ex.Message);
            }
        }

        private static string BuildCompileNamedResponse(CompiledViewScript compiled, bool cacheHit)
        {
            return "{" +
                   "\"name\":\"" + JsonEscape(compiled.Name) + "\"," +
                   "\"hash\":\"" + JsonEscape(compiled.Hash) + "\"," +
                   "\"cacheHit\":" + (cacheHit ? "true" : "false") + "," +
                   "\"assemblyId\":\"" + JsonEscape(compiled.AssemblyId) + "\"," +
                   "\"domainFingerprint\":\"" + JsonEscape(_viewScriptDomainFingerprint) + "\"," +
                   "\"path\":\"" + JsonEscape(compiled.Path) + "\"" +
                   "}";
        }

        private static string BuildInvokeNamedResponse(
            CompiledViewScript compiled,
            bool cacheHit,
            string method,
            object result)
        {
            return "{" +
                   "\"compile\":" + BuildCompileNamedResponse(compiled, cacheHit) + "," +
                   "\"method\":\"" + JsonEscape(method) + "\"," +
                   "\"result\":" + ToJsonValue(result, 0) +
                   "}";
        }

        private static string BuildViewScriptDiagnosticErrorText(IEnumerable<Diagnostic> diagnostics)
        {
            if (diagnostics == null)
                return "compilation failed";

            var sb = new StringBuilder();
            bool hasError = false;

            foreach (Diagnostic diagnostic in diagnostics)
            {
                if (diagnostic == null || diagnostic.Severity != DiagnosticSeverity.Error)
                    continue;

                if (!hasError)
                {
                    hasError = true;
                    sb.Append("compilation failed:\n");
                }

                FileLinePositionSpan span = diagnostic.Location.GetMappedLineSpan();
                sb.Append("  ");
                sb.Append(diagnostic.Id);
                sb.Append(" at ");
                sb.Append(string.IsNullOrEmpty(span.Path) ? "ViewScript.cs" : span.Path.Replace('\\', '/'));
                sb.Append(":");
                sb.Append(span.StartLinePosition.Line + 1);
                sb.Append(":");
                sb.Append(span.StartLinePosition.Character + 1);
                sb.Append(": ");
                sb.Append(diagnostic.GetMessage());
                sb.Append("\n");
            }

            return hasError ? sb.ToString() : "compilation failed";
        }

        private static string SanitizeAssemblyNamePart(string value)
        {
            if (string.IsNullOrEmpty(value))
                return "Script";

            var sb = new StringBuilder(value.Length);
            for (int i = 0; i < value.Length; i++)
            {
                char ch = value[i];
                sb.Append(char.IsLetterOrDigit(ch) ? ch : '_');
            }
            return sb.Length == 0 ? "Script" : sb.ToString();
        }

        private static string ToJsonValue(object value, int depth)
        {
            if (value == null)
                return "null";
            if (depth > 5)
                return "\"...\"";

            string stringValue = value as string;
            if (stringValue != null)
                return "\"" + JsonEscape(stringValue) + "\"";

            if (value is bool)
                return ((bool)value) ? "true" : "false";

            if (IsJsonNumber(value))
            {
                string number = Convert.ToString(value, CultureInfo.InvariantCulture);
                if (string.IsNullOrEmpty(number) ||
                    string.Equals(number, "NaN", StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(number, "Infinity", StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(number, "-Infinity", StringComparison.OrdinalIgnoreCase))
                    return "null";
                return number;
            }

            UnityEngine.Object unityObject = value as UnityEngine.Object;
            if (unityObject != null)
            {
                return "{" +
                       "\"name\":\"" + JsonEscape(unityObject.name) + "\"," +
                       "\"type\":\"" + JsonEscape(unityObject.GetType().FullName) + "\"" +
                       "}";
            }

            IDictionary dictionary = value as IDictionary;
            if (dictionary != null)
                return DictionaryToJson(dictionary, depth + 1);

            IEnumerable enumerable = value as IEnumerable;
            if (enumerable != null)
                return EnumerableToJson(enumerable, depth + 1);

            return ObjectToJson(value, depth + 1);
        }

        private static bool IsJsonNumber(object value)
        {
            return value is byte ||
                   value is sbyte ||
                   value is short ||
                   value is ushort ||
                   value is int ||
                   value is uint ||
                   value is long ||
                   value is ulong ||
                   value is float ||
                   value is double ||
                   value is decimal;
        }

        private static string DictionaryToJson(IDictionary dictionary, int depth)
        {
            var sb = new StringBuilder();
            sb.Append("{");
            bool first = true;
            foreach (DictionaryEntry entry in dictionary)
            {
                if (!first)
                    sb.Append(",");
                first = false;
                sb.Append("\"");
                sb.Append(JsonEscape(Convert.ToString(entry.Key, CultureInfo.InvariantCulture)));
                sb.Append("\":");
                sb.Append(ToJsonValue(entry.Value, depth));
            }
            sb.Append("}");
            return sb.ToString();
        }

        private static string EnumerableToJson(IEnumerable enumerable, int depth)
        {
            var sb = new StringBuilder();
            sb.Append("[");
            bool first = true;
            foreach (object item in enumerable)
            {
                if (!first)
                    sb.Append(",");
                first = false;
                sb.Append(ToJsonValue(item, depth));
            }
            sb.Append("]");
            return sb.ToString();
        }

        private static string ObjectToJson(object value, int depth)
        {
            var sb = new StringBuilder();
            sb.Append("{");
            bool first = true;
            Type type = value.GetType();

            foreach (FieldInfo field in type.GetFields(BindingFlags.Public | BindingFlags.Instance))
            {
                if (!first)
                    sb.Append(",");
                first = false;
                sb.Append("\"");
                sb.Append(JsonEscape(field.Name));
                sb.Append("\":");
                sb.Append(ToJsonValue(field.GetValue(value), depth));
            }

            foreach (PropertyInfo property in type.GetProperties(BindingFlags.Public | BindingFlags.Instance))
            {
                if (!property.CanRead || property.GetIndexParameters().Length > 0)
                    continue;

                if (!first)
                    sb.Append(",");
                first = false;
                sb.Append("\"");
                sb.Append(JsonEscape(property.Name));
                sb.Append("\":");
                object propertyValue;
                try
                {
                    propertyValue = property.GetValue(value, null);
                }
                catch
                {
                    propertyValue = null;
                }
                sb.Append(ToJsonValue(propertyValue, depth));
            }

            if (first)
            {
                sb.Append("\"value\":\"");
                sb.Append(JsonEscape(value.ToString()));
                sb.Append("\"");
            }

            sb.Append("}");
            return sb.ToString();
        }

        private static string JsonEscape(string value)
        {
            if (string.IsNullOrEmpty(value))
                return "";

            var sb = new StringBuilder(value.Length + 8);
            for (int i = 0; i < value.Length; i++)
            {
                char ch = value[i];
                switch (ch)
                {
                    case '\\': sb.Append("\\\\"); break;
                    case '"': sb.Append("\\\""); break;
                    case '\b': sb.Append("\\b"); break;
                    case '\f': sb.Append("\\f"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (char.IsControl(ch))
                            sb.Append("\\u").Append(((int)ch).ToString("x4"));
                        else
                            sb.Append(ch);
                        break;
                }
            }
            return sb.ToString();
        }
    }
}
