using System.Collections.Immutable;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
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

    /// <summary>Project-level "Allow unsafe code" (any script assembly with
    /// AllowUnsafeCode on). Hot patches follow it (B4); absent = false, so
    /// older Unity plugins keep the previous behavior.</summary>
    [JsonPropertyName("allowUnsafe")]
    public bool AllowUnsafe { get; set; }
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

public sealed class HotDiffFileDto
{
    [JsonPropertyName("path")]
    public string? Path { get; set; }

    [JsonPropertyName("oldText")]
    public string? OldText { get; set; }

    [JsonPropertyName("newText")]
    public string? NewText { get; set; }
}

public sealed class AnalyzeHotDiffRequestDto
{
    [JsonPropertyName("files")]
    public HotDiffFileDto[]? Files { get; set; }

    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }
}

public sealed class IndexTypesRequestDto
{
    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }
}

public sealed class RegisterImageRequestDto
{
    [JsonPropertyName("domainGeneration")]
    public string? DomainGeneration { get; set; }

    [JsonPropertyName("assemblyName")]
    public string? AssemblyName { get; set; }

    [JsonPropertyName("assemblyB64")]
    public string? AssemblyB64 { get; set; }
}

public sealed class CompileHotPatchRequestDto
{
    [JsonPropertyName("files")]
    public HotDiffFileDto[]? Files { get; set; }

    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }

    [JsonPropertyName("referenceSessionImages")]
    public bool ReferenceSessionImages { get; set; } = true;

    [JsonPropertyName("registerImage")]
    public bool RegisterImage { get; set; } = true;

    /// <summary>Extra reference DLLs for THIS compile only (the plugin's
    /// Locus.HotReload.Runtime.dll for field stores). Kept out of `params`
    /// so the fingerprint-keyed reference cache stays untouched.</summary>
    [JsonPropertyName("extraReferencePaths")]
    public string[]? ExtraReferencePaths { get; set; }

    /// <summary>C0 runtime capability matrix, measured per domain generation
    /// by the desktop coordinator. Absent = conservative (all false). Kept
    /// out of `params` for the same fingerprint reason as the extra
    /// references. Echoed back in the verdict (link proof); C2′a gates the
    /// added-member non-public BODY access relaxation on the cell matrix
    /// (PatchRewriter.FindShimAccessViolation).</summary>
    [JsonPropertyName("runtimeCaps")]
    public RuntimeCapsDto? RuntimeCaps { get; set; }

    /// <summary>B6: candidate sibling part files for the partial types the
    /// edited files declare, discovered by the desktop coordinator with
    /// grep-grade matching. The sidecar parses each candidate and folds in
    /// ONLY the files that really declare a matching partial type, as
    /// UNCHANGED baselines (they complete the patch copies and the layout
    /// merge; they never produce detours or new surface). Optional — an old
    /// coordinator simply never sends it, and partial batches then fail
    /// closed through the completeness gate.</summary>
    [JsonPropertyName("baselineSiblings")]
    public BaselineSiblingDto[]? BaselineSiblings { get; set; }
}

/// <summary>One candidate sibling part file (B6): current disk text only —
/// by definition it carries no edit.</summary>
public sealed class BaselineSiblingDto
{
    [JsonPropertyName("path")]
    public string? Path { get; set; }

    [JsonPropertyName("text")]
    public string? Text { get; set; }
}

/// <summary>Unity Mono runtime capability matrix measured by the C0 access
/// probe (`compile/accessProbe` + the plugin's `hot_reload_access_probe`
/// message). Every field defaults to false = conservative: treat every
/// non-public access as cold, which is today's behavior.</summary>
public sealed class RuntimeCapsDto
{
    /// <summary>Delegate.CreateDelegate bound a non-public method AND the
    /// invocation returned the right value.</summary>
    [JsonPropertyName("createDelegateNonPublic")]
    public bool CreateDelegateNonPublic { get; set; }

    /// <summary>DynamicMethod(restrictedSkipVisibility: true) read a private
    /// field of another type successfully.</summary>
    [JsonPropertyName("dynamicMethodSkipVisibility")]
    public bool DynamicMethodSkipVisibility { get; set; }

    /// <summary>A byref-returning DynamicMethod (ldflda) round-tripped a
    /// read/write through the returned reference.</summary>
    [JsonPropertyName("dynamicMethodByrefReturn")]
    public bool DynamicMethodByrefReturn { get; set; }

    /// <summary>"{op}_{visibility}" (e.g. "ldfld_private") → the JIT-time
    /// access check passed on the running editor's Mono.</summary>
    [JsonPropertyName("cells")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public Dictionary<string, bool>? Cells { get; set; }
}

public sealed class CompileAccessProbeRequestDto
{
    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }
}

// ── service ──────────────────────────────────────────────────────────

public sealed class CompileService
{
    public const int ProtocolVersion = 3;

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
    private readonly MemberSurfaceRegistry _memberSurfaceRegistry = new();
    private readonly FieldStoreRegistry _fieldStoreRegistry = new();

    // Registrations of compiled-but-not-yet-accepted hot patches, keyed by
    // assembly name; committed into the registries when image/register
    // confirms Unity loaded the patch.
    private readonly object _pendingShimLock = new();
    private readonly Dictionary<string, (string Generation, List<ShimRegistration> Shims, List<FieldStoreRegistration> FieldStores)> _pendingShims =
        new(StringComparer.Ordinal);

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

