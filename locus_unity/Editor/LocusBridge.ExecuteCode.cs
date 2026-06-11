
using UnityEngine;
using UnityEditor;
using UnityEditor.Compilation;

using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Reflection;
using System.Collections.Generic;
using System.Linq;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;
using Assembly = System.Reflection.Assembly;

namespace Locus
{
    public static partial class LocusBridge
    {
        // ───────────────── execute_code shared helpers ─────────────────

        private static async Task<string> EnsureExecuteCodeCompilationReadyAsync(
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            ThrowIfExecuteCodeCanceled(cancellationToken);
            ReportExecuteCodeCompilerStage(reportStage, "Checking compiler cache");
            lock (_compileCacheLock)
            {
                if (_metadataReferencesReady && _cachedMetadataReferences != null)
                {
                    ReportExecuteCodeCompilerStage(reportStage, "Compiler cache ready");
                    return null;
                }
            }

            var tcs = new TaskCompletionSource<string>();

            // Build Unity-dependent metadata references on the main thread the first time execute_code runs.
            ReportExecuteCodeCompilerStage(reportStage, "Waiting for Unity main thread");
            PostToMainThread(delegate
            {
                try
                {
                    ThrowIfExecuteCodeCanceled(cancellationToken);
                    EnsureMetadataReferences(reportStage, cancellationToken);
                    tcs.TrySetResult(null);
                }
                catch (OperationCanceledException)
                {
                    tcs.TrySetResult("execute_code canceled");
                }
                catch (Exception ex)
                {
                    tcs.TrySetResult("prepare execute_code failed: " + ex.Message);
                }
            });

            Task delayTask = Task.Delay(ExecuteTimeoutMs, cancellationToken);
            Task completed = await Task.WhenAny(tcs.Task, delayTask);
            if (cancellationToken.IsCancellationRequested)
                return "execute_code canceled";
            if (completed != tcs.Task)
                return "prepare execute_code timed out";

            return tcs.Task.Result;
        }

        private static void ReportExecuteCodeCompilerStage(Action<string> reportStage, string stage)
        {
            if (reportStage == null || string.IsNullOrEmpty(stage))
                return;

            try
            {
                reportStage(stage);
            }
            catch
            {
            }
        }

        private static void SplitLeadingUsings(string code, out string leadingUsings, out string bodyCode)
        {
            if (string.IsNullOrEmpty(code))
            {
                leadingUsings = "";
                bodyCode = "";
                return;
            }

            string normalized = code.Replace("\r\n", "\n");
            string[] lines = normalized.Split('\n');

            var usingSb = new StringBuilder();
            var bodySb = new StringBuilder();

            bool stillInUsingBlock = true;

            for (int i = 0; i < lines.Length; i++)
            {
                string line = lines[i];
                string trimmed = line.Trim();

                if (stillInUsingBlock)
                {
                    if (string.IsNullOrEmpty(trimmed))
                    {
                        if (usingSb.Length > 0)
                            usingSb.AppendLine(line);
                        else
                            bodySb.AppendLine(line);

                        continue;
                    }

                    if (trimmed.StartsWith("using ", StringComparison.Ordinal) &&
                        trimmed.EndsWith(";", StringComparison.Ordinal))
                    {
                        usingSb.AppendLine(line);
                        continue;
                    }

                    stillInUsingBlock = false;
                }

                bodySb.AppendLine(line);
            }

            leadingUsings = usingSb.ToString().TrimEnd();
            bodyCode = bodySb.ToString().TrimEnd();
        }

        // ───────────────── Diagnostic formatting ─────────────────

        private static string BuildDiagnosticErrorText(IEnumerable<Diagnostic> diagnostics)
        {
            if (diagnostics == null)
                return null;

            var sb = new StringBuilder();
            bool hasError = false;

            foreach (Diagnostic diagnostic in diagnostics)
            {
                if (diagnostic == null)
                    continue;

                if (diagnostic.Severity != DiagnosticSeverity.Error)
                    continue;

                if (!hasError)
                {
                    hasError = true;
                    sb.Append("compilation failed:\n");
                }

                int line = 0;
                int column = 0;

                try
                {
                    FileLinePositionSpan span = diagnostic.Location.GetMappedLineSpan();
                    line = span.StartLinePosition.Line + 1;
                    column = span.StartLinePosition.Character + 1;
                }
                catch
                {
                }

                sb.Append("  ");
                sb.Append(diagnostic.Id);
                sb.Append(" at ");
                sb.Append(line);
                sb.Append(":");
                sb.Append(column);
                sb.Append(": ");
                sb.Append(diagnostic.GetMessage());
                sb.Append("\n");
            }

            return hasError ? sb.ToString() : null;
        }

