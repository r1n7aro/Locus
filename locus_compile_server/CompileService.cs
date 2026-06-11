using System.Collections.Immutable;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;

namespace Locus.CompileServer;

// ── request DTOs ─────────────────────────────────────────────────────

/// <summary>Compile parameters collected by the Unity-side `get_compile_params`.</summary>
public sealed class CompileParamsDto
{
    [JsonPropertyName("fingerprint")]
    public string? Fingerprint { get; set; }

    [JsonPropertyName("domainGeneration")]
    public string? DomainGeneration { get; set; }

    [JsonPropertyName("langVersion")]
    public string? LangVersion { get; set; }

    [JsonPropertyName("referencePaths")]
    public string[]? ReferencePaths { get; set; }

    [JsonPropertyName("defines")]
    public string[]? Defines { get; set; }
}

public sealed class RawSourceDto
{
    [JsonPropertyName("path")]
    public string? Path { get; set; }

    [JsonPropertyName("text")]
    public string? Text { get; set; }
}

public sealed class CompileRawRequestDto
{
    [JsonPropertyName("assemblyName")]
    public string? AssemblyName { get; set; }

    [JsonPropertyName("sources")]
    public RawSourceDto[]? Sources { get; set; }

    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }

    /// <summary>Use the server's own runtime assemblies as references
    /// (warm-up and transport tests; no Unity involved).</summary>
    [JsonPropertyName("useHostBcl")]
    public bool UseHostBcl { get; set; }

    [JsonPropertyName("referenceSessionImages")]
    public bool ReferenceSessionImages { get; set; }

    [JsonPropertyName("registerImage")]
    public bool RegisterImage { get; set; }
}

public sealed class CompileSnippetRequestDto
{
    [JsonPropertyName("code")]
    public string? Code { get; set; }

    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }

    [JsonPropertyName("referenceSessionImages")]
    public bool ReferenceSessionImages { get; set; }

    /// <summary>Register the emitted image in the session registry. Only
    /// assemblies that will actually be loaded into the Unity domain should
    /// register (e.g. not the compile_run_states pre-check).</summary>
    [JsonPropertyName("registerImage")]
    public bool RegisterImage { get; set; }
}

public sealed class CompileRunStatesRequestDto
{
    [JsonPropertyName("request")]
    public RunStatesRequest? Request { get; set; }

    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }

    [JsonPropertyName("referenceSessionImages")]
    public bool ReferenceSessionImages { get; set; }

    [JsonPropertyName("registerImage")]
    public bool RegisterImage { get; set; }
}

public sealed class CompileViewScriptRequestDto
{
    [JsonPropertyName("source")]
    public string? Source { get; set; }

    [JsonPropertyName("path")]
    public string? Path { get; set; }

    [JsonPropertyName("scriptName")]
    public string? ScriptName { get; set; }

    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }
}

// ── service ──────────────────────────────────────────────────────────

public sealed class CompileService
{
    public const int ProtocolVersion = 1;

    /// <summary>
    /// Version of the generated wrapper's entry-point contract with the Unity
    /// plugin (ScriptGlobals/ExecuteCodeContext signature, host type names).
    /// Bump together with the Unity-side `execute_loaded` expectations.
    /// </summary>
    public const int WrapperContractVersion = 1;

    private static readonly UTF8Encoding Utf8NoBom = new(false);

    /// <summary>Mirror of the Unity-side SnippetCompilationOptions.</summary>
    private static readonly CSharpCompilationOptions SnippetCompilationOptions = new(
        outputKind: OutputKind.DynamicallyLinkedLibrary,
        optimizationLevel: OptimizationLevel.Release,
        allowUnsafe: false,
        assemblyIdentityComparer: DesktopAssemblyIdentityComparer.Default);

    /// <summary>
    /// Unlike the legacy Unity-side path (no PDB), emit an embedded portable
    /// PDB: `Assembly.Load(byte[])` then yields line numbers in stack traces.
    /// </summary>
    private static readonly EmitOptions SnippetEmitOptions = new(
        debugInformationFormat: DebugInformationFormat.Embedded);

    private static readonly Lazy<ImmutableArray<MetadataReference>> HostBclReferences =
        new(BuildHostBclReferences);

