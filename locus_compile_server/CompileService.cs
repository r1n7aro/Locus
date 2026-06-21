using System.Collections.Immutable;
using System.Diagnostics;
using System.Globalization;
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

    [JsonPropertyName("emitDebugSymbols")]
    public bool? EmitDebugSymbols { get; set; }

    [JsonPropertyName("returnAssemblyPath")]
    public bool ReturnAssemblyPath { get; set; }

    /// <summary>Optional diagnostic formatter. "viewScript" mirrors
    /// LocusBridge.ViewScripts.cs with path-qualified errors.</summary>
    [JsonPropertyName("diagnosticStyle")]
    public string? DiagnosticStyle { get; set; }
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

    [JsonPropertyName("returnAssemblyPath")]
    public bool ReturnAssemblyPath { get; set; }
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

    [JsonPropertyName("returnAssemblyPath")]
    public bool ReturnAssemblyPath { get; set; }
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

    [JsonPropertyName("returnAssemblyPath")]
    public bool ReturnAssemblyPath { get; set; }
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

public sealed class ForceDetourDto
{
    [JsonPropertyName("path")]
    public string? Path { get; set; }

    /// <summary>
    /// CallerScan caller method keys:
    /// DeclaringType|MetadataName|ParameterCount|s/i.
    /// </summary>
    [JsonPropertyName("methodKeys")]
    public string[]? MethodKeys { get; set; }
}

public sealed class CallerQueryTargetDto
{
    [JsonPropertyName("declaringType")]
    public string? DeclaringType { get; set; }

    [JsonPropertyName("memberName")]
    public string? MemberName { get; set; }
}

public sealed class CallerQueryRequestDto
{
    [JsonPropertyName("params")]
    public CompileParamsDto? Params { get; set; }

    [JsonPropertyName("targets")]
    public CallerQueryTargetDto[]? Targets { get; set; }
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

public sealed class IndexSchemaRequestDto
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

    [JsonPropertyName("assemblyPath")]
    public string? AssemblyPath { get; set; }
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

    [JsonPropertyName("returnAssemblyPath")]
    public bool ReturnAssemblyPath { get; set; }

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

    /// <summary>
    /// Release-inline caller refresh: compile these unchanged caller files and
    /// detour only the listed caller methods, so stale inlined call sites can
    /// be refreshed without rehooking a whole file.
    /// </summary>
    [JsonPropertyName("forceDetours")]
    public ForceDetourDto[]? ForceDetours { get; set; }
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
    public const int ProtocolVersion = 6;

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

    private static readonly EmitOptions LightweightEmitOptions = new();

    /// <summary>Optional embedded portable PDB for diagnostics-heavy paths.</summary>
    private static readonly EmitOptions EmbeddedPdbEmitOptions = new(
        debugInformationFormat: DebugInformationFormat.Embedded);

    private static EmitOptions EmitOptionsFor(bool emitDebugSymbols)
    {
        return emitDebugSymbols ? EmbeddedPdbEmitOptions : LightweightEmitOptions;
    }

    private const string AssemblyArtifactRootName = "LocusCompileServer";
    private const int MaxAssemblyArtifactFiles = 256;
    private static readonly TimeSpan AssemblyArtifactMaxAge = TimeSpan.FromHours(6);
    private static int _assemblyArtifactRootPruned;

    private static readonly Lazy<ImmutableArray<MetadataReference>> HostBclReferences =
        new(BuildHostBclReferences);

    private readonly ReferenceCache _referenceCache = new();
    private readonly ImageRegistry _imageRegistry = new();
    private readonly MemberSurfaceRegistry _memberSurfaceRegistry = new();
    private readonly FieldStoreRegistry _fieldStoreRegistry = new();
    private readonly NewTypeRegistry _newTypeRegistry = new();

    // Registrations of compiled-but-not-yet-accepted hot patches, keyed by
    // assembly name; committed into the registries when image/register
    // confirms Unity loaded the patch.
    private readonly object _pendingShimLock = new();
    private readonly Dictionary<string, (string Generation, List<ShimRegistration> Shims, List<FieldStoreRegistration> FieldStores)> _pendingShims =
        new(StringComparer.Ordinal);

    // Same deferral for play-mode-born new-type files (kept separate from the
    // shim map so the shim path stays untouched): committed into the
    // NewTypeRegistry when image/register confirms Unity loaded the patch.
    private readonly Dictionary<string, (string Generation, List<KeyValuePair<string, NewTypeRegistry.FileEntry>> Entries)> _pendingNewTypes =
        new(StringComparer.Ordinal);

    private int _assemblyCounter;

    // Reference set + parse options for the last seen params fingerprint.
    private string? _cachedFingerprint;
    private ImmutableArray<MetadataReference> _cachedReferences;
    private CSharpParseOptions _cachedParseOptions = DefaultParseOptions(Array.Empty<string>());