        // ───────────────── MetadataReference collection ─────────────────

        private static void ThrowIfExecuteCodeCanceled(CancellationToken cancellationToken)
        {
            if (cancellationToken.IsCancellationRequested)
                throw new OperationCanceledException(cancellationToken);
        }

        /// <summary>
        /// Path-collection layer: the reference set as absolute file paths.
        /// Shared by the in-Unity compiler (materialized below) and the
        /// `get_compile_params` provider for the compile-server sidecar.
        /// </summary>
        private static List<string> EnsureCompileReferencePaths(
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            ThrowIfExecuteCodeCanceled(cancellationToken);
            lock (_compileCacheLock)
            {
                ThrowIfExecuteCodeCanceled(cancellationToken);
                if (_compileReferencePathsReady && _cachedCompileReferencePaths != null)
                    return _cachedCompileReferencePaths;

                _cachedCompileReferencePaths = BuildCompileReferencePaths(reportStage, cancellationToken);
                _compileReferencePathsReady = true;
                return _cachedCompileReferencePaths;
            }
        }

        private static List<MetadataReference> EnsureMetadataReferences(
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            ThrowIfExecuteCodeCanceled(cancellationToken);
            ReportExecuteCodeCompilerStage(reportStage, "Locking compiler reference cache");
            lock (_compileCacheLock)
            {
                ThrowIfExecuteCodeCanceled(cancellationToken);
                if (_metadataReferencesReady && _cachedMetadataReferences != null)
                {
                    ReportExecuteCodeCompilerStage(reportStage, "Compiler reference cache ready");
                    return _cachedMetadataReferences;
                }

                // Monitor locks are reentrant: collecting paths under the
                // same cache lock keeps both layers consistent.
                List<string> referencePaths =
                    EnsureCompileReferencePaths(reportStage, cancellationToken);
                ReportExecuteCodeCompilerStage(reportStage, "Materializing compiler references");
                _cachedMetadataReferences =
                    MaterializeMetadataReferences(referencePaths, cancellationToken);
                _metadataReferencesReady = true;
                ReportExecuteCodeCompilerStage(reportStage, "Compiler reference cache ready");
                return _cachedMetadataReferences;
            }
        }

        /// <summary>
        /// Materialization layer for the legacy in-Unity compile path.
        /// Invalid/missing files are skipped silently, matching the old
        /// combined collection behavior.
        /// </summary>
        private static List<MetadataReference> MaterializeMetadataReferences(
            List<string> referencePaths,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            var references = new List<MetadataReference>(referencePaths.Count);
            for (int i = 0; i < referencePaths.Count; i++)
            {
                ThrowIfExecuteCodeCanceled(cancellationToken);
                try
                {
                    references.Add(MetadataReference.CreateFromFile(referencePaths[i]));
                }
                catch
                {
                }
            }

            return references;
        }