    private readonly ReferenceCache _referenceCache = new();
    private readonly ImageRegistry _imageRegistry = new();

    private int _assemblyCounter;

    // Reference set + parse options for the last seen params fingerprint.
    private string? _cachedFingerprint;
    private ImmutableArray<MetadataReference> _cachedReferences;
    private CSharpParseOptions _cachedParseOptions = DefaultParseOptions(Array.Empty<string>());

    // ── handlers ─────────────────────────────────────────────────────

    public JsonNode HandleInitialize(JsonNode? @params)
    {
        var roslyn = typeof(CSharpCompilation).Assembly.GetName().Version?.ToString() ?? "unknown";
        return new JsonObject
        {
            ["serverName"] = "LocusCompileServer",
            ["protocolVersion"] = ProtocolVersion,
            ["wrapperContractVersion"] = WrapperContractVersion,
            ["roslynVersion"] = roslyn,
            ["pid"] = Environment.ProcessId,
        };
    }

    public JsonNode HandleCompileRaw(JsonNode? @params)
    {
        var request = Deserialize<CompileRawRequestDto>(@params);
        if (request.Sources == null || request.Sources.Length == 0)
            throw new RpcInvalidParamsException("compile/raw requires at least one source");

        var sources = new List<(string Path, string Text)>(request.Sources.Length);
        foreach (RawSourceDto source in request.Sources)
        {
            if (string.IsNullOrEmpty(source.Path) || source.Text == null)
                throw new RpcInvalidParamsException("compile/raw sources require path and text");
            sources.Add((source.Path, source.Text));
        }

        long startedAt = Environment.TickCount64;
        string assemblyName = string.IsNullOrWhiteSpace(request.AssemblyName)
            ? NextAssemblyName("Raw", request.Params?.DomainGeneration)
            : request.AssemblyName!;

        var (bytes, error) = CompileSources(
            assemblyName,
            sources,
            request.Params,
            request.UseHostBcl,
            request.ReferenceSessionImages);

        if (bytes == null)
            return FailureResult(error!, "compile", startedAt);

        if (request.RegisterImage && !string.IsNullOrEmpty(request.Params?.DomainGeneration))
            _imageRegistry.Register(request.Params!.DomainGeneration!, assemblyName, bytes);

        return SuccessResult(bytes, assemblyName, startedAt);
    }

    public JsonNode HandleCompileSnippet(JsonNode? @params)
    {
        var request = Deserialize<CompileSnippetRequestDto>(@params);
        if (string.IsNullOrWhiteSpace(request.Code))
            throw new RpcInvalidParamsException("compile/snippet requires code");

        long startedAt = Environment.TickCount64;

        UnitySnippetSource.SplitLeadingUsings(request.Code, out string leadingUsings, out string bodyCode);

        // Same two-attempt semantics as the Unity-side CompileAsyncSnippet:
        // statement mode first, expression mode as the fallback, errors of
        // both attempts combined in the legacy format.
        var (bytes, assemblyName, primaryError) = CompileSnippetAttempt(
            request, leadingUsings, bodyCode, expressionMode: false);
        if (bytes != null)
            return SnippetSuccessResult(bytes, assemblyName!, "statements", request, startedAt);

        var (fallbackBytes, fallbackAssemblyName, fallbackError) = CompileSnippetAttempt(
            request, leadingUsings, bodyCode, expressionMode: true);
        if (fallbackBytes != null)
            return SnippetSuccessResult(fallbackBytes, fallbackAssemblyName!, "expression", request, startedAt);

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

        string combined = sb.Length > 0 ? sb.ToString() : "unknown async compilation failure";
        return FailureResult(combined, "compile", startedAt);
    }

