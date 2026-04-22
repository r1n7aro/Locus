
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
        // ───────────────── execute_code handler ─────────────────

        private static async Task<PipeEnvelope> HandleExecuteCode(string requestId, string code)
        {
            if (string.IsNullOrWhiteSpace(code))
                return ErrorResponse(requestId, "empty code");

            await _executeCodeLock.WaitAsync();
            try
            {
                string prepareError = await EnsureExecuteCodeCompilationReadyAsync();
                if (!string.IsNullOrEmpty(prepareError))
                    return ErrorResponse(requestId, prepareError);

                CompiledSnippet snippet;
                try
                {
                    snippet = CompileSnippet(code);
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "snippet compilation exception: " + ex.Message);
                }

                string resultText = await ExecuteSnippetOnMainThreadAsync(snippet);

                if (resultText == "__TIMEOUT__")
                    return ErrorResponse(requestId, "execution timed out after " + (ExecuteTimeoutMs / 1000) + " seconds");

                if (resultText.StartsWith("__ERROR__: ", StringComparison.Ordinal))
                    return ErrorResponse(requestId, resultText.Substring("__ERROR__: ".Length));

                return OkResponse(requestId, resultText);
            }
            finally
            {
                _executeCodeLock.Release();
            }
        }

        private static async Task<string> EnsureExecuteCodeCompilationReadyAsync()
        {
            lock (_compileCacheLock)
            {
                if (_metadataReferencesReady && _cachedMetadataReferences != null)
                    return null;
            }

            var tcs = new TaskCompletionSource<string>();

            // Build Unity-dependent metadata references on the main thread the first time execute_code runs.
            PostToMainThread(delegate
            {
                try
                {
                    EnsureMetadataReferences();
                    tcs.TrySetResult(null);
                }
                catch (Exception ex)
                {
                    tcs.TrySetResult("prepare execute_code failed: " + ex.Message);
                }
            });

            Task completed = await Task.WhenAny(tcs.Task, Task.Delay(ExecuteTimeoutMs));
            if (completed != tcs.Task)
                return "prepare execute_code timed out";

            return tcs.Task.Result;
        }

        // ───────────────── Snippet compilation ─────────────────

        private static CompiledSnippet CompileSnippet(string code)
        {
            string leadingUsings;
            string bodyCode;
            SplitLeadingUsings(code, out leadingUsings, out bodyCode);

            CompiledSnippet snippet;
            string primaryError;

            if (TryCompileSnippet(bodyCode, leadingUsings, false, out snippet, out primaryError))
                return snippet;

            string fallbackError;

            if (TryCompileSnippet(bodyCode, leadingUsings, true, out snippet, out fallbackError))
                return snippet;

            var sb = new StringBuilder();

            if (!string.IsNullOrEmpty(primaryError))
                sb.Append(primaryError);

            if (!string.IsNullOrEmpty(fallbackError) &&
                !string.Equals(primaryError, fallbackError, StringComparison.Ordinal))
            {
                if (sb.Length > 0)
                    sb.Append("\n\nexpression fallback:\n");

                sb.Append(fallbackError);
            }

            throw new Exception(sb.Length > 0 ? sb.ToString() : "unknown compilation failure");
        }

        private static bool TryCompileSnippet(
            string bodyCode,
            string leadingUsings,
            bool expressionMode,
            out CompiledSnippet snippet,
            out string error)
        {
            snippet = null;
            error = null;

            const string hostTypeName = "__LocusSnippetHost";
            const string fullTypeName = "Locus.RuntimeSnippets.__LocusSnippetHost";

            string source = BuildSnippetSource(hostTypeName, leadingUsings, bodyCode, expressionMode);

            SyntaxTree syntaxTree;
            try
            {
                syntaxTree = CSharpSyntaxTree.ParseText(
                    source,
                    SnippetParseOptions,
                    path: "LocusRuntimeSnippet.cs",
                    encoding: Utf8NoBom
                );
            }
            catch (Exception ex)
            {
                error = "parse failed: " + ex;
                return false;
            }

            string assemblyName =
                "__LocusRuntime_" + Interlocked.Increment(ref _snippetAssemblyCounter).ToString("X8");

            CSharpCompilation compilation = CSharpCompilation.Create(
                assemblyName: assemblyName,
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
                    error = "emit failed: " + ex;
                    return false;
                }

                if (!emitResult.Success)
                {
                    error = BuildDiagnosticErrorText(emitResult.Diagnostics);
                    return false;
                }

                try
                {
                    byte[] assemblyBytes = peStream.ToArray();
                    Assembly assembly = Assembly.Load(assemblyBytes);

                    Type hostType = assembly.GetType(fullTypeName, true);
                    MethodInfo executeMethod = hostType.GetMethod(
                        "Execute",
                        BindingFlags.Public | BindingFlags.Static
                    );

                    if (executeMethod == null)
                    {
                        error = "compiled snippet missing Execute method";
                        return false;
                    }

                    Func<ScriptGlobals, object> executor =
                        (Func<ScriptGlobals, object>)Delegate.CreateDelegate(
                            typeof(Func<ScriptGlobals, object>),
                            executeMethod
                        );

                    snippet = new CompiledSnippet(executor);
                    return true;
                }
                catch (Exception ex)
                {
                    error = "assembly load/bootstrap failed: " + ex;
                    return false;
                }
            }
        }

        // ───────────────── Snippet source generation ─────────────────

        private static string BuildSnippetSource(
            string hostTypeName,
            string leadingUsings,
            string bodyCode,
            bool expressionMode)
        {
            var sb = new StringBuilder(4096);

            sb.AppendLine("using System;");
            sb.AppendLine("using System.IO;");
            sb.AppendLine("using System.Text;");
            sb.AppendLine("using System.Linq;");
            sb.AppendLine("using System.Reflection;");
            sb.AppendLine("using System.Threading;");
            sb.AppendLine("using System.Threading.Tasks;");
            sb.AppendLine("using System.Collections;");
            sb.AppendLine("using System.Collections.Generic;");
            sb.AppendLine("using UnityEngine;");
            sb.AppendLine("using UnityEngine.SceneManagement;");
            sb.AppendLine("using UnityEngine.UI;");
            sb.AppendLine("using UnityEditor;");
            sb.AppendLine("using UnityEditor.SceneManagement;");
            sb.AppendLine("using UnityEditor.Animations;");
            sb.AppendLine("using static UnityEngine.Object;");
            sb.AppendLine("using Object = UnityEngine.Object;");

            if (!string.IsNullOrWhiteSpace(leadingUsings))
                sb.AppendLine(leadingUsings);

            sb.AppendLine("namespace Locus.RuntimeSnippets");
            sb.AppendLine("{");
            sb.Append("    public static class ").Append(hostTypeName).AppendLine();
            sb.AppendLine("    {");
            sb.AppendLine("        public static object Execute(global::Locus.LocusBridge.ScriptGlobals globals)");
            sb.AppendLine("        {");
            sb.AppendLine("            var print = new global::System.Action<object>(globals.print);");
            sb.AppendLine("            var printJson = new global::System.Action<object>(globals.printJson);");
            sb.AppendLine("            var clear = new global::System.Action(globals.clear);");
            sb.AppendLine("            #line 1");

            if (expressionMode)
            {
                if (string.IsNullOrWhiteSpace(bodyCode))
                {
                    sb.AppendLine("            return null;");
                }
                else
                {
                    sb.Append("            return (object)(");
                    sb.Append(bodyCode);
                    sb.AppendLine(");");
                }
            }
            else
            {
                if (!string.IsNullOrWhiteSpace(bodyCode))
                    sb.AppendLine(bodyCode);

                sb.AppendLine("            return null;");
            }

            sb.AppendLine("            #line default");
            sb.AppendLine("        }");
            sb.AppendLine("    }");
            sb.AppendLine("}");

            return sb.ToString();
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

        // ───────────────── Main-thread execution ─────────────────

        private static async Task<string> ExecuteSnippetOnMainThreadAsync(CompiledSnippet snippet)
        {
            TaskCompletionSource<string> tcs = new TaskCompletionSource<string>();

            PostToMainThread(delegate
            {
                try
                {
                    ScriptGlobals globals = new ScriptGlobals();

                    object returnValue = snippet.Executor(globals);

                    if (returnValue != null)
                        globals.print(returnValue);

                    tcs.TrySetResult(globals.GetOutput());
                }
                catch (Exception ex)
                {
                    tcs.TrySetResult("__ERROR__: runtime error: " + ex);
                }
            });

            Task completed = await Task.WhenAny(tcs.Task, Task.Delay(ExecuteTimeoutMs));
            if (completed != tcs.Task)
                return "__TIMEOUT__";

            return tcs.Task.Result ?? "";
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

        private static List<MetadataReference> EnsureMetadataReferences()
        {
            lock (_compileCacheLock)
            {
                if (_metadataReferencesReady && _cachedMetadataReferences != null)
                    return _cachedMetadataReferences;

                _cachedMetadataReferences = BuildMetadataReferences();
                _metadataReferencesReady = true;
                return _cachedMetadataReferences;
            }
        }

        private static List<MetadataReference> BuildMetadataReferences()
        {
            List<MetadataReference> references = new List<MetadataReference>(384);
            HashSet<string> referencedPaths = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            TryAddMetadataReference(references, referencedPaths, SafeGetAssemblyLocation(typeof(object).Assembly));
            TryAddMetadataReference(references, referencedPaths, SafeGetAssemblyLocation(typeof(Enumerable).Assembly));
            TryAddMetadataReference(references, referencedPaths, SafeGetAssemblyLocation(typeof(UnityEngine.Debug).Assembly));
            TryAddMetadataReference(references, referencedPaths, SafeGetAssemblyLocation(typeof(UnityEditor.Editor).Assembly));
            TryAddMetadataReference(references, referencedPaths, SafeGetAssemblyLocation(typeof(LocusBridge).Assembly));

            AddSystemAssemblyDirectories(references, referencedPaths);

            AddPrecompiledAssemblies(references, referencedPaths);

            AddCompilationAssemblies(references, referencedPaths, AssembliesType.Editor);
            AddCompilationAssemblies(references, referencedPaths, AssembliesType.PlayerWithoutTestAssemblies);

            foreach (Assembly asm in AppDomain.CurrentDomain.GetAssemblies())
            {
                try
                {
                    if (asm == null || asm.IsDynamic)
                        continue;

                    TryAddMetadataReference(references, referencedPaths, SafeGetAssemblyLocation(asm));
                }
                catch
                {
                }
            }

            AddScriptAssembliesDirectory(references, referencedPaths);

            return references;
        }

        private static void AddSystemAssemblyDirectories(
            List<MetadataReference> references,
            HashSet<string> referencedPaths)
        {
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
                        TryAddMetadataReference(references, referencedPaths, dlls[j]);
                }
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
            List<MetadataReference> references,
            HashSet<string> referencedPaths)
        {
            try
            {
                string[] precompiledPaths =
                    CompilationPipeline.GetPrecompiledAssemblyPaths(
                        CompilationPipeline.PrecompiledAssemblySources.All);

                if (precompiledPaths == null)
                    return;

                for (int i = 0; i < precompiledPaths.Length; i++)
                    TryAddMetadataReference(references, referencedPaths, precompiledPaths[i]);
            }
            catch
            {
            }
        }

        private static void AddCompilationAssemblies(
            List<MetadataReference> references,
            HashSet<string> referencedPaths,
            AssembliesType assembliesType)
        {
            UnityEditor.Compilation.Assembly[] assemblies = null;

            try
            {
                assemblies = CompilationPipeline.GetAssemblies(assembliesType);
            }
            catch
            {
                return;
            }

            if (assemblies == null)
                return;

            for (int i = 0; i < assemblies.Length; i++)
            {
                UnityEditor.Compilation.Assembly asm = assemblies[i];
                if (asm == null)
                    continue;

                TryAddMetadataReference(references, referencedPaths, asm.outputPath);

                string[] allRefs = asm.allReferences;
                if (allRefs == null)
                    continue;

                for (int j = 0; j < allRefs.Length; j++)
                    TryAddMetadataReference(references, referencedPaths, allRefs[j]);
            }
        }

        private static void AddScriptAssembliesDirectory(
            List<MetadataReference> references,
            HashSet<string> referencedPaths)
        {
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
                    TryAddMetadataReference(references, referencedPaths, dlls[i]);
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

        private static void TryAddMetadataReference(
            List<MetadataReference> references,
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

            try
            {
                references.Add(MetadataReference.CreateFromFile(path));
            }
            catch
            {
            }
        }
    }
}