        private static List<string> BuildCompileReferencePaths(
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            List<string> references = new List<string>(384);
            HashSet<string> referencedPaths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            ThrowIfExecuteCodeCanceled(cancellationToken);
            ReportExecuteCodeCompilerStage(reportStage, "Adding core compiler references");
            TryAddCompileReferencePath(references, referencedPaths, SafeGetAssemblyLocation(typeof(object).Assembly));
            TryAddCompileReferencePath(references, referencedPaths, SafeGetAssemblyLocation(typeof(Enumerable).Assembly));
            TryAddCompileReferencePath(references, referencedPaths, SafeGetAssemblyLocation(typeof(UnityEngine.Debug).Assembly));
            TryAddCompileReferencePath(references, referencedPaths, SafeGetAssemblyLocation(typeof(UnityEditor.Editor).Assembly));
            TryAddCompileReferencePath(references, referencedPaths, SafeGetAssemblyLocation(typeof(LocusBridge).Assembly));

            AddSystemAssemblyDirectories(references, referencedPaths, reportStage, cancellationToken);

            AddPrecompiledAssemblies(references, referencedPaths, reportStage, cancellationToken);

            AddCompilationAssemblies(references, referencedPaths, AssembliesType.Editor, reportStage, cancellationToken);
            AddCompilationAssemblies(references, referencedPaths, AssembliesType.PlayerWithoutTestAssemblies, reportStage, cancellationToken);

            ReportExecuteCodeCompilerStage(reportStage, "Adding loaded AppDomain assemblies");
            foreach (Assembly asm in AppDomain.CurrentDomain.GetAssemblies())
            {
                try
                {
                    ThrowIfExecuteCodeCanceled(cancellationToken);
                    if (asm == null || asm.IsDynamic)
                        continue;

                    string assemblyName = SafeAssemblyName(asm);
                    if (IsInactiveSkillPackageAssemblyName(assemblyName))
                        continue;

                    TryAddCompileReferencePath(references, referencedPaths, SafeGetAssemblyLocation(asm));
                }
                catch (OperationCanceledException)
                {
                    throw;
                }
                catch
                {
                }
            }

            AddScriptAssembliesDirectory(references, referencedPaths, reportStage, cancellationToken);

            return references;
        }

        private static void AddSystemAssemblyDirectories(
            List<string> references,
            HashSet<string> referencedPaths,
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            ThrowIfExecuteCodeCanceled(cancellationToken);
            ReportExecuteCodeCompilerStage(reportStage, "Adding Unity system assemblies");
            try
            {
                ApiCompatibilityLevel apiCompatibilityLevel;
                if (!TryGetCurrentApiCompatibilityLevel(out apiCompatibilityLevel))
                    return;

                string[] systemDirs = CompilationPipeline.GetSystemAssemblyDirectories(apiCompatibilityLevel);
                if (systemDirs == null)
                    return;

                for (int i = 0; i < systemDirs.Length; i++)
                {
                    string dir = systemDirs[i];
                    if (string.IsNullOrEmpty(dir) || !Directory.Exists(dir))
                        continue;

                    string[] dlls;
                    try
                    {
                        dlls = Directory.GetFiles(dir, "*.dll", SearchOption.TopDirectoryOnly);
                    }
                    catch
                    {
                        continue;
                    }

                    for (int j = 0; j < dlls.Length; j++)
                    {
                        ThrowIfExecuteCodeCanceled(cancellationToken);
                        TryAddCompileReferencePath(references, referencedPaths, dlls[j]);
                    }
                }
            }
            catch (OperationCanceledException)
            {
                throw;
            }
            catch
            {
            }
        }