    public JsonNode HandleCompileViewScript(JsonNode? @params)
    {
        var request = Deserialize<CompileViewScriptRequestDto>(@params);
        if (string.IsNullOrWhiteSpace(request.Source))
            throw new RpcInvalidParamsException("compile/viewScript requires source");

        long startedAt = Environment.TickCount64;
        string sourcePath = string.IsNullOrWhiteSpace(request.Path) ? "ViewScript.cs" : request.Path!;

        CSharpParseOptions parseOptions = ResolveParseOptions(request.Params);
        SyntaxTree syntaxTree;
        try
        {
            syntaxTree = CSharpSyntaxTree.ParseText(
                request.Source!,
                parseOptions,
                path: sourcePath,
                encoding: Utf8NoBom);
        }
        catch (Exception ex)
        {
            // View Script error wording uses ex.Message (LocusBridge.ViewScripts.cs).
            return FailureResult("parse failed: " + ex.Message, "compile", startedAt);
        }

        string assemblyName = NextAssemblyName(
            "View_" + SanitizeAssemblyNamePart(request.ScriptName),
            request.Params?.DomainGeneration);

        var (bytes, error) = EmitCompilation(
            assemblyName,
            new[] { syntaxTree },
            request.Params,
            useHostBcl: false,
            referenceSessionImages: false,
            style: DiagnosticStyle.ViewScript);

        if (bytes == null)
            return FailureResult(error!, "compile", startedAt);

        return SuccessResult(bytes, assemblyName, startedAt);
    }

    public JsonNode HandleCompileRunStates(JsonNode? @params)
    {
        var request = Deserialize<CompileRunStatesRequestDto>(@params);
        long startedAt = Environment.TickCount64;

        string? validationError = RunStatesSource.ValidateRunStatesRequest(request.Request);
        if (validationError != null)
            return FailureResult(validationError, "validation", startedAt);

        string source = RunStatesSource.BuildRunStatesSource(request.Request!);
        string assemblyName = NextAssemblyName("RunStates", request.Params?.DomainGeneration);

        var (bytes, error) = CompileWrappedSource(
            assemblyName,
            source,
            RunStatesSource.SourcePath,
            request.Params,
            request.ReferenceSessionImages);

        if (bytes == null)
            return FailureResult(error!, "compile", startedAt);

        if (request.RegisterImage && !string.IsNullOrEmpty(request.Params?.DomainGeneration))
            _imageRegistry.Register(request.Params!.DomainGeneration!, assemblyName, bytes);

        JsonNode result = SuccessResult(bytes, assemblyName, startedAt);
        result["entryType"] = RunStatesSource.FullHostTypeName;
        return result;
    }

    // ── snippet helpers ──────────────────────────────────────────────

    private (byte[]? Bytes, string? AssemblyName, string? Error) CompileSnippetAttempt(
        CompileSnippetRequestDto request,
        string leadingUsings,
        string bodyCode,
        bool expressionMode)
    {
        string source = UnitySnippetSource.BuildAsyncSnippetSource(
            UnitySnippetSource.HostTypeName, leadingUsings, bodyCode, expressionMode);
        string assemblyName = NextAssemblyName("Snippet", request.Params?.DomainGeneration);

        var (bytes, error) = CompileWrappedSource(
            assemblyName,
            source,
            UnitySnippetSource.SourcePath,
            request.Params,
            request.ReferenceSessionImages);

        return (bytes, bytes != null ? assemblyName : null, error);
    }

    private JsonNode SnippetSuccessResult(
        byte[] bytes,
        string assemblyName,
        string mode,
        CompileSnippetRequestDto request,
        long startedAt)
    {
        if (request.RegisterImage && !string.IsNullOrEmpty(request.Params?.DomainGeneration))
            _imageRegistry.Register(request.Params!.DomainGeneration!, assemblyName, bytes);

        JsonNode result = SuccessResult(bytes, assemblyName, startedAt);
        result["entryType"] = UnitySnippetSource.FullHostTypeName;
        result["mode"] = mode;
        return result;
    }

    // ── compilation core ─────────────────────────────────────────────

    /// <summary>
    /// Compile one generated wrapper source. Error strings mirror the
    /// Unity-side TryCompileAsyncSnippet / CompileRunStates stages
    /// ("parse failed:", "emit failed:", diagnostic text).
    /// </summary>
    private (byte[]? Bytes, string? Error) CompileWrappedSource(
        string assemblyName,
        string source,
        string sourcePath,
        CompileParamsDto? compileParams,
        bool referenceSessionImages)
    {
        CSharpParseOptions parseOptions = ResolveParseOptions(compileParams);

        SyntaxTree syntaxTree;
        try
        {
            syntaxTree = CSharpSyntaxTree.ParseText(
                source,
                parseOptions,
                path: sourcePath,
                encoding: Utf8NoBom);
        }
        catch (Exception ex)
        {
            return (null, "parse failed: " + ex);
        }

        return EmitCompilation(assemblyName, new[] { syntaxTree }, compileParams, useHostBcl: false, referenceSessionImages);
    }

