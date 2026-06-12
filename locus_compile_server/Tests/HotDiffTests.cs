using Microsoft.CodeAnalysis.CSharp;
using Xunit;

namespace Locus.CompileServer.Tests;

/// <summary>
/// Classification matrix for the hot-reload diff. These pin the hot/cold
/// decision table from unity-hotreload-plan.md §3: anything that can change
/// field layout, inlined constants, signatures, or type shape must be cold;
/// body-level edits and additive-safe members are hot.
/// </summary>
public class HotDiffTests
{
    private static readonly CSharpParseOptions Options = new(
        languageVersion: LanguageVersion.CSharp9,
        preprocessorSymbols: new[] { "UNITY_EDITOR" });

    private static HotDiffFileResult Analyze(string oldText, string newText, CSharpParseOptions? options = null)
    {
        return HotDiff.Analyze(oldText, newText, options ?? Options);
    }

    private const string PlayerOld = @"
using UnityEngine;
namespace Game
{
    public class Player : MonoBehaviour
    {
        private int _health = 100;
        public void Update() { _health += 1; }
        private void Helper(string name) { Debug.Log(name); }
    }
}";

    // ── hot: body-level edits ────────────────────────────────────────

    [Fact]
    public void Method_body_change_is_hot()
    {
        var result = Analyze(PlayerOld, PlayerOld.Replace("_health += 1;", "_health += 2;"));

        Assert.True(result.Hot);
        var method = Assert.Single(result.ChangedMethods);
        Assert.Equal("Game.Player", method.DeclaringType);
        Assert.Equal("Update", method.Name);
        Assert.Empty(method.ParamTypeNames);
        Assert.False(method.IsStatic);
        Assert.False(method.Added);
        Assert.Equal(new[] { "Game.Player" }, result.PatchedTypes);
        Assert.Empty(result.NewTypes);
    }