    public JsonNode HandleRegisterImage(JsonNode? @params)
    {
        var request = Deserialize<RegisterImageRequestDto>(@params);
        if (string.IsNullOrWhiteSpace(request.DomainGeneration))
            throw new RpcInvalidParamsException("image/register requires domainGeneration");
        if (string.IsNullOrWhiteSpace(request.AssemblyName))
            throw new RpcInvalidParamsException("image/register requires assemblyName");
        if (string.IsNullOrWhiteSpace(request.AssemblyB64))
            throw new RpcInvalidParamsException("image/register requires assemblyB64");

        byte[] bytes;
        try
        {
            bytes = Convert.FromBase64String(request.AssemblyB64!);
        }
        catch (FormatException ex)
        {
            throw new RpcInvalidParamsException("image/register assemblyB64 is invalid: " + ex.Message);
        }

        _imageRegistry.Register(request.DomainGeneration!, request.AssemblyName!, bytes);

        // A hot patch registered after Unity acceptance: its new-surface
        // shims and field stores become visible to later batches now.
        (string Generation, List<ShimRegistration> Shims, List<FieldStoreRegistration> FieldStores) pending = default;
        lock (_pendingShimLock)
        {
            if (_pendingShims.TryGetValue(request.AssemblyName!, out pending!))
                _pendingShims.Remove(request.AssemblyName!);
        }
        if (pending.Shims != null &&
            string.Equals(pending.Generation, request.DomainGeneration, StringComparison.Ordinal))
        {
            CommitShimRegistrations(pending.Generation, pending.Shims);
            CommitFieldStoreRegistrations(pending.Generation, pending.FieldStores);
        }

        return new JsonObject
        {
            ["success"] = true,
        };
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

    // ── hot reload ───────────────────────────────────────────────────

    /// <summary>
    /// Classify edited files for the hot path. Pure analysis: parse with the
    /// project's real defines/langversion, then a member-level diff (see
    /// HotDiff). No compilation happens here.
    /// </summary>
    public JsonNode HandleAnalyzeHotDiff(JsonNode? @params)
    {
        var request = Deserialize<AnalyzeHotDiffRequestDto>(@params);
        if (request.Files == null || request.Files.Length == 0)
            throw new RpcInvalidParamsException("analyze/hotDiff requires at least one file");

        CSharpParseOptions parseOptions = ResolveParseOptions(request.Params);

        bool allHot = true;
        var files = new JsonArray();
        foreach (HotDiffFileDto file in request.Files)
        {
            if (string.IsNullOrEmpty(file.Path) || file.OldText == null || file.NewText == null)
                throw new RpcInvalidParamsException("analyze/hotDiff files require path, oldText and newText");

            HotDiffFileResult diff = HotDiff.Analyze(file.OldText, file.NewText, parseOptions);
            allHot &= diff.Hot;
            files.Add(HotDiffFileJson(file.Path!, diff));
        }

        return new JsonObject
        {
            ["hot"] = allHot,
            ["files"] = files,
        };
    }

    private static JsonObject HotDiffFileJson(string path, HotDiffFileResult diff)
    {
        var methods = new JsonArray();
        foreach (HotDiffMethod method in diff.ChangedMethods)
            methods.Add(HotDiffMethodJson(method));

        var json = new JsonObject
        {
            ["path"] = path,
            ["hot"] = diff.Hot,
            ["reasons"] = new JsonArray(diff.Reasons.Select(r => (JsonNode)r).ToArray()),
            ["changedMethods"] = methods,
            ["newTypes"] = new JsonArray(diff.NewTypes.Select(t => (JsonNode)t).ToArray()),
            ["patchedTypes"] = new JsonArray(diff.PatchedTypes.Select(t => (JsonNode)t).ToArray()),
        };
        if (diff.RequiresCallerCheck.Count > 0)
        {
            json["requiresCallerCheck"] = new JsonArray(diff.RequiresCallerCheck
                .Select(m => (JsonNode)new JsonObject
                {
                    ["declaringType"] = m.DeclaringType,
                    ["name"] = m.Name,
                    ["paramTypeNames"] = new JsonArray(m.ParamTypeNames.Select(p => (JsonNode)p).ToArray()),
                    ["kind"] = m.Kind,
                    ["detail"] = m.Detail,
                })
                .ToArray());
        }
        if (diff.SyntaxError != null)
            json["syntaxError"] = diff.SyntaxError;
        return json;
    }

    private static JsonObject HotDiffMethodJson(HotDiffMethod method)
    {
        return new JsonObject
        {
            ["declaringType"] = method.DeclaringType,
            ["name"] = method.Name,
            ["paramTypeNames"] = new JsonArray(method.ParamTypeNames.Select(p => (JsonNode)p).ToArray()),
            ["isStatic"] = method.IsStatic,
            ["isCtor"] = method.IsCtor,
            ["added"] = method.Added,
        };
    }

    /// <summary>
    /// Full hot-patch pipeline: diff every file (same classification as
    /// analyze/hotDiff), rewrite the hot ones (PatchRewriter), and compile a
    /// single patch assembly with accessibility checks suppressed (the patch
    /// legitimately touches the original assembly's private members).
    ///
    /// Response shapes:
    ///   cold     → { hot: false, files: [{path, hot, reasons}] }
    ///   no-op    → { hot: true, success: true, noop: true }
    ///   compiled → { hot: true, success: true, assemblyName/B64, methods, newTypes }
    ///   error    → { hot: true, success: false, error, errorStage } (deterministic, agent-facing)
    /// </summary>
    public JsonNode HandleCompileHotPatch(JsonNode? @params)
    {
        var request = Deserialize<CompileHotPatchRequestDto>(@params);
        if (request.Files == null || request.Files.Length == 0)
            throw new RpcInvalidParamsException("compile/hotPatch requires at least one file");

        long startedAt = Environment.TickCount64;
        CSharpParseOptions parseOptions = ResolveParseOptions(request.Params);
        ImmutableArray<MetadataReference> references = ResolveReferences(request.Params, useHostBcl: false);
        if (request.ReferenceSessionImages)
        {
            var images = _imageRegistry.ReferencesFor(request.Params?.DomainGeneration);
            if (images.Count > 0)
                references = references.AddRange(images);
        }
        foreach (string extraPath in request.ExtraReferencePaths ?? Array.Empty<string>())
        {
            try
            {
                if (File.Exists(extraPath))
                    references = references.Add(MetadataReference.CreateFromFile(extraPath));
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine("[LocusCompileServer] extra reference skipped: " + extraPath + ": " + ex.Message);
            }
        }

        var diffs = new List<(HotDiffFileDto File, HotDiffFileResult Diff)>(request.Files.Length);
        var syntaxErrors = new StringBuilder();
        var coldFiles = new JsonArray();
        bool anyCold = false;

        foreach (HotDiffFileDto file in request.Files)
        {
            if (string.IsNullOrEmpty(file.Path) || file.OldText == null || file.NewText == null)
                throw new RpcInvalidParamsException("compile/hotPatch files require path, oldText and newText");

            HotDiffFileResult diff = HotDiff.Analyze(file.OldText, file.NewText, parseOptions);
            if (diff.SyntaxError != null)
            {
                if (syntaxErrors.Length > 0)
                    syntaxErrors.Append('\n');
                syntaxErrors.Append(diff.SyntaxError);
                continue;
            }
            if (!diff.Hot)
            {
                anyCold = true;
                coldFiles.Add(HotDiffFileJson(file.Path!, diff));
                continue;
            }
            diffs.Add((file, diff));
        }

        if (syntaxErrors.Length > 0)
        {
            JsonNode failure = FailureResult(syntaxErrors.ToString(), "compile", startedAt);
            failure["hot"] = true;
            return failure;
        }

        if (anyCold)
        {
            return new JsonObject
            {
                ["hot"] = false,
                ["files"] = coldFiles,
                ["durationMs"] = Environment.TickCount64 - startedAt,
            };
        }

        var trees = new List<SyntaxTree>();
        var methods = new JsonArray();
        var newTypes = new JsonArray();
        var accessAssemblies = new HashSet<string>(StringComparer.Ordinal);
        var shimRegistrations = new List<ShimRegistration>();
        var fieldStoreRegistrations = new List<FieldStoreRegistration>();

        // M1: ONE binding compilation over the whole batch's un-renamed
        // trees, so cross-file references — including calls to members added
        // in another file of this batch — bind symbolically.
        var batchFiles = new List<(string Path, SyntaxTree Tree, HotDiffFileResult Diff)>();
        foreach (var (file, diff) in diffs)
        {
            if (diff.ChangedMethods.Count == 0 && diff.NewTypes.Count == 0 &&
                diff.RemovedMembers.Count == 0 && diff.RemovedTypes.Count == 0 &&
                diff.EnumAdditions.Count == 0 && diff.RequiresCallerCheck.Count == 0)
            {
                continue; // formatting/comment-only edit: nothing to patch
            }
            SyntaxTree tree = CSharpSyntaxTree.ParseText(file.NewText!, parseOptions, path: file.Path!);
            batchFiles.Add((file.Path!, tree, diff));
        }

        // B6: partial types. (1) fold the matching sibling part files in as
        // unchanged baselines, so the patch re-declares the COMPLETE type;
        // (2) order the trees so the multi-part field merge can reproduce
        // the original assembly's layout (the rewriter's guard still
        // verifies — this pass only avoids false colds).
        {
            JsonNode? siblingCold = IncludeBaselineSiblings(request, batchFiles, parseOptions, startedAt);
            if (siblingCold != null)
                return siblingCold;
            OrderBatchFilesForPartialLayout(batchFiles, references);
        }

        // M3: deletions / signature changes / accessibility narrowing are
        // hot ONLY when every compiled call site of the OLD surface lives in
        // this batch — scan the project assemblies' IL and fold the verdict
        // into hot/cold (with the exact uncovered files in the reasons).
        string? callerScanNote = null;
        {
            var checks = new List<(string File, CallerCheckMember Check)>();
            foreach (var (filePath, _, diff) in batchFiles)
            {
                foreach (CallerCheckMember check in diff.RequiresCallerCheck)
                    checks.Add((filePath, check));
            }

            if (checks.Count > 0)
            {
                JsonNode? coldVerdict = RunCallerScan(request, batchFiles, checks, startedAt, out callerScanNote);
                if (coldVerdict != null)
                    return coldVerdict;
            }
        }

        string? generation = request.Params?.DomainGeneration;
        string assemblyName = NextAssemblyName("HotPatch", request.Params?.DomainGeneration);
        // New field-store holders take the assembly's unique counter as a
        // name suffix: a later batch adding fields to the SAME type must not
        // declare a holder with the same name as an earlier batch's (its
        // source declaration would shadow the metadata type that this
        // batch's re-sent earlier fields still bind to — CS0117).
        string storeDiscriminator = assemblyName[(assemblyName.LastIndexOf('_') + 1)..];
        PatchBatchContext batch = PatchBatchContext.Build(
            batchFiles, references,
            _memberSurfaceRegistry.SnapshotFor(generation),
            _fieldStoreRegistry.SnapshotFor(generation),
            storeDiscriminator,
            allowUnsafe: request.Params?.AllowUnsafe ?? false,
            runtimeCaps: AccessCaps.FromCells(request.RuntimeCaps?.Cells));

        // B6 fail-closed gate: every batch-declared partial type must
        // account, across its disk parts, for every member the ORIGINAL
        // assembly's type carries — a member with no source means a
        // source-generator part or an undiscovered sibling file.
        {
            JsonNode? incompleteCold = VerifyPartialPartsComplete(batch, batchFiles, startedAt);
            if (incompleteCold != null)
                return incompleteCold;
        }

        foreach (var (filePath, tree, diff) in batchFiles)
        {
            PatchRewriteResult rewrite = PatchRewriter.Rewrite(
                filePath, tree, diff, parseOptions, batch);

            if (rewrite.Error != null)
            {
                JsonNode failure = FailureResult(rewrite.Error, "rewrite", startedAt);
                failure["hot"] = true;
                return failure;
            }
            if (rewrite.ColdReason != null)
            {
                var fileJson = new JsonObject
                {
                    ["path"] = filePath,
                    ["hot"] = false,
                    ["reasons"] = new JsonArray(rewrite.ColdReason),
                };
                return new JsonObject
                {
                    ["hot"] = false,
                    ["files"] = new JsonArray(fileJson),
                    ["durationMs"] = Environment.TickCount64 - startedAt,
                };
            }

            trees.Add(rewrite.Tree!);
            foreach (string assembly in rewrite.OriginalAssemblies)
                accessAssemblies.Add(assembly);
            shimRegistrations.AddRange(rewrite.ShimRegistrations);
            fieldStoreRegistrations.AddRange(rewrite.FieldStoreRegistrations);

            foreach (PatchMethodMap map in rewrite.Methods)
            {
                var method = new JsonObject
                {
                    ["declaringType"] = map.DeclaringType,
                    ["patchDeclaringType"] = map.PatchDeclaringType,
                    ["name"] = map.Name,
                    ["paramTypeNames"] = new JsonArray(map.ParamTypeNames.Select(p => (JsonNode)p).ToArray()),
                    ["isStatic"] = map.IsStatic,
                    ["isCtor"] = map.IsCtor,
                };
                if (map.OriginalAssembly != null)
                    method["originalAssembly"] = map.OriginalAssembly;
                if (map.IsStub)
                    method["isStub"] = true;
                methods.Add(method);
            }
            foreach (PatchNewType newType in rewrite.NewTypes)
            {
                newTypes.Add(new JsonObject
                {
                    ["metadataName"] = newType.MetadataName,
                    ["ns"] = newType.Namespace,
                    ["simpleName"] = newType.SimpleName,
                    ["isPublic"] = newType.IsPublic,
                    ["isTopLevel"] = newType.IsTopLevel,
                });
            }
        }

        if (methods.Count == 0 && newTypes.Count == 0 &&
            fieldStoreRegistrations.Count == 0 &&
            shimRegistrations.All(r => r.Entry.Kind == "tombstone"))
        {
            // Nothing to detour and nothing new to load: pure deletions
            // (non-magic member removals — the loaded code is already
            // correct, the members are merely unreachable) and/or pure
            // accessibility narrowing. Commit tombstones so later batches
            // fail deterministically on references; skip the pointless
            // assembly.
            if (!string.IsNullOrEmpty(generation))
                CommitShimRegistrations(generation!, shimRegistrations);
            var verdict = new JsonObject
            {
                ["hot"] = true,
                ["success"] = true,
                ["noop"] = true,
                ["durationMs"] = Environment.TickCount64 - startedAt,
            };
            if (shimRegistrations.Count > 0)
                verdict["deletionsNoted"] = shimRegistrations.Count;
            if (callerScanNote != null)
                verdict["callerScan"] = callerScanNote;
            if (request.RuntimeCaps != null)
                verdict["runtimeCaps"] = JsonSerializer.SerializeToNode(request.RuntimeCaps);
            return verdict;
        }

        // Patched bodies may touch internals of any assembly the original
        // file could (its own assembly plus InternalsVisibleTo friends);
        // suppressing checks for every reference is the safe superset.
        foreach (string name in ReferenceAssemblyNames(references))
            accessAssemblies.Add(name);
        trees.Add(BuildAccessChecksTree(accessAssemblies, parseOptions));

        CSharpCompilationOptions options = SnippetCompilationOptions
            .WithMetadataImportOptions(MetadataImportOptions.All)
            .WithAllowUnsafe(request.Params?.AllowUnsafe ?? false);
        ApplyIgnoreAccessibility(options);

        CSharpCompilation compilation = CSharpCompilation.Create(
            assemblyName: assemblyName,
            syntaxTrees: trees,
            references: references,
            options: options);

        using var peStream = new MemoryStream(128 * 1024);
        EmitResult emitResult;
        try
        {
            emitResult = compilation.Emit(peStream, options: SnippetEmitOptions);
        }
        catch (Exception ex)
        {
            JsonNode failure = FailureResult("emit failed: " + ex, "compile", startedAt);
            failure["hot"] = true;
            return failure;
        }

        if (!emitResult.Success)
        {
            string? text = DiagnosticText.BuildDiagnosticErrorText(emitResult.Diagnostics);
            JsonNode failure = FailureResult(text ?? "unknown compilation failure", "compile", startedAt);
            failure["hot"] = true;
            return failure;
        }

        byte[] bytes = peStream.ToArray();

        // New-surface bookkeeping (M2/M4): shims and field stores become
        // visible to later batches only after the patch is actually live in
        // Unity — which is signaled by the image registration (the Rust
        // side registers via image/register after Unity accepts; tests
        // register inline).
        foreach (ShimRegistration registration in shimRegistrations)
            registration.Entry.ShimAssembly = assemblyName;
        foreach (FieldStoreRegistration registration in fieldStoreRegistrations)
            registration.Entry.StoreAssembly = assemblyName;

        if (request.RegisterImage && !string.IsNullOrEmpty(generation))
        {
            _imageRegistry.Register(generation!, assemblyName, bytes);
            CommitShimRegistrations(generation!, shimRegistrations);
            CommitFieldStoreRegistrations(generation!, fieldStoreRegistrations);
        }
        else if ((shimRegistrations.Count > 0 || fieldStoreRegistrations.Count > 0) &&
                 !string.IsNullOrEmpty(generation))
        {
            lock (_pendingShimLock)
            {
                _pendingShims[assemblyName] = (generation!, shimRegistrations, fieldStoreRegistrations);
                // Keep the pending map bounded: entries for other
                // generations can never commit.
                foreach (string stale in _pendingShims
                             .Where(p => p.Value.Generation != generation)
                             .Select(p => p.Key)
                             .ToList())
                {
                    _pendingShims.Remove(stale);
                }
            }
        }

        JsonNode result = SuccessResult(bytes, assemblyName, startedAt);
        result["hot"] = true;
        result["methods"] = methods;
        result["newTypes"] = newTypes;
        if (callerScanNote != null)
            result["callerScan"] = callerScanNote;
        if (request.RuntimeCaps != null)
            result["runtimeCaps"] = JsonSerializer.SerializeToNode(request.RuntimeCaps);
        return result;
    }

    /// <summary>
    /// C0: compile the fixed access-probe source (AccessProbeSource) against
    /// the project's reference set, with the same accessibility suppression
    /// as hot patches (IgnoreAccessibility + IgnoresAccessChecksTo tree +
    /// MetadataImportOptions.All; allowUnsafe stays false). No diff/rewrite,
    /// no session images, and NO image registration: the assembly is loaded
    /// once on the Unity side, JIT-probed, and never referenced again. The
    /// assembly name deliberately avoids the __LocusHotPatch_ prefix — it
    /// must not be skipped by the Unity original-type resolution, and it
    /// never enters the patch registries.
    /// </summary>
    public JsonNode HandleCompileAccessProbe(JsonNode? @params)
    {
        var request = Deserialize<CompileAccessProbeRequestDto>(@params);
        long startedAt = Environment.TickCount64;

        CSharpParseOptions parseOptions = ResolveParseOptions(request.Params);
        ImmutableArray<MetadataReference> references = ResolveReferences(request.Params, useHostBcl: false);

        var trees = new List<SyntaxTree>
        {
            CSharpSyntaxTree.ParseText(
                AccessProbeSource.BuildSource(),
                parseOptions,
                path: AccessProbeSource.SourcePath,
                encoding: Utf8NoBom),
            BuildAccessChecksTree(ReferenceAssemblyNames(references), parseOptions),
        };

        string assemblyName = NextAssemblyName("AccessProbe", request.Params?.DomainGeneration);

        CSharpCompilationOptions options = SnippetCompilationOptions
            .WithMetadataImportOptions(MetadataImportOptions.All);
        ApplyIgnoreAccessibility(options);

        CSharpCompilation compilation = CSharpCompilation.Create(
            assemblyName: assemblyName,
            syntaxTrees: trees,
            references: references,
            options: options);

        using var peStream = new MemoryStream(64 * 1024);
        EmitResult emitResult;
        try
        {
            emitResult = compilation.Emit(peStream, options: SnippetEmitOptions);
        }
        catch (Exception ex)
        {
            return FailureResult("emit failed: " + ex, "compile", startedAt);
        }

        if (!emitResult.Success)
        {
            string? text = DiagnosticText.BuildDiagnosticErrorText(emitResult.Diagnostics);
            return FailureResult(text ?? "unknown compilation failure", "compile", startedAt);
        }

        JsonNode result = SuccessResult(peStream.ToArray(), assemblyName, startedAt);
        var cells = new JsonArray();
        foreach (AccessProbeCell cell in AccessProbeSource.Cells)
        {
            cells.Add(new JsonObject
            {
                ["method"] = cell.Method,
                ["op"] = cell.Op,
                ["visibility"] = cell.Visibility,
            });
        }
        result["cells"] = cells;
        return result;
    }

    /// <summary>Run the M3 caller scan for the batch's pending checks.
    /// Returns a cold/error verdict node, or null to proceed (note set).</summary>
    private static JsonNode? RunCallerScan(
        CompileHotPatchRequestDto request,
        List<(string Path, SyntaxTree Tree, HotDiffFileResult Diff)> batchFiles,
        List<(string File, CallerCheckMember Check)> checks,
        long startedAt,
        out string? note)
    {
        note = null;

        var projectAssemblies = (request.Params?.ReferencePaths ?? Array.Empty<string>())
            .Where(CallerScan.IsProjectAssemblyPath)
            .ToList();
        if (projectAssemblies.Count == 0)
        {
            return ColdVerdict(
                checks[0].File,
                "cannot verify call sites: no project assemblies (Library/ScriptAssemblies) in the reference set; use unity_recompile",
                startedAt);
        }

        var targets = new List<CallerScanTarget>();
        foreach (var (_, check) in checks)
        {
            if (check.ScanMemberNames.Length == 0)
            {
                targets.Add(new CallerScanTarget { DeclaringType = check.DeclaringType, MemberName = "" });
                continue;
            }
            foreach (string scanName in check.ScanMemberNames)
                targets.Add(new CallerScanTarget { DeclaringType = check.DeclaringType, MemberName = scanName });
        }

        CallerScanResult scan = CallerScan.Scan(projectAssemblies, targets);
        if (scan.Error != null)
            return ColdVerdict(checks[0].File, scan.Error, startedAt);

        // A caller file is covered when it IS one of the batch files. PDB
        // documents may be project-relative ("Assets/X.cs") while batch
        // paths are absolute — compare by path-segment-anchored suffix.
        bool Covered(string callerFile)
        {
            string caller = callerFile.Replace('\\', '/').TrimStart('/').ToLowerInvariant();
            foreach (var (batchPath, _, _) in batchFiles)
            {
                string batch = batchPath.Replace('\\', '/').TrimStart('/').ToLowerInvariant();
                if (batch == caller ||
                    batch.EndsWith("/" + caller, StringComparison.Ordinal) ||
                    caller.EndsWith("/" + batch, StringComparison.Ordinal))
                {
                    return true;
                }
            }
            return false;
        }

        var coldReasonsByFile = new Dictionary<string, List<string>>(StringComparer.Ordinal);
        foreach (var (checkFile, check) in checks)
        {
            var uncovered = new SortedSet<string>(StringComparer.OrdinalIgnoreCase);
            IEnumerable<string> scanNames = check.ScanMemberNames.Length == 0
                ? new[] { "" }
                : check.ScanMemberNames;
            foreach (string scanName in scanNames)
            {
                if (!scan.CallerFiles.TryGetValue(
                        CallerScanTarget.Key(check.DeclaringType, scanName), out HashSet<string>? files))
                {
                    continue;
                }
                foreach (string file in files)
                {
                    if (!Covered(file))
                        uncovered.Add(file);
                }
            }
            if (uncovered.Count > 0)
            {
                if (!coldReasonsByFile.TryGetValue(checkFile, out List<string>? reasons))
                    coldReasonsByFile[checkFile] = reasons = new List<string>();
                reasons.Add(
                    check.Detail + " is still referenced by " + string.Join(", ", uncovered) +
                    " — edit those call sites in the same batch and retry, or run unity_recompile");
            }
        }

        if (coldReasonsByFile.Count > 0)
        {
            var files = new JsonArray();
            foreach (var pair in coldReasonsByFile)
            {
                files.Add(new JsonObject
                {
                    ["path"] = pair.Key,
                    ["hot"] = false,
                    ["reasons"] = new JsonArray(pair.Value.Select(r => (JsonNode)r).ToArray()),
                });
            }
            return new JsonObject
            {
                ["hot"] = false,
                ["files"] = files,
                ["durationMs"] = Environment.TickCount64 - startedAt,
            };
        }

        note =
            "call sites of " + checks.Count + " changed member surface(s) verified across " +
            projectAssemblies.Count + " project assembly(ies); reflection, SendMessage(string) and " +
            "UnityEvent serialized bindings cannot be verified and only converge at unity_recompile";
        return null;
    }

    private static JsonNode ColdVerdict(string path, string reason, long startedAt)
    {
        return new JsonObject
        {
            ["hot"] = false,
            ["files"] = new JsonArray(new JsonObject
            {
                ["path"] = path,
                ["hot"] = false,
                ["reasons"] = new JsonArray(reason),
            }),
            ["durationMs"] = Environment.TickCount64 - startedAt,
        };
    }

    // ── B6: partial types (sibling parts, layout order, completeness) ──

    /// <summary>Fold the candidate sibling part files into the batch as
    /// UNCHANGED baselines (empty diff: no detours, no new surface — they
    /// only complete the patch copies and the member/layout merge). The
    /// coordinator's candidates are grep-grade; only files that really
    /// declare a partial type matching one the batch needs are kept, to a
    /// fixpoint (a sibling can itself declare further partial types whose
    /// parts must come along). A MATCHING sibling that does not parse fails
    /// the batch closed: its part is needed and cannot be trusted.</summary>
    private static JsonNode? IncludeBaselineSiblings(
        CompileHotPatchRequestDto request,
        List<(string Path, SyntaxTree Tree, HotDiffFileResult Diff)> batchFiles,
        CSharpParseOptions parseOptions,
        long startedAt)
    {
        if (request.BaselineSiblings == null || request.BaselineSiblings.Length == 0 || batchFiles.Count == 0)
            return null;

        var needed = new HashSet<string>(StringComparer.Ordinal);
        foreach (var (_, tree, _) in batchFiles)
            CollectPartialTypeNames((CompilationUnitSyntax)tree.GetRoot(), needed);
        if (needed.Count == 0)
            return null;

        var inBatch = new HashSet<string>(
            batchFiles.Select(f => NormalizePathKey(f.Path)), StringComparer.Ordinal);
        var candidates = new List<(string Path, SyntaxTree Tree, HashSet<string> PartialNames)>();
        foreach (BaselineSiblingDto sibling in request.BaselineSiblings)
        {
            if (string.IsNullOrEmpty(sibling.Path) || sibling.Text == null)
                continue;
            if (!inBatch.Add(NormalizePathKey(sibling.Path!)))
                continue; // already an edited batch file (or a duplicate)
            SyntaxTree tree = CSharpSyntaxTree.ParseText(sibling.Text!, parseOptions, path: sibling.Path!);
            var names = new HashSet<string>(StringComparer.Ordinal);
            CollectPartialTypeNames((CompilationUnitSyntax)tree.GetRoot(), names);
            if (names.Count > 0)
                candidates.Add((sibling.Path!, tree, names));
        }

        bool folded = true;
        while (folded)
        {
            folded = false;
            for (int i = candidates.Count - 1; i >= 0; i--)
            {
                var (path, tree, names) = candidates[i];
                if (!names.Overlaps(needed))
                    continue;
                if (tree.GetDiagnostics().Any(d => d.Severity == DiagnosticSeverity.Error))
                {
                    return ColdVerdict(
                        path,
                        "partial sibling part does not parse: " + path +
                        " (fix the file or use unity_recompile)",
                        startedAt);
                }
                batchFiles.Add((path, tree, new HotDiffFileResult { Hot = true }));
                needed.UnionWith(names);
                candidates.RemoveAt(i);
                folded = true;
            }
        }
        return null;
    }

    private static void CollectPartialTypeNames(CompilationUnitSyntax root, HashSet<string> names)
    {
        foreach (TypeDeclarationSyntax decl in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
        {
            if (decl.Modifiers.Any(SyntaxKind.PartialKeyword))
                names.Add(HotDiff.MetadataName(decl));
        }
    }

    private static string NormalizePathKey(string path) =>
        path.Replace('\\', '/').ToLowerInvariant();

    /// <summary>When a partial type's instance fields are split across
    /// SEVERAL batch files, the patch type's field order is the source-merge
    /// order — i.e. the tree order of the part files. Reorder the batch
    /// (stable; constraints only between files contributing fields to the
    /// same partial type, ranked by where their fields sit in the ORIGINAL
    /// assembly's sequence) so the merge can match the original layout. The
    /// rewriter's layout guard still VERIFIES the result and fails closed on
    /// any mismatch — this pass exists purely to avoid false colds.
    /// Conflicting or cyclic constraints keep the natural order (the guard
    /// then decides).</summary>
    private static void OrderBatchFilesForPartialLayout(
        List<(string Path, SyntaxTree Tree, HotDiffFileResult Diff)> batchFiles,
        ImmutableArray<MetadataReference> references)
    {
        if (batchFiles.Count < 2)
            return;

        // type → the batch files declaring its instance fields (file index,
        // field-ish names in that file's source order).
        var fieldOwners = new Dictionary<string, List<(int FileIndex, List<string> Names)>>(StringComparer.Ordinal);
        for (int i = 0; i < batchFiles.Count; i++)
        {
            var root = (CompilationUnitSyntax)batchFiles[i].Tree.GetRoot();
            foreach (TypeDeclarationSyntax decl in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
            {
                if (!decl.Modifiers.Any(SyntaxKind.PartialKeyword))
                    continue;
                List<string> names = InstanceFieldishNames(decl);
                if (names.Count == 0)
                    continue;
                string metadataName = HotDiff.MetadataName(decl);
                if (!fieldOwners.TryGetValue(metadataName, out var owners))
                    fieldOwners[metadataName] = owners = new List<(int, List<string>)>();
                int existing = owners.FindIndex(o => o.FileIndex == i);
                if (existing >= 0)
                    owners[existing].Names.AddRange(names); // same-file parts merge in source order
                else
                    owners.Add((i, names));
            }
        }
        if (!fieldOwners.Any(p => p.Value.Count > 1))
            return;

        // Original field order per type — a metadata-only lookup compilation.
        // MetadataImportOptions.All: the layout-relevant fields are private.
        CSharpCompilation lookup = CSharpCompilation.Create(
            "LocusPartialLayoutLookup",
            references: references,
            options: new CSharpCompilationOptions(
                OutputKind.DynamicallyLinkedLibrary,
                metadataImportOptions: MetadataImportOptions.All));
        var edges = new HashSet<(int Before, int After)>();
        foreach (var pair in fieldOwners.Where(p => p.Value.Count > 1))
        {
            INamedTypeSymbol? original = PatchRewriter.FindOriginalType(lookup, pair.Key, out _);
            if (original == null)
                continue; // the rewriter's guard reports it
            var originalIndex = new Dictionary<string, int>(StringComparer.Ordinal);
            int at = 0;
            foreach (ISymbol member in original.GetMembers())
            {
                if (member is IFieldSymbol field && !field.IsStatic && !field.IsConst)
                    originalIndex[field.Name] = at++;
            }

            var ranked = new List<(int FileIndex, int Rank)>();
            foreach (var (fileIndex, names) in pair.Value)
            {
                int rank = int.MaxValue;
                foreach (string name in names)
                {
                    if (originalIndex.TryGetValue(name, out int index))
                        rank = Math.Min(rank, index);
                }
                if (rank != int.MaxValue)
                    ranked.Add((fileIndex, rank));
            }
            ranked.Sort((a, b) => a.Rank.CompareTo(b.Rank));
            for (int i = 0; i + 1 < ranked.Count; i++)
            {
                if (ranked[i].FileIndex != ranked[i + 1].FileIndex)
                    edges.Add((ranked[i].FileIndex, ranked[i + 1].FileIndex));
            }
        }
        if (edges.Count == 0)
            return;

        // Stable topological order; the original position breaks ties.
        int count = batchFiles.Count;
        var indegree = new int[count];
        var adjacency = new List<int>?[count];
        foreach (var (before, after) in edges)
        {
            (adjacency[before] ??= new List<int>()).Add(after);
            indegree[after]++;
        }
        var ready = new SortedSet<int>();
        for (int i = 0; i < count; i++)
        {
            if (indegree[i] == 0)
                ready.Add(i);
        }
        var order = new List<int>(count);
        while (ready.Count > 0)
        {
            int next = ready.Min;
            ready.Remove(next);
            order.Add(next);
            foreach (int after in adjacency[next] ?? Enumerable.Empty<int>())
            {
                if (--indegree[after] == 0)
                    ready.Add(after);
            }
        }
        if (order.Count != count)
            return; // cyclic constraints: keep the natural order

        var reordered = order.Select(i => batchFiles[i]).ToList();
        batchFiles.Clear();
        batchFiles.AddRange(reordered);
    }

    /// <summary>The names a part contributes to the instance-field LAYOUT,
    /// in source order: plain instance fields, auto-property backing fields,
    /// field-like event backing fields (mirrors InstanceFieldSequence's
    /// symbol view, syntax-side).</summary>
    private static List<string> InstanceFieldishNames(TypeDeclarationSyntax type)
    {
        var names = new List<string>();
        foreach (MemberDeclarationSyntax member in type.Members)
        {
            switch (member)
            {
                case FieldDeclarationSyntax field when
                    !field.Modifiers.Any(SyntaxKind.ConstKeyword) &&
                    !field.Modifiers.Any(SyntaxKind.StaticKeyword):
                    foreach (VariableDeclaratorSyntax declarator in field.Declaration.Variables)
                        names.Add(declarator.Identifier.Text);
                    break;
                case PropertyDeclarationSyntax property when
                    HotDiff.IsAutoProperty(property) &&
                    !property.Modifiers.Any(SyntaxKind.StaticKeyword):
                    names.Add(HotDiff.AutoPropertyBackingFieldName(property.Identifier.Text));
                    break;
                case EventFieldDeclarationSyntax eventField when
                    !eventField.Modifiers.Any(SyntaxKind.StaticKeyword):
                    foreach (VariableDeclaratorSyntax declarator in eventField.Declaration.Variables)
                        names.Add(declarator.Identifier.Text);
                    break;
            }
        }
        return names;
    }

    /// <summary>B6 fail-closed completeness gate: for every partial type the
    /// batch declares, every member the ORIGINAL assembly's type carries must
    /// have a source declaration across the batch's disk parts (modulo the
    /// members this batch deliberately REMOVES). A metadata member with no
    /// source = a source-generator part (DOTS codegen, UI Toolkit, …) or an
    /// undiscovered sibling file — the patch copy would re-declare the type
    /// incompletely, so the batch stays cold with the member named.
    ///
    /// Instance-field layout equality is verified separately (and more
    /// strictly, order included) by the rewriter's guard; this pass closes
    /// the method/accessor/static-field dimension. Methods are matched by
    /// name + arity + fully-qualified parameter types; removed members by
    /// name + parameter COUNT (the diff carries reflection-style simple
    /// names) — an over-match there can only skip the gate for a same-name
    /// same-count overload, which then fails at compile time or is inert,
    /// never a silent layout break. Compiler-generated (unspeakable) names
    /// and cctors are out of scope: patches empty static constructors, and
    /// lowering artifacts regenerate per compilation.</summary>
    private static JsonNode? VerifyPartialPartsComplete(
        PatchBatchContext batch,
        List<(string Path, SyntaxTree Tree, HotDiffFileResult Diff)> batchFiles,
        long startedAt)
    {
        // Partial types declared anywhere in the batch → first declaring
        // file (for the cold verdict), plus the batch's removed-member keys.
        var partialTypes = new Dictionary<string, string>(StringComparer.Ordinal);
        var newTypeNames = new HashSet<string>(StringComparer.Ordinal);
        var removedKeys = new HashSet<string>(StringComparer.Ordinal);
        foreach (var (path, tree, diff) in batchFiles)
        {
            foreach (string newType in diff.NewTypes)
                newTypeNames.Add(newType);
            foreach (HotDiffRemovedMember removed in diff.RemovedMembers)
            {
                removedKeys.Add(RemovedMemberKey(
                    removed.DeclaringType, removed.Name, removed.ParamTypeNames.Length, removed.IsStatic));
            }
            var root = (CompilationUnitSyntax)tree.GetRoot();
            foreach (TypeDeclarationSyntax decl in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
            {
                if (!decl.Modifiers.Any(SyntaxKind.PartialKeyword))
                    continue;
                string metadataName = HotDiff.MetadataName(decl);
                if (newTypeNames.Contains(metadataName))
                    continue; // HotDiff already fails new partial types closed
                if (!partialTypes.ContainsKey(metadataName))
                    partialTypes[metadataName] = path;
            }
        }
        if (partialTypes.Count == 0)
            return null;

        foreach (var pair in partialTypes)
        {
            // Any declaring tree's model resolves the symbol MERGED across
            // every part in the batch (Roslyn merges partial declarations).
            INamedTypeSymbol? sourceSymbol = null;
            foreach (var (_, tree, _) in batchFiles)
            {
                var root = (CompilationUnitSyntax)tree.GetRoot();
                TypeDeclarationSyntax? decl = root.DescendantNodes().OfType<TypeDeclarationSyntax>()
                    .FirstOrDefault(d => HotDiff.MetadataName(d) == pair.Key);
                if (decl == null)
                    continue;
                sourceSymbol = batch.ModelFor(tree).GetDeclaredSymbol(decl);
                break;
            }
            if (sourceSymbol == null)
                continue;

            INamedTypeSymbol? original = PatchRewriter.FindOriginalType(batch.Binding, pair.Key, out _);
            if (original == null)
            {
                return ColdVerdict(
                    pair.Value,
                    "original type not found in references: " + pair.Key,
                    startedAt);
            }

            string? missing = FindMemberMissingFromParts(pair.Key, original, sourceSymbol, removedKeys);
            if (missing != null)
            {
                return ColdVerdict(
                    pair.Value,
                    "partial type member has no source on disk: " + pair.Key + "." + missing +
                    " (a source generator contributes a part, or a sibling part file was not found; " +
                    "use unity_recompile)",
                    startedAt);
            }
        }
        return null;
    }

    private static string RemovedMemberKey(string declaringType, string name, int paramCount, bool isStatic) =>
        declaringType + "|" + name + "|" + paramCount + (isStatic ? "|s" : "|i");

    /// <summary>First original-assembly member (method/accessor/ctor/static
    /// field) with no counterpart in the merged source parts, or null when
    /// the disk parts are complete.</summary>
    private static string? FindMemberMissingFromParts(
        string metadataName,
        INamedTypeSymbol original,
        INamedTypeSymbol source,
        HashSet<string> removedKeys)
    {
        static bool Speakable(string name) => !name.Contains('<');

        static string MethodKey(IMethodSymbol method) =>
            method.Name + "`" + method.Arity + "(" +
            string.Join(",", method.Parameters.Select(p =>
                (p.RefKind != RefKind.None ? "&" : "") +
                p.Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat))) +
            ")" + (method.IsStatic ? "|s" : "|i");

        var sourceMethods = new HashSet<string>(StringComparer.Ordinal);
        var sourceStaticFields = new HashSet<string>(StringComparer.Ordinal);
        foreach (ISymbol member in source.GetMembers())
        {
            switch (member)
            {
                case IMethodSymbol method:
                    sourceMethods.Add(MethodKey(method));
                    break;
                case IPropertySymbol property:
                    // Accessor symbols usually appear in GetMembers too;
                    // adding them through the property is belt-and-braces.
                    if (property.GetMethod != null)
                        sourceMethods.Add(MethodKey(property.GetMethod));
                    if (property.SetMethod != null)
                        sourceMethods.Add(MethodKey(property.SetMethod));
                    break;
                case IEventSymbol @event:
                    if (@event.AddMethod != null)
                        sourceMethods.Add(MethodKey(@event.AddMethod));
                    if (@event.RemoveMethod != null)
                        sourceMethods.Add(MethodKey(@event.RemoveMethod));
                    break;
                case IFieldSymbol field when field.IsStatic:
                    sourceStaticFields.Add(field.Name);
                    break;
            }
        }

        foreach (ISymbol member in original.GetMembers())
        {
            switch (member)
            {
                case IMethodSymbol method:
                    if (!Speakable(method.Name))
                        continue; // lowering artifact (local function, lambda cache, …)
                    if (method.MethodKind == MethodKind.StaticConstructor)
                        continue; // patches empty cctors; synthesis differs between source and metadata
                    if (sourceMethods.Contains(MethodKey(method)))
                        continue;
                    if (removedKeys.Contains(RemovedMemberKey(
                            metadataName, method.Name, method.Parameters.Length, method.IsStatic)))
                        continue; // deliberately removed by this batch (M5)
                    return method.Name;

                case IFieldSymbol field when field.IsStatic && !field.IsConst:
                    // Instance fields are the layout guard's job (order
                    // included); generator consts are inlined anyway.
                    if (!Speakable(field.Name))
                        continue;
                    if (sourceStaticFields.Contains(field.Name))
                        continue;
                    return field.Name;
            }
        }
        return null;
    }

    private void CommitShimRegistrations(string generation, List<ShimRegistration> registrations)
    {
        if (registrations.Count == 0)
            return;
        _memberSurfaceRegistry.Commit(
            generation,
            registrations.Select(r =>
                new KeyValuePair<string, MemberSurfaceRegistry.ShimEntry>(r.MemberKey, r.Entry)));
    }

    private void CommitFieldStoreRegistrations(string generation, List<FieldStoreRegistration>? registrations)
    {
        if (registrations == null || registrations.Count == 0)
            return;
        _fieldStoreRegistry.Commit(
            generation,
            registrations.Select(r =>
                new KeyValuePair<string, FieldStoreRegistry.StoreEntry>(r.FieldKey, r.Entry)));
    }

    /// <summary>
    /// TI-B: produce the Unity type index from reference metadata. The
    /// fingerprint stays Unity-owned (the Rust side pairs this type set with
    /// the cheap `export_type_index_fingerprint` roundtrip), so every cache
    /// currency check and delta channel keeps a single fingerprint scheme.
    /// </summary>
    public JsonNode HandleIndexTypes(JsonNode? @params)
    {
        var request = Deserialize<IndexTypesRequestDto>(@params);
        long startedAt = Environment.TickCount64;

        ImmutableArray<MetadataReference> references = ResolveReferences(request.Params, useHostBcl: false);
        List<TypeIndexSource.Entry> entries = TypeIndexSource.Build(
            references.OfType<PortableExecutableReference>());

        JsonObject result = TypeIndexSource.ToJson(entries);
        result["durationMs"] = Environment.TickCount64 - startedAt;
        return result;
    }

    /// <summary>Assembly definition names straight from the PE metadata (no
    /// symbol materialization).</summary>
    private static List<string> ReferenceAssemblyNames(ImmutableArray<MetadataReference> references)
    {
        var names = new List<string>(references.Length);
        foreach (MetadataReference reference in references)
        {
            if (reference is not PortableExecutableReference peReference)
                continue;
            try
            {
                if (peReference.GetMetadata() is AssemblyMetadata assembly)
                {
                    var module = assembly.GetModules()[0];
                    var reader = module.GetMetadataReader();
                    names.Add(reader.GetString(reader.GetAssemblyDefinition().Name));
                }
            }
            catch
            {
                // Modules without an assembly definition (netmodules) or
                // unreadable metadata: skip — worst case a private access in
                // that assembly fails the compile with a clear diagnostic.
            }
        }
        return names;
    }

    private static SyntaxTree BuildAccessChecksTree(IEnumerable<string> assemblyNames, CSharpParseOptions parseOptions)
    {
        var sb = new StringBuilder(8 * 1024);
        foreach (string name in assemblyNames.Distinct(StringComparer.Ordinal).OrderBy(n => n, StringComparer.Ordinal))
        {
            sb.Append("[assembly: System.Runtime.CompilerServices.IgnoresAccessChecksTo(\"")
              .Append(name.Replace("\"", "\\\""))
              .Append("\")]\n");
        }
        sb.Append(@"
namespace System.Runtime.CompilerServices
{
    [global::System.AttributeUsage(global::System.AttributeTargets.Assembly, AllowMultiple = true)]
    internal sealed class IgnoresAccessChecksToAttribute : global::System.Attribute
    {
        public IgnoresAccessChecksToAttribute(string assemblyName) { AssemblyName = assemblyName; }
        public string AssemblyName { get; }
    }
}
");
        return CSharpSyntaxTree.ParseText(sb.ToString(), parseOptions, path: "LocusIgnoresAccessChecks.cs", encoding: Utf8NoBom);
    }

    /// <summary>
    /// Flip Roslyn's internal TopLevelBinderFlags to IgnoreAccessibility
    /// (1 &lt;&lt; 22): the established mechanism for compiling code that
    /// reaches private members of referenced assemblies, paired with the
    /// IgnoresAccessChecksTo attribute for the runtime side. When the
    /// internal property disappears in a future Roslyn the compile falls
    /// back to normal accessibility and private access surfaces as a
    /// deterministic diagnostic. Internal: the batch BINDING compilation
    /// (PatchBatchContext.Build) applies the same flag so the semantic
    /// model resolves what the emit will actually bind — a non-public
    /// metadata symbol that binds to null would otherwise slip past the
    /// access scans while the emit happily compiles it (C2′b).
    /// </summary>
    internal static void ApplyIgnoreAccessibility(CSharpCompilationOptions options)
    {
        try
        {
            var property = typeof(CSharpCompilationOptions).GetProperty(
                "TopLevelBinderFlags",
                System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
            property?.SetValue(options, (uint)1 << 22);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("[LocusCompileServer] IgnoreAccessibility unavailable: " + ex.Message);
        }
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
        // Keep the legacy "__LocusRuntimeAsync_" prefix: the Unity-side type
        // index skips snippet assemblies by that prefix, and a different name
        // would invalidate (and force a full re-export of) the type index
        // after every executed snippet.
        string assemblyName = NextAssemblyName("RuntimeAsync", request.Params?.DomainGeneration);

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