    private (byte[]? Bytes, string? Error) CompileSources(
        string assemblyName,
        IReadOnlyList<(string Path, string Text)> sources,
        CompileParamsDto? compileParams,
        bool useHostBcl,
        bool referenceSessionImages)
    {
        CSharpParseOptions parseOptions = ResolveParseOptions(compileParams);

        var trees = new List<SyntaxTree>(sources.Count);
        try
        {
            foreach (var (path, text) in sources)
            {
                trees.Add(CSharpSyntaxTree.ParseText(
                    text,
                    parseOptions,
                    path: path,
                    encoding: Utf8NoBom));
            }
        }
        catch (Exception ex)
        {
            return (null, "parse failed: " + ex);
        }

        return EmitCompilation(assemblyName, trees, compileParams, useHostBcl, referenceSessionImages);
    }

    /// <summary>Which Unity-side error formatting a compile mirrors.</summary>
    private enum DiagnosticStyle
    {
        /// <summary>execute/run_states: BuildDiagnosticErrorText, full exception text.</summary>
        Snippet,
        /// <summary>View Scripts: path-qualified diagnostics, exception .Message only.</summary>
        ViewScript,
    }

    private (byte[]? Bytes, string? Error) EmitCompilation(
        string assemblyName,
        IReadOnlyList<SyntaxTree> trees,
        CompileParamsDto? compileParams,
        bool useHostBcl,
        bool referenceSessionImages,
        DiagnosticStyle style = DiagnosticStyle.Snippet)
    {
        ImmutableArray<MetadataReference> references = ResolveReferences(compileParams, useHostBcl);
        if (referenceSessionImages)
        {
            var images = _imageRegistry.ReferencesFor(compileParams?.DomainGeneration);
            if (images.Count > 0)
                references = references.AddRange(images);
        }

        CSharpCompilation compilation = CSharpCompilation.Create(
            assemblyName: assemblyName,
            syntaxTrees: trees,
            references: references,
            options: SnippetCompilationOptions);

        using var peStream = new MemoryStream(64 * 1024);
        EmitResult emitResult;
        try
        {
            emitResult = compilation.Emit(peStream, options: SnippetEmitOptions);
        }
        catch (Exception ex)
        {
            return (null, "emit failed: " + (style == DiagnosticStyle.ViewScript ? ex.Message : ex.ToString()));
        }

        if (!emitResult.Success)
        {
            if (style == DiagnosticStyle.ViewScript)
                return (null, DiagnosticText.BuildViewScriptDiagnosticErrorText(emitResult.Diagnostics));

            string? text = DiagnosticText.BuildDiagnosticErrorText(emitResult.Diagnostics);
            return (null, text ?? "unknown compilation failure");
        }

        return (peStream.ToArray(), null);
    }

    // ── references / options ─────────────────────────────────────────

    private CSharpParseOptions ResolveParseOptions(CompileParamsDto? compileParams)
    {
        if (compileParams?.Fingerprint != null &&
            string.Equals(compileParams.Fingerprint, _cachedFingerprint, StringComparison.Ordinal))
        {
            return _cachedParseOptions;
        }

        return BuildParseOptions(compileParams);
    }

    private static CSharpParseOptions BuildParseOptions(CompileParamsDto? compileParams)
    {
        LanguageVersion langVersion = LanguageVersion.CSharp9;
        string? requested = compileParams?.LangVersion;
        if (!string.IsNullOrWhiteSpace(requested) &&
            !LanguageVersionFacts.TryParse(requested.Trim(), out langVersion))
        {
            langVersion = LanguageVersion.CSharp9;
        }

        return new CSharpParseOptions(
            languageVersion: langVersion,
            documentationMode: DocumentationMode.None,
            kind: SourceCodeKind.Regular,
            preprocessorSymbols: compileParams?.Defines ?? Array.Empty<string>());
    }