        private static bool TryGetCurrentApiCompatibilityLevel(out ApiCompatibilityLevel apiCompatibilityLevel)
        {
            apiCompatibilityLevel = default(ApiCompatibilityLevel);

            try
            {
                apiCompatibilityLevel =
                    PlayerSettings.GetApiCompatibilityLevel(EditorUserBuildSettings.selectedBuildTargetGroup);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static void AddPrecompiledAssemblies(
            List<string> references,
            HashSet<string> referencedPaths,
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            ThrowIfExecuteCodeCanceled(cancellationToken);
            ReportExecuteCodeCompilerStage(reportStage, "Adding precompiled assemblies");
            try
            {
                string[] precompiledPaths =
                    CompilationPipeline.GetPrecompiledAssemblyPaths(
                        CompilationPipeline.PrecompiledAssemblySources.All);

                if (precompiledPaths == null)
                    return;

                for (int i = 0; i < precompiledPaths.Length; i++)
                {
                    ThrowIfExecuteCodeCanceled(cancellationToken);
                    TryAddCompileReferencePath(references, referencedPaths, precompiledPaths[i]);
                }
            }
            catch (OperationCanceledException)
            {
                throw;
            }
            catch
            {
            }
        }

        private static void AddCompilationAssemblies(
            List<string> references,
            HashSet<string> referencedPaths,
            AssembliesType assembliesType,
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            ThrowIfExecuteCodeCanceled(cancellationToken);
            ReportExecuteCodeCompilerStage(
                reportStage,
                assembliesType == AssembliesType.Editor
                    ? "Adding editor compilation assemblies"
                    : "Adding player compilation assemblies");

            UnityEditor.Compilation.Assembly[] assemblies = null;

            try
            {
                assemblies = CompilationPipeline.GetAssemblies(assembliesType);
            }
            catch (OperationCanceledException)
            {
                throw;
            }
            catch
            {
                return;
            }

            if (assemblies == null)
                return;

            for (int i = 0; i < assemblies.Length; i++)
            {
                ThrowIfExecuteCodeCanceled(cancellationToken);
                UnityEditor.Compilation.Assembly asm = assemblies[i];
                if (asm == null)
                    continue;

                TryAddCompileReferencePath(references, referencedPaths, asm.outputPath);

                string[] allRefs = asm.allReferences;
                if (allRefs == null)
                    continue;

                for (int j = 0; j < allRefs.Length; j++)
                {
                    ThrowIfExecuteCodeCanceled(cancellationToken);
                    TryAddCompileReferencePath(references, referencedPaths, allRefs[j]);
                }
            }
        }

        private static void AddScriptAssembliesDirectory(
            List<string> references,
            HashSet<string> referencedPaths,
            Action<string> reportStage = null,
            CancellationToken cancellationToken = default(CancellationToken))
        {
            ThrowIfExecuteCodeCanceled(cancellationToken);
            ReportExecuteCodeCompilerStage(reportStage, "Adding ScriptAssemblies");
            try
            {
                string projectRoot = Path.GetDirectoryName(Application.dataPath);
                string scriptAssembliesDir = Path.Combine(projectRoot, "Library", "ScriptAssemblies");

                if (!Directory.Exists(scriptAssembliesDir))
                    return;

                string[] dlls;
                try
                {
                    dlls = Directory.GetFiles(scriptAssembliesDir, "*.dll", SearchOption.TopDirectoryOnly);
                }
                catch
                {
                    return;
                }

                for (int i = 0; i < dlls.Length; i++)
                {
                    ThrowIfExecuteCodeCanceled(cancellationToken);
                    TryAddCompileReferencePath(references, referencedPaths, dlls[i]);
                }
            }
            catch (OperationCanceledException)
            {
                throw;
            }
            catch
            {
            }
        }

        private static string SafeGetAssemblyLocation(Assembly asm)
        {
            try
            {
                if (asm == null || asm.IsDynamic)
                    return null;

                string location = asm.Location;
                return string.IsNullOrEmpty(location) ? null : location;
            }
            catch
            {
                return null;
            }
        }

        private static void TryAddCompileReferencePath(
            List<string> references,
            HashSet<string> referencedPaths,
            string path)
        {
            if (string.IsNullOrEmpty(path))
                return;

            try
            {
                if (!Path.IsPathRooted(path))
                    path = Path.GetFullPath(path);
            }
            catch
            {
                return;
            }

            if (!File.Exists(path))
                return;

            string normalizedPath = path.Replace('\\', '/');
            if (normalizedPath.IndexOf("/NetStandard/", StringComparison.OrdinalIgnoreCase) >= 0)
                return;

            if (!referencedPaths.Add(path))
                return;

            try
            {
                AssemblyName asmName = AssemblyName.GetAssemblyName(path);
                byte[] tokenBytes = asmName.GetPublicKeyToken();
                string token = tokenBytes != null && tokenBytes.Length > 0
                    ? BitConverter.ToString(tokenBytes).Replace("-", "").ToLowerInvariant()
                    : "null";
                string identityKey = "__identity__:" + asmName.Name + ":" + token;
                if (!referencedPaths.Add(identityKey))
                    return;
            }
            catch
            {
                string fileName = Path.GetFileNameWithoutExtension(path);
                if (!string.IsNullOrEmpty(fileName) && !referencedPaths.Add("__filename__:" + fileName.ToLowerInvariant()))
                    return;
            }

            references.Add(path);
        }
    }
}