    public CompileService()
    {
        PruneAssemblyArtifactRootOnce();
    }

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
            request.ReferenceSessionImages,
            request.EmitDebugSymbols ?? true,
            ResolveDiagnosticStyle(request.DiagnosticStyle));

        if (bytes == null)
            return FailureResult(error!, "compile", startedAt);

        if (request.RegisterImage && !string.IsNullOrEmpty(request.Params?.DomainGeneration))
            _imageRegistry.Register(request.Params!.DomainGeneration!, assemblyName, bytes);

        return SuccessResult(bytes, assemblyName, startedAt, request.ReturnAssemblyPath);
    }

    public JsonNode HandleRegisterImage(JsonNode? @params)
    {
        var request = Deserialize<RegisterImageRequestDto>(@params);
        if (string.IsNullOrWhiteSpace(request.DomainGeneration))
            throw new RpcInvalidParamsException("image/register requires domainGeneration");
        if (string.IsNullOrWhiteSpace(request.AssemblyName))
            throw new RpcInvalidParamsException("image/register requires assemblyName");
        if (string.IsNullOrWhiteSpace(request.AssemblyB64) &&
            string.IsNullOrWhiteSpace(request.AssemblyPath))
        {
            throw new RpcInvalidParamsException("image/register requires assemblyB64 or assemblyPath");
        }

        byte[] bytes;
        try
        {
            if (!string.IsNullOrWhiteSpace(request.AssemblyPath))
                bytes = File.ReadAllBytes(request.AssemblyPath!);
            else
                bytes = Convert.FromBase64String(request.AssemblyB64!);
        }
        catch (Exception ex)
        {
            throw new RpcInvalidParamsException("image/register assembly payload is invalid: " + ex.Message);
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

        // Same acceptance gate for play-mode-born new-type files: only now is
        // the type actually live in the running domain, so later re-edits may
        // redirect onto it.
        (string Generation, List<KeyValuePair<string, NewTypeRegistry.FileEntry>> Entries) pendingNewTypes = default;
        lock (_pendingShimLock)
        {
            if (_pendingNewTypes.TryGetValue(request.AssemblyName!, out pendingNewTypes!))
                _pendingNewTypes.Remove(request.AssemblyName!);
        }
        if (pendingNewTypes.Entries != null &&
            string.Equals(pendingNewTypes.Generation, request.DomainGeneration, StringComparison.Ordinal))
        {
            _newTypeRegistry.Commit(pendingNewTypes.Generation, pendingNewTypes.Entries);
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

        return SuccessResult(bytes, assemblyName, startedAt, request.ReturnAssemblyPath);
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

        JsonNode result = SuccessResult(bytes, assemblyName, startedAt, request.ReturnAssemblyPath);
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
        var json = new JsonObject
        {
            ["declaringType"] = method.DeclaringType,
            ["name"] = method.Name,
            ["paramTypeNames"] = new JsonArray(method.ParamTypeNames.Select(p => (JsonNode)p).ToArray()),
            ["isStatic"] = method.IsStatic,
            ["isCtor"] = method.IsCtor,
            ["added"] = method.Added,
        };
        // A newly added Unity message drives via the runtime (pump or proxy)
        // rather than a detour; surface the driver kind so analyze/hotDiff
        // explains why an added message is hot.
        if (method.MessageDriverKind.Length > 0)
            json["messageDriverKind"] = method.MessageDriverKind;
        return json;
    }

    private static Dictionary<string, HashSet<string>> ForceDetourMap(ForceDetourDto[]? forceDetours)
    {
        var map = new Dictionary<string, HashSet<string>>(StringComparer.OrdinalIgnoreCase);
        foreach (ForceDetourDto force in forceDetours ?? Array.Empty<ForceDetourDto>())
        {
            if (string.IsNullOrWhiteSpace(force.Path))
                continue;
            string key = NormalizePathKey(force.Path!);
            if (!map.TryGetValue(key, out HashSet<string>? methods))
                map[key] = methods = new HashSet<string>(StringComparer.Ordinal);
            foreach (string methodKey in force.MethodKeys ?? Array.Empty<string>())
            {
                if (!string.IsNullOrWhiteSpace(methodKey))
                    methods.Add(methodKey);
            }
        }
        return map;
    }

    private sealed class ForcedMethodKey
    {
        public string DeclaringType = "";
        public string Name = "";
        public int ParameterCount;
        public bool IsStatic;
    }

    private static ForcedMethodKey? ParseForcedMethodKey(string key)
    {
        string[] parts = key.Split('|');
        if (parts.Length != 4)
            return null;
        if (!int.TryParse(parts[2], NumberStyles.Integer, CultureInfo.InvariantCulture, out int parameterCount))
            return null;
        return new ForcedMethodKey
        {
            DeclaringType = parts[0],
            Name = parts[1],
            ParameterCount = parameterCount,
            IsStatic = string.Equals(parts[3], "s", StringComparison.Ordinal),
        };
    }

    private static void ApplyForcedDetours(
        string path,
        string newText,
        CSharpParseOptions parseOptions,
        HotDiffFileResult diff,
        HashSet<string> methodKeys)
    {
        if (methodKeys.Count == 0)
            return;

        CompilationUnitSyntax root;
        try
        {
            root = CSharpSyntaxTree.ParseText(newText, parseOptions, path: path)
                .GetCompilationUnitRoot();
        }
        catch (Exception ex)
        {
            diff.Hot = false;
            diff.Reasons.Add("inline caller refresh could not parse " + path + ": " + ex.Message);
            return;
        }

        var existing = new HashSet<string>(
            diff.ChangedMethods.Select(m => ForceKey(m.DeclaringType, m.Name, m.ParamTypeNames.Length, m.IsStatic)),
            StringComparer.Ordinal);

        foreach (string rawKey in methodKeys)
        {
            ForcedMethodKey? key = ParseForcedMethodKey(rawKey);
            if (key == null)
            {
                diff.Hot = false;
                diff.Reasons.Add("inline caller refresh received malformed method key: " + rawKey);
                return;
            }

            var matches = new List<(TypeDeclarationSyntax Host, MemberDeclarationSyntax Member, HotDiffMethod Method)>();
            foreach (TypeDeclarationSyntax type in root.DescendantNodes().OfType<TypeDeclarationSyntax>())
            {
                string metadataName = HotDiff.MetadataName(type);
                if (!string.Equals(metadataName, key.DeclaringType, StringComparison.Ordinal))
                    continue;

                foreach (MemberDeclarationSyntax member in type.Members)
                {
                    HotDiffMethod? method = ForcedMethodForMember(metadataName, member, key);
                    if (method != null)
                        matches.Add((type, member, method));
                }
            }

            if (matches.Count == 0)
            {
                diff.Hot = false;
                diff.Reasons.Add("inline caller refresh could not find " + rawKey + " in " + path);
                return;
            }
            if (matches.Count > 1)
            {
                diff.Hot = false;
                diff.Reasons.Add("inline caller refresh found ambiguous overloads for " + rawKey + " in " + path);
                return;
            }

            var (host, memberDecl, forcedMethod) = matches[0];
            string? gate = ForcedDetourGate(host, memberDecl, forcedMethod);
            if (gate != null)
            {
                diff.Hot = false;
                diff.Reasons.Add(gate);
                return;
            }

            string forceKey = ForceKey(
                forcedMethod.DeclaringType,
                forcedMethod.Name,
                forcedMethod.ParamTypeNames.Length,
                forcedMethod.IsStatic);
            if (existing.Add(forceKey))
                diff.ChangedMethods.Add(forcedMethod);
            if (!diff.PatchedTypes.Contains(forcedMethod.DeclaringType, StringComparer.Ordinal))
                diff.PatchedTypes.Add(forcedMethod.DeclaringType);
        }

        diff.PatchedTypes = diff.PatchedTypes.Distinct(StringComparer.Ordinal).OrderBy(t => t, StringComparer.Ordinal).ToList();
    }

    private static string ForceKey(string declaringType, string name, int parameterCount, bool isStatic) =>
        declaringType + "|" + name + "|" + parameterCount + (isStatic ? "|s" : "|i");

    /// <summary>Release inline caller-refresh, instance arm (Option A): for each
    /// CHANGED instance method eligible for the self-shim redirect, inject a
    /// uniquely-named clone into its type in the new-side tree and a synthetic
    /// ADDED diff entry, so the existing M2 shim pipeline emits a static
    /// self-shim while the original method keeps its normal detour. Returns the
    /// clone specs so <c>PatchBatchContext.Build</c> can gate them on the access
    /// caps and wire the original method's in-batch call sites to the shim.
    /// <para>Static callees already redirect unconditionally (cheap table
    /// entry); the instance arm runs only in refresh compiles (force detours
    /// present), where alone a same-batch caller exists to inline into — a
    /// normal edit has no in-batch caller, so the extra shim would be dead
    /// weight.</para>
    /// Mutates <paramref name="batchFiles"/> trees and diffs in place.</summary>
    private static List<InlineRedirectClone> BuildInlineRedirectClones(
        List<(string Path, SyntaxTree Tree, HotDiffFileResult Diff)> batchFiles,
        CSharpParseOptions parseOptions)
    {
        var clones = new List<InlineRedirectClone>();
        for (int i = 0; i < batchFiles.Count; i++)
        {
            var (path, tree, diff) = batchFiles[i];
            var root = (CompilationUnitSyntax)tree.GetRoot();

            var additions = new Dictionary<TypeDeclarationSyntax, List<MethodDeclarationSyntax>>();
            var pending = new List<(HotDiffMethod Clone, HotDiffMethod Original)>();
            int counter = 0;

            foreach (HotDiffMethod changed in diff.ChangedMethods.Where(m => !m.Added).ToList())
            {
                if (changed.IsCtor || changed.IsStatic || changed.TypeParameterCount != 0)
                    continue;
                MethodDeclarationSyntax? decl = PatchBatchContext.FindAddedMethodDeclaration(root, changed);
                if (decl == null || !IsInlineCloneEligible(decl))
                    continue;
                if (decl.Ancestors().OfType<TypeDeclarationSyntax>().FirstOrDefault() is not TypeDeclarationSyntax host)
                    continue;

                string cloneName = changed.Name + "__LocusInline_" + counter++;
                MethodDeclarationSyntax clone = decl
                    .WithIdentifier(SyntaxFactory.Identifier(cloneName))
                    .WithLeadingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed)
                    .WithTrailingTrivia(SyntaxFactory.ElasticCarriageReturnLineFeed);

                if (!additions.TryGetValue(host, out List<MethodDeclarationSyntax>? list))
                    additions[host] = list = new List<MethodDeclarationSyntax>();
                list.Add(clone);

                pending.Add((
                    new HotDiffMethod
                    {
                        DeclaringType = changed.DeclaringType,
                        Name = cloneName,
                        ParamTypeNames = changed.ParamTypeNames,
                        IsStatic = false,
                        IsCtor = false,
                        Added = true,
                        TypeParameterCount = 0,
                    },
                    changed));
            }

            if (additions.Count == 0)
                continue;

            // computeReplacement receives the node with its descendants already
            // rewritten — add to THAT so a clone in a nested type isn't lost
            // when its outer type is also rewritten.
            CompilationUnitSyntax newRoot = root.ReplaceNodes(
                additions.Keys,
                (original, rewritten) => ((TypeDeclarationSyntax)rewritten).AddMembers(additions[original].ToArray()));
            batchFiles[i] = (path, CSharpSyntaxTree.Create(newRoot, parseOptions, path), diff);

            foreach (var (cloneDiff, original) in pending)
            {
                diff.ChangedMethods.Add(cloneDiff);
                clones.Add(new InlineRedirectClone { FilePath = path, Original = original, Clone = cloneDiff });
            }
        }
        return clones;
    }

    /// <summary>An instance method whose changed body may be re-emitted as a
    /// static self-shim: it must have a body, carry no method type parameters,
    /// not be virtual/override/abstract/extern/partial/static or an explicit
    /// interface impl (virtual dispatch and bodiless members can't be flattened
    /// to a dispatch-free shim), and live in a non-interface, non-generic,
    /// non-ref-struct type (the `((Foo)(object)this)` self-cast needs a boxable
    /// reference identity for the implicit-receiver case).</summary>
    private static bool IsInlineCloneEligible(MethodDeclarationSyntax decl)
    {
        if (decl.Body == null && decl.ExpressionBody == null)
            return false;
        if (decl.TypeParameterList is { Parameters.Count: > 0 })
            return false;
        if (decl.ExplicitInterfaceSpecifier != null)
            return false;
        foreach (SyntaxToken modifier in decl.Modifiers)
        {
            if (modifier.IsKind(SyntaxKind.VirtualKeyword) ||
                modifier.IsKind(SyntaxKind.OverrideKeyword) ||
                modifier.IsKind(SyntaxKind.AbstractKeyword) ||
                modifier.IsKind(SyntaxKind.ExternKeyword) ||
                modifier.IsKind(SyntaxKind.PartialKeyword) ||
                modifier.IsKind(SyntaxKind.StaticKeyword))
            {
                return false;
            }
        }
        foreach (TypeDeclarationSyntax type in decl.Ancestors().OfType<TypeDeclarationSyntax>())
        {
            if (type is InterfaceDeclarationSyntax)
                return false;
            if (type.TypeParameterList is { Parameters.Count: > 0 })
                return false;
            // Value types are deferred: a `ref self` shim cannot bind a temp
            // receiver (`new S().M()`) and a by-value self would drop `this`
            // mutations. Such edits converge via the queued recompile. (Build
            // re-checks IsValueType at the symbol level for record structs.)
            if (type is StructDeclarationSyntax)
                return false;
        }
        return true;
    }

    private static HotDiffMethod? ForcedMethodForMember(
        string metadataName,
        MemberDeclarationSyntax member,
        ForcedMethodKey key)
    {
        switch (member)
        {
            case MethodDeclarationSyntax method when
                string.Equals(method.Identifier.Text, key.Name, StringComparison.Ordinal) &&
                method.ParameterList.Parameters.Count == key.ParameterCount &&
                method.Modifiers.Any(SyntaxKind.StaticKeyword) == key.IsStatic:
                return new HotDiffMethod
                {
                    DeclaringType = metadataName,
                    Name = method.Identifier.Text,
                    ParamTypeNames = HotDiff.ParamTypeNames(method.ParameterList),
                    ParamTypeSigs = HotDiff.ParamTypeSigs(method.ParameterList),
                    IsStatic = key.IsStatic,
                    IsCtor = false,
                    Added = false,
                };
            case ConstructorDeclarationSyntax ctor when
                string.Equals(key.Name, ".ctor", StringComparison.Ordinal) &&
                ctor.ParameterList.Parameters.Count == key.ParameterCount &&
                !key.IsStatic:
                return new HotDiffMethod
                {
                    DeclaringType = metadataName,
                    Name = ".ctor",
                    ParamTypeNames = HotDiff.ParamTypeNames(ctor.ParameterList),
                    ParamTypeSigs = HotDiff.ParamTypeSigs(ctor.ParameterList),
                    IsStatic = false,
                    IsCtor = true,
                    Added = false,
                };
            default:
                return null;
        }
    }

    private static string? ForcedDetourGate(
        TypeDeclarationSyntax host,
        MemberDeclarationSyntax member,
        HotDiffMethod method)
    {
        string Reason(string why) =>
            "inline caller refresh cannot re-detour " + method.DeclaringType + "." + method.Name +
            ": " + why + "; use unity_recompile";

        if (host is InterfaceDeclarationSyntax)
            return Reason("interface members are not supported");

        for (SyntaxNode? current = host; current != null; current = current.Parent)
        {
            if (current is TypeDeclarationSyntax type && (type.TypeParameterList?.Parameters.Count ?? 0) > 0)
                return Reason("generic type members are not supported");
        }

        if (HasBurstCompileAttributeText(host.AttributeLists) || HasBurstCompileAttributeText(member.AttributeLists))
            return Reason("Burst-compiled members are not supported");

        switch (member)
        {
            case MethodDeclarationSyntax methodDecl:
                if (methodDecl.TypeParameterList != null && methodDecl.TypeParameterList.Parameters.Count > 0)
                    return Reason("generic methods are not supported");
                if (methodDecl.ExplicitInterfaceSpecifier != null)
                    return Reason("explicit interface implementations are not supported");
                if (methodDecl.Body == null && methodDecl.ExpressionBody == null)
                    return Reason("the method has no body");
                break;
            case ConstructorDeclarationSyntax ctor:
                if (ctor.Modifiers.Any(SyntaxKind.StaticKeyword))
                    return Reason("static constructors are not supported");
                if (ctor.Body == null && ctor.ExpressionBody == null)
                    return Reason("the constructor has no body");
                break;
            default:
                return Reason("only methods and instance constructors are supported");
        }

        return null;
    }

    private static bool HasBurstCompileAttributeText(SyntaxList<AttributeListSyntax> attributes) =>
        attributes.SelectMany(list => list.Attributes)
            .Any(attribute => attribute.Name.ToString().Contains("BurstCompile", StringComparison.Ordinal));

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
                PortableExecutableReference? reference = _referenceCache.GetOrCreate(extraPath);
                if (reference != null)
                    references = references.Add(reference);
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
        Dictionary<string, HashSet<string>> forceDetours = ForceDetourMap(request.ForceDetours);

        // Play-mode-born new-type files registered by earlier batches of this
        // generation: a re-edit re-diffs against the original loaded text (not
        // the empty coordinator baseline) so a body change becomes a detour
        // onto the FIRST loaded type. reeditFileAssemblies records, per file
        // that took this path, that first assembly — the detour ORIGINAL side.
        IReadOnlyDictionary<string, NewTypeRegistry.FileEntry> newTypeBaselines =
            _newTypeRegistry.SnapshotFor(request.Params?.DomainGeneration);
        var reeditFileAssemblies = new Dictionary<string, string>(StringComparer.Ordinal);
        // Feature #5: per-type override of reeditFileAssemblies for sibling types
        // born into a play-mode-born file after its first batch (they live in
        // their own assembly). Keyed by the sibling's metadata name.
        var reeditTypeAssemblies = new Dictionary<string, string>(StringComparer.Ordinal);
        var fileByPathKey = new Dictionary<string, HotDiffFileDto>(StringComparer.Ordinal);
        foreach (HotDiffFileDto f in request.Files)
            if (!string.IsNullOrEmpty(f.Path))
                fileByPathKey[NormalizePathKey(f.Path!)] = f;
        // New-type registry commits, collected through the batch and committed
        // once the image is accepted: NEW play-mode-born files (assembly pinned
        // after emit) and redirect marks (Redirected=true, assembly already the
        // first one — left un-pinned). Declared here because the redirect marks
        // are discovered in the diff loop below.
        var newTypeRegistrations = new List<KeyValuePair<string, NewTypeRegistry.FileEntry>>();
        // Play-mode-born re-edits that REMOVED a previously hot-applied Unity
        // message: the runtime drove it by the shim (not native dispatch), so it
        // is silenced by CLEARING that driver. Collected through the diff loop and
        // emitted as clear-marker message drivers after the rewrite (so the
        // plugin's replace-by-source teardown removes the stale pump).
        var bornDriverClears = new List<BornDriverClear>();

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
            // If this file's types live only in a prior hot-patch assembly,
            // re-route the diff against the first loaded text (not the empty
            // coordinator baseline, which re-classifies the whole type as new
            // every time). RouteBornReedit picks one of: a body / additions /
            // structural HOT apply onto the first assembly, a clean no-op
            // (unchanged re-send), or COLD (recompile to converge).
            if (newTypeBaselines.TryGetValue(NormalizePathKey(file.Path!), out NewTypeRegistry.FileEntry? bornEntry))
            {
                diff = RouteBornReedit(
                    file, bornEntry, request, parseOptions,
                    reeditFileAssemblies, reeditTypeAssemblies, newTypeRegistrations, bornDriverClears);
            }
            if (forceDetours.TryGetValue(NormalizePathKey(file.Path!), out HashSet<string>? methodKeys))
                ApplyForcedDetours(file.Path!, file.NewText, parseOptions, diff, methodKeys);
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
        var messageDrivers = new JsonArray();
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

        // Release inline caller-refresh (instance arm): in a refresh compile the
        // force-detoured callers are in this batch, so a CHANGED instance method
        // they inline must be reachable through a static self-shim. Inject the
        // clones into batchFiles BEFORE binding so both the Build model and the
        // per-file rewrite see them; Build gates each on the access caps.
        List<InlineRedirectClone>? inlineClones =
            forceDetours.Count > 0 ? BuildInlineRedirectClones(batchFiles, parseOptions) : null;

        PatchBatchContext batch = PatchBatchContext.Build(
            batchFiles, references,
            _memberSurfaceRegistry.SnapshotFor(generation),
            _fieldStoreRegistry.SnapshotFor(generation),
            storeDiscriminator,
            allowUnsafe: request.Params?.AllowUnsafe ?? false,
            runtimeCaps: AccessCaps.FromCells(request.RuntimeCaps?.Cells),
            inlineClones: inlineClones,
            reeditFileAssemblies: reeditFileAssemblies,
            reeditTypeAssemblies: reeditTypeAssemblies);

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
                    ["paramTypeSigs"] = new JsonArray(map.ParamTypeSigs.Select(p => (JsonNode)p).ToArray()),
                    ["isStatic"] = map.IsStatic,
                    ["isCtor"] = map.IsCtor,
                    // The edited file whose rewrite produced this detour. The
                    // desktop maps a returned inlined MethodKey back to this path
                    // to queue only the affected file(s) for recompile
                    // convergence, instead of the whole batch.
                    ["sourcePath"] = filePath,
                };
                if (map.OriginalAssembly != null)
                    method["originalAssembly"] = map.OriginalAssembly;
                if (map.IsStub)
                    method["isStub"] = true;
                methods.Add(method);
            }
            // M2 message drivers: a newly added Unity message that the engine
            // never dispatches after load materialized as an ordinary instance
            // shim above. Hand the runtime the shim coordinates plus the driver
            // kind so it wires the right driver — a PlayerLoop pump (player_loop)
            // or a forwarding proxy MonoBehaviour (component_proxy). Joined to the
            // shim registration by member identity for the authoritative metadata.
            foreach (HotDiffMethod added in diff.ChangedMethods)
            {
                if (!added.Added || added.MessageDriverKind.Length == 0)
                    continue;
                string memberKey = MemberSurfaceRegistry.MemberKey(
                    added.DeclaringType, added.Name, added.ParamTypeNames, added.IsStatic);
                MemberSurfaceRegistry.ShimEntry? shim = rewrite.ShimRegistrations
                    .FirstOrDefault(r => r.MemberKey == memberKey)?.Entry;
                if (shim == null)
                    continue;
                var driver = new JsonObject
                {
                    ["kind"] = added.MessageDriverKind,
                    ["declaringType"] = added.DeclaringType,
                    ["shimType"] = shim.ShimTypeMetadataName,
                    ["shimMethod"] = shim.ShimMethod,
                    ["message"] = added.Name,
                    // The engine-delivered argument type (component_proxy with an
                    // argument; empty for parameterless callbacks).
                    ["paramType"] = added.ParamTypeNames.Length > 0 ? added.ParamTypeNames[0] : "",
                    // Agent-facing caveat (lifecycle timing / approximate order);
                    // empty when the driver matches native behavior.
                    ["note"] = added.MessageNote,
                    ["sourcePath"] = filePath,
                };
                // Tier-2: an added message on a play-mode-born type drives instances
                // that live ONLY in a hot-patch assembly. Pin it so the runtime
                // resolves declaringType THERE (its default resolver skips
                // __LocusHotPatch_ assemblies, exactly like the M2 method detour).
                // ReeditAssemblyFor resolves PER TYPE — a late-born sibling (feature
                // #5) and its NESTED types live in their OWN assembly, not the file's
                // first one. Null (the ordinary compiled-type case) → the runtime
                // uses the default cross-domain resolution, unchanged.
                string? driverOriginalAssembly = batch.ReeditAssemblyFor(filePath, added.DeclaringType);
                if (driverOriginalAssembly != null)
                    driver["originalAssembly"] = driverOriginalAssembly;
                messageDrivers.Add(driver);
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

            // Register a brand-new FILE (empty coordinator baseline) that
            // introduced a top-level type via load_only, so a later body-only
            // re-edit redirects onto this assembly instead of re-loading the
            // type afresh (which would leave live instances on the old body).
            // Scoped to whole-new files: every type in them anchors to this one
            // assembly, so the re-edit OriginalAssembly is unambiguous. Mixed
            // files (a new type added beside compiled types) keep today's
            // load_only behavior. Committed only once image/register confirms
            // Unity loaded the patch (below) — if that call is dropped (e.g. a
            // transport error), the entry is lost and the file's next re-edit
            // simply falls back to load_only, exactly as before this feature.
            // An ALREADY-registered file is excluded: its re-edits route through
            // RouteBornReedit (which re-commits the birth baseline) and must not
            // re-birth here against the current text — that would erase the
            // baseline and re-load the live type into a fresh assembly.
            if (rewrite.NewTypes.Any(t => t.IsTopLevel) &&
                !newTypeBaselines.ContainsKey(NormalizePathKey(filePath)) &&
                fileByPathKey.TryGetValue(NormalizePathKey(filePath), out HotDiffFileDto? bornFile) &&
                string.IsNullOrWhiteSpace(bornFile.OldText))
            {
                newTypeRegistrations.Add(new KeyValuePair<string, NewTypeRegistry.FileEntry>(
                    NormalizePathKey(filePath),
                    new NewTypeRegistry.FileEntry
                    {
                        OriginalText = bornFile.NewText!,
                        // At birth the loaded text IS the live text — the baseline for
                        // detecting members removed by a later re-edit.
                        LastAppliedText = bornFile.NewText!,
                    }));
            }
        }

        // Clear-markers (play-mode-born feature #1 / [P1] fix): a re-edit removed
        // a Unity message that an earlier hot patch had wired to the runtime pump.
        // The pump drives that message by the SHIM, not native dispatch, so the
        // only way to stop it is to clear the driver. Emit a marker carrying the
        // file as the touched source; the plugin's replace-by-source teardown
        // clears every driver registered for the file and re-adds only those still
        // present (this removed one is not). kind="clear"/empty shim keeps it out
        // of the re-add loop, and a non-empty messageDrivers list keeps the Rust
        // "nothing detourable" early-return from skipping Unity when the surviving
        // payload (e.g. a parked field add) is otherwise empty.
        foreach (BornDriverClear clear in bornDriverClears)
        {
            messageDrivers.Add(new JsonObject
            {
                ["kind"] = "clear",
                ["declaringType"] = clear.DeclaringType,
                ["shimType"] = "",
                ["shimMethod"] = "",
                ["message"] = clear.Message,
                ["paramType"] = "",
                ["note"] = "",
                ["sourcePath"] = clear.SourcePath,
                ["originalAssembly"] = clear.OriginalAssembly,
            });
        }

        if (methods.Count == 0 && newTypes.Count == 0 &&
            fieldStoreRegistrations.Count == 0 &&
            messageDrivers.Count == 0 &&
            shimRegistrations.All(r => r.Entry.Kind == "tombstone"))
        {
            // Nothing to detour and nothing new to load: pure deletions
            // (non-magic member removals — the loaded code is already
            // correct, the members are merely unreachable) and/or pure
            // accessibility narrowing. Commit tombstones so later batches
            // fail deterministically on references; skip the pointless
            // assembly. (A clear-marker keeps messageDrivers non-empty so a
            // driver-clearing re-edit still ships to Unity, not a no-op.)
            if (!string.IsNullOrEmpty(generation))
            {
                CommitShimRegistrations(generation!, shimRegistrations);
                // A play-mode-born re-edit that only removed a post-birth member /
                // field produces this empty-patch no-op, but its registry re-commit
                // (advancing LastAppliedText so the live baseline does not drift) is
                // still pending — commit it here, since the emit path below is
                // skipped. No new sibling can reach this branch (a load_only is not
                // a no-op), so no after-emit assembly pin is owed.
                if (newTypeRegistrations.Count > 0)
                    _newTypeRegistry.Commit(generation!, newTypeRegistrations);
            }
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
            emitResult = compilation.Emit(peStream, options: LightweightEmitOptions);
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
        // Pin the just-emitted assembly as the detour ORIGINAL side for NEW
        // play-mode-born files (the same emit-time bookkeeping as shims). Redirect
        // marks already carry the first assembly (non-empty) and must keep it.
        foreach (var registration in newTypeRegistrations)
            if (string.IsNullOrEmpty(registration.Value.OriginalAssembly))
                registration.Value.OriginalAssembly = assemblyName;
        // Feature #5: a genuinely-new SIBLING was load_only'd into THIS assembly;
        // pin it so the sibling's later re-edits redirect onto it. Existing
        // siblings already carry their (different) assembly and are left alone.
        foreach (var registration in newTypeRegistrations)
            if (registration.Value.Siblings != null)
                foreach (NewTypeRegistry.SiblingType sibling in registration.Value.Siblings.Values)
                    if (string.IsNullOrEmpty(sibling.Assembly))
                        sibling.Assembly = assemblyName;

        if (request.RegisterImage && !string.IsNullOrEmpty(generation))
        {
            _imageRegistry.Register(generation!, assemblyName, bytes);
            CommitShimRegistrations(generation!, shimRegistrations);
            CommitFieldStoreRegistrations(generation!, fieldStoreRegistrations);
            if (newTypeRegistrations.Count > 0)
                _newTypeRegistry.Commit(generation!, newTypeRegistrations);
        }
        else if ((shimRegistrations.Count > 0 || fieldStoreRegistrations.Count > 0 ||
                  newTypeRegistrations.Count > 0) &&
                 !string.IsNullOrEmpty(generation))
        {
            lock (_pendingShimLock)
            {
                if (shimRegistrations.Count > 0 || fieldStoreRegistrations.Count > 0)
                    _pendingShims[assemblyName] = (generation!, shimRegistrations, fieldStoreRegistrations);
                if (newTypeRegistrations.Count > 0)
                    _pendingNewTypes[assemblyName] = (generation!, newTypeRegistrations);
                // Keep the pending maps bounded: entries for other
                // generations can never commit.
                foreach (string stale in _pendingShims
                             .Where(p => p.Value.Generation != generation)
                             .Select(p => p.Key)
                             .ToList())
                {
                    _pendingShims.Remove(stale);
                }
                foreach (string stale in _pendingNewTypes
                             .Where(p => p.Value.Generation != generation)
                             .Select(p => p.Key)
                             .ToList())
                {
                    _pendingNewTypes.Remove(stale);
                }
            }
        }

        JsonNode result = SuccessResult(bytes, assemblyName, startedAt, request.ReturnAssemblyPath);
        result["hot"] = true;
        result["methods"] = methods;
        result["newTypes"] = newTypes;
        if (messageDrivers.Count > 0)
            result["messageDrivers"] = messageDrivers;
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
            emitResult = compilation.Emit(peStream, options: LightweightEmitOptions);
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

    public JsonNode HandleCallerQuery(JsonNode? @params)
    {
        var request = Deserialize<CallerQueryRequestDto>(@params);
        if (request.Targets == null || request.Targets.Length == 0)
            throw new RpcInvalidParamsException("caller/query requires at least one target");

        var projectAssemblies = (request.Params?.ReferencePaths ?? Array.Empty<string>())
            .Where(CallerScan.IsProjectAssemblyPath)
            .ToList();
        if (projectAssemblies.Count == 0)
        {
            return new JsonObject
            {
                ["success"] = false,
                ["error"] = "cannot query callers: no project assemblies (Library/ScriptAssemblies) in the reference set",
            };
        }

        var targets = new List<CallerScanTarget>();
        foreach (CallerQueryTargetDto target in request.Targets)
        {
            if (string.IsNullOrWhiteSpace(target.DeclaringType))
                throw new RpcInvalidParamsException("caller/query targets require declaringType");
            targets.Add(new CallerScanTarget
            {
                DeclaringType = target.DeclaringType!,
                MemberName = target.MemberName ?? "",
            });
        }

        CallerScanResult scan = CallerScan.Scan(projectAssemblies, targets);
        if (scan.Error != null)
        {
            return new JsonObject
            {
                ["success"] = false,
                ["error"] = scan.Error,
            };
        }

        var targetJson = new JsonArray();
        foreach (CallerScanTarget target in targets)
        {
            string key = CallerScanTarget.Key(target.DeclaringType, target.MemberName);
            var callers = new JsonArray();
            if (scan.CallerLocations.TryGetValue(key, out List<CallerScanLocation>? locations))
            {
                foreach (CallerScanLocation location in locations
                    .OrderBy(l => l.File, StringComparer.OrdinalIgnoreCase)
                    .ThenBy(l => l.CallerMethodKey, StringComparer.Ordinal))
                {
                    callers.Add(new JsonObject
                    {
                        ["file"] = location.File,
                        ["methodKey"] = location.CallerMethodKey,
                        ["declaringType"] = location.DeclaringType,
                        ["memberName"] = location.MemberName,
                    });
                }
            }
            targetJson.Add(new JsonObject
            {
                ["declaringType"] = target.DeclaringType,
                ["memberName"] = target.MemberName,
                ["key"] = key,
                ["callers"] = callers,
            });
        }

        return new JsonObject
        {
            ["success"] = true,
            ["assemblyCount"] = projectAssemblies.Count,
            ["targets"] = targetJson,
        };
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

    /// <summary>A play-mode-born re-edit that REMOVED a Unity message an earlier
    /// hot patch wired to the runtime pump: the pump drove it through the shim,
    /// not native dispatch, so it is silenced by CLEARING the driver (a
    /// clear-marker), not by an empty-body stub. Carries the file (the
    /// replace-by-source key) and the first assembly.</summary>
    private sealed class BornDriverClear
    {
        public string SourcePath = "";
        public string DeclaringType = "";
        public string Message = "";
        public string OriginalAssembly = "";
    }

    /// <summary>Route a re-edit of a play-mode-born file (whose types live only
    /// in a prior hot-patch assembly). The cumulative diff against the BIRTH text
    /// drives the patch — body redirects, additions (M2/M4/driver), removals (M5
    /// tombstone/stub), field add/remove/retype (M4), enum additions (H7e) — all
    /// pinned to the first assembly so EXISTING instances update. A second diff
    /// against the LAST APPLIED text catches what the birth diff cannot: a member
    /// ADDED after birth and now removed (e.g. an Update() added by an earlier
    /// patch, then deleted), which the birth diff never saw because the member
    /// was never in OriginalText. SIBLING types added after the first batch
    /// (feature #5) are folded in via their own per-type baseline and assembly.
    /// Outcomes:
    ///   • HOT — the cumulative diff, assemblies pinned per type, post-birth
    ///     removals folded in (tombstones; removed messages → clear-markers),
    ///     sibling additions load_only'd and registered, sibling re-edits
    ///     redirected onto their own assembly;
    ///   • NO-OP — an unchanged re-send of a never-touched file;
    ///   • COLD — a shape change (base type / kind / record), a redirected body
    ///     reverted to its birth version, a removal on a sibling type, or an
    ///     unresolvable assembly.
    /// Mutates <paramref name="reeditFileAssemblies"/> /
    /// <paramref name="reeditTypeAssemblies"/> (detour origins),
    /// <paramref name="newTypeRegistrations"/> (live-text re-commit) and
    /// <paramref name="bornDriverClears"/> (removed-message clear-markers).</summary>
    private HotDiffFileResult RouteBornReedit(
        HotDiffFileDto file,
        NewTypeRegistry.FileEntry bornEntry,
        CompileHotPatchRequestDto request,
        CSharpParseOptions parseOptions,
        Dictionary<string, string> reeditFileAssemblies,
        Dictionary<string, string> reeditTypeAssemblies,
        List<KeyValuePair<string, NewTypeRegistry.FileEntry>> newTypeRegistrations,
        List<BornDriverClear> bornDriverClears)
    {
        string pathKey = NormalizePathKey(file.Path!);
        string? generation = request.Params?.DomainGeneration;

        // The redirect target (first assembly) must still be referenceable — the
        // patch's layout guard resolves the original type there.
        bool canRedirect =
            request.ReferenceSessionImages &&
            _imageRegistry.Contains(generation, bornEntry.OriginalAssembly);
        if (!canRedirect)
            return ColdBornReedit("the first hot-patch assembly is no longer resolvable; recompile to converge");

        HotDiffFileResult cumulative = HotDiff.Analyze(bornEntry.OriginalText, file.NewText!, parseOptions);

        // Truly structural — base type / kind / record: HotDiff already classified
        // the cumulative diff cold (no shim or store can express it).
        if (!cumulative.Hot)
            return ColdBornReedit(cumulative.Reasons.DefaultIfEmpty(
                "the re-edit changes the type's shape; recompile to converge").First());

        // Feature #5: SIBLING types. Fold already-born siblings' body/additive
        // changes into `cumulative` (redirected onto their OWN assembly), collect
        // genuinely-new ones (load_only this batch + register), and steer a sibling
        // TYPE removal / birth-member removal cold (conservative). Post-birth-added
        // sibling members removed are caught by the file live diff below — exactly
        // like first-batch types.
        var newSiblings = new List<string>();
        HotDiffFileResult? siblingCold = HandleBornSiblings(
            file, bornEntry, generation, cumulative, parseOptions, reeditTypeAssemblies, newSiblings);
        if (siblingCold != null)
            return siblingCold;

        string liveBaseline = string.IsNullOrEmpty(bornEntry.LastAppliedText)
            ? bornEntry.OriginalText
            : bornEntry.LastAppliedText;
        HotDiffFileResult live = HotDiff.Analyze(liveBaseline, file.NewText!, parseOptions);

        HashSet<string> cumulativeKeys = CumulativeMemberKeys(cumulative);

        // Members the LIVE text carried but the new text dropped, that the birth
        // diff cannot see (post-birth additions never existed in OriginalText): a
        // removed Unity MESSAGE → clear its driver; a removed plain member →
        // tombstone. Birth-member removals are already in cumulative.RemovedMembers
        // (skipped here via cumulativeKeys). SIBLING members are NOT skipped: the
        // file live diff is the ONLY place a member ADDED after the sibling's birth
        // and now removed surfaces (the sibling diff is against its FIXED BirthText,
        // which never had it). They route through the same machinery — a removed
        // sibling MESSAGE clears via a clear-marker (its driver was registered under
        // this file path, so ClearSource(file) tears it down regardless of which
        // assembly the type lives in), and the clear-marker's originalAssembly is
        // resolved per-type. Defensive: only trust `live` when it is itself hot
        // (LastAppliedText is always birth + hot changes, so a cold `live` cannot
        // actually arise here).
        var removedMagic = new List<HotDiffRemovedMember>();
        var removedNormal = new List<HotDiffRemovedMember>();
        bool removedLiveField = false;
        bool revertToBirth = false;
        if (live.Hot)
        {
            foreach (HotDiffRemovedMember removed in live.RemovedMembers)
            {
                string key = MemberSurfaceRegistry.MemberKey(
                    removed.DeclaringType, removed.Name, removed.ParamTypeNames, removed.IsStatic);
                if (cumulativeKeys.Contains(key))
                    continue; // a birth / sib-birth member — the cumulative diff handles it
                if (removed.IsUnityMagic)
                    removedMagic.Add(removed);
                else
                    removedNormal.Add(removed);
            }

            // A FIELD added after birth and now removed surfaces only in
            // live.FieldChanges (Kind="removed"), never RemovedMembers. There is
            // nothing to emit (the side store is simply abandoned, harmless — no
            // live instance referenced it), but it must NOT pass as an unchanged
            // re-send: routing it hot advances LastAppliedText so the baseline does
            // not drift. A BIRTH / sib-birth field removal is in
            // cumulative.FieldChanges (layout placeholder) — skip those.
            var cumulativeFieldKeys = new HashSet<string>(
                cumulative.FieldChanges.Select(FieldKey), StringComparer.Ordinal);
            removedLiveField = live.FieldChanges.Any(f =>
                string.Equals(f.Kind, "removed", StringComparison.Ordinal) &&
                !cumulativeFieldKeys.Contains(FieldKey(f)));

            // A redirected BODY reverted to its birth version: the live text
            // changed a member that the new text returns to its birth body (so the
            // cumulative/sibling diff shows no change for it — its key is absent
            // from cumulativeKeys, which already folds in the merged sibling diff).
            // A live detour cannot be un-redirected in place → recompile. Siblings
            // are NOT excluded: a sibling body revert is exactly the case the
            // sibling's fixed-BirthText diff misses, and like a first-batch revert
            // it must go cold (the only removal we cannot do hot).
            revertToBirth = live.ChangedMethods.Any(m =>
                !m.Added &&
                !cumulativeKeys.Contains(MemberSurfaceRegistry.MemberKey(
                    m.DeclaringType, m.Name, m.ParamTypeNames, m.IsStatic)));
        }
        if (revertToBirth)
            return ColdBornReedit("a redirected method body was reverted to its birth version; recompile to converge");

        bool hasRemovedLiveAdditions = removedMagic.Count > 0 || removedNormal.Count > 0 || removedLiveField;
        bool cumulativeHasChange = CumulativeHasAnyChange(cumulative);

        // (NO-OP) nothing to apply: an unchanged re-send — the new text equals the
        // last applied text (an already-born sibling whose body is unchanged folds
        // to an empty diff), or the birth text of a never-touched file. The
        // coordinator re-ships every dirty file each convergence batch, so this
        // MUST stay a clean no-op, never cold (the self-test replay relies on it).
        // A genuine REVERT was already steered cold above (revertToBirth), and
        // post-birth removals are folded in below — so an empty diff here is truly
        // nothing to do. The Redirected flag is deliberately NOT consulted: a
        // redirected file re-sending its applied text is a no-op, not a revert.
        if (!cumulativeHasChange && !hasRemovedLiveAdditions)
            return cumulative; // empty diff contributes nothing

        // (HOT) the cumulative diff drives the patch; fold in post-birth removals.
        // A play-mode-born type has NO compiled call sites (it lives only in a
        // dynamic assembly), so the M3 caller scan is vacuous AND would cold on
        // "no project assemblies". Drop the checks — by construction no compiled
        // caller of this type can exist to break.
        cumulative.RequiresCallerCheck.Clear();

        foreach (HotDiffRemovedMember removed in removedNormal)
            cumulative.RemovedMembers.Add(TombstoneOnly(removed));
        foreach (HotDiffRemovedMember removed in removedMagic)
        {
            // A removed added MESSAGE needs ONLY a clear-marker (stop the pump) —
            // no tombstone. A tombstone would force the rewriter to emit the
            // (otherwise unchanged) born type into this patch assembly as a
            // name-colliding duplicate of the first assembly's type; a pure clear
            // instead compiles to an empty assembly whose sole job is to carry the
            // clear-marker to Unity.
            bornDriverClears.Add(new BornDriverClear
            {
                SourcePath = file.Path!,
                DeclaringType = removed.DeclaringType,
                Message = removed.Name,
                // Per-type: a late-born sibling's message (and its nested types)
                // lived in its OWN assembly (informational — the clear keys off
                // sourcePath, but keep it consistent with how the driver was pinned).
                OriginalAssembly = PatchBatchContext.ReeditTypeAssembly(reeditTypeAssemblies, removed.DeclaringType)
                    ?? bornEntry.OriginalAssembly,
            });
        }

        reeditFileAssemblies[file.Path!] = bornEntry.OriginalAssembly;
        // Re-commit with the new live text and the sibling set. Redirected reflects
        // whether any patch state REMAINS: a pure driver-clear back to the birth
        // text leaves none, so a later unchanged re-send is a clean no-op.
        // Committed on image accept; genuinely-new siblings get their assembly
        // pinned after emit.
        newTypeRegistrations.Add(new KeyValuePair<string, NewTypeRegistry.FileEntry>(
            pathKey,
            BuildBornRecommit(bornEntry, file.NewText!, cumulativeHasChange, newSiblings)));
        return cumulative;
    }

    /// <summary>Feature #5: fold sibling top-level types into a born re-edit.
    /// Returns a COLD result to abort, or null to proceed (mutating
    /// <paramref name="cumulative"/>, <paramref name="reeditTypeAssemblies"/> and
    /// <paramref name="newSiblings"/>):
    ///   • a registered sibling missing from the new text → COLD (removed type);
    ///   • a NewType already registered as a sibling → redirect: diff it against
    ///     its OWN birth text, fold its hot changes into the cumulative diff, and
    ///     pin its assembly (a sib-BIRTH member removal on it is conservatively
    ///     COLD; a member added AFTER birth and removed is caught hot by the file
    ///     live diff in the caller);
    ///   • a NewType not yet registered → a genuinely-new sibling: leave it in
    ///     cumulative.NewTypes (load_only this batch) and record it for
    ///     registration.</summary>
    private HotDiffFileResult? HandleBornSiblings(
        HotDiffFileDto file,
        NewTypeRegistry.FileEntry bornEntry,
        string? generation,
        HotDiffFileResult cumulative,
        CSharpParseOptions parseOptions,
        Dictionary<string, string> reeditTypeAssemblies,
        List<string> newSiblings)
    {
        if (bornEntry.Siblings != null)
        {
            // A registered sibling that VANISHED from the new text is a removed
            // type — its live instances cannot be cleaned up in place. Recompile.
            HashSet<string> present = TopLevelTypeMetadataNames(file.NewText!, parseOptions);
            foreach (string sib in bornEntry.Siblings.Keys)
                if (!present.Contains(sib))
                    return ColdBornReedit("a sibling type was removed from a play-mode-born file; recompile to converge");
        }

        if (cumulative.NewTypes.Count == 0)
            return null;

        var alreadyBorn = new List<string>();
        foreach (string nt in cumulative.NewTypes)
        {
            // Only TOP-LEVEL types are tracked as siblings. A NESTED new type
            // (metadata names nest with '+') belongs to its parent — it is
            // load_only'd WITH the parent and matched via BelongsToType, never
            // registered on its own (else the next re-edit's top-level-only
            // presence check would mistake it for a removed sibling and cold).
            if (nt.Contains('+'))
                continue;
            if (bornEntry.Siblings != null && bornEntry.Siblings.ContainsKey(nt))
            {
                alreadyBorn.Add(nt);
            }
            else
            {
                // Genuinely new: load_only this batch (stays in NewTypes), register.
                newSiblings.Add(nt);
            }
        }
        // Drop already-born siblings AND their nested types from the load_only set
        // (they redirect via the sibling's own diff — re-loading would strand the
        // sibling's live instances). Genuinely-new siblings and their nested types
        // stay, to be load_only'd this batch.
        cumulative.NewTypes.RemoveAll(nt => alreadyBorn.Any(sib => BelongsToType(nt, sib)));

        // [P2] A remaining NESTED NewType whose top-level parent is NOT a
        // genuinely-new sibling means a nested type was added to an ALREADY-LOADED
        // type (a first-batch type, or an already-born sibling — the latter also
        // colds in the loop below). You cannot add a nested type to a loaded type
        // in place, and it would otherwise re-load every resend (cumulative.NewTypes
        // never empties → never a clean no-op). Cold.
        foreach (string nt in cumulative.NewTypes)
        {
            int plus = nt.IndexOf('+');
            if (plus >= 0 && !newSiblings.Contains(nt.Substring(0, plus)))
                return ColdBornReedit("a nested type was added to an existing play-mode-born type; recompile to converge");
        }

        foreach (string sib in alreadyBorn)
        {
            NewTypeRegistry.SiblingType entry = bornEntry.Siblings![sib];
            if (!_imageRegistry.Contains(generation, entry.Assembly))
                return ColdBornReedit("a sibling type's assembly is no longer resolvable; recompile to converge");

            HotDiffFileResult sibDiff = HotDiff.Analyze(entry.BirthText, file.NewText!, parseOptions);
            if (!sibDiff.Hot)
                return ColdBornReedit("a sibling type changed shape; recompile to converge");
            // Conservative boundary: a member/type REMOVAL or signature change on a
            // sibling (which surfaces RemovedMembers/RemovedTypes) → recompile,
            // rather than re-deriving the per-sibling clear-marker / tombstone logic.
            if (sibDiff.RemovedMembers.Any(r => BelongsToType(r.DeclaringType, sib)) ||
                sibDiff.RemovedTypes.Any(t => BelongsToType(t.MetadataName, sib)) ||
                sibDiff.NewTypes.Any(n => BelongsToType(n, sib)))
            {
                return ColdBornReedit("a member was removed from, or a nested type added to, a sibling type; recompile to converge");
            }

            MergeSiblingInto(cumulative, sibDiff, sib);
            reeditTypeAssemblies[sib] = entry.Assembly;
        }
        return null;
    }

    /// <summary>Merge a sibling's own-baseline diff entries (only those belonging
    /// to <paramref name="sib"/>) into the file's cumulative diff, so the
    /// whole-file rewrite redirects/extends the sibling onto its own assembly.</summary>
    private static void MergeSiblingInto(HotDiffFileResult cumulative, HotDiffFileResult sibDiff, string sib)
    {
        cumulative.ChangedMethods.AddRange(sibDiff.ChangedMethods.Where(m => BelongsToType(m.DeclaringType, sib)));
        cumulative.FieldChanges.AddRange(sibDiff.FieldChanges.Where(f => BelongsToType(f.DeclaringType, sib)));
        cumulative.EnumAdditions.AddRange(sibDiff.EnumAdditions.Where(e => BelongsToType(e.EnumType, sib)));
        foreach (string pt in sibDiff.PatchedTypes.Where(p => BelongsToType(p, sib)))
            if (!cumulative.PatchedTypes.Contains(pt))
                cumulative.PatchedTypes.Add(pt);
    }

    /// <summary>True when <paramref name="typeName"/> is the sibling type or one
    /// of its nested types (metadata names nest with '+').</summary>
    private static bool BelongsToType(string typeName, string sib) =>
        string.Equals(typeName, sib, StringComparison.Ordinal) ||
        typeName.StartsWith(sib + "+", StringComparison.Ordinal);

    /// <summary>Metadata names of every TOP-LEVEL type (class/struct/interface/
    /// record/enum) declared in <paramref name="text"/>.</summary>
    private static HashSet<string> TopLevelTypeMetadataNames(string text, CSharpParseOptions parseOptions)
    {
        var names = new HashSet<string>(StringComparer.Ordinal);
        var root = (CompilationUnitSyntax)CSharpSyntaxTree.ParseText(text, parseOptions).GetRoot();
        foreach (BaseTypeDeclarationSyntax decl in root.DescendantNodes().OfType<BaseTypeDeclarationSyntax>())
        {
            if (decl.Ancestors().OfType<BaseTypeDeclarationSyntax>().Any())
                continue; // nested
            names.Add(HotDiff.MetadataName(decl));
        }
        return names;
    }

    /// <summary>Build the live-text re-commit for a born file, carrying the
    /// sibling set forward: existing siblings keep their (pinned) assembly, and
    /// each genuinely-new sibling is added with this file's birth text and an
    /// EMPTY assembly to be pinned after emit.</summary>
    private static NewTypeRegistry.FileEntry BuildBornRecommit(
        NewTypeRegistry.FileEntry bornEntry, string newText, bool redirected, List<string> newSiblings)
    {
        Dictionary<string, NewTypeRegistry.SiblingType>? siblings = null;
        if (bornEntry.Siblings != null || newSiblings.Count > 0)
        {
            siblings = new Dictionary<string, NewTypeRegistry.SiblingType>(StringComparer.Ordinal);
            if (bornEntry.Siblings != null)
                foreach (var kv in bornEntry.Siblings)
                    siblings[kv.Key] = kv.Value; // already pinned; never re-pinned below
            foreach (string sib in newSiblings)
                siblings[sib] = new NewTypeRegistry.SiblingType { Assembly = "", BirthText = newText };
        }
        return new NewTypeRegistry.FileEntry
        {
            OriginalText = bornEntry.OriginalText,
            OriginalAssembly = bornEntry.OriginalAssembly,
            Redirected = redirected,
            LastAppliedText = newText,
            Siblings = siblings,
        };
    }

    /// <summary>Member-identity keys the cumulative (birth) diff already knows
    /// about — changed (added or not) and removed members. A live-diff removal or
    /// change whose key is ABSENT was added after birth (removal) or reverted to
    /// the birth body (change).</summary>
    private static HashSet<string> CumulativeMemberKeys(HotDiffFileResult diff)
    {
        var keys = new HashSet<string>(StringComparer.Ordinal);
        foreach (HotDiffMethod m in diff.ChangedMethods)
            keys.Add(MemberSurfaceRegistry.MemberKey(m.DeclaringType, m.Name, m.ParamTypeNames, m.IsStatic));
        foreach (HotDiffRemovedMember r in diff.RemovedMembers)
            keys.Add(MemberSurfaceRegistry.MemberKey(r.DeclaringType, r.Name, r.ParamTypeNames, r.IsStatic));
        return keys;
    }

    /// <summary>Any hot-applicable change in the cumulative (birth) diff. An
    /// all-false result with no post-birth removals is the unchanged-re-send
    /// no-op.</summary>
    private static bool CumulativeHasAnyChange(HotDiffFileResult diff)
    {
        return diff.ChangedMethods.Count > 0
            || diff.NewTypes.Count > 0
            || diff.RemovedMembers.Count > 0
            || diff.RemovedTypes.Count > 0
            || diff.FieldChanges.Count > 0
            || diff.EnumAdditions.Count > 0
            || diff.RequiresCallerCheck.Count > 0;
    }

    /// <summary>A post-birth-added member removed: tombstone it (so later
    /// references fail deterministically) but DON'T re-materialize a magic stub —
    /// a play-mode-born message was driven by the runtime pump (cleared
    /// separately via a clear-marker), never by native dispatch, so a stub detour
    /// would target a call the engine never makes.</summary>
    private static HotDiffRemovedMember TombstoneOnly(HotDiffRemovedMember removed)
    {
        return new HotDiffRemovedMember
        {
            DeclaringType = removed.DeclaringType,
            Name = removed.Name,
            ParamTypeNames = removed.ParamTypeNames,
            IsStatic = removed.IsStatic,
            IsUnityMagic = false,
            StubSource = null,
        };
    }

    private static HotDiffFileResult ColdBornReedit(string reason)
    {
        var cold = new HotDiffFileResult { Hot = false };
        cold.Reasons.Add(reason);
        return cold;
    }

    /// <summary>Field identity across diffs (declaring type + name + staticness).</summary>
    private static string FieldKey(HotDiffFieldChange f) =>
        f.DeclaringType + "|field|" + f.Name + (f.IsStatic ? "|s" : "|i");

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

    /// <summary>
    /// Produce the project SerializedProperty schema from reference metadata:
    /// member field types, Unity attributes, enum shape and assignability
    /// inputs for managed reference candidates.
    /// </summary>
    public JsonNode HandleIndexSchema(JsonNode? @params)
    {
        var request = Deserialize<IndexSchemaRequestDto>(@params);
        long startedAt = Environment.TickCount64;

        ImmutableArray<MetadataReference> references = ResolveReferences(request.Params, useHostBcl: false);
        JsonObject result = SerializedSchemaSource.Build(
            references.OfType<PortableExecutableReference>());
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

        JsonNode result = SuccessResult(bytes, assemblyName, startedAt, request.ReturnAssemblyPath);
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
        bool referenceSessionImages,
        bool emitDebugSymbols,
        DiagnosticStyle style = DiagnosticStyle.Snippet)
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
            return (null, "parse failed: " + (style == DiagnosticStyle.ViewScript ? ex.Message : ex.ToString()));
        }

        return EmitCompilation(
            assemblyName,
            trees,
            compileParams,
            useHostBcl,
            referenceSessionImages,
            style,
            emitDebugSymbols: emitDebugSymbols);
    }

    /// <summary>Which Unity-side error formatting a compile mirrors.</summary>
    private enum DiagnosticStyle
    {
        /// <summary>execute/run_states: BuildDiagnosticErrorText, full exception text.</summary>
        Snippet,
        /// <summary>View Scripts: path-qualified diagnostics, exception .Message only.</summary>
        ViewScript,
    }

    private static DiagnosticStyle ResolveDiagnosticStyle(string? value)
    {
        return string.Equals(value, "viewScript", StringComparison.OrdinalIgnoreCase)
            ? DiagnosticStyle.ViewScript
            : DiagnosticStyle.Snippet;
    }

    private (byte[]? Bytes, string? Error) EmitCompilation(
        string assemblyName,
        IReadOnlyList<SyntaxTree> trees,
        CompileParamsDto? compileParams,
        bool useHostBcl,
        bool referenceSessionImages,
        DiagnosticStyle style = DiagnosticStyle.Snippet,
        bool emitDebugSymbols = false)
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
            emitResult = compilation.Emit(peStream, options: EmitOptionsFor(emitDebugSymbols));
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

    private static JsonNode SuccessResult(
        byte[] bytes,
        string assemblyName,
        long startedAt,
        bool returnAssemblyPath = false)
    {
        var result = new JsonObject
        {
            ["success"] = true,
            ["assemblyName"] = assemblyName,
            ["durationMs"] = Environment.TickCount64 - startedAt,
        };

        string? assemblyPath = returnAssemblyPath
            ? TryWriteAssemblyArtifact(bytes, assemblyName)
            : null;
        if (!string.IsNullOrEmpty(assemblyPath))
            result["assemblyPath"] = assemblyPath;
        else
            result["assemblyB64"] = Convert.ToBase64String(bytes);

        return result;
    }

    private static string? TryWriteAssemblyArtifact(byte[] bytes, string assemblyName)
    {
        try
        {
            string dir = CurrentAssemblyArtifactDir();
            Directory.CreateDirectory(dir);
            string fileName = SanitizeAssemblyNamePart(assemblyName) + "_" +
                              Guid.NewGuid().ToString("N") + ".dll";
            string path = Path.Combine(dir, fileName);
            File.WriteAllBytes(path, bytes);
            PruneAssemblyArtifacts(dir);
            return path;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("[LocusCompileServer] assembly artifact write failed: " + ex.Message);
            return null;
        }
    }

    private static string AssemblyArtifactRootPath()
    {
        return Path.Combine(Path.GetTempPath(), AssemblyArtifactRootName);
    }

    private static string CurrentAssemblyArtifactDir()
    {
        return Path.Combine(
            AssemblyArtifactRootPath(),
            Environment.ProcessId.ToString(CultureInfo.InvariantCulture));
    }

    private static void PruneAssemblyArtifactRootOnce()
    {
        if (Interlocked.Exchange(ref _assemblyArtifactRootPruned, 1) != 0)
            return;

        PruneAssemblyArtifactRoot(AssemblyArtifactRootPath(), DateTime.UtcNow);
    }

    private static void PruneAssemblyArtifactRoot(string root, DateTime now)
    {
        try
        {
            var rootInfo = new DirectoryInfo(root);
            if (!rootInfo.Exists)
                return;

            int currentPid = Environment.ProcessId;
            foreach (DirectoryInfo dir in rootInfo.EnumerateDirectories())
            {
                try
                {
                    if (!int.TryParse(dir.Name, NumberStyles.None, CultureInfo.InvariantCulture, out int pid))
                        continue;

                    if (pid == currentPid)
                    {
                        PruneAssemblyArtifacts(dir.FullName);
                        continue;
                    }

                    bool pidIsAlive = IsProcessAlive(pid);
                    bool directoryTooOld = now - LastActivityUtc(dir) > AssemblyArtifactMaxAge;
                    if (!pidIsAlive || directoryTooOld)
                    {
                        dir.Delete(recursive: true);
                        continue;
                    }

                    PruneAssemblyArtifacts(dir.FullName);
                }
                catch (Exception ex)
                {
                    Console.Error.WriteLine(
                        "[LocusCompileServer] assembly artifact directory prune failed: " +
                        dir.FullName + ": " + ex.Message);
                }
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("[LocusCompileServer] assembly artifact root prune failed: " + ex.Message);
        }
    }

    private static DateTime LastActivityUtc(DirectoryInfo dir)
    {
        dir.Refresh();
        DateTime last = dir.LastWriteTimeUtc;
        foreach (FileSystemInfo entry in dir.EnumerateFileSystemInfos())
        {
            if (entry.LastWriteTimeUtc > last)
                last = entry.LastWriteTimeUtc;
        }
        return last;
    }

    private static bool IsProcessAlive(int pid)
    {
        if (pid <= 0)
            return false;

        try
        {
            using Process process = Process.GetProcessById(pid);
            return !process.HasExited;
        }
        catch (ArgumentException)
        {
            return false;
        }
        catch (InvalidOperationException)
        {
            return false;
        }
        catch
        {
            return true;
        }
    }

    private static void PruneAssemblyArtifacts(string dir)
    {
        try
        {
            var now = DateTime.UtcNow;
            var files = new DirectoryInfo(dir)
                .GetFiles("*.dll")
                .OrderByDescending(file => file.LastWriteTimeUtc)
                .ToArray();

            for (int i = 0; i < files.Length; i++)
            {
                bool tooMany = i >= MaxAssemblyArtifactFiles;
                bool tooOld = now - files[i].LastWriteTimeUtc > AssemblyArtifactMaxAge;
                if (tooMany || tooOld)
                    files[i].Delete();
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("[LocusCompileServer] assembly artifact prune failed: " + ex.Message);
        }
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
