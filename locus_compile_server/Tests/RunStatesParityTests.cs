using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// Golden parity for the run_states source generator and request validation
/// against the Unity-side implementation.
/// </summary>
public class RunStatesParityTests
{
    private static (RunStatesRequest Port, UnityReferenceImpl.RunStatesRequestRef Reference) BuildPair(
        string? editorStatus,
        string? initialState,
        (string? Name, string? Variables, string? Start, string? Update, string? End)[] states,
        string[]? autoUsings)
    {
        var port = new RunStatesRequest
        {
            RequestEditorStatus = editorStatus,
            InitialState = initialState,
            States = states
                .Select(s => new RunStatesStateRequest
                {
                    Name = s.Name,
                    Variables = s.Variables,
                    Start = s.Start,
                    Update = s.Update,
                    End = s.End,
                })
                .ToArray(),
            AutoUsings = autoUsings,
        };
        var reference = new UnityReferenceImpl.RunStatesRequestRef
        {
            request_editor_status = editorStatus,
            initial_state = initialState,
            states = states
                .Select(s => new UnityReferenceImpl.RunStatesStateRequestRef
                {
                    name = s.Name,
                    variables = s.Variables,
                    start = s.Start,
                    update = s.Update,
                    end = s.End,
                })
                .ToArray(),
            auto_usings = autoUsings,
        };
        return (port, reference);
    }

    [Fact]
    public void Minimal_single_state_source_matches_unity()
    {
        var (port, reference) = BuildPair(
            "playing",
            "main",
            new[] { ((string?)"main", (string?)null, (string?)null, (string?)"ctx.Done(\"ok\");", (string?)null) },
            null);

        Assert.Equal(
            UnityReferenceImpl.BuildRunStatesSource(reference),
            RunStatesSource.BuildRunStatesSource(port));
    }

    [Fact]
    public void Full_matrix_source_matches_unity()
    {
        var states = new[]
        {
            ((string?)" spaced name ", (string?)"var counter = ctx.Var<int>(\"counter\");",
                (string?)"counter.Value = 0;", (string?)"counter.Value++;\nif (counter.Value > 3) ctx.Goto(\"second\");", (string?)null),
            ((string?)"second", (string?)null,
                (string?)null, (string?)"ctx.Done(\"done \\\"quoted\\\"\");", (string?)"Debug.Log(\"end\\n\");"),
            ((string?)"weird\"name\twith\nbreaks", (string?)null,
                (string?)null, (string?)"ctx.Sleep(1);", (string?)null),
        };
        var autoUsings = new[]
        {
            "UnityEngine.Rendering",
            "UnityEngine.Rendering", // duplicate: dropped
            "  TMPro  ",             // trimmed
            "bad namespace!",        // invalid: dropped
            "",                       // empty: dropped
            "My_Game.Systems",
        };

        var (port, reference) = BuildPair("editing", " spaced name ", states, autoUsings);

        Assert.Equal(
            UnityReferenceImpl.BuildRunStatesSource(reference),
            RunStatesSource.BuildRunStatesSource(port));
    }

    [Fact]
    public void Entry_type_matches_the_unity_contract()
    {
        Assert.Equal(
            "Locus.RuntimeStateMachines.__LocusRunStatesHost",
            RunStatesSource.FullHostTypeName);
        Assert.Equal("LocusRunStates.cs", RunStatesSource.SourcePath);
    }

    // ── validation message parity (strings locked by the Unity-side
    //    ValidateRunStatesRequest implementation) ─────────────────────

    [Fact]
    public void Validation_messages_match_unity_wording()
    {
        Assert.Equal("run_states request is empty", RunStatesSource.ValidateRunStatesRequest(null));

        Assert.Equal(
            "request_editor_status is required",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest()));

        Assert.Equal(
            "unsupported request_editor_status: paused",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
            {
                RequestEditorStatus = "paused",
            }));

        Assert.Equal(
            "initial_state is required",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
            {
                RequestEditorStatus = "playing",
            }));

        Assert.Equal(
            "states must contain at least one state",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
            {
                RequestEditorStatus = "playing",
                InitialState = "main",
                States = Array.Empty<RunStatesStateRequest>(),
            }));

        Assert.Equal(
            "states[0].name is required",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
            {
                RequestEditorStatus = "playing",
                InitialState = "main",
                States = new[] { new RunStatesStateRequest { Update = "x" } },
            }));

        Assert.Equal(
            "duplicate state name: main",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
            {
                RequestEditorStatus = "playing",
                InitialState = "main",
                States = new[]
                {
                    new RunStatesStateRequest { Name = "main", Update = "x" },
                    new RunStatesStateRequest { Name = "main", Update = "x" },
                },
            }));

        Assert.Equal(
            "state 'main' requires update code",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
            {
                RequestEditorStatus = "playing",
                InitialState = "main",
                States = new[] { new RunStatesStateRequest { Name = "main" } },
            }));

        Assert.Equal(
            "initial_state not found in states: missing",
            RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
            {
                RequestEditorStatus = "playing",
                InitialState = "missing",
                States = new[] { new RunStatesStateRequest { Name = "main", Update = "x" } },
            }));

        Assert.Null(RunStatesSource.ValidateRunStatesRequest(new RunStatesRequest
        {
            RequestEditorStatus = "playing",
            InitialState = "main",
            States = new[] { new RunStatesStateRequest { Name = "main", Update = "x" } },
        }));
    }
}