    [Fact]
    public void Comment_and_whitespace_only_change_is_a_hot_noop()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace("_health += 1;", "_health += 1; // tick\n        "));

        Assert.True(result.Hot);
        Assert.Empty(result.ChangedMethods);
        Assert.Empty(result.PatchedTypes);
    }

    [Fact]
    public void Expression_bodied_method_change_is_hot()
    {
        const string oldText = "class A { int M() => 1; }";
        const string newText = "class A { int M() => 2; }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    [Fact]
    public void Async_method_body_change_is_hot()
    {
        const string oldText = "using System.Threading.Tasks; class A { async Task M() { await Task.Yield(); } }";
        const string newText = "using System.Threading.Tasks; class A { async Task M() { await Task.Delay(1); } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    [Fact]
    public void Async_to_sync_conversion_is_hot()
    {
        const string oldText = "using System.Threading.Tasks; class A { async Task M() { await Task.Yield(); } }";
        const string newText = "using System.Threading.Tasks; class A { Task M() { return Task.CompletedTask; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    [Fact]
    public void Async_flip_with_identical_body_tokens_is_a_body_change()
    {
        // Same token body, different compiled body: async wraps the throw in
        // the returned Task, sync throws at the call site.
        const string oldText = "using System.Threading.Tasks; class A { async Task M() { throw new System.Exception(); } }";
        const string newText = "using System.Threading.Tasks; class A { Task M() { throw new System.Exception(); } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    // ── accessibility: widening is hot/noop, narrowing is cold ───────

    [Fact]
    public void Accessibility_widening_without_body_change_is_a_hot_noop()
    {
        const string oldText = "class A { void M() { } }";
        const string newText = "class A { public void M() { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Empty(result.ChangedMethods);
        Assert.Empty(result.PatchedTypes);
    }

    [Fact]
    public void Accessibility_widening_with_body_change_is_hot()
    {
        const string oldText = "class A { protected int M() { return 1; } }";
        const string newText = "class A { public int M() { return 2; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    [Fact]
    public void Accessibility_narrowing_is_conditionally_hot_with_caller_check_entry()
    {
        const string oldText = "class A { public void M() { } }";
        const string newText = "class A { private void M() { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var check = Assert.Single(result.RequiresCallerCheck);
        Assert.Equal("accessibility-narrowed", check.Kind);
        Assert.Equal("A", check.DeclaringType);
        Assert.Equal("M", check.Name);
        Assert.Equal(new[] { "M" }, check.ScanMemberNames);
        Assert.Empty(result.ChangedMethods);
    }

    [Fact]
    public void Protected_to_internal_counts_as_narrowing()
    {
        const string oldText = "class A { protected void M() { } }";
        const string newText = "class A { internal void M() { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var check = Assert.Single(result.RequiresCallerCheck);
        Assert.Equal("accessibility-narrowed", check.Kind);
    }

    // ── interfaces stay cold (IMT dispatch unverified) ───────────────

    [Fact]
    public void Interface_default_implementation_change_is_cold()
    {
        const string oldText = "interface I { int M() { return 1; } }";
        const string newText = "interface I { int M() { return 2; } }";

        var result = Analyze(oldText, newText, new CSharpParseOptions(languageVersion: LanguageVersion.CSharp9));

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("interface changed"));
    }

    [Fact]
    public void Unchanged_interface_alongside_hot_edit_stays_hot()
    {
        const string oldText = "interface I { void M(); } class A : I { public void M() { } }";
        const string newText = "interface I { void M(); } class A : I { public void M() { int x = 1; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    [Fact]
    public void Constructor_body_change_is_hot()
    {
        const string oldText = "class A { public A(int x) { } }";
        const string newText = "class A { public A(int x) { System.Console.WriteLine(x); } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        var method = Assert.Single(result.ChangedMethods);
        Assert.Equal(".ctor", method.Name);
        Assert.True(method.IsCtor);
        Assert.Equal(new[] { "Int32" }, method.ParamTypeNames);
    }

    [Fact]
    public void Property_getter_body_change_is_hot()
    {
        const string oldText = "class A { int _v; public int Value { get { return _v; } set { _v = value; } } }";
        const string newText = "class A { int _v; public int Value { get { return _v + 1; } set { _v = value; } } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        var method = Assert.Single(result.ChangedMethods);
        Assert.Equal("get_Value", method.Name);
        Assert.Empty(method.ParamTypeNames);
    }

    [Fact]
    public void Indexer_setter_body_change_is_hot_with_indexer_params()
    {
        const string oldText = "class A { int[] _v = new int[8]; public int this[int i] { get { return _v[i]; } set { _v[i] = value; } } }";
        const string newText = "class A { int[] _v = new int[8]; public int this[int i] { get { return _v[i]; } set { _v[i] = value + 1; } } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        var method = Assert.Single(result.ChangedMethods);
        Assert.Equal("set_Item", method.Name);
        Assert.Equal(new[] { "Int32", "Int32" }, method.ParamTypeNames);
    }

    [Fact]
    public void Operator_body_change_is_hot()
    {
        const string oldText = "class A { public static A operator +(A a, A b) { return a; } }";
        const string newText = "class A { public static A operator +(A a, A b) { return b; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        var method = Assert.Single(result.ChangedMethods);
        Assert.Equal("op_Addition", method.Name);
        Assert.True(method.IsStatic);
    }

    [Fact]
    public void Event_accessor_body_change_is_hot()
    {
        const string oldText = "using System; class A { Action _h; public event Action Tick { add { _h += value; } remove { _h -= value; } } }";
        const string newText = "using System; class A { Action _h; public event Action Tick { add { _h += value; Console.WriteLine(1); } remove { _h -= value; } } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        var method = Assert.Single(result.ChangedMethods);
        Assert.Equal("add_Tick", method.Name);
        Assert.Equal(new[] { "Action" }, method.ParamTypeNames);
    }

    [Fact]
    public void Orphan_added_private_method_is_hot_and_not_detoured()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace(
                "private void Helper(string name) { Debug.Log(name); }",
                "private void Helper(string name) { Debug.Log(name); }\n        private int Compute(System.Collections.Generic.List<int> xs) { return xs.Count; }"));

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var method = Assert.Single(result.ChangedMethods);
        Assert.True(method.Added);
        Assert.Equal("Compute", method.Name);
        Assert.Equal(new[] { "Game.Player" }, result.PatchedTypes);
    }

    [Fact]
    public void Added_private_method_with_changed_caller_is_hot_and_not_detoured()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld
                .Replace("_health += 1;", "_health += Compute(new System.Collections.Generic.List<int> { 1, 2 });")
                .Replace(
                    "private void Helper(string name) { Debug.Log(name); }",
                    "private void Helper(string name) { Debug.Log(name); }\n        private int Compute(System.Collections.Generic.List<int> xs) { return xs.Count; }"));

        Assert.True(result.Hot);
        Assert.Equal(2, result.ChangedMethods.Count);
        Assert.Contains(result.ChangedMethods, m => m.Name == "Update" && !m.Added);
        Assert.Contains(result.ChangedMethods, m =>
            m.Name == "Compute" && m.Added && m.ParamTypeNames.SequenceEqual(new[] { "List`1" }));
    }

    [Fact]
    public void Added_type_is_hot_and_reported()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld + "\nnamespace Game { public class Spawner { public int Count() { return 3; } } }");

        Assert.True(result.Hot);
        Assert.Equal(new[] { "Game.Spawner" }, result.NewTypes);
        Assert.Empty(result.ChangedMethods);
    }

    [Fact]
    public void Instance_field_initializer_change_redirects_constructors()
    {
        var result = Analyze(PlayerOld, PlayerOld.Replace("_health = 100", "_health = 50"));

        Assert.True(result.Hot);
        var method = Assert.Single(result.ChangedMethods);
        Assert.Equal(".ctor", method.Name);
        Assert.True(method.IsCtor);
        Assert.Empty(method.ParamTypeNames);
    }

    [Fact]
    public void Instance_initializer_change_with_explicit_ctors_redirects_each()
    {
        const string oldText = "class A { int _x = 1; public A() { } public A(string s) { } }";
        const string newText = "class A { int _x = 2; public A() { } public A(string s) { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal(2, result.ChangedMethods.Count);
        Assert.All(result.ChangedMethods, m => Assert.Equal(".ctor", m.Name));
        Assert.Contains(result.ChangedMethods, m => m.ParamTypeNames.Length == 0);
        Assert.Contains(result.ChangedMethods, m => m.ParamTypeNames.SequenceEqual(new[] { "String" }));
    }

    [Fact]
    public void Edit_inside_inactive_preprocessor_block_is_a_hot_noop()
    {
        const string oldText = "class A { void M() {\n#if LOCUS_MISSING_SYMBOL\n int x = 1;\n#endif\n } }";
        const string newText = "class A { void M() {\n#if LOCUS_MISSING_SYMBOL\n int x = 2;\n#endif\n } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Empty(result.ChangedMethods);
    }

    [Fact]
    public void Directive_change_that_alters_active_code_is_a_body_change()
    {
        const string oldText = "class A { void M() {\n#if UNITY_EDITOR\n int x = 1;\n#endif\n } }";
        const string newText = "class A { void M() {\n#if !UNITY_EDITOR\n int x = 1;\n#endif\n } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    [Fact]
    public void Ref_and_array_and_nullable_params_use_reflection_names()
    {
        const string oldText = "class A { void M(ref int a, string[] b, int? c, out double d) { d = 0; } }";
        const string newText = "class A { void M(ref int a, string[] b, int? c, out double d) { d = 1; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal(
            new[] { "Int32&", "String[]", "Nullable`1", "Double&" },
            Assert.Single(result.ChangedMethods).ParamTypeNames);
    }

    // ── cold: layout / signature / type shape ────────────────────────

    [Fact]
    public void Field_added_is_cold()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace("private int _health = 100;", "private int _health = 100;\n        private int _mana;"));

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("field layout changed"));
    }

    [Fact]
    public void Field_reorder_is_cold()
    {
        const string oldText = "class A { int _a; int _b; }";
        const string newText = "class A { int _b; int _a; }";

        Assert.False(Analyze(oldText, newText).Hot);
    }

    [Fact]
    public void Auto_property_added_is_cold()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace("public void Update()", "public int Mana { get; set; }\n        public void Update()"));

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("field layout changed"));
    }

    [Fact]
    public void Auto_property_to_bodied_conversion_is_cold()
    {
        const string oldText = "class A { public int X { get; set; } }";
        const string newText = "class A { int _x; public int X { get { return _x; } set { _x = value; } } }";

        Assert.False(Analyze(oldText, newText).Hot);
    }

    [Fact]
    public void Signature_change_decomposes_into_remove_plus_add_with_caller_check()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld
                .Replace("private void Helper(string name) { Debug.Log(name); }",
                         "private void Helper(string name, int times) { Debug.Log(name + times); }"));

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var removed = Assert.Single(result.RemovedMembers);
        Assert.Equal("Helper", removed.Name);
        Assert.Equal(new[] { "String" }, removed.ParamTypeNames);
        var added = Assert.Single(result.ChangedMethods);
        Assert.True(added.Added);
        Assert.Equal(new[] { "String", "Int32" }, added.ParamTypeNames);
        var check = Assert.Single(result.RequiresCallerCheck);
        Assert.Equal("member-removed", check.Kind);
        Assert.Equal(new[] { "Helper" }, check.ScanMemberNames);
    }

    [Fact]
    public void Return_type_change_decomposes_into_remove_plus_add()
    {
        const string oldText = "class A { int M() { return 1; } }";
        const string newText = "class A { long M() { return 1; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Single(result.RemovedMembers);
        var added = Assert.Single(result.ChangedMethods);
        Assert.True(added.Added);
        Assert.Single(result.RequiresCallerCheck);
    }

    [Fact]
    public void Static_flip_decomposes_into_remove_plus_add()
    {
        const string oldText = "class A { int M() { return 1; } }";
        const string newText = "class A { static int M() { return 1; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var removed = Assert.Single(result.RemovedMembers);
        Assert.False(removed.IsStatic);
        var added = Assert.Single(result.ChangedMethods);
        Assert.True(added.IsStatic);
    }

    [Fact]
    public void Virtual_signature_change_is_cold()
    {
        const string oldText = "class A { public virtual int M() { return 1; } }";
        const string newText = "class A { public virtual long M() { return 1; } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("virtual member removed"));
    }

    [Fact]
    public void Magic_method_signature_change_is_cold()
    {
        const string oldText = "class A { void Update() { } }";
        const string newText = "class A { int Update() { return 1; } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("Unity message method signature changed"));
    }

    [Fact]
    public void Method_attribute_change_is_cold()
    {
        const string oldText = "class A { void M() { } }";
        const string newText = "class A { [System.Obsolete] void M() { } }";

        Assert.False(Analyze(oldText, newText).Hot);
    }

    // ── M6: using changes re-detour the whole file ───────────────────

    [Fact]
    public void Using_change_rehooks_every_detourable_member()
    {
        const string oldText = @"
using System;
class C
{
    int _v;
    public void M() { _v = 1; }
    private int Helper() { return _v; }
    public int Value { get { return _v; } set { _v = value; } }
}";
        string newText = oldText.Replace("using System;", "using System;\nusing System.Collections.Generic;");

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Equal(new[] { "C" }, result.PatchedTypes);
        var names = result.ChangedMethods.Select(m => m.Name).OrderBy(n => n, StringComparer.Ordinal).ToArray();
        Assert.Equal(new[] { "Helper", "M", "get_Value", "set_Value" }, names);
        Assert.All(result.ChangedMethods, m => Assert.False(m.Added));
    }

    [Fact]
    public void Using_change_with_body_edit_does_not_duplicate_methods()
    {
        const string oldText = "using System;\nclass C { void M() { } void N() { } }";
        const string newText = "using System.Text;\nclass C { void M() { int x = 1; } void N() { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal(2, result.ChangedMethods.Count);
        Assert.Single(result.ChangedMethods, m => m.Name == "M");
        Assert.Single(result.ChangedMethods, m => m.Name == "N");
    }

    [Fact]
    public void Using_change_with_non_literal_const_is_cold()
    {
        const string oldText = "class C { const int Max = int.MaxValue; void M() { } }";
        const string newText = "using System;\nclass C { const int Max = int.MaxValue; void M() { } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("non-literal const"));
        Assert.Empty(result.ChangedMethods);
    }

    [Fact]
    public void Using_change_with_literal_const_rehooks()
    {
        const string oldText = "class C { const int Max = 10; void M() { } }";
        const string newText = "using System;\nclass C { const int Max = 10; void M() { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal("M", Assert.Single(result.ChangedMethods).Name);
    }

    [Fact]
    public void Using_change_with_non_literal_static_initializer_is_cold()
    {
        const string oldText = "class C { static int S = Compute(); static int Compute() { return 1; } }";
        const string newText = "using System;\nclass C { static int S = Compute(); static int Compute() { return 1; } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("static initializer already ran"));
    }

    [Fact]
    public void Using_change_with_generic_member_is_cold()
    {
        const string oldText = "class C { void M<T>() { } }";
        const string newText = "using System;\nclass C { void M<T>() { } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("generic members cannot be re-detoured"));
    }

    [Fact]
    public void Using_change_covers_implicit_ctor_when_initializers_exist()
    {
        const string oldText = "class C { int _x = 1; void M() { } }";
        const string newText = "using System;\nclass C { int _x = 1; void M() { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Contains(result.ChangedMethods, m => m.Name == ".ctor" && m.IsCtor);
        Assert.Contains(result.ChangedMethods, m => m.Name == "M");
    }

    [Fact]
    public void New_file_with_usings_is_hot()
    {
        var result = Analyze(
            "",
            "using UnityEngine;\nnamespace Game { public class Fresh { public int N() { return 1; } } }");

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Equal(new[] { "Game.Fresh" }, result.NewTypes);
        Assert.Empty(result.ChangedMethods);
    }

    [Fact]
    public void Field_attribute_change_is_cold()
    {
        const string oldText = "class A { int _x; }";
        const string newText = "class A { [System.NonSerialized] int _x; }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("field layout changed"));
    }

    [Fact]
    public void Delegate_change_is_cold()
    {
        const string oldText = "delegate void D(int x); class A { void M() { } }";
        const string newText = "delegate void D(string x); class A { void M() { } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("delegate declarations changed"));
    }

    [Fact]
    public void Added_public_method_is_hot_via_shim()
    {
        const string oldText = "class A { void M() { } }";
        const string newText = "class A { void M() { } public void N() { } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var method = Assert.Single(result.ChangedMethods);
        Assert.True(method.Added);
        Assert.Equal("N", method.Name);
    }

    [Fact]
    public void Added_method_on_generic_type_is_hot_via_shim()
    {
        const string oldText = "class A<T> { void M() { } }";
        const string newText = "class A<T> { void M() { } public int Count() { return 1; } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var method = Assert.Single(result.ChangedMethods);
        Assert.True(method.Added);
        Assert.Equal("Count", method.Name);
    }

    [Fact]
    public void Added_virtual_method_is_cold()
    {
        const string oldText = "class A { void M() { } }";
        const string newText = "class A { void M() { } public virtual void N() { } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("virtual member added"));
    }

    [Fact]
    public void Added_method_using_base_access_is_cold()
    {
        const string oldText = "class A { public override string ToString() { return \"a\"; } }";
        const string newText = "class A { public override string ToString() { return \"a\"; } public string Both() { return base.ToString() + \"!\"; } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("base access"));
    }

    [Fact]
    public void Property_added_is_cold()
    {
        const string oldText = "class A { void M() { } }";
        const string newText = "class A { void M() { } int Value { get { return 1; } } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("property added"));
    }

    [Fact]
    public void Burst_compiled_method_change_is_cold()
    {
        const string oldText = "class BurstCompileAttribute : System.Attribute { } class A { [BurstCompile] void M() { } }";
        const string newText = "class BurstCompileAttribute : System.Attribute { } class A { [BurstCompile] void M() { System.Console.WriteLine(1); } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("Burst-compiled method body changed"));
    }

    [Fact]
    public void Member_removed_is_conditionally_hot_with_caller_check()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace("private void Helper(string name) { Debug.Log(name); }", ""));

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var removed = Assert.Single(result.RemovedMembers);
        Assert.Equal("Helper", removed.Name);
        Assert.False(removed.IsUnityMagic);
        Assert.Null(removed.StubSource);
        Assert.Single(result.RequiresCallerCheck);
        Assert.Empty(result.ChangedMethods);
    }

    [Fact]
    public void Removed_unity_message_method_produces_stub()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace("public void Update() { _health += 1; }", ""));

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        var removed = Assert.Single(result.RemovedMembers);
        Assert.Equal("Update", removed.Name);
        Assert.True(removed.IsUnityMagic);
        Assert.NotNull(removed.StubSource);
        Assert.Contains("Update", removed.StubSource);
        Assert.Equal(new[] { "Game.Player" }, result.PatchedTypes);
    }

    [Fact]
    public void Removed_virtual_member_is_cold()
    {
        const string oldText = "class A { public virtual void M() { } }";
        const string newText = "class A { }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("virtual member removed"));
    }

    [Fact]
    public void Removed_constructor_is_cold()
    {
        const string oldText = "class A { public A(int x) { } }";
        const string newText = "class A { }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("constructor removed"));
    }

    [Fact]
    public void Removed_property_records_accessor_removals()
    {
        const string oldText = "class A { int _v; public int Value { get { return _v; } set { _v = value; } } }";
        const string newText = "class A { int _v; }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot, string.Join("; ", result.Reasons));
        Assert.Equal(2, result.RemovedMembers.Count);
        Assert.Contains(result.RemovedMembers, m => m.Name == "get_Value");
        Assert.Contains(result.RemovedMembers, m => m.Name == "set_Value");
        var check = Assert.Single(result.RequiresCallerCheck);
        Assert.Equal(new[] { "get_Value", "set_Value" }, check.ScanMemberNames);
    }

    [Fact]
    public void Type_removed_is_cold()
    {
        const string oldText = "class A { } class B { }";
        const string newText = "class A { }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("type removed: B"));
    }

    [Fact]
    public void Base_list_change_is_cold()
    {
        const string oldText = "class A { }";
        const string newText = "class A : System.IDisposable { public void Dispose() { } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("type declaration changed"));
    }

    [Fact]
    public void Const_value_change_is_cold()
    {
        const string oldText = "class A { const int Max = 10; int M() { return Max; } }";
        const string newText = "class A { const int Max = 20; int M() { return Max; } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("const or static initializer changed"));
    }

    [Fact]
    public void Static_field_initializer_change_is_cold()
    {
        const string oldText = "class A { static int S = 1; }";
        const string newText = "class A { static int S = 2; }";

        Assert.False(Analyze(oldText, newText).Hot);
    }

    [Fact]
    public void Static_constructor_body_change_is_cold()
    {
        const string oldText = "class A { static A() { } }";
        const string newText = "class A { static A() { System.Console.WriteLine(1); } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("static constructor changed"));
    }

    [Fact]
    public void Constructor_added_is_cold()
    {
        const string oldText = "class A { }";
        const string newText = "class A { public A(int x) { } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("constructor added"));
    }

    [Fact]
    public void Generic_type_method_body_change_is_cold()
    {
        const string oldText = "class A<T> { void M() { } }";
        const string newText = "class A<T> { void M() { System.Console.WriteLine(1); } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("generic"));
    }

    [Fact]
    public void Generic_method_body_change_is_cold()
    {
        const string oldText = "class A { T M<T>(T x) { return x; } }";
        const string newText = "class A { T M<T>(T x) { System.Console.WriteLine(1); return x; } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("generic method body changed"));
    }

    [Fact]
    public void Partial_type_in_file_is_cold()
    {
        const string oldText = "partial class A { void M() { } }";
        const string newText = "partial class A { void M() { System.Console.WriteLine(1); } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("partial type in file"));
    }

    [Fact]
    public void Record_type_is_cold()
    {
        const string oldText = "record A(int X);";
        const string newText = "record A(int X) { public int Y() { return X; } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("record"));
    }

    [Fact]
    public void Enum_member_change_is_cold()
    {
        const string oldText = "enum E { A = 1, B = 2 }";
        const string newText = "enum E { A = 1, B = 3 }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("enum changed"));
    }

    [Fact]
    public void New_unity_message_method_is_cold()
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace(
                "private void Helper(string name) { Debug.Log(name); }",
                "private void Helper(string name) { Debug.Log(name); }\n        private void FixedUpdate() { }"));

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("new Unity message method"));
    }

    [Theory]
    [InlineData("OnRectTransformDimensionsChange")]
    [InlineData("OnBeforeTransformParentChanged")]
    [InlineData("OnCanvasGroupChanged")]
    [InlineData("OnCanvasHierarchyChanged")]
    [InlineData("OnDidApplyAnimationProperties")]
    [InlineData("OnParticleUpdateJobScheduled")]
    [InlineData("OnLevelWasLoaded")]
    public void Newly_listed_unity_messages_stay_cold(string magicName)
    {
        var result = Analyze(
            PlayerOld,
            PlayerOld.Replace(
                "private void Helper(string name) { Debug.Log(name); }",
                "private void Helper(string name) { Debug.Log(name); }\n        private void " + magicName + "() { }"));

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("new Unity message method"));
    }

    [Fact]
    public void Explicit_interface_implementation_change_is_cold()
    {
        const string oldText = "class A : System.IDisposable { void System.IDisposable.Dispose() { } }";
        const string newText = "class A : System.IDisposable { void System.IDisposable.Dispose() { System.Console.WriteLine(1); } }";

        var result = Analyze(oldText, newText);

        Assert.False(result.Hot);
        Assert.Contains(result.Reasons, r => r.Contains("explicit interface implementation changed"));
    }

    [Fact]
    public void Nested_type_method_uses_plus_separated_metadata_name()
    {
        const string oldText = "namespace N { class Outer { class Inner { void M() { } } } }";
        const string newText = "namespace N { class Outer { class Inner { void M() { System.Console.WriteLine(1); } } } }";

        var result = Analyze(oldText, newText);

        Assert.True(result.Hot);
        Assert.Equal("N.Outer+Inner", Assert.Single(result.ChangedMethods).DeclaringType);
    }

    // ── deterministic syntax errors ──────────────────────────────────

    [Fact]
    public void New_text_parse_error_reports_syntax_error_not_cold()
    {
        var result = Analyze(PlayerOld, PlayerOld.Replace("_health += 1;", "_health += ;"));

        Assert.False(result.Hot);
        Assert.NotNull(result.SyntaxError);
        Assert.Empty(result.Reasons);
        Assert.StartsWith("compilation failed:", result.SyntaxError);
    }
}
