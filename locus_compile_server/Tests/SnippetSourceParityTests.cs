using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// Golden parity: the server's snippet source generators must produce
/// byte-identical output to the Unity-side implementation
/// (UnityReferenceImpl) for any input, in both wrapper modes.
/// </summary>
public class SnippetSourceParityTests
{
    public static TheoryData<string> SnippetInputs => new()
    {
        "",
        "1 + 1",
        "var x = 1;\nreturn x;",
        "using UnityEngine.Rendering;\n\nvar c = new Color(1, 0, 0);\nprint(c);",
        "using A.B;\nusing C;\n\n// comment, not a using\nusing (var d = new MemoryStream()) { }\nreturn 2;",
        "\r\nusing X.Y;\r\nvar v = GameObject.Find(\"Player\");\r\nprint(v);\r\n",
        "   \n\nusing Z;\nreturn 0;",
        "print(\"unicode 中文 🚀\");",
        "using NotTerminated\nvar tail = 1;",
        "// leading comment\nusing After.Comment;\nreturn 1;",
    };

    [Theory]
    [MemberData(nameof(SnippetInputs))]
    public void SplitLeadingUsings_matches_unity(string code)
    {
        UnityReferenceImpl.SplitLeadingUsings(code, out string expectedUsings, out string expectedBody);
        UnitySnippetSource.SplitLeadingUsings(code, out string actualUsings, out string actualBody);

        Assert.Equal(expectedUsings, actualUsings);
        Assert.Equal(expectedBody, actualBody);
    }

    [Theory]
    [MemberData(nameof(SnippetInputs))]
    public void BuildAsyncSnippetSource_matches_unity_in_both_modes(string code)
    {
        UnityReferenceImpl.SplitLeadingUsings(code, out string leadingUsings, out string bodyCode);

        foreach (bool expressionMode in new[] { false, true })
        {
            string expected = UnityReferenceImpl.BuildAsyncSnippetSource(
                "__LocusAsyncSnippetHost", leadingUsings, bodyCode, expressionMode);
            string actual = UnitySnippetSource.BuildAsyncSnippetSource(
                "__LocusAsyncSnippetHost", leadingUsings, bodyCode, expressionMode);

            Assert.Equal(expected, actual);
        }
    }

    [Fact]
    public void Host_type_names_match_the_unity_contract()
    {
        Assert.Equal("__LocusAsyncSnippetHost", UnitySnippetSource.HostTypeName);
        Assert.Equal(
            "Locus.RuntimeSnippets.__LocusAsyncSnippetHost",
            UnitySnippetSource.FullHostTypeName);
        Assert.Equal("LocusRuntimeAsyncSnippet.cs", UnitySnippetSource.SourcePath);
    }
}