    private static CSharpParseOptions DefaultParseOptions(string[] defines)
    {
        return new CSharpParseOptions(
            languageVersion: LanguageVersion.CSharp9,
            documentationMode: DocumentationMode.None,
            kind: SourceCodeKind.Regular,
            preprocessorSymbols: defines);
    }

    private ImmutableArray<MetadataReference> ResolveReferences(
        CompileParamsDto? compileParams,
        bool useHostBcl)
    {
        if (useHostBcl)
            return HostBclReferences.Value;

        string[] paths = compileParams?.ReferencePaths ?? Array.Empty<string>();
        if (paths.Length == 0)
            throw new RpcInvalidParamsException("compile params with referencePaths are required (or set useHostBcl)");

        string? fingerprint = compileParams?.Fingerprint;
        if (fingerprint != null &&
            string.Equals(fingerprint, _cachedFingerprint, StringComparison.Ordinal) &&
            !_cachedReferences.IsDefault)
        {
            return _cachedReferences;
        }

        var references = ImmutableArray.CreateBuilder<MetadataReference>(paths.Length);
        foreach (string path in paths)
        {
            PortableExecutableReference? reference = _referenceCache.GetOrCreate(path);
            if (reference != null)
                references.Add(reference);
        }
        _referenceCache.PruneExcept(paths);

        var resolved = references.ToImmutable();
        if (fingerprint != null)
        {
            _cachedFingerprint = fingerprint;
            _cachedReferences = resolved;
            _cachedParseOptions = BuildParseOptions(compileParams);
        }
        return resolved;
    }

    private static ImmutableArray<MetadataReference> BuildHostBclReferences()
    {
        var builder = ImmutableArray.CreateBuilder<MetadataReference>();
        if (AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES") is string tpa)
        {
            foreach (string path in tpa.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries))
            {
                try
                {
                    if (File.Exists(path))
                        builder.Add(MetadataReference.CreateFromFile(path));
                }
                catch
                {
                }
            }
        }
        return builder.ToImmutable();
    }

    // ── results / misc ───────────────────────────────────────────────

    /// <summary>Port of LocusBridge.ViewScripts.cs SanitizeAssemblyNamePart.</summary>
    private static string SanitizeAssemblyNamePart(string? value)
    {
        if (string.IsNullOrEmpty(value))
            return "Script";

        var sb = new StringBuilder(value!.Length);
        for (int i = 0; i < value.Length; i++)
        {
            char ch = value[i];
            sb.Append(char.IsLetterOrDigit(ch) ? ch : '_');
        }
        return sb.Length == 0 ? "Script" : sb.ToString();
    }

    private string NextAssemblyName(string kind, string? domainGeneration)
    {
        string gen8 = "00000000";
        if (!string.IsNullOrEmpty(domainGeneration))
        {
            string compact = new(domainGeneration.Where(char.IsLetterOrDigit).ToArray());
            if (compact.Length > 0)
                gen8 = compact.Length >= 8 ? compact[..8] : compact.PadRight(8, '0');
        }

        int counter = Interlocked.Increment(ref _assemblyCounter);
        return $"__Locus{kind}_{gen8}_{counter:X8}";
    }

    private static JsonNode SuccessResult(byte[] bytes, string assemblyName, long startedAt)
    {
        return new JsonObject
        {
            ["success"] = true,
            ["assemblyName"] = assemblyName,
            ["assemblyB64"] = Convert.ToBase64String(bytes),
            ["durationMs"] = Environment.TickCount64 - startedAt,
        };
    }

    private static JsonNode FailureResult(string error, string stage, long startedAt)
    {
        return new JsonObject
        {
            ["success"] = false,
            ["error"] = error,
            ["errorStage"] = stage,
            ["durationMs"] = Environment.TickCount64 - startedAt,
        };
    }

    private static T Deserialize<T>(JsonNode? @params) where T : new()
    {
        if (@params == null)
            throw new RpcInvalidParamsException("params object is required");
        try
        {
            return @params.Deserialize<T>() ?? new T();
        }
        catch (JsonException ex)
        {
            throw new RpcInvalidParamsException("invalid params: " + ex.Message);
        }
    }
}
