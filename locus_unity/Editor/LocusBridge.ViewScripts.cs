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
        private static readonly object _skillPackageAssemblyCacheLock = new object();
        private static readonly Dictionary<string, CompiledSkillPackageAssembly> _skillPackageAssemblyCache =
            new Dictionary<string, CompiledSkillPackageAssembly>(StringComparer.Ordinal);
        private static readonly object _skillPackageAssemblyRegistryLock = new object();
        private static readonly Dictionary<string, string> _activeSkillPackageAssemblyByPackage =
            new Dictionary<string, string>(StringComparer.Ordinal);
        private static readonly HashSet<string> _activeSkillPackageAssemblyIds =
            new HashSet<string>(StringComparer.Ordinal);
        private const string SkillPackageAssemblyPrefix = "__LocusSkillPackage_";
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

            // Optional: assembly pre-compiled by the Locus compile-server
            // sidecar. When present, a cache miss loads these bytes instead
            // of compiling `source`; on any load failure the local compile
            // path still runs. Older plugin builds ignore these fields.
            public string assembly_b64;
            public string assembly_path;
            public string assembly_id;
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

        [Serializable]
        private sealed class SkillPackageCompileRequest
        {
            public string packageId;
            public string sourceHash;
            public SkillPackageScriptSource[] scripts;

            // Optional: assembly pre-compiled by the Locus compile-server
            // sidecar. Current desktop builds include these fields; older
            // builds omit them and take the local compile fallback.
            public string assembly_b64;
            public string assembly_path;
            public string assembly_id;
        }

        [Serializable]
        private sealed class SkillPackageInvokeRequest
        {
            public string packageId;
            public string assemblyId;
            public string typeName;
            public string entryType;
            public string method;
            public string argsJson;
        }

        [Serializable]
        private sealed class SkillPackageScriptSource
        {
            public string path;
            public string source;
        }

        private sealed class CompiledSkillPackageAssembly
        {
            public string PackageId;
            public string Hash;
            public string AssemblyId;
            public string PreviousAssemblyId;
            public string AssemblyPath;
            public int ScriptCount;
            public int PublicTypeCount;
            public string PreviousTypeIndexFingerprint;
            public string TypeIndexFingerprint;
            public TypeIndexEntry[] PublicTypes;
            public Assembly Assembly;
        }

        private sealed class SkillPackageAssemblyCompileResult
        {
            public CompiledSkillPackageAssembly Compiled;
            public bool CacheHit;
        }

        private static void InvalidateViewScriptCache()
        {
            lock (_viewScriptCacheLock)
            {
                _viewScriptCache.Clear();
            }
        }

        private static void InvalidateSkillPackageAssemblyCache()
        {
            lock (_skillPackageAssemblyCacheLock)
            {
                _skillPackageAssemblyCache.Clear();
            }
            lock (_skillPackageAssemblyRegistryLock)
            {
                _activeSkillPackageAssemblyByPackage.Clear();
                _activeSkillPackageAssemblyIds.Clear();
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

        private static async Task<PipeEnvelope> HandleCompileSkillPackage(string requestId, string message)
        {
            SkillPackageCompileRequest request;
            try
            {
                request = ParseSkillPackageCompileRequest(message);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            SkillPackageAssemblyCompileResult result;
            try
            {
                result = await CompileOrGetSkillPackageAssemblyAsync(request);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            return OkResponse(requestId, BuildCompileSkillPackageResponse(result.Compiled, result.CacheHit));
        }

        private static async Task<PipeEnvelope> HandleInvokeSkillPackage(string requestId, string message)
        {
            SkillPackageInvokeRequest request;
            try
            {
                request = ParseSkillPackageInvokeRequest(message);
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, ex.Message);
            }

            var tcs = new TaskCompletionSource<string>();
            PostToMainThread(delegate
            {
                try
                {
                    object result = InvokeCompiledSkillPackageMethod(request);
                    tcs.TrySetResult(BuildInvokeSkillPackageResponse(request, result));
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
                return ErrorResponse(requestId, "invoke_skill_package timed out");

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

        private static SkillPackageCompileRequest ParseSkillPackageCompileRequest(string message)
        {
            SkillPackageCompileRequest request = JsonUtility.FromJson<SkillPackageCompileRequest>(message ?? "{}");
            ValidateSkillPackageCompileRequest(request);
            return request;
        }

        private static SkillPackageInvokeRequest ParseSkillPackageInvokeRequest(string message)
        {
            SkillPackageInvokeRequest request = JsonUtility.FromJson<SkillPackageInvokeRequest>(message ?? "{}");
            ValidateSkillPackageInvokeRequest(request);
            return request;
        }

        private static void ValidateSkillPackageCompileRequest(SkillPackageCompileRequest request)
        {
            if (request == null)
                throw new Exception("compile_skill_package request is empty");
            if (string.IsNullOrWhiteSpace(request.packageId))
                throw new Exception("compile_skill_package request missing packageId");
            if (string.IsNullOrWhiteSpace(request.sourceHash))
                throw new Exception("compile_skill_package request missing sourceHash");
            if (request.scripts == null || request.scripts.Length == 0)
                throw new Exception("compile_skill_package request has no scripts");

            for (int i = 0; i < request.scripts.Length; i++)
            {
                SkillPackageScriptSource script = request.scripts[i];
                if (script == null)
                    throw new Exception("compile_skill_package script entry is empty");
                if (string.IsNullOrWhiteSpace(script.path))
                    throw new Exception("compile_skill_package script missing path");
                if (script.source == null)
                    throw new Exception("compile_skill_package script missing source: " + script.path);
            }
        }

        private static void ValidateSkillPackageInvokeRequest(SkillPackageInvokeRequest request)
        {
            if (request == null)
                throw new Exception("invoke_skill_package request is empty");
            if (string.IsNullOrWhiteSpace(request.packageId))
                throw new Exception("invoke_skill_package request missing packageId");
            if (string.IsNullOrWhiteSpace(EffectiveSkillPackageInvokeTypeName(request)))
                throw new Exception("invoke_skill_package request missing typeName");
            if (string.IsNullOrWhiteSpace(request.method))
                throw new Exception("invoke_skill_package request missing method");
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

                CompiledViewScript compiled = TryLoadPrecompiledViewScript(request);
                if (compiled == null)
                    compiled = CompileViewScript(request);
                _viewScriptCache[cacheKey] = compiled;
                cacheHit = false;
                return compiled;
            }
        }

        /// <summary>
        /// Load a View Script assembly pre-compiled by the compile-server
        /// sidecar (optional assembly path/base64 on the request). Returns
        /// null — falling back to the local compile — when the request has no
        /// bytes or the load fails for any reason; the source is always
        /// present.
        /// </summary>
        private static CompiledViewScript TryLoadPrecompiledViewScript(ViewCompileNamedRequest request)
        {
            if (string.IsNullOrEmpty(request.assembly_b64) &&
                string.IsNullOrEmpty(request.assembly_path))
                return null;

            try
            {
                byte[] assemblyBytes = ReadAssemblyPayload(request.assembly_b64, request.assembly_path);
                Assembly assembly = Assembly.Load(assemblyBytes);
                Type entryType = ResolveEntryType(assembly, request.entryType);
                string assemblyId = string.IsNullOrEmpty(request.assembly_id)
                    ? SafeAssemblyName(assembly)
                    : request.assembly_id;

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
                Debug.LogWarning(
                    "[Locus] precompiled View Script load failed; compiling in Unity instead: " +
                    ex.Message);
                return null;
            }
        }

        private static bool HasPrecompiledSkillPackageAssemblyPayload(SkillPackageCompileRequest request)
        {
            return request != null &&
                   (!string.IsNullOrEmpty(request.assembly_b64) ||
                    !string.IsNullOrEmpty(request.assembly_path));
        }

        private static async Task<SkillPackageAssemblyCompileResult> CompileOrGetSkillPackageAssemblyAsync(
            SkillPackageCompileRequest request)
        {
            string cacheKey = BuildSkillPackageAssemblyCacheKey(request);

            lock (_skillPackageAssemblyCacheLock)
            {
                CompiledSkillPackageAssembly cached;
                if (_skillPackageAssemblyCache.TryGetValue(cacheKey, out cached))
                {
                    ActivateSkillPackageAssembly(cached);
                    return new SkillPackageAssemblyCompileResult
                    {
                        Compiled = cached,
                        CacheHit = true
                    };
                }
            }

            CompiledSkillPackageAssembly precompiled = TryLoadPrecompiledSkillPackageAssembly(request);
            if (precompiled != null)
            {
                lock (_skillPackageAssemblyCacheLock)
                {
                    CompiledSkillPackageAssembly cached;
                    if (_skillPackageAssemblyCache.TryGetValue(cacheKey, out cached))
                    {
                        ActivateSkillPackageAssembly(cached);
                        return new SkillPackageAssemblyCompileResult
                        {
                            Compiled = cached,
                            CacheHit = true
                        };
                    }

                    _skillPackageAssemblyCache[cacheKey] = precompiled;
                }
                ActivateSkillPackageAssembly(precompiled);
                return new SkillPackageAssemblyCompileResult
                {
                    Compiled = precompiled,
                    CacheHit = false
                };
            }

            string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
            if (!string.IsNullOrEmpty(prepareError))
                throw new Exception(prepareError);

            lock (_skillPackageAssemblyCacheLock)
            {
                CompiledSkillPackageAssembly cached;
                if (_skillPackageAssemblyCache.TryGetValue(cacheKey, out cached))
                {
                    ActivateSkillPackageAssembly(cached);
                    return new SkillPackageAssemblyCompileResult
                    {
                        Compiled = cached,
                        CacheHit = true
                    };
                }

                CompiledSkillPackageAssembly compiled = CompileSkillPackageAssembly(request);
                _skillPackageAssemblyCache[cacheKey] = compiled;
                return new SkillPackageAssemblyCompileResult
                {
                    Compiled = compiled,
                    CacheHit = false
                };
            }
        }

        private static CompiledSkillPackageAssembly TryLoadPrecompiledSkillPackageAssembly(
            SkillPackageCompileRequest request)
        {
            if (!HasPrecompiledSkillPackageAssemblyPayload(request))
                return null;

            try
            {
                string assemblyId = string.IsNullOrEmpty(request.assembly_id)
                    ? BuildSkillPackageAssemblyId(request)
                    : request.assembly_id;
                string assemblyPath = SkillPackageAssemblyPath(assemblyId);
                Assembly assembly = FindLoadedAssemblyByName(assemblyId);
                if (assembly == null)
                {
                    byte[] assemblyBytes = ReadAssemblyPayload(request.assembly_b64, request.assembly_path);
                    Directory.CreateDirectory(Path.GetDirectoryName(assemblyPath));
                    File.WriteAllBytes(assemblyPath, assemblyBytes);
                    assembly = Assembly.LoadFile(assemblyPath);
                }
                else
                {
                    string loadedPath = SafeAssemblyLocation(assembly);
                    if (!string.IsNullOrEmpty(loadedPath))
                        assemblyPath = loadedPath;
                }

                string loadedName = SafeAssemblyName(assembly);
                if (!string.Equals(loadedName, assemblyId, StringComparison.Ordinal))
                    throw new Exception("assembly name mismatch: expected " + assemblyId + ", got " + loadedName);

                Type[] assemblyTypes = SafeGetAssemblyTypes(assembly) ?? new Type[0];
                int publicTypeCount = assemblyTypes
                    .Count(type => type != null && type.IsPublic && !type.IsNested);

                return new CompiledSkillPackageAssembly
                {
                    PackageId = request.packageId,
                    Hash = request.sourceHash,
                    AssemblyId = assemblyId,
                    AssemblyPath = assemblyPath.Replace('\\', '/'),
                    ScriptCount = request.scripts != null ? request.scripts.Length : 0,
                    PublicTypeCount = publicTypeCount,
                    PublicTypes = BuildTypeIndexEntriesForAssembly(assembly, assemblyId),
                    Assembly = assembly
                };
            }
            catch (Exception ex)
            {
                Debug.LogWarning(
                    "[Locus] precompiled Skill Package load failed; compiling in Unity instead: " +
                    ex.Message);
                return null;
            }
        }

        private static void ActivateSkillPackageAssembly(CompiledSkillPackageAssembly compiled)
        {
            if (compiled == null)
                return;

            string previousFingerprint = ComputeTypeIndexFingerprint();
            string previousAssemblyId;
            bool changed = RegisterActiveSkillPackageAssembly(
                compiled.PackageId,
                compiled.AssemblyId,
                out previousAssemblyId);
            string currentFingerprint = ComputeTypeIndexFingerprint();

            compiled.PreviousAssemblyId = previousAssemblyId ?? "";
            compiled.PreviousTypeIndexFingerprint = previousFingerprint;
            compiled.TypeIndexFingerprint = currentFingerprint;

            if (changed)
                InvalidateExecuteCodeMetadataReferences();
        }

        private static bool RegisterActiveSkillPackageAssembly(
            string packageId,
            string assemblyId,
            out string previousAssemblyId)
        {
            previousAssemblyId = "";
            if (string.IsNullOrWhiteSpace(packageId) || string.IsNullOrWhiteSpace(assemblyId))
                return false;

            lock (_skillPackageAssemblyRegistryLock)
            {
                string previous = "";
                if (_activeSkillPackageAssemblyByPackage.TryGetValue(packageId, out previous))
                {
                    if (string.Equals(previous, assemblyId, StringComparison.Ordinal))
                    {
                        _activeSkillPackageAssemblyIds.Add(assemblyId);
                        return false;
                    }

                    previousAssemblyId = previous ?? "";
                }

                if (!string.IsNullOrEmpty(previous))
                    _activeSkillPackageAssemblyIds.Remove(previous);

                _activeSkillPackageAssemblyByPackage[packageId] = assemblyId;
                _activeSkillPackageAssemblyIds.Add(assemblyId);
                return true;
            }
        }

        private static bool IsInactiveSkillPackageAssemblyName(string assemblyName)
        {
            if (string.IsNullOrEmpty(assemblyName) ||
                !assemblyName.StartsWith(SkillPackageAssemblyPrefix, StringComparison.Ordinal))
                return false;

            lock (_skillPackageAssemblyRegistryLock)
            {
                return !_activeSkillPackageAssemblyIds.Contains(assemblyName);
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

        private static string BuildSkillPackageAssemblyCacheKey(SkillPackageCompileRequest request)
        {
            return (request.packageId ?? "") + "|" +
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

        private static CompiledSkillPackageAssembly CompileSkillPackageAssembly(SkillPackageCompileRequest request)
        {
            SyntaxTree[] syntaxTrees = new SyntaxTree[request.scripts.Length];
            try
            {
                for (int i = 0; i < request.scripts.Length; i++)
                {
                    SkillPackageScriptSource script = request.scripts[i];
                    syntaxTrees[i] = CSharpSyntaxTree.ParseText(
                        script.source ?? "",
                        SnippetParseOptions,
                        path: string.IsNullOrWhiteSpace(script.path) ? "SkillPackageScript.cs" : script.path.Replace('\\', '/'),
                        encoding: Utf8NoBom
                    );
                }
            }
            catch (Exception ex)
            {
                throw new Exception("parse failed: " + ex.Message);
            }

            string assemblyId = BuildSkillPackageAssemblyId(request);

            CSharpCompilation compilation = CSharpCompilation.Create(
                assemblyName: assemblyId,
                syntaxTrees: syntaxTrees,
                references: EnsureMetadataReferences(),
                options: SnippetCompilationOptions
            );

            using (var peStream = new MemoryStream(64 * 1024))
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
                    string assemblyPath = SkillPackageAssemblyPath(assemblyId);
                    Assembly assembly = FindLoadedAssemblyByName(assemblyId);
                    if (assembly == null)
                    {
                        Directory.CreateDirectory(Path.GetDirectoryName(assemblyPath));
                        File.WriteAllBytes(assemblyPath, peStream.ToArray());
                        assembly = Assembly.LoadFile(assemblyPath);
                    }
                    else
                    {
                        string loadedPath = SafeAssemblyLocation(assembly);
                        if (!string.IsNullOrEmpty(loadedPath))
                            assemblyPath = loadedPath;
                    }

                    Type[] assemblyTypes = SafeGetAssemblyTypes(assembly) ?? new Type[0];
                    int publicTypeCount = assemblyTypes
                        .Count(type => type != null && type.IsPublic && !type.IsNested);

                    var compiled = new CompiledSkillPackageAssembly
                    {
                        PackageId = request.packageId,
                        Hash = request.sourceHash,
                        AssemblyId = assemblyId,
                        AssemblyPath = assemblyPath.Replace('\\', '/'),
                        ScriptCount = request.scripts.Length,
                        PublicTypeCount = publicTypeCount,
                        PublicTypes = BuildTypeIndexEntriesForAssembly(assembly, assemblyId),
                        Assembly = assembly
                    };
                    ActivateSkillPackageAssembly(compiled);
                    return compiled;
                }
                catch (Exception ex)
                {
                    throw new Exception("assembly load/bootstrap failed: " + ex.Message);
                }
            }
        }

        private static string BuildSkillPackageAssemblyId(SkillPackageCompileRequest request)
        {
            string shortHash = SanitizeAssemblyNamePart(request.sourceHash);
            if (shortHash.Length > 12)
                shortHash = shortHash.Substring(0, 12);

            return "__LocusSkillPackage_" +
                   SanitizeAssemblyNamePart(request.packageId) +
                   "_" +
                   shortHash;
        }

        private static Assembly FindLoadedAssemblyByName(string assemblyName)
        {
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            for (int i = 0; i < assemblies.Length; i++)
            {
                Assembly assembly = assemblies[i];
                if (assembly == null || assembly.IsDynamic)
                    continue;

                string name = SafeAssemblyName(assembly);
                if (string.Equals(name, assemblyName, StringComparison.Ordinal))
                    return assembly;
            }

            return null;
        }

        private static string SkillPackageAssemblyPath(string assemblyId)
        {
            string projectRoot = Path.GetDirectoryName(Application.dataPath);
            return Path.Combine(projectRoot, "Library", "Locus", "SkillAssemblies", assemblyId + ".dll");
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
                return Locus.Json.LocusJson.Deserialize(json, parameterType);
            }
            catch (Exception ex)
            {
                throw new Exception("failed to parse View Script args as " + parameterType.FullName + ": " + ex.Message);
            }
        }

        private static string EffectiveSkillPackageInvokeTypeName(SkillPackageInvokeRequest request)
        {
            if (request == null)
                return "";
            if (!string.IsNullOrWhiteSpace(request.typeName))
                return request.typeName.Trim();
            return (request.entryType ?? "").Trim();
        }

        private static object InvokeCompiledSkillPackageMethod(SkillPackageInvokeRequest request)
        {
            string typeName = EffectiveSkillPackageInvokeTypeName(request);
            Assembly assembly = FindSkillPackageInvokeAssembly(request);
            Type type = ResolveSkillPackageInvokeType(request, assembly, typeName);
            if (type == null)
                throw new Exception("Skill package " + request.packageId + " cannot find type: " + typeName);

            MethodInfo method = type.GetMethod(
                request.method,
                BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static
            );
            if (method == null)
                throw new Exception("Skill package " + request.packageId + " cannot find static method: " + typeName + "." + request.method);

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
                throw new Exception("Skill package methods may accept zero parameters or one JSON argument");
            }

            return CompleteTaskResult(method.Invoke(null, args));
        }

        private static Assembly FindSkillPackageInvokeAssembly(SkillPackageInvokeRequest request)
        {
            if (!string.IsNullOrWhiteSpace(request.assemblyId))
            {
                Assembly exact = FindLoadedAssemblyByName(request.assemblyId.Trim());
                if (exact == null)
                    throw new Exception("Skill package " + request.packageId + " cannot find assembly: " + request.assemblyId);
                return exact;
            }

            return null;
        }

        private static Assembly FindActiveSkillPackageAssembly(string packageId)
        {
            if (string.IsNullOrWhiteSpace(packageId))
                return null;

            string assemblyId = "";
            lock (_skillPackageAssemblyRegistryLock)
            {
                _activeSkillPackageAssemblyByPackage.TryGetValue(packageId.Trim(), out assemblyId);
            }

            return string.IsNullOrWhiteSpace(assemblyId)
                ? null
                : FindLoadedAssemblyByName(assemblyId.Trim());
        }

        private static Type ResolveSkillPackageInvokeType(
            SkillPackageInvokeRequest request,
            Assembly assembly,
            string typeName)
        {
            if (assembly != null)
                return ResolveTypeByFullOrSimpleName(assembly, typeName);

            Assembly activeAssembly = FindActiveSkillPackageAssembly(request.packageId);
            Type activeType = ResolveTypeByFullOrSimpleName(activeAssembly, typeName);
            if (activeType != null)
                return activeType;

            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();
            for (int i = 0; i < assemblies.Length; i++)
            {
                Assembly candidateAssembly = assemblies[i];
                if (candidateAssembly == null || candidateAssembly.IsDynamic)
                    continue;

                if (IsInactiveSkillPackageAssemblyName(SafeAssemblyName(candidateAssembly)))
                    continue;

                Type type = ResolveTypeByFullOrSimpleName(candidateAssembly, typeName);
                if (type != null)
                    return type;
            }

            return null;
        }

        private static Type ResolveTypeByFullOrSimpleName(Assembly assembly, string typeName)
        {
            if (assembly == null || string.IsNullOrWhiteSpace(typeName))
                return null;

            Type type = assembly.GetType(typeName, false);
            if (type != null)
                return type;

            Type[] types = SafeGetAssemblyTypes(assembly);
            if (types == null)
                return null;

            for (int i = 0; i < types.Length; i++)
            {
                Type candidate = types[i];
                if (candidate == null)
                    continue;
                if (string.Equals(candidate.FullName, typeName, StringComparison.Ordinal) ||
                    string.Equals(candidate.Name, typeName, StringComparison.Ordinal))
                    return candidate;
            }

            return null;
        }

        private static object CompleteTaskResult(object result)
        {
            Task task = result as Task;
            if (task == null)
                return result;

            task.GetAwaiter().GetResult();
            PropertyInfo resultProperty = task.GetType().GetProperty("Result");
            return resultProperty != null ? resultProperty.GetValue(task, null) : null;
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

        private static string BuildCompileSkillPackageResponse(CompiledSkillPackageAssembly compiled, bool cacheHit)
        {
            return "{" +
                   "\"packageId\":\"" + JsonEscape(compiled.PackageId) + "\"," +
                   "\"hash\":\"" + JsonEscape(compiled.Hash) + "\"," +
                   "\"cacheHit\":" + (cacheHit ? "true" : "false") + "," +
                   "\"assemblyId\":\"" + JsonEscape(compiled.AssemblyId) + "\"," +
                   "\"previousAssemblyId\":\"" + JsonEscape(compiled.PreviousAssemblyId) + "\"," +
                   "\"assemblyPath\":\"" + JsonEscape(compiled.AssemblyPath) + "\"," +
                   "\"scriptCount\":" + compiled.ScriptCount.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"publicTypeCount\":" + compiled.PublicTypeCount.ToString(CultureInfo.InvariantCulture) + "," +
                   "\"previousTypeIndexFingerprint\":\"" + JsonEscape(compiled.PreviousTypeIndexFingerprint) + "\"," +
                   "\"typeIndexFingerprint\":\"" + JsonEscape(compiled.TypeIndexFingerprint) + "\"," +
                   "\"types\":" + BuildTypeIndexEntriesJson(compiled.PublicTypes) + "," +
                   "\"domainFingerprint\":\"" + JsonEscape(_viewScriptDomainFingerprint) + "\"" +
                   "}";
        }

        private static string BuildInvokeSkillPackageResponse(SkillPackageInvokeRequest request, object result)
        {
            return "{" +
                   "\"packageId\":\"" + JsonEscape(request.packageId) + "\"," +
                   "\"assemblyId\":\"" + JsonEscape(request.assemblyId) + "\"," +
                   "\"typeName\":\"" + JsonEscape(EffectiveSkillPackageInvokeTypeName(request)) + "\"," +
                   "\"method\":\"" + JsonEscape(request.method) + "\"," +
                   "\"result\":" + ToJsonValue(result, 0) +
                   "}";
        }

        private static string BuildTypeIndexEntriesJson(TypeIndexEntry[] entries)
        {
            if (entries == null || entries.Length == 0)
                return "[]";

            var sb = new StringBuilder();
            sb.Append("[");
            for (int i = 0; i < entries.Length; i++)
            {
                TypeIndexEntry entry = entries[i];
                if (i > 0)
                    sb.Append(",");
                sb.Append("{");
                sb.Append("\"simpleName\":\"").Append(JsonEscape(entry.simpleName)).Append("\",");
                sb.Append("\"ns\":\"").Append(JsonEscape(entry.ns)).Append("\",");
                sb.Append("\"fullName\":\"").Append(JsonEscape(entry.fullName)).Append("\",");
                sb.Append("\"assembly\":\"").Append(JsonEscape(entry.assembly)).Append("\"");
                sb.Append("}");
            }
            sb.Append("]");
            return sb.ToString();
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

        // Arbitrary view-script results may contain reference cycles, so their
        // reflection serialization keeps a tight depth cap.
        private const int ViewScriptResultJsonDepthLimit = 12;

        private static string ToJsonValue(object value, int depth)
        {
            return ToJsonValue(value, depth, ViewScriptResultJsonDepthLimit, false);
        }

        private static string ToJsonValue(object value, int depth, int maxDepth, bool schemaSafeTruncation)
        {
            if (value == null)
                return "null";

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

            if (depth > maxDepth)
                return TruncatedJsonValue(value, schemaSafeTruncation);

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
                return DictionaryToJson(dictionary, depth + 1, maxDepth, schemaSafeTruncation);

            IEnumerable enumerable = value as IEnumerable;
            if (enumerable != null)
                return EnumerableToJson(enumerable, depth + 1, maxDepth, schemaSafeTruncation);

            return ObjectToJson(value, depth + 1, maxDepth, schemaSafeTruncation);
        }

        private static string TruncatedJsonValue(object value, bool schemaSafe)
        {
            // Typed consumers (the Rust bridge structs) reject a "..." string
            // standing in for an object or array, so schema-safe callers get
            // an empty value of the matching JSON shape instead.
            if (!schemaSafe)
                return "\"...\"";
            if (value is IDictionary)
                return "{}";
            if (value is IEnumerable)
                return "[]";
            return "null";
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

        private static string DictionaryToJson(IDictionary dictionary, int depth, int maxDepth, bool schemaSafeTruncation)
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
                sb.Append(ToJsonValue(entry.Value, depth, maxDepth, schemaSafeTruncation));
            }
            sb.Append("}");
            return sb.ToString();
        }

        private static string EnumerableToJson(IEnumerable enumerable, int depth, int maxDepth, bool schemaSafeTruncation)
        {
            var sb = new StringBuilder();
            sb.Append("[");
            bool first = true;
            foreach (object item in enumerable)
            {
                if (!first)
                    sb.Append(",");
                first = false;
                sb.Append(ToJsonValue(item, depth, maxDepth, schemaSafeTruncation));
            }
            sb.Append("]");
            return sb.ToString();
        }

        private static string ObjectToJson(object value, int depth, int maxDepth, bool schemaSafeTruncation)
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
                sb.Append(ToJsonValue(field.GetValue(value), depth, maxDepth, schemaSafeTruncation));
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
                sb.Append(ToJsonValue(propertyValue, depth, maxDepth, schemaSafeTruncation));
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
