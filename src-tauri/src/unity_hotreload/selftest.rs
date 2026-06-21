//! Built-in hot-reload self-test: drives the WHOLE hot-reload surface
//! (H0–H7) against the connected Unity Editor through the same internal
//! interfaces the agent tools use — coordinator baselines, the sidecar
//! compile, the pipe — and reports a step-by-step diagnostic log.
//!
//! Coverage maps to the public hot-reload feature matrix, positive and negative:
//!   • positives: method/property(get+set)/indexer/event/operator/
//!     conversion/ctor body edits, expression-bodied members, lambda +
//!     closure (including NEW captures), local functions, anonymous types,
//!     pattern matching, nested types, iterator (coroutine) bodies, async
//!     body edits and async↔sync, added methods (shim→shim chains) +
//!     fields (instance and static, across separate batches),
//!     instance-initializer edits, field deletion, field RETYPE (the
//!     remove+add decomposition — a live instance reads the new field's
//!     default), added-const inlining, signature changes
//!     (params / ref→out / static flip / rename) with call-site
//!     verification, accessibility narrowing, using add/remove with
//!     whole-file rehook, enum append, new files, new types in existing
//!     files (top-level and nested), struct method bodies, interface-impl
//!     bodies, deletions (members, properties, Unity messages, whole
//!     files); plus edit-mode reloading of EDITOR-assembly code, in-flight
//!     delegates following detours, Unity message body edits, hot-added Unity
//!     message drivers (PlayerLoop, lifecycle catch-up, plus component-proxy
//!     forwarding for physics/trigger and non-physics messages), a
//!     runtime diagnostic for hot-added MonoBehaviour AddComponent(Type), store-held
//!     static persistence across patches, generic-typed and nested-type
//!     field additions, #if-block edits, iterator→plain conversions,
//!     extension-method additions (with the call site surviving LATER
//!     batches where the accepted patch image is also in scope), a
//!     five-file batch, generic method bodies (generic methods AND
//!     methods of generic types) via the B1 remove+add shim path with
//!     their untouched callers re-detoured, added members reaching the
//!     original type's PRIVATE field/method/static through the C0-measured
//!     access caps (C2′a), the same access from the compiler-generated
//!     SUB-METHODS of added members — async state machines, capturing
//!     lambdas, iterators (C2′b), an added member constructing through a
//!     PRIVATE constructor (the newobj creation-node check), a kept
//!     body touching a cross-file internal type (binding-model alignment
//!     guard), the B2 accessor additions: a new PROPERTY
//!     (write/compound-assign/read through get_/set_ shims), a new INDEXER
//!     overload, a new accessor EVENT (subscribe/unsubscribe), and a new
//!     AUTO-PROPERTY (store-backed: default for existing instances, value
//!     persistent across calls, initializer on new instances, ++ across a
//!     later batch), plus nameof(...) over an added auto-property; and the B3
//!     cross-asmdef shapes against a SEPARATE assembly (Lib/ + .asmdef):
//!     a lib method body edit observed through an Assembly-CSharp caller,
//!     a lib signature change whose CROSS-ASSEMBLY caller must join the
//!     batch (M3 names the Assembly-CSharp file first, then the covered
//!     batch goes hot), and a lib added method called from Assembly-CSharp
//!     through the cross-assembly shim, plus a lib added property through
//!     cross-assembly accessor shims; and the B6 partial shapes: a type
//!     split across TWO part files (both with private instance fields),
//!     where editing one part's body pulls the never-edited sibling in as
//!     a baseline, then editing both parts in one batch keeps the merged
//!     patch copy complete. The Enter Play Mode Options matrix
//!     (B5) is probed right before the play transition: the run logs
//!     whether the play-enter domain reload is enabled, and the Phase-2
//!     editor patch is deliberately carried across the transition — E02
//!     asserts the mode-matched outcome (the detour survives with Reload
//!     Domain disabled, it dies with the classic reload).
//!   • negatives (must come back COLD with the precise reason): generic
//!     bodies whose compiled caller is OUTSIDE the batch (named caller
//!     file), generic-type constructor bodies, constructor/
//!     finalizer surface and finalizer bodies, virtual members (including
//!     virtual PROPERTY additions — plain property/indexer/event additions
//!     are hot since B2), field-like event additions, struct
//!     field layout, enum value edits, attribute edits, const edits,
//!     unsupported Unity message names/signatures, interface changes,
//!     base-list changes, partial-type FIELD layout changes (body edits
//!     are hot since B6), delegate signature changes,
//!     conversions returning the declaring type, added members whose
//!     SIGNATURE names a non-public type (C2′a relaxes body access only),
//!     uncovered call sites (named caller file); plus the shim-only
//!     "parked new surface" verdict for caller-less additions and the
//!     untracked-input verdict for .asmdef edits (assembly restructuring
//!     always needs unity_recompile), along with B2's pointed cold guards:
//!     full-property ++, ??= set-skip, auto-property ref/out, and compound
//!     indexers with non-repeatable index expressions; plus the N30+ extra
//!     cold surface: type-kind flips (class↔struct), static-constructor
//!     bodies, explicit-interface-implementation bodies, the enum-append
//!     guards (non-literal value, value conflict) and enum removal, const
//!     and constructor and finalizer REMOVAL, field-modifier changes,
//!     operator additions, record types (rejected on presence — created
//!     fresh so the C# 9 syntax never compiles), the M6 using-rehook gates
//!     (non-literal const / non-literal static initializer / generic member /
//!     explicit-interface / finalizer), and the remaining B6 partial
//!     boundaries (part add/remove, new-part declaration, using-in-partial,
//!     part-count change, initializer change, partial-method-twice). The
//!     Burst, unsupported-operator and default-interface-method gate variants
//!     stay in the HotDiff unit tests (package / unreachable / runtime-DIM).
//!
//! Flow: with the editor connected and NOT playing, it materializes a test
//! corpus under Assets/LocusHotReloadSelfTest, imports + recompiles it as
//! the baseline (a real domain reload), enters play mode, spawns the test
//! component, then hot-reloads one feature after another, verifying
//! observable behavior through `unity_execute_code` snippets. Added-member
//! behavior is always asserted through a PRE-EXISTING member (`Probe`)
//! re-pointed at the new surface: snippets compile against the original
//! assembly metadata, which never contains hot-added members. The hot-added
//! MonoBehaviour AddComponent diagnostic uses reflection for the same reason.
//! Every step
//! is atomic: a failed apply reverts its file(s) on disk and in the ledger
//! so one rejected patch cannot poison the following batches.
//!
//! It finishes by leaving play mode, waiting for the automatic convergence
//! (H6), and deleting the corpus.
//!
//! Triggered from Settings > Code Analysis; progress streams to the UI via
//! the `unity-hotreload-selftest` event.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use super::coordinator;

static RUNNING: AtomicBool = AtomicBool::new(false);

const EXIT_PLAY_MODE_TIMEOUT: Duration = Duration::from_secs(90);
const UNITY_SEMANTIC_READY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const UNITY_SEMANTIC_READY_POLL: Duration = Duration::from_millis(500);
const TEST_DIR: &str = "Assets/LocusHotReloadSelfTest";
const SUBJECT_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestSubject.cs";
const MESSAGE_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestMessages.cs";
const HELPER_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestHelper.cs";
const MODE_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestMode.cs";
const FRESH_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestFresh.cs";
const HOT_ADDED_BEHAVIOUR_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestHotAddedBehaviour.cs";
const STRUCT_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestStruct.cs";
const CTOR_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestCtor.cs";
const IFACE_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestIface.cs";
const NEG_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestNegative.cs";
const EDITOR_DIR: &str = "Assets/LocusHotReloadSelfTest/Editor";
const EDITOR_FILE: &str = "Assets/LocusHotReloadSelfTest/Editor/LocusSelfTestEditorTool.cs";
// B3: a corpus subdirectory with its own .asmdef — Unity compiles it into a
// SEPARATE assembly (LocusSelfTestLib), so the cross-asmdef cases run against
// a real two-assembly graph instead of everything living in Assembly-CSharp.
const LIB_DIR: &str = "Assets/LocusHotReloadSelfTest/Lib";
const LIB_ASMDEF_FILE: &str = "Assets/LocusHotReloadSelfTest/Lib/LocusSelfTestLib.asmdef";
const LIB_FILE: &str = "Assets/LocusHotReloadSelfTest/Lib/LocusSelfTestLibType.cs";
// B6: a hand-written partial type split across TWO files — both parts carry
// instance fields, so the cross-part layout merge and the sibling-discovery
// path are exercised against the real compiler's part ordering.
const PARTIAL_A_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestPartialA.cs";
const PARTIAL_B_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestPartialB.cs";
// Adversarial C#-syntax cases (A1–A4): pin known resolver / inlining gaps.
const ADVERSARIAL_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestAdversarial.cs";
const INLINE_CALLEE_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestInlineCallee.cs";
const INLINE_CALLER_DIRECT_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestInlineCallerDirect.cs";
const INLINE_CALLER_NESTED_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestInlineCallerNested.cs";
const INLINE_CALLER_OVERLOAD_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestInlineCallerOverload.cs";
const INLINE_CALLER_LAMBDA_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestInlineCallerLambda.cs";
const INLINE_CALLER_BRANCH_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestInlineCallerBranch.cs";
const INLINE_CALLER_ARRAY_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestInlineCallerArray.cs";
// R12 — a depth-2 static inline chain across THREE files (Leaf→Mid→Top, each
// hop AggressiveInlining bar the top): editing the leaf must refresh the mid
// (round 1) and the top (round 2), exactly the INLINE_REFRESH_MAX_DEPTH limit.
const INLINE_CHAIN_LEAF_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestChainLeaf.cs";
const INLINE_CHAIN_MID_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestChainMid.cs";
const INLINE_CHAIN_TOP_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestChainTop.cs";
// R13 — cross-ASSEMBLY static inline refresh: a static lib method (Lib/ →
// LocusSelfTestLib) inlined into an Assembly-CSharp caller. The refresh
// recompiles callee+caller into ONE patch assembly, so the static patch-copy
// redirect should erase the boundary.
const LIB_INLINE_FILE: &str = "Assets/LocusHotReloadSelfTest/Lib/LocusSelfTestLibInline.cs";
const LIB_INLINE_CALLER_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestLibInlineCaller.cs";
// R14 — same-ASSEMBLY INSTANCE inline refresh: an instance callee inlined into
// a static caller. Exercises the instance self-shim redirect (Option A) with
// the assembly boundary ruled out — the same-assembly counterpart of R05.
const INST_INLINEE_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestInstInlinee.cs";
const INST_INLINE_CALLER_FILE: &str =
    "Assets/LocusHotReloadSelfTest/LocusSelfTestInstInlineCaller.cs";
// Extra cold-classification surface (negative phase only). COLD_FILE holds
// several independent types, each mutated in isolation to pin one rejection
// reason: type-kind flip, static ctor, explicit-interface body, enum guards,
// const/field/ctor/finalizer removal, operator addition.
const COLD_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestColdSurface.cs";
// A record type is rejected on PRESENCE (CollectTypes), so it cannot share a
// file with anything else and must never reach a real compile — it is created
// fresh inside the negative test, classified cold, and deleted (never imported
// into the baseline), so a C#-9-shy editor can never abort the whole suite.
const RECORD_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestRecord.cs";
// Using-rehook gates (M6): each file pairs ONE un-re-detourable member with a
// using directive; toggling the using fails the whole-file re-detour closed
// with that member's precise reason. One member per file — the gate reports
// the FIRST it finds, so they cannot be combined.
const USE_CONST_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestUseConst.cs";
const USE_STATIC_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestUseStatic.cs";
const USE_GENERIC_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestUseGeneric.cs";
const USE_EXPLICIT_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestUseExplicit.cs";
const USE_FINALIZER_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestUseFinalizer.cs";
// Partial-type cold boundaries (B6 v1). PARTIAL_COLD_FILE is a single-part
// partial type used for part-removed / new-part / using-in-partial /
// initializer / partial-method-twice; PARTIAL_COUNT_FILE carries TWO parts in
// one file so the part-count change can fire.
const PARTIAL_COLD_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestPartialCold.cs";
const PARTIAL_COUNT_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestPartialCount.cs";

const ALL_FILES: &[&str] = &[
    SUBJECT_FILE,
    MESSAGE_FILE,
    HELPER_FILE,
    MODE_FILE,
    FRESH_FILE,
    HOT_ADDED_BEHAVIOUR_FILE,
    STRUCT_FILE,
    CTOR_FILE,
    IFACE_FILE,
    NEG_FILE,
    EDITOR_FILE,
    PARTIAL_A_FILE,
    PARTIAL_B_FILE,
    ADVERSARIAL_FILE,
    INLINE_CALLEE_FILE,
    INLINE_CALLER_DIRECT_FILE,
    INLINE_CALLER_NESTED_FILE,
    INLINE_CALLER_OVERLOAD_FILE,
    INLINE_CALLER_LAMBDA_FILE,
    INLINE_CALLER_BRANCH_FILE,
    INLINE_CALLER_ARRAY_FILE,
    // Multi-file inline caller-refresh characterization probes (R12–R14).
    INLINE_CHAIN_LEAF_FILE,
    INLINE_CHAIN_MID_FILE,
    INLINE_CHAIN_TOP_FILE,
    LIB_INLINE_CALLER_FILE,
    INST_INLINEE_FILE,
    INST_INLINE_CALLER_FILE,
    // Extra cold-classification corpus (negative phase). RECORD_FILE is
    // deliberately ABSENT: it is created fresh inside its test and never
    // baseline-imported (see the FRESH_FILE-style filter in initialize_corpus).
    COLD_FILE,
    USE_CONST_FILE,
    USE_STATIC_FILE,
    USE_GENERIC_FILE,
    USE_EXPLICIT_FILE,
    USE_FINALIZER_FILE,
    PARTIAL_COLD_FILE,
    PARTIAL_COUNT_FILE,
    RECORD_FILE,
    // The .asmdef imports BEFORE the lib source so the assembly exists by
    // the time its first script imports (both flush in one batch anyway —
    // the compilation pipeline recomputes asmdef ownership per compile).
    LIB_ASMDEF_FILE,
    LIB_FILE,
    LIB_INLINE_FILE,
];

// Adversarial corpus: four legal-C# constructs that stress overload identity
// and inlining. All members are static and self-contained so editing them can
// only affect this file's own steps (the type never participates in other
// cases). See `run_adversarial_tests`.
const ADVERSARIAL_BASELINE: &str = r#"namespace LocusSelfTestAdvA { public struct Tag { } }
namespace LocusSelfTestAdvB { public struct Tag { } }

public static class LocusSelfTestAdversarial
{
    // A1: overloads distinguishable ONLY by parameter namespace — both reflect
    // to the simple type name "Tag"; the enriched signature carries the rest.
    public static int ProbeNs(LocusSelfTestAdvA.Tag t) { return 1001; }
    public static int ProbeNs(LocusSelfTestAdvB.Tag t) { return 2002; }

    // A2: overloads distinguishable ONLY by generic argument — both reflect to
    // "List`1".
    public static int ProbeGen(System.Collections.Generic.List<int> a) { return 3003; }
    public static int ProbeGen(System.Collections.Generic.List<string> a) { return 4004; }

    // A3: inlined at call sites even in Debug; the detour is bypassed there.
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int Inlined() { return 5005; }
    public static int CallInlined() { return Inlined(); }

    // A4: callers bake the default value at compile time, like a const.
    public static int Defaulted(int x, int y = 1000) { return x + y; }
    public static int CallDefaulted() { return Defaulted(7000); }
}
"#;

const INLINE_CALLEE_BASELINE: &str = r#"public static class LocusSelfTestInlineCallee
{
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int Direct() { return 101; }

    public static class Inner
    {
        [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
        public static int Nested() { return 201; }
    }

    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int Pick(int x) { return x + 301; }

    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int Pick(string x) { return x.Length + 401; }

    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int LambdaSeed() { return 501; }

    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int BranchSeed(int x) { return x > 0 ? 601 : 0; }

    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int ArraySeed() { return 701; }
}
"#;

const INLINE_CALLER_DIRECT_BASELINE: &str = r#"public static class LocusSelfTestInlineCallerDirect
{
    public static int Run() { return LocusSelfTestInlineCallee.Direct() + 1; }
}
"#;

const INLINE_CALLER_NESTED_BASELINE: &str = r#"public static class LocusSelfTestInlineCallerNested
{
    public static int Run() { return LocusSelfTestInlineCallee.Inner.Nested() + 2; }
}
"#;

const INLINE_CALLER_OVERLOAD_BASELINE: &str = r#"public static class LocusSelfTestInlineCallerOverload
{
    public static int Run() { return LocusSelfTestInlineCallee.Pick(3) + 3; }
}
"#;

const INLINE_CALLER_LAMBDA_BASELINE: &str = r#"public static class LocusSelfTestInlineCallerLambda
{
    public static int Run()
    {
        System.Func<int> read = () => LocusSelfTestInlineCallee.LambdaSeed();
        return read() + 4;
    }
}
"#;

const INLINE_CALLER_BRANCH_BASELINE: &str = r#"public static class LocusSelfTestInlineCallerBranch
{
    public static int Run(int x)
    {
        return x > 0 ? LocusSelfTestInlineCallee.BranchSeed(x) + 5 : -1;
    }
}
"#;

const INLINE_CALLER_ARRAY_BASELINE: &str = r#"public static class LocusSelfTestInlineCallerArray
{
    public static int Run()
    {
        var values = new System.Collections.Generic.List<int> { LocusSelfTestInlineCallee.ArraySeed() };
        return values[0] + 6;
    }
}
"#;

// R12 — depth-2 static inline chain. Leaf and Mid are AggressiveInlining, so a
// leaf edit must propagate through TWO refresh rounds to reach Top.
const INLINE_CHAIN_LEAF_BASELINE: &str = r#"public static class LocusSelfTestChainLeaf
{
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int Leaf() { return 3100; }
}
"#;

const INLINE_CHAIN_MID_BASELINE: &str = r#"public static class LocusSelfTestChainMid
{
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int Mid() { return LocusSelfTestChainLeaf.Leaf() + 30; }
}
"#;

const INLINE_CHAIN_TOP_BASELINE: &str = r#"public static class LocusSelfTestChainTop
{
    public static int Top() { return LocusSelfTestChainMid.Mid() + 300; }
}
"#;

// R13 — cross-asmdef STATIC inline refresh. The callee lives in the lib
// assembly (Lib/ folder), the caller in Assembly-CSharp.
const LIB_INLINE_BASELINE: &str = r#"public static class LocusSelfTestLibInline
{
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public static int Seed() { return 6200; }
}
"#;

const LIB_INLINE_CALLER_BASELINE: &str = r#"public static class LocusSelfTestLibInlineCaller
{
    public static int Run() { return LocusSelfTestLibInline.Seed() + 6; }
}
"#;

// R14 — same-assembly INSTANCE inline refresh. Tap() is an instance method, so
// the patched-method redirect does not cover it and the refreshed caller
// re-inlines the stale body.
const INST_INLINEE_BASELINE: &str = r#"public class LocusSelfTestInstInlinee
{
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.AggressiveInlining)]
    public int Tap() { return 8400; }
}
"#;

const INST_INLINE_CALLER_BASELINE: &str = r#"public static class LocusSelfTestInstInlineCaller
{
    public static int Run() { return new LocusSelfTestInstInlinee().Tap() + 8; }
}
"#;

const SUBJECT_BASELINE: &str = r#"using UnityEngine;
using System.Threading.Tasks;

public class LocusSelfTestSubject : MonoBehaviour
{
    public static LocusSelfTestSubject Instance;
    public static int EvtCount;
    public static int UpdateBeats;
    public static System.Func<int> Captured;
    private static int s_secret = 600;
    private int _ticks = 0;
    private int _seed = 40;
    private int _legacy = 3;
    private int _flux = 5;

    public int Ticks { get { return _ticks; } }
    public int Gauge { get { return 17; } }
    public int Doomed { get { return 1; } }
    public int Arrow => 12;
    private int _stash = 8;
    public int Stash { get { return _stash; } set { _stash = value; } }
    public int this[int i] { get { return i; } }
    public event System.Action Surge { add { EvtCount += 1; } remove { } }

    public class Inner
    {
        public int Nine() { return 1; }
    }

    void Awake() { if (Instance == null) Instance = this; }
    void Update() { _ticks += Step(); }

    public int Step() { return 1; }
    public int Mult() { return 1002; }
    public int Snare() { return 5; }
    public int Cond()
    {
#if UNITY_EDITOR
        return 1;
#else
        return 2;
#endif
    }
    public int Probe() { return 0; }
    public int Spare() { return 1; }
    public int Flux() { return _flux; }
    public int Spark() { return 0; }
    private int SecretCore() { return 7000; }
    public int Vaulted() { return SecretCore() + s_secret; }
    public int Relay() { return new LocusSelfTestNegative().Echo(20) + 1; }
    public int RelayVal() { return new LocusSelfTestNegGeneric<int>().Val(); }
    public int LibRelay() { return new LocusSelfTestLibType().LibBody() + 3; }
    public int LibSigRelay() { return new LocusSelfTestLibType().LibSig(10); }
    public int Seed() { return _seed; }
    public int Legacy() { return _legacy; }
    public int Lambda() { int basis = 1; System.Func<int> f = () => basis + 1; return f(); }
    public int Local() { int InnerFn() { return 1; } return InnerFn(); }
    public int Anon() { var a = new { V = 1 }; return a.V; }
    public int Match(object k) { return k is int ? 1 : 0; }
    public int Flip() { return 3; }
    public int Shrink() { return 2; }
    public int CallRenamed() { return LocusSelfTestHelper.Renamed(); }
    public int CallBump() { int v = 1; LocusSelfTestHelper.Bump(ref v); return v; }
    public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a); }
    public int ModeValue(LocusSelfTestMode mode)
    {
        switch (mode)
        {
            case LocusSelfTestMode.A: return 11;
            case LocusSelfTestMode.B: return 22;
            default: return 0;
        }
    }
    public Task<int> Pulse() { return Task.FromResult(2001); }
    public System.Collections.IEnumerator Counting() { yield return 1; }
}
"#;

const MESSAGE_BASELINE: &str = r#"using UnityEngine;

public class LocusSelfTestMessages : MonoBehaviour
{
    public static LocusSelfTestMessages Instance;
    public static LocusSelfTestMessages Physics3D;
    public static LocusSelfTestMessages Physics2D;
    public static int UpdateCount;
    public static int LateUpdateCount;
    public static int FixedUpdateCount;
    public static int Trigger3D;
    public static int TriggerStay3D;
    public static int TriggerExit3D;
    public static int Collision3D;
    public static int CollisionStay3D;
    public static int CollisionExit3D;
    public static int Trigger2D;
    public static int TriggerStay2D;
    public static int TriggerExit2D;
    public static int Collision2D;
    public static int CollisionStay2D;
    public static int CollisionExit2D;

    void Awake()
    {
        if (gameObject.name == "LocusHotReloadSelfTestMessages") Instance = this;
        if (gameObject.name.EndsWith("3D")) Physics3D = this;
        if (gameObject.name.EndsWith("2D")) Physics2D = this;
    }

    public int Marker() { return 1; }

    public static void ResetFrameCounters()
    {
        UpdateCount = 0;
        LateUpdateCount = 0;
        FixedUpdateCount = 0;
    }

    public static void ResetProxyCounters()
    {
        Trigger3D = 0;
        TriggerStay3D = 0;
        TriggerExit3D = 0;
        Collision3D = 0;
        CollisionStay3D = 0;
        CollisionExit3D = 0;
        Trigger2D = 0;
        TriggerStay2D = 0;
        TriggerExit2D = 0;
        Collision2D = 0;
        CollisionStay2D = 0;
        CollisionExit2D = 0;
    }

    public static int FrameTotal() { return UpdateCount + LateUpdateCount + FixedUpdateCount; }
}

public class LocusSelfTestLifecycleMessages : MonoBehaviour
{
    public static LocusSelfTestLifecycleMessages Instance;
    public static int AwakeCatchUp;
    public static int StartCatchUp;
    public static int ValidateCatchUp;
    public static int MouseDownCount;
    public static int AnimatorIkLayer;
    public static int DestroyCount;

    public int Marker() { return 2; }

    public static void ResetLifecycleCounters()
    {
        AwakeCatchUp = 0;
        StartCatchUp = 0;
        ValidateCatchUp = 0;
    }

    public static void ResetUtilityProxyCounters()
    {
        MouseDownCount = 0;
        AnimatorIkLayer = 0;
        DestroyCount = 0;
    }

    public static int LifecycleTotal() { return AwakeCatchUp + StartCatchUp + ValidateCatchUp; }
    public static int UtilityProxyTotal() { return MouseDownCount + AnimatorIkLayer + DestroyCount; }
}
"#;

const EDITOR_BASELINE: &str = r#"public static class LocusSelfTestEditorTool
{
    public static int Reading() { return 1; }
}
"#;

const HELPER_BASELINE: &str = r#"public static class LocusSelfTestHelper
{
    public static int Twice(int a) { return a * 2; }
    public static int Pick() { return 1; }
    public static int Renamed() { return 21; }
    public static void Bump(ref int v) { v += 1; }
}
"#;

const MODE_BASELINE: &str = r#"public enum LocusSelfTestMode { A = 0, B = 1 }
"#;

const STRUCT_BASELINE: &str = r#"public struct LocusSelfTestStruct
{
    public int Value;

    public int Get() { return 1; }

    public static LocusSelfTestStruct operator +(LocusSelfTestStruct a, LocusSelfTestStruct b)
    {
        var r = new LocusSelfTestStruct();
        r.Value = a.Value + b.Value;
        return r;
    }

    public static implicit operator int(LocusSelfTestStruct s) { return s.Value; }

    public static implicit operator LocusSelfTestStruct(int v)
    {
        var r = new LocusSelfTestStruct();
        r.Value = v;
        return r;
    }
}
"#;

const CTOR_BASELINE: &str = r#"public class LocusSelfTestCtor
{
    public int Seed;

    public LocusSelfTestCtor() { Seed = 1; }
}
"#;

const IFACE_BASELINE: &str = r#"public interface ILocusSelfTestContract
{
    int Plan();
}

public class LocusSelfTestContractImpl : ILocusSelfTestContract
{
    public int Plan() { return 1; }
}
"#;

const NEG_BASELINE: &str = r#"public enum LocusSelfTestNegEnum { X = 1, Y = 2 }

public delegate int LocusSelfTestNegDel(int x);

public class LocusSelfTestNegative
{
    public const int Limit = 3;
    private int _hidden = 9;

    public int Plain() { return Limit; }
    public int Solid() { return 1; }
    public int Hidden() { return _hidden; }
    internal int Wide() { return 5; }
    public T Echo<T>(T value) { return value; }
}

public class LocusSelfTestNegBehaviour : UnityEngine.MonoBehaviour
{
    public int Solid() { return 1; }
}

public class LocusSelfTestNegGeneric<T>
{
    public int Marker;

    public LocusSelfTestNegGeneric() { Marker = 1; }

    public int Val() { return 1; }
}

public class LocusSelfTestNegFin
{
    ~LocusSelfTestNegFin() { }
}

internal class LocusSelfTestNegHidden
{
    public int V;
}

public class LocusSelfTestLocked
{
    public int Worth;

    private LocusSelfTestLocked(int worth) { Worth = worth; }

    public static int Spawn() { return 1; }
}
"#;

// B3: assembly definition for the lib corpus. No platform restriction (it
// compiles for the Editor too), autoReferenced so Assembly-CSharp picks it
// up without an explicit reference (predefined assemblies auto-reference
// every autoReferenced asmdef).
const LIB_ASMDEF_BASELINE: &str = r#"{
    "name": "LocusSelfTestLib",
    "rootNamespace": "",
    "references": [],
    "includePlatforms": [],
    "excludePlatforms": [],
    "allowUnsafeCode": false,
    "overrideReferences": false,
    "precompiledReferences": [],
    "autoReferenced": true,
    "defineConstraints": [],
    "versionDefines": [],
    "noEngineReferences": false
}
"#;

// All-public surface on purpose: the cross-asmdef cases measure assembly
// plumbing (original-type resolution, cross-assembly M3, cross-assembly
// shims), not the access-caps machinery (P34-P37 cover that).
const LIB_BASELINE: &str = r#"public class LocusSelfTestLibType
{
    public int LibSeed = 8;

    public int LibBody() { return 5; }
    public int LibSig(int x) { return x + 1; }
}
"#;

// B6: both parts declare PRIVATE instance fields, so the patch copy must
// merge the parts in the original assembly's field order (the layout guard
// verifies) and the edited part-A body must reach part B's field and method.
const PARTIAL_A_BASELINE: &str = r#"public partial class LocusSelfTestPartial
{
    private int _alpha = 30;

    public int Combine() { return _alpha + Basis() + _beta; }
}
"#;

const PARTIAL_B_BASELINE: &str = r#"public partial class LocusSelfTestPartial
{
    private int _beta = 400;

    private int Basis() { return 5; }
}
"#;

// Cold-classification corpus: independent types, each mutated alone so exactly
// one rejection reason fires (the unchanged siblings never diff). The Mark()
// bodies differ on purpose so every swap anchor below is unique.
const COLD_BASELINE: &str = r#"public class LocusSelfTestKind
{
    public int Mark() { return 1; }
}

public class LocusSelfTestStaticCtor
{
    public static int Counter;

    static LocusSelfTestStaticCtor() { Counter = 1; }

    public int Mark() { return Counter; }
}

public interface ILocusSelfTestExplicit
{
    int Plan();
}

public class LocusSelfTestExplicitImpl : ILocusSelfTestExplicit
{
    int ILocusSelfTestExplicit.Plan() { return 1; }
}

public enum LocusSelfTestColdEnum { P = 1, Q = 2 }

public class LocusSelfTestConstHost
{
    public const int Cap = 5;

    public int Use() { return 9; }
}

public class LocusSelfTestFieldMods
{
    public int Field;

    public int Read() { return Field; }
}

public class LocusSelfTestCtorDrop
{
    public int Seed;

    public LocusSelfTestCtorDrop() { Seed = 1; }
    public LocusSelfTestCtorDrop(int seed) { Seed = seed; }
}

public class LocusSelfTestFinDrop
{
    public int Mark() { return 3; }

    ~LocusSelfTestFinDrop() { }
}

public class LocusSelfTestOpHost
{
    public int Value;

    public int Mark() { return Value; }
}
"#;

// Created fresh and deleted inside its test — the record is rejected on
// presence, so this text never reaches a real Unity compile.
const RECORD_BASELINE: &str = r#"public record LocusSelfTestRecord
{
    public int Mark() { return 1; }
}
"#;

// Using-rehook gate corpus (M6). Each file holds ONE member the whole-file
// re-detour cannot reproduce; the test toggles a using directive to drive the
// gate. Mark()/plain methods are present only as inert filler.
const USE_CONST_BASELINE: &str = r#"public class LocusSelfTestUseConst
{
    public const int Cap = 1 + 2;

    public int Mark() { return 7; }
}
"#;

const USE_STATIC_BASELINE: &str = r#"public class LocusSelfTestUseStatic
{
    public static int Seed = Compute();

    static int Compute() { return 3; }

    public int Mark() { return 7; }
}
"#;

const USE_GENERIC_BASELINE: &str = r#"public class LocusSelfTestUseGeneric
{
    public int Pick<T>(T value) { return 1; }
}
"#;

const USE_EXPLICIT_BASELINE: &str = r#"public interface ILocusSelfTestUseExpl
{
    int Go();
}

public class LocusSelfTestUseExpl : ILocusSelfTestUseExpl
{
    int ILocusSelfTestUseExpl.Go() { return 1; }
}
"#;

const USE_FINALIZER_BASELINE: &str = r#"public class LocusSelfTestUseFin
{
    public int Mark() { return 1; }

    ~LocusSelfTestUseFin() { }
}
"#;

// Single-part partial type: part-removed / new-part / using-in-partial /
// initializer-changed / partial-method-twice all mutate this one file.
const PARTIAL_COLD_BASELINE: &str = r#"public partial class LocusSelfTestPartialCold
{
    private int _value = 10;

    partial void Hook();

    public int Read() { return _value; }
}
"#;

// Two parts of one type in one file, so dropping a part changes the count.
const PARTIAL_COUNT_BASELINE: &str = r#"public partial class LocusSelfTestPartialCount
{
    public int A() { return 1; }
}

public partial class LocusSelfTestPartialCount
{
    public int B() { return 2; }
}
"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfTestEvent {
    running: bool,
    finished: bool,
    line: Option<String>,
    passed: u32,
    failed: u32,
}

#[derive(Default)]
struct NegativeLedgers {
    struct_text: String,
    ctor_text: String,
    iface_text: String,
    partial_a_text: String,
}

/// Reply of the `hot_reload_inlining_active` bridge command: the editor's
/// runtime "is Mono inlining right now?" verdict (force-JIT a canary, read its
/// inline_info bit) plus the codeOptimization setting it disagreed with.
#[derive(serde::Deserialize, Default)]
struct InliningActiveResponse {
    inlining_active: bool,
    #[serde(default)]
    code_optimization: String,
    #[serde(default)]
    detail: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct MessageDriverCapabilities {
    physics3d: bool,
    physics2d: bool,
}

impl MessageDriverCapabilities {
    fn has_physics_proxy(self) -> bool {
        self.physics3d || self.physics2d
    }

    fn label(self) -> String {
        format!("physics3d={} physics2d={}", self.physics3d, self.physics2d)
    }
}

struct SelfTest {
    app: tauri::AppHandle,
    project: String,
    passed: u32,
    failed: u32,
    /// Per-file texts as the positive phase left them; the negative phase
    /// restores files to exactly these after each cold-classification probe.
    negative_ledgers: NegativeLedgers,
    /// B5 — Enter Play Mode Options, probed in edit mode right before the
    /// play transition: Some(true) on the classic reload-domain path,
    /// Some(false) when Reload Domain is disabled, None when the probe
    /// failed (mode-aware assertions then assume the classic path).
    domain_reload_on_play: Option<bool>,
    /// Enter Play Mode Options master switch: Some(true) when enabled (fast
    /// play-enter, editor detours persist), Some(false) on the classic full
    /// reload, None when the probe failed (assume classic).
    epmo_enabled: Option<bool>,
    /// E01 applied AND observed: the editor-assembly patch is deliberately
    /// left live so Phase 3 can assert its fate across the play transition
    /// (B5), reverting the file afterwards.
    editor_patch_live: bool,
    /// Release-first: the connected editor is in Code Optimization = Release,
    /// where Mono inlines some methods past the detour. Behavioral asserts then
    /// tolerate an "inlined in Release" apply (the change converges on recompile
    /// rather than through the live detour). Detected once at the start of `run`.
    release_mode: bool,
    /// Original Code Optimization mode, restored after the self-test forces
    /// Release to exercise Mono inlining behavior.
    original_code_optimization: Option<String>,
    /// Set by `apply_texts` from the latest apply summary: true when Unity
    /// reported methods it inlined (Release). Consulted by `expect_output`.
    last_apply_inlined: bool,
    /// The most recent hot-reload apply summary verbatim. Surfaced in
    /// Release-strict failure diagnostics so a stale immediate read can be
    /// localized (did the inline caller refresh engage / patch the caller, or
    /// report a note?).
    last_apply_summary: String,
    /// Unity projects can include only the 3D physics module, only the 2D
    /// physics module, both, or neither. Proxy-message coverage follows the
    /// modules actually loaded by the editor.
    message_driver_capabilities: MessageDriverCapabilities,
}

/// Replace exactly one occurrence, failing loudly when the anchor text is
/// missing (corpus drift would otherwise silently turn an edit into a noop).
fn swap(text: &mut String, from: &str, to: &str) -> Result<(), String> {
    if !text.contains(from) {
        return Err(format!("internal corpus error: anchor not found: {from}"));
    }
    *text = text.replacen(from, to, 1);
    Ok(())
}

/// Replace the single line that STARTS with `line_prefix` — robust against
/// earlier steps having rewritten the member body after the prefix.
fn swap_line(text: &mut String, line_prefix: &str, replacement: &str) -> Result<(), String> {
    let mut indices = text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with(line_prefix))
        .map(|(index, _)| index);
    let Some(index) = indices.next() else {
        return Err(format!(
            "internal corpus error: no line starts with: {line_prefix}"
        ));
    };
    if indices.next().is_some() {
        return Err(format!(
            "internal corpus error: multiple lines start with: {line_prefix}"
        ));
    }
    let mut lines: Vec<&str> = text.lines().collect();
    lines[index] = replacement;
    let mut rebuilt = lines.join("\n");
    if text.ends_with('\n') {
        rebuilt.push('\n');
    }
    *text = rebuilt;
    Ok(())
}

fn squash(text: &str) -> String {
    let mut line = text.replace('\n', " | ");
    if line.len() > 360 {
        line.truncate(360);
    }
    line
}

fn reload_boundary_error(error: &str) -> bool {
    matches!(error, "managed_reloading" | "domain_reload_interrupted")
        || error.contains("managed_reloading")
        || error.contains("domain_reload_interrupted")
}

fn remaining_or_timeout(
    started: Instant,
    timeout: Duration,
    action: &str,
) -> Result<Duration, String> {
    timeout.checked_sub(started.elapsed()).ok_or_else(|| {
        format!(
            "{action} did not become ready within {}s",
            timeout.as_secs()
        )
    })
}

fn spawn_test_objects_code(caps: MessageDriverCapabilities) -> String {
    let mut code = String::from(
        "foreach (var existing in UnityEngine.Object.FindObjectsByType<UnityEngine.GameObject>(UnityEngine.FindObjectsInactive.Include, UnityEngine.FindObjectsSortMode.None))\n\
         {\n\
             if (existing != null && existing.name.StartsWith(\"LocusHotReloadSelfTest\")) UnityEngine.Object.Destroy(existing);\n\
         }\n\
         await ctx.WaitFrames(2);\n\
         var go = new UnityEngine.GameObject(\"LocusHotReloadSelfTest\");\n\
         go.AddComponent<LocusSelfTestSubject>();\n\
         var messageGo = new UnityEngine.GameObject(\"LocusHotReloadSelfTestMessages\");\n\
         messageGo.AddComponent<LocusSelfTestMessages>();\n",
    );
    code.push_str(
        "var lifecycleGo = new UnityEngine.GameObject(\"LocusHotReloadSelfTestLifecycleMessages\");\n\
         LocusSelfTestLifecycleMessages.Instance = lifecycleGo.AddComponent<LocusSelfTestLifecycleMessages>();\n",
    );
    if caps.physics3d {
        code.push_str(
            "var message3DGo = new UnityEngine.GameObject(\"LocusHotReloadSelfTestMessages3D\");\n\
             message3DGo.AddComponent<LocusSelfTestMessages>();\n\
             var box3D = message3DGo.AddComponent<UnityEngine.BoxCollider>();\n\
             if (box3D == null) throw new System.InvalidOperationException(\"BoxCollider was not added\");\n\
             box3D.isTrigger = true;\n",
        );
    }
    if caps.physics2d {
        code.push_str(
            "var message2DGo = new UnityEngine.GameObject(\"LocusHotReloadSelfTestMessages2D\");\n\
             message2DGo.AddComponent<LocusSelfTestMessages>();\n\
             var box2D = message2DGo.AddComponent<UnityEngine.BoxCollider2D>();\n\
             if (box2D == null) throw new System.InvalidOperationException(\"BoxCollider2D was not added\");\n\
             box2D.isTrigger = true;\n",
        );
    }
    code.push_str("return \"spawned\";");
    code
}

fn initial_proxy_methods(caps: MessageDriverCapabilities) -> String {
    let mut methods = String::new();
    if caps.physics3d {
        methods.push_str(
            "    void OnTriggerEnter(Collider other) { Trigger3D += other != null ? 10 : 1; }\n\
             void OnTriggerStay(Collider other) { TriggerStay3D += other != null ? 11 : 1; }\n\
             void OnTriggerExit(Collider other) { TriggerExit3D += other != null ? 100 : 1; }\n\
             void OnCollisionEnter(Collision collision) { Collision3D += collision == null ? 1 : 10; }\n\
             void OnCollisionStay(Collision collision) { CollisionStay3D += collision == null ? 1 : 11; }\n\
             void OnCollisionExit(Collision collision) { CollisionExit3D += collision == null ? 1 : 100; }\n",
        );
    }
    if caps.physics2d {
        methods.push_str(
            "    void OnTriggerEnter2D(Collider2D other) { Trigger2D += other != null ? 20 : 1; }\n\
             void OnTriggerStay2D(Collider2D other) { TriggerStay2D += other != null ? 21 : 1; }\n\
             void OnTriggerExit2D(Collider2D other) { TriggerExit2D += other != null ? 200 : 1; }\n\
             void OnCollisionEnter2D(Collision2D collision) { Collision2D += collision == null ? 1 : 20; }\n\
             void OnCollisionStay2D(Collision2D collision) { CollisionStay2D += collision == null ? 1 : 21; }\n\
             void OnCollisionExit2D(Collision2D collision) { CollisionExit2D += collision == null ? 1 : 200; }\n",
        );
    }
    methods
}

fn physics3d_proxy_initial_assertion() -> &'static str {
    "LocusSelfTestMessages.ResetProxyCounters();\n\
     string InvokeEventProxy(UnityEngine.GameObject go, string message, object arg)\n\
     {\n\
         foreach (var behaviour in go.GetComponents<UnityEngine.MonoBehaviour>())\n\
         {\n\
             if (behaviour == null) continue;\n\
             var type = behaviour.GetType();\n\
             if (type.FullName != \"Locus.LocusEventProxy\") continue;\n\
             var method = type.GetMethod(message, System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);\n\
             if (method == null) return \"proxy-missing-method-\" + message;\n\
             method.Invoke(behaviour, new object[] { arg });\n\
             return \"\";\n\
         }\n\
         return \"proxy-missing-component-\" + message;\n\
     }\n\
     var instance = LocusSelfTestMessages.Physics3D;\n\
     if (instance == null) return \"proxy-3d-missing-instance\";\n\
     var go = instance.gameObject;\n\
     var c3 = go.GetComponent<UnityEngine.BoxCollider>();\n\
     if (c3 == null) return \"proxy-3d-missing-collider\";\n\
     var err = InvokeEventProxy(go, \"OnTriggerEnter\", c3); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerStay\", c3); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerExit\", c3); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionEnter\", null); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionStay\", null); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionExit\", null); if (err.Length > 0) return err;\n\
     return LocusSelfTestMessages.Trigger3D == 10 && LocusSelfTestMessages.TriggerStay3D == 11 && LocusSelfTestMessages.TriggerExit3D == 100\n\
         && LocusSelfTestMessages.Collision3D == 1 && LocusSelfTestMessages.CollisionStay3D == 1 && LocusSelfTestMessages.CollisionExit3D == 1\n\
         ? \"proxy-3d-ok\"\n\
         : (\"proxy-3d-missing t3=\" + LocusSelfTestMessages.Trigger3D + \" ts3=\" + LocusSelfTestMessages.TriggerStay3D + \" tx3=\" + LocusSelfTestMessages.TriggerExit3D\n\
             + \" c3=\" + LocusSelfTestMessages.Collision3D + \" cs3=\" + LocusSelfTestMessages.CollisionStay3D + \" cx3=\" + LocusSelfTestMessages.CollisionExit3D);"
}

fn physics2d_proxy_initial_assertion() -> &'static str {
    "LocusSelfTestMessages.ResetProxyCounters();\n\
     string InvokeEventProxy(UnityEngine.GameObject go, string message, object arg)\n\
     {\n\
         foreach (var behaviour in go.GetComponents<UnityEngine.MonoBehaviour>())\n\
         {\n\
             if (behaviour == null) continue;\n\
             var type = behaviour.GetType();\n\
             if (type.FullName != \"Locus.LocusEventProxy\") continue;\n\
             var method = type.GetMethod(message, System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);\n\
             if (method == null) return \"proxy-missing-method-\" + message;\n\
             method.Invoke(behaviour, new object[] { arg });\n\
             return \"\";\n\
         }\n\
         return \"proxy-missing-component-\" + message;\n\
     }\n\
     var instance = LocusSelfTestMessages.Physics2D;\n\
     if (instance == null) return \"proxy-2d-missing-instance\";\n\
     var go = instance.gameObject;\n\
     var c2 = go.GetComponent<UnityEngine.BoxCollider2D>();\n\
     if (c2 == null) return \"proxy-2d-missing-collider\";\n\
     var err = InvokeEventProxy(go, \"OnTriggerEnter2D\", c2); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerStay2D\", c2); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerExit2D\", c2); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionEnter2D\", null); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionStay2D\", null); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionExit2D\", null); if (err.Length > 0) return err;\n\
     return LocusSelfTestMessages.Trigger2D == 20 && LocusSelfTestMessages.TriggerStay2D == 21 && LocusSelfTestMessages.TriggerExit2D == 200\n\
         && LocusSelfTestMessages.Collision2D == 1 && LocusSelfTestMessages.CollisionStay2D == 1 && LocusSelfTestMessages.CollisionExit2D == 1\n\
         ? \"proxy-2d-ok\"\n\
         : (\"proxy-2d-missing t2=\" + LocusSelfTestMessages.Trigger2D + \" ts2=\" + LocusSelfTestMessages.TriggerStay2D + \" tx2=\" + LocusSelfTestMessages.TriggerExit2D\n\
             + \" c2=\" + LocusSelfTestMessages.Collision2D + \" cs2=\" + LocusSelfTestMessages.CollisionStay2D + \" cx2=\" + LocusSelfTestMessages.CollisionExit2D);"
}

fn physics3d_proxy_reedit_assertion() -> &'static str {
    "LocusSelfTestMessages.ResetProxyCounters();\n\
     string InvokeEventProxy(UnityEngine.GameObject go, string message, object arg)\n\
     {\n\
         foreach (var behaviour in go.GetComponents<UnityEngine.MonoBehaviour>())\n\
         {\n\
             if (behaviour == null) continue;\n\
             var type = behaviour.GetType();\n\
             if (type.FullName != \"Locus.LocusEventProxy\") continue;\n\
             var method = type.GetMethod(message, System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);\n\
             if (method == null) return \"proxy-missing-method-\" + message;\n\
             method.Invoke(behaviour, new object[] { arg });\n\
             return \"\";\n\
         }\n\
         return \"proxy-missing-component-\" + message;\n\
     }\n\
     var instance = LocusSelfTestMessages.Physics3D;\n\
     if (instance == null) return \"proxy-3d-reedit-missing-instance\";\n\
     var go = instance.gameObject;\n\
     var c3 = go.GetComponent<UnityEngine.BoxCollider>();\n\
     if (c3 == null) return \"proxy-3d-reedit-missing-collider\";\n\
     var err = InvokeEventProxy(go, \"OnTriggerEnter\", c3); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerStay\", c3); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionEnter\", null); if (err.Length > 0) return err;\n\
     return LocusSelfTestMessages.Trigger3D == 30 && LocusSelfTestMessages.TriggerStay3D == 3011 && LocusSelfTestMessages.Collision3D == 1\n\
         ? \"proxy-3d-reedit-ok\"\n\
         : (\"proxy-3d-reedit-mismatch t3=\" + LocusSelfTestMessages.Trigger3D + \" ts3=\" + LocusSelfTestMessages.TriggerStay3D + \" c3=\" + LocusSelfTestMessages.Collision3D);"
}

fn physics2d_proxy_reedit_assertion() -> &'static str {
    "LocusSelfTestMessages.ResetProxyCounters();\n\
     string InvokeEventProxy(UnityEngine.GameObject go, string message, object arg)\n\
     {\n\
         foreach (var behaviour in go.GetComponents<UnityEngine.MonoBehaviour>())\n\
         {\n\
             if (behaviour == null) continue;\n\
             var type = behaviour.GetType();\n\
             if (type.FullName != \"Locus.LocusEventProxy\") continue;\n\
             var method = type.GetMethod(message, System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);\n\
             if (method == null) return \"proxy-missing-method-\" + message;\n\
             method.Invoke(behaviour, new object[] { arg });\n\
             return \"\";\n\
         }\n\
         return \"proxy-missing-component-\" + message;\n\
     }\n\
     var instance = LocusSelfTestMessages.Physics2D;\n\
     if (instance == null) return \"proxy-2d-reedit-missing-instance\";\n\
     var go = instance.gameObject;\n\
     var c2 = go.GetComponent<UnityEngine.BoxCollider2D>();\n\
     if (c2 == null) return \"proxy-2d-reedit-missing-collider\";\n\
     var err = InvokeEventProxy(go, \"OnTriggerEnter2D\", c2); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerStay2D\", c2); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionEnter2D\", null); if (err.Length > 0) return err;\n\
     return LocusSelfTestMessages.Trigger2D == 40 && LocusSelfTestMessages.TriggerStay2D == 4021 && LocusSelfTestMessages.Collision2D == 1\n\
         ? \"proxy-2d-reedit-ok\"\n\
         : (\"proxy-2d-reedit-mismatch t2=\" + LocusSelfTestMessages.Trigger2D + \" ts2=\" + LocusSelfTestMessages.TriggerStay2D + \" c2=\" + LocusSelfTestMessages.Collision2D);"
}

fn physics3d_proxy_second_reedit_assertion() -> &'static str {
    "LocusSelfTestMessages.ResetProxyCounters();\n\
     string InvokeEventProxy(UnityEngine.GameObject go, string message, object arg)\n\
     {\n\
         foreach (var behaviour in go.GetComponents<UnityEngine.MonoBehaviour>())\n\
         {\n\
             if (behaviour == null) continue;\n\
             var type = behaviour.GetType();\n\
             if (type.FullName != \"Locus.LocusEventProxy\") continue;\n\
             var method = type.GetMethod(message, System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);\n\
             if (method == null) return \"proxy-missing-method-\" + message;\n\
             method.Invoke(behaviour, new object[] { arg });\n\
             return \"\";\n\
         }\n\
         return \"proxy-missing-component-\" + message;\n\
     }\n\
     var instance = LocusSelfTestMessages.Physics3D;\n\
     if (instance == null) return \"proxy-3d-second-reedit-missing-instance\";\n\
     var go = instance.gameObject;\n\
     var c3 = go.GetComponent<UnityEngine.BoxCollider>();\n\
     if (c3 == null) return \"proxy-3d-second-reedit-missing-collider\";\n\
     var err = InvokeEventProxy(go, \"OnTriggerEnter\", c3); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerExit\", c3); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionExit\", null); if (err.Length > 0) return err;\n\
     return LocusSelfTestMessages.Trigger3D == 50 && LocusSelfTestMessages.TriggerExit3D == 5100 && LocusSelfTestMessages.CollisionExit3D == 1\n\
         ? \"proxy-3d-second-reedit-ok\"\n\
         : (\"proxy-3d-second-reedit-mismatch t3=\" + LocusSelfTestMessages.Trigger3D + \" tx3=\" + LocusSelfTestMessages.TriggerExit3D + \" cx3=\" + LocusSelfTestMessages.CollisionExit3D);"
}

fn physics2d_proxy_second_reedit_assertion() -> &'static str {
    "LocusSelfTestMessages.ResetProxyCounters();\n\
     string InvokeEventProxy(UnityEngine.GameObject go, string message, object arg)\n\
     {\n\
         foreach (var behaviour in go.GetComponents<UnityEngine.MonoBehaviour>())\n\
         {\n\
             if (behaviour == null) continue;\n\
             var type = behaviour.GetType();\n\
             if (type.FullName != \"Locus.LocusEventProxy\") continue;\n\
             var method = type.GetMethod(message, System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);\n\
             if (method == null) return \"proxy-missing-method-\" + message;\n\
             method.Invoke(behaviour, new object[] { arg });\n\
             return \"\";\n\
         }\n\
         return \"proxy-missing-component-\" + message;\n\
     }\n\
     var instance = LocusSelfTestMessages.Physics2D;\n\
     if (instance == null) return \"proxy-2d-second-reedit-missing-instance\";\n\
     var go = instance.gameObject;\n\
     var c2 = go.GetComponent<UnityEngine.BoxCollider2D>();\n\
     if (c2 == null) return \"proxy-2d-second-reedit-missing-collider\";\n\
     var err = InvokeEventProxy(go, \"OnTriggerEnter2D\", c2); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnTriggerExit2D\", c2); if (err.Length > 0) return err;\n\
     err = InvokeEventProxy(go, \"OnCollisionExit2D\", null); if (err.Length > 0) return err;\n\
     return LocusSelfTestMessages.Trigger2D == 60 && LocusSelfTestMessages.TriggerExit2D == 6200 && LocusSelfTestMessages.CollisionExit2D == 1\n\
         ? \"proxy-2d-second-reedit-ok\"\n\
         : (\"proxy-2d-second-reedit-mismatch t2=\" + LocusSelfTestMessages.Trigger2D + \" tx2=\" + LocusSelfTestMessages.TriggerExit2D + \" cx2=\" + LocusSelfTestMessages.CollisionExit2D);"
}

fn utility_proxy_assertion() -> &'static str {
    "LocusSelfTestLifecycleMessages.ResetUtilityProxyCounters();\n\
     string InvokeMessageProxy(UnityEngine.GameObject go, string proxyTypeName, string message, object arg)\n\
     {\n\
         foreach (var behaviour in go.GetComponents<UnityEngine.MonoBehaviour>())\n\
         {\n\
             if (behaviour == null) continue;\n\
             var type = behaviour.GetType();\n\
             if (type.FullName != proxyTypeName) continue;\n\
             var method = type.GetMethod(message, System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);\n\
             if (method == null) return \"utility-proxy-missing-method-\" + proxyTypeName + \".\" + message;\n\
             object[] args = arg == null ? null : new object[] { arg };\n\
             method.Invoke(behaviour, args);\n\
             return \"\";\n\
         }\n\
         return \"utility-proxy-missing-component-\" + proxyTypeName + \".\" + message;\n\
     }\n\
     var instance = LocusSelfTestLifecycleMessages.Instance;\n\
     if (instance == null) return \"utility-proxy-missing-instance\";\n\
     var go = instance.gameObject;\n\
     var err = InvokeMessageProxy(go, \"Locus.LocusMouseProxy\", \"OnMouseDown\", null); if (err.Length > 0) return err;\n\
     err = InvokeMessageProxy(go, \"Locus.LocusEventProxy\", \"OnAnimatorIK\", 7); if (err.Length > 0) return err;\n\
     err = InvokeMessageProxy(go, \"Locus.LocusEventProxy\", \"OnDestroy\", null); if (err.Length > 0) return err;\n\
     return LocusSelfTestLifecycleMessages.MouseDownCount == 1 && LocusSelfTestLifecycleMessages.AnimatorIkLayer == 7 && LocusSelfTestLifecycleMessages.DestroyCount == 1\n\
         ? \"utility-proxy-ok\"\n\
         : (\"utility-proxy-mismatch mouse=\" + LocusSelfTestLifecycleMessages.MouseDownCount + \" ik=\" + LocusSelfTestLifecycleMessages.AnimatorIkLayer + \" destroy=\" + LocusSelfTestLifecycleMessages.DestroyCount);"
}

impl SelfTest {
    fn emit(&self, line: Option<String>, finished: bool) {
        let _ = self.app.emit(
            "unity-hotreload-selftest",
            SelfTestEvent {
                running: !finished,
                finished,
                line,
                passed: self.passed,
                failed: self.failed,
            },
        );
    }

    fn log(&self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("[HotReload SelfTest] {line}");
        self.emit(Some(line), false);
    }

    fn pass(&mut self, name: &str, detail: impl Into<String>) {
        self.passed += 1;
        self.log(format!("PASS  {name}: {}", detail.into()));
    }

    fn fail(&mut self, name: &str, detail: impl Into<String>) {
        self.failed += 1;
        self.log(format!("FAIL  {name}: {}", detail.into()));
    }

    // ── primitives ───────────────────────────────────────────────────

    fn absolute(&self, relative: &str) -> std::path::PathBuf {
        std::path::Path::new(&self.project).join(relative)
    }

    /// Write a test file the way the agent tools do: capture the prior text
    /// as the hot-reload baseline FIRST, then put the new content on disk.
    async fn write_tracked(&self, relative: &str, content: &str) -> Result<(), String> {
        let path = self.absolute(relative);
        let prior = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        coordinator::note_cs_written(&self.project, &path.to_string_lossy(), prior).await;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| format!("write {relative}: {e}"))
    }

    async fn delete_tracked(&self, relative: &str) -> Result<(), String> {
        let path = self.absolute(relative);
        let prior = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        coordinator::note_cs_written(&self.project, &path.to_string_lossy(), prior).await;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("delete {relative}: {e}")),
        }
    }

    async fn hot_reload(&self, paths: Option<Vec<String>>) -> Result<String, String> {
        coordinator::hot_reload(&self.project, paths).await
    }

    /// Run a C# snippet in the editor and return its output text.
    async fn execute(&self, code: &str) -> Result<String, String> {
        crate::unity_bridge::unity_execute_code(&self.project, code).await
    }

    async fn wait_for_semantic_ready(
        &self,
        action: &str,
        timeout: Duration,
        require_asset_modification: bool,
    ) -> Result<(), String> {
        crate::unity_bridge::set_state_probe_enabled(true);
        crate::unity_bridge::start_unity_semantic_state_observer(&self.project);

        let started = Instant::now();
        let mut last_signature = String::new();
        loop {
            let state = crate::unity_bridge::unity_semantic_state(&self.project).await;
            let signature = format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                state.phase,
                state.source,
                state.reload_phase.as_deref().unwrap_or(""),
                state.domain.phase,
                state.editor_mode.value,
                state.safety.can_call_unity_api,
                state.safety.can_modify_assets_safely,
                state.safety.recommended_action
            );
            if signature != last_signature {
                last_signature = signature;
                self.log(format!(
                    "  semantic wait ({action}): requirement={} phase={} source={} domain={} editorMode={} canCall={} canModify={} action={}",
                    if require_asset_modification {
                        "assetModification"
                    } else {
                        "unityApi"
                    },
                    state.phase,
                    state.source,
                    state.domain.phase,
                    state.editor_mode.value,
                    state.safety.can_call_unity_api,
                    state.safety.can_modify_assets_safely,
                    state.safety.recommended_action
                ));
            }
            let ready = if require_asset_modification {
                state.safety.can_modify_assets_safely
            } else {
                state.safety.can_call_unity_api
            };
            if ready {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err(format!(
                    "Unity was not ready for {action} within {}s; phase={} source={} recommendedAction={}",
                    timeout.as_secs(),
                    state.phase,
                    state.source,
                    state.safety.recommended_action
                ));
            }
            tokio::time::sleep(UNITY_SEMANTIC_READY_POLL).await;
        }
    }

    async fn wait_for_semantic_asset_ready(
        &self,
        action: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        self.wait_for_semantic_ready(action, timeout, true).await
    }

    async fn wait_for_semantic_unity_api_ready(
        &self,
        action: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        self.wait_for_semantic_ready(action, timeout, false).await
    }

    async fn set_code_optimization_retrying(
        &self,
        desired: &str,
        action: &str,
    ) -> Result<String, String> {
        let started = Instant::now();
        loop {
            self.wait_for_semantic_asset_ready(
                action,
                remaining_or_timeout(started, UNITY_SEMANTIC_READY_TIMEOUT, action)?,
            )
            .await?;

            match coordinator::set_code_optimization(&self.project, desired).await {
                Ok(value) => {
                    self.wait_for_semantic_asset_ready(
                        action,
                        remaining_or_timeout(started, UNITY_SEMANTIC_READY_TIMEOUT, action)?,
                    )
                    .await?;
                    return Ok(value);
                }
                Err(error)
                    if reload_boundary_error(&error)
                        && started.elapsed() < UNITY_SEMANTIC_READY_TIMEOUT =>
                {
                    self.log(format!(
                        "  {action}: Unity is reloading while switching Code Optimization ({error}); retrying"
                    ));
                    tokio::time::sleep(UNITY_SEMANTIC_READY_POLL).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Runtime "is Mono inlining right now?" check. Force-JITs a small canary in
    /// the editor and reports whether its inline_info bit was set — the JIT's
    /// EFFECTIVE behavior, which can disagree with the codeOptimization setting
    /// (play-mode has been observed Debug-effective while the setting reads
    /// release, so nothing inlines). Returns (active, human_detail); false on any
    /// transport/parse error so a failure only ever soft-skips an inline assert,
    /// never falsely claims inlining.
    async fn inlining_active(&self) -> (bool, String) {
        match crate::unity_bridge::send_message_with_timeout(
            &self.project,
            "hot_reload_inlining_active",
            "",
            Duration::from_secs(15),
        )
        .await
        {
            Ok(resp) if resp.ok => {
                let message = resp.message.unwrap_or_default();
                match serde_json::from_str::<InliningActiveResponse>(&message) {
                    Ok(parsed) => (
                        parsed.inlining_active,
                        format!(
                            "code_optimization={} {}",
                            parsed.code_optimization, parsed.detail
                        ),
                    ),
                    Err(error) => (
                        false,
                        format!("parse failed: {error}; raw: {}", squash(&message)),
                    ),
                }
            }
            Ok(resp) => (
                false,
                resp.error
                    .unwrap_or_else(|| "inlining_active failed".to_string()),
            ),
            Err(error) => (false, format!("transport: {}", squash(&error))),
        }
    }

    /// Snippet whose output must contain `expected` (sentinel values are
    /// chosen to be unambiguous).
    ///
    /// STRICT — no inlining tolerance. The inline caller refresh now re-patches the
    /// parent method to un-inline (and the edited method's own detour is live), so
    /// the new behavior MUST be observable immediately even in Release. A stale read
    /// is a real gap — a refresh that could not redirect or hit its budget, or the
    /// snippet caller inlining the callee — and is surfaced as a failure rather than
    /// deferred to the queued recompile.
    async fn expect_output(&mut self, name: &str, code: &str, expected: &str) {
        match self.execute(code).await {
            Ok(output) if output.contains(expected) => {
                self.pass(name, format!("observed {expected}"));
            }
            Ok(output) => {
                // Help localize: a stale read that the apply ALSO reported inlined is
                // an un-inline (caller-refresh) gap; a stale read with no inlining is
                // a different bug (wrong patch / wrong expectation).
                let hint = if self.release_mode && self.last_apply_inlined {
                    " — apply reported inlined in Release, so caller refresh did not make it live (un-inline gap)"
                } else {
                    ""
                };
                self.fail(
                    name,
                    format!(
                        "expected '{expected}' in output, got: {}{hint}",
                        output.trim()
                    ),
                );
            }
            Err(error) => self.fail(name, format!("snippet failed: {error}")),
        }
    }

    /// Adversarial release-only assertion: unlike `expect_output`, this does
    /// not tolerate the Release inlining fallback. It verifies whether the
    /// just-applied patch is observable immediately through live detours.
    async fn expect_release_immediate_output(&mut self, name: &str, code: &str, expected: &str) {
        if !self.release_mode {
            return;
        }
        match self.execute(code).await {
            Ok(output) => {
                if output.contains(expected) {
                    self.pass(name, format!("observed {expected} immediately"));
                } else {
                    // Surface the apply summary so a stale read is diagnosable:
                    // whether the inline caller refresh reported inlining and
                    // actually patched the caller, or emitted a note instead.
                    self.fail(
                        name,
                        format!(
                            "Release immediate effect missing; expected '{expected}' in output, got: {}. \
                             Last apply summary: {}",
                            output.trim(),
                            squash(&self.last_apply_summary),
                        ),
                    );
                }
            }
            Err(error) => self.fail(name, format!("snippet failed: {error}")),
        }
    }

    async fn revert_files(&mut self, reverts: &[(&str, &str)]) {
        for (relative, text) in reverts {
            if let Err(error) = self.write_tracked(relative, text).await {
                self.log(format!("  revert of {relative} failed: {error}"));
            }
        }
    }

    /// Write the given texts and hot-reload the batch. A transport error, a
    /// Unity rejection or an unexpected COLD verdict counts as failure: the
    /// revert texts go back on disk so later batches stay clean, and the
    /// caller must restore its ledgers.
    async fn apply_texts(
        &mut self,
        name: &str,
        writes: &[(&str, &str)],
        reverts: &[(&str, &str)],
    ) -> Option<String> {
        for (relative, text) in writes {
            if let Err(error) = self.write_tracked(relative, text).await {
                self.fail(name, error);
                self.revert_files(reverts).await;
                return None;
            }
        }
        self.last_apply_inlined = false;
        match self.hot_reload(None).await {
            Ok(summary) if summary.contains("Hot reload not applicable") => {
                self.last_apply_summary = summary.clone();
                self.fail(
                    name,
                    format!("unexpected cold verdict: {}", squash(&summary)),
                );
                self.revert_files(reverts).await;
                None
            }
            Ok(summary) => {
                // Release-first: remember whether Unity inlined any method in
                // this batch so the following behavioral assert can tolerate it,
                // and keep the full summary for failure diagnostics.
                self.last_apply_inlined = summary.contains("inlined in Release");
                self.last_apply_summary = summary.clone();
                Some(summary)
            }
            Err(error) => {
                self.fail(name, squash(&error));
                self.revert_files(reverts).await;
                None
            }
        }
    }

    /// One positive step on a single file: mutate the ledger, write, hot
    /// reload. On failure the ledger AND the disk file revert to the
    /// pre-step text, so one rejected patch cannot poison later batches.
    async fn step_file(
        &mut self,
        name: &str,
        relative: &str,
        ledger: &mut String,
        mutate: impl FnOnce(&mut String) -> Result<(), String>,
    ) -> Option<String> {
        self.log(format!("— {name}"));
        let snapshot = ledger.clone();
        if let Err(error) = mutate(ledger) {
            self.fail(name, error);
            *ledger = snapshot;
            return None;
        }
        let outcome = self
            .apply_texts(
                name,
                &[(relative, ledger.as_str())],
                &[(relative, snapshot.as_str())],
            )
            .await;
        if outcome.is_none() {
            *ledger = snapshot;
        }
        outcome
    }

    /// Negative case: the edit must classify COLD and the verdict must name
    /// the precise reason. The file is restored afterwards so later steps
    /// see the pre-test state.
    async fn expect_cold(
        &mut self,
        name: &str,
        relative: &str,
        mutated: &str,
        reason_fragment: &str,
        restore: &str,
    ) {
        self.log(format!("— {name}"));
        if let Err(error) = self.write_tracked(relative, mutated).await {
            self.fail(name, error);
            return;
        }
        let verdict = self.hot_reload(Some(vec![relative.to_string()])).await;
        let text = match &verdict {
            Ok(summary) => summary.clone(),
            Err(error) => error.clone(),
        };
        if text.contains("Hot reload not applicable") && text.contains(reason_fragment) {
            self.pass(name, format!("cold with reason \"{reason_fragment}\""));
        } else {
            self.fail(
                name,
                format!(
                    "expected a cold verdict with reason \"{reason_fragment}\", got: {}",
                    squash(&text)
                ),
            );
        }
        if let Err(error) = self.write_tracked(relative, restore).await {
            self.fail(name, format!("restore after {name} failed: {error}"));
        }
    }

    async fn wait_for_play_state(&self, playing: bool, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        loop {
            let (connected, status, _) =
                crate::unity_bridge::query_unity_status(&self.project).await;
            if connected && crate::unity_bridge::is_play_mode_status(status) == playing {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "editor did not reach {} within {}s",
                    if playing { "play mode" } else { "edit mode" },
                    timeout.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    // ── phases ───────────────────────────────────────────────────────

    async fn initialize_corpus(&mut self) -> Result<(), String> {
        self.log("Phase 1/7 — initializing the test corpus (edit mode)");

        // Inside an edit session the imports queue instead of firing one by
        // one; the recompile below releases the session, flushes the queue
        // and compiles in a single deterministic pass.
        if let Err(error) =
            crate::unity_bridge::begin_edit_session(&self.project, "hotreload-selftest").await
        {
            self.log(format!(
                "note: edit session not started ({error}); continuing"
            ));
        }

        self.write_tracked(SUBJECT_FILE, SUBJECT_BASELINE).await?;
        self.write_tracked(MESSAGE_FILE, MESSAGE_BASELINE).await?;
        self.write_tracked(HELPER_FILE, HELPER_BASELINE).await?;
        self.write_tracked(MODE_FILE, MODE_BASELINE).await?;
        self.write_tracked(STRUCT_FILE, STRUCT_BASELINE).await?;
        self.write_tracked(CTOR_FILE, CTOR_BASELINE).await?;
        self.write_tracked(IFACE_FILE, IFACE_BASELINE).await?;
        self.write_tracked(NEG_FILE, NEG_BASELINE).await?;
        // B6: the two-part partial corpus (sibling discovery + layout merge).
        self.write_tracked(PARTIAL_A_FILE, PARTIAL_A_BASELINE)
            .await?;
        self.write_tracked(PARTIAL_B_FILE, PARTIAL_B_BASELINE)
            .await?;
        // Extra cold-classification corpus for the negative phase. RECORD_FILE
        // is intentionally NOT written here — its test creates it fresh so the
        // record syntax never reaches a real compile.
        self.write_tracked(COLD_FILE, COLD_BASELINE).await?;
        self.write_tracked(USE_CONST_FILE, USE_CONST_BASELINE)
            .await?;
        self.write_tracked(USE_STATIC_FILE, USE_STATIC_BASELINE)
            .await?;
        self.write_tracked(USE_GENERIC_FILE, USE_GENERIC_BASELINE)
            .await?;
        self.write_tracked(USE_EXPLICIT_FILE, USE_EXPLICIT_BASELINE)
            .await?;
        self.write_tracked(USE_FINALIZER_FILE, USE_FINALIZER_BASELINE)
            .await?;
        self.write_tracked(PARTIAL_COLD_FILE, PARTIAL_COLD_BASELINE)
            .await?;
        self.write_tracked(PARTIAL_COUNT_FILE, PARTIAL_COUNT_BASELINE)
            .await?;
        // Adversarial syntax edge cases (A1–A4).
        self.write_tracked(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)
            .await?;
        self.write_tracked(INLINE_CALLEE_FILE, INLINE_CALLEE_BASELINE)
            .await?;
        self.write_tracked(INLINE_CALLER_DIRECT_FILE, INLINE_CALLER_DIRECT_BASELINE)
            .await?;
        self.write_tracked(INLINE_CALLER_NESTED_FILE, INLINE_CALLER_NESTED_BASELINE)
            .await?;
        self.write_tracked(INLINE_CALLER_OVERLOAD_FILE, INLINE_CALLER_OVERLOAD_BASELINE)
            .await?;
        self.write_tracked(INLINE_CALLER_LAMBDA_FILE, INLINE_CALLER_LAMBDA_BASELINE)
            .await?;
        self.write_tracked(INLINE_CALLER_BRANCH_FILE, INLINE_CALLER_BRANCH_BASELINE)
            .await?;
        self.write_tracked(INLINE_CALLER_ARRAY_FILE, INLINE_CALLER_ARRAY_BASELINE)
            .await?;
        // Multi-file inline caller-refresh probes (R12–R14). The lib-side file
        // (LIB_INLINE_FILE) is written with the other Lib/ source below.
        self.write_tracked(INLINE_CHAIN_LEAF_FILE, INLINE_CHAIN_LEAF_BASELINE)
            .await?;
        self.write_tracked(INLINE_CHAIN_MID_FILE, INLINE_CHAIN_MID_BASELINE)
            .await?;
        self.write_tracked(INLINE_CHAIN_TOP_FILE, INLINE_CHAIN_TOP_BASELINE)
            .await?;
        self.write_tracked(LIB_INLINE_CALLER_FILE, LIB_INLINE_CALLER_BASELINE)
            .await?;
        self.write_tracked(INST_INLINEE_FILE, INST_INLINEE_BASELINE)
            .await?;
        self.write_tracked(INST_INLINE_CALLER_FILE, INST_INLINE_CALLER_BASELINE)
            .await?;
        // Editor/ folder → Assembly-CSharp-Editor: edit-mode hot reload of
        // editor tooling is its own phase.
        self.write_tracked(EDITOR_FILE, EDITOR_BASELINE).await?;
        // Lib/ folder + .asmdef → the separate LocusSelfTestLib assembly
        // (B3 cross-asmdef coverage). The asmdef is plain JSON — tracking
        // no-ops for non-.cs files, which is exactly right: asmdef changes
        // are never hot-reload inputs.
        self.write_tracked(LIB_ASMDEF_FILE, LIB_ASMDEF_BASELINE)
            .await?;
        self.write_tracked(LIB_FILE, LIB_BASELINE).await?;
        self.write_tracked(LIB_INLINE_FILE, LIB_INLINE_BASELINE)
            .await?;

        // The corpus was written behind Unity's back: the AssetDatabase must
        // import it or the compile would not include the new files at all
        // (the folder goes first so children import into an existing parent).
        let mut imports: Vec<String> = vec![
            TEST_DIR.to_string(),
            EDITOR_DIR.to_string(),
            LIB_DIR.to_string(),
        ];
        imports.extend(
            ALL_FILES
                .iter()
                .filter(|relative| {
                    // Created later, mid-play, so their baselines are empty and
                    // the sidecar sees the full file as hot-added surface.
                    // RECORD_FILE is created fresh in its own negative test and
                    // never baseline-compiled (record syntax would otherwise
                    // need a C# 9 editor just to import the corpus).
                    **relative != FRESH_FILE
                        && **relative != HOT_ADDED_BEHAVIOUR_FILE
                        && **relative != RECORD_FILE
                })
                .map(|relative| relative.to_string()),
        );
        crate::unity_bridge::import_assets(&self.project, &imports)
            .await
            .map_err(|e| format!("queueing corpus imports failed: {e}"))?;

        self.log("Baseline recompile (this includes a domain reload)...");
        crate::unity_bridge::recompile_and_wait(&self.project)
            .await
            .map_err(|e| format!("baseline recompile failed: {e}"))?;

        // Hard gate: the corpus types must actually be in the loaded
        // Assembly-CSharp — a clear diagnostic beats 40 downstream failures.
        let check = self
            .execute(
                "return System.Type.GetType(\"LocusSelfTestSubject, Assembly-CSharp\") != null \
                 ? \"corpus-ok\" : \"corpus-missing\";",
            )
            .await
            .map_err(|e| format!("corpus verification snippet failed: {e}"))?;
        if !check.contains("corpus-ok") {
            return Err(
                "corpus did not compile into Assembly-CSharp (the baseline recompile succeeded \
                 but the test scripts are not in the loaded domain) — check the Unity console \
                 for import/compile errors"
                    .to_string(),
            );
        }

        // Second gate (B3): the .asmdef must have produced its OWN assembly.
        // If the lib type compiled but landed in Assembly-CSharp instead,
        // every cross-asmdef case would silently degenerate to same-assembly
        // coverage — fail loudly here with the precise shape.
        let lib_check = self
            .execute(
                "var t = System.Type.GetType(\"LocusSelfTestLibType, LocusSelfTestLib\");\n\
                 if (t != null) return \"lib-ok\";\n\
                 var stray = System.Type.GetType(\"LocusSelfTestLibType, Assembly-CSharp\");\n\
                 return stray != null ? \"lib-in-assembly-csharp\" : \"lib-missing\";",
            )
            .await
            .map_err(|e| format!("lib corpus verification snippet failed: {e}"))?;
        if !lib_check.contains("lib-ok") {
            return Err(format!(
                "the LocusSelfTestLib asmdef did not produce its own assembly (probe says: {}) — \
                 the .asmdef import may not have taken effect before the baseline compile; check \
                 the Unity console and Library/ScriptAssemblies for LocusSelfTestLib.dll",
                squash(&lib_check)
            ));
        }
        self.log("Baseline compiled; corpus is the loaded truth (lib assembly present).");
        Ok(())
    }

    /// Edit-mode hot reload, BEFORE play mode: editor tooling (custom
    /// editors, menu commands) lives in Assembly-CSharp-Editor and detours
    /// exactly like player code. The patch is deliberately NOT reverted on
    /// success: Phase 3 carries it across the play transition and asserts
    /// the mode-matched outcome (B5 — it survives when Reload Domain is
    /// disabled, it dies with the classic play-enter reload), then reverts
    /// the file before the first play-phase batch so the cumulative ledgers
    /// never see it. On any E01 failure the revert happens here and Phase 3
    /// skips the carry-over assertion.
    async fn run_editmode_tests(&mut self) {
        self.log("Phase 2/7 — edit-mode hot reload (editor assembly, no play mode)");
        let name = "E01 editor-assembly body edit (edit mode)";
        self.log(format!("— {name}"));
        let edited = EDITOR_BASELINE.replace("return 1;", "return 8118;");
        match self.write_tracked(EDITOR_FILE, &edited).await {
            Ok(()) => match self.hot_reload(Some(vec![EDITOR_FILE.to_string()])).await {
                Ok(summary) if summary.contains("Hot reload not applicable") => {
                    self.fail(
                        name,
                        format!("unexpected cold verdict: {}", squash(&summary)),
                    );
                }
                // STRICT: assert the editor method is live immediately even in
                // Release (its detour holds at non-inlined sites and the refresh
                // un-inlines any caller). On success editor_patch_live → E02 asserts
                // the carry-over; a stale read fails loudly instead of being skipped.
                Ok(_) => match self
                    .execute("return LocusSelfTestEditorTool.Reading();")
                    .await
                {
                    Ok(output) if output.contains("8118") => {
                        self.pass(name, "observed 8118");
                        self.editor_patch_live = true;
                    }
                    Ok(output) => {
                        self.fail(name, format!("expected '8118' in output, got: {output}"));
                    }
                    Err(error) => self.fail(name, format!("snippet failed: {error}")),
                },
                Err(error) => self.fail(name, squash(&error)),
            },
            Err(error) => self.fail(name, error),
        }
        if self.editor_patch_live {
            self.log("  editor patch held live across the play transition (B5 — E02 asserts, then reverts)");
        } else if let Err(error) = self.write_tracked(EDITOR_FILE, EDITOR_BASELINE).await {
            self.log(format!("  editor corpus revert failed: {error}"));
        }
    }

    async fn probe_message_driver_capabilities(&mut self) {
        let snippet = "bool HasUnityType(string fullName)\n\
                       {\n\
                           return System.AppDomain.CurrentDomain.GetAssemblies()\n\
                               .Any(a => a.GetType(fullName, false) != null);\n\
                       }\n\
                       var physics3d = HasUnityType(\"UnityEngine.Collider\")\n\
                           && HasUnityType(\"UnityEngine.Collision\")\n\
                           && HasUnityType(\"UnityEngine.BoxCollider\");\n\
                       var physics2d = HasUnityType(\"UnityEngine.Collider2D\")\n\
                           && HasUnityType(\"UnityEngine.Collision2D\")\n\
                           && HasUnityType(\"UnityEngine.BoxCollider2D\");\n\
                       return \"physics3d=\" + (physics3d ? \"true\" : \"false\")\n\
                           + \" physics2d=\" + (physics2d ? \"true\" : \"false\");";
        match self.execute(snippet).await {
            Ok(output) => {
                let lower = output.to_ascii_lowercase();
                self.message_driver_capabilities = MessageDriverCapabilities {
                    physics3d: lower.contains("physics3d=true"),
                    physics2d: lower.contains("physics2d=true"),
                };
                self.log(format!(
                    "Unity message proxy modules: {}",
                    self.message_driver_capabilities.label()
                ));
            }
            Err(error) => {
                self.message_driver_capabilities = MessageDriverCapabilities::default();
                self.fail("message driver capability probe", error);
            }
        }
    }

    /// B5 — probe the Enter Play Mode Options BEFORE the play transition and
    /// log which matrix branch this run exercises. Best-effort: on a probe
    /// failure the mode-aware assertions assume the classic reload path
    /// (`enterPlayModeOptions` exists since 2019.3, well below the plugin
    /// floor, so a failure here means the snippet machinery is broken and
    /// E01 has already failed loudly).
    async fn probe_play_mode_options(&mut self) {
        let snippet = "var enabled = UnityEditor.EditorSettings.enterPlayModeOptionsEnabled;\n\
                       var options = UnityEditor.EditorSettings.enterPlayModeOptions;\n\
                       bool domainReload = !enabled || !options.HasFlag(UnityEditor.EnterPlayModeOptions.DisableDomainReload);\n\
                       bool sceneReload = !enabled || !options.HasFlag(UnityEditor.EnterPlayModeOptions.DisableSceneReload);\n\
                       return \"epmo-enabled=\" + enabled + \" domain-reload=\" + (domainReload ? \"on\" : \"off\") + \" scene-reload=\" + (sceneReload ? \"on\" : \"off\");";
        match self.execute(snippet).await {
            Ok(output) => {
                let reload = !output.contains("domain-reload=off");
                self.epmo_enabled = Some(output.contains("epmo-enabled=True"));
                self.domain_reload_on_play = Some(reload);
                self.log(format!(
                    "Enter Play Mode Options: domain reload {} on play enter ({})",
                    if reload { "ENABLED" } else { "DISABLED" },
                    squash(&output)
                ));
            }
            Err(error) => {
                self.log(format!(
                    "Enter Play Mode Options probe failed ({error}); assuming the classic domain-reload path"
                ));
            }
        }
    }

    async fn enter_play_mode(&mut self) -> Result<(), String> {
        self.log("Phase 3/7 — entering play mode");
        self.probe_play_mode_options().await;
        self.execute("UnityEditor.EditorApplication.EnterPlaymode(); return \"entering\";")
            .await
            .map_err(|e| format!("EnterPlaymode failed: {e}"))?;
        self.wait_for_play_state(true, Duration::from_secs(90))
            .await?;
        // The play transition settles behind the status flip (classic mode:
        // the play-mode domain reload; no-reload mode: scene setup only).
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.wait_for_semantic_unity_api_ready(
            "play-mode transition",
            UNITY_SEMANTIC_READY_TIMEOUT,
        )
        .await?;

        self.execute(&spawn_test_objects_code(self.message_driver_capabilities))
            .await
            .map_err(|e| format!("spawning the test component failed: {e}"))?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.log("Test component is live in play mode.");

        // B5/E02 — the E01 editor patch crossed the play transition. With
        // Enter Play Mode Options ENABLED (either sub-mode: DisableSceneReload
        // only, or also DisableDomainReload) Unity takes the fast play-enter
        // path and the editor-assembly detour stays live — MEASURED on
        // 2022.3.47f1: the patch survives even when the DisableDomainReload
        // flag is not set, because already-loaded editor assemblies are not
        // re-JITed on play-enter, so the NativeDetour on the native entry
        // point persists. (A full domain reload — EPMO disabled entirely —
        // would drop it; that mode is outside B5's no-reload focus.) So with
        // EPMO enabled the edit-mode patch must keep reading 8118 in play.
        // Either way the file reverts right here, BEFORE the first Phase-4
        // batch: disk returns to the recorded baseline, so hot_reload(None)
        // skips the editor file from then on.
        if self.editor_patch_live {
            if self.epmo_enabled == Some(false) {
                // Classic full reload (EPMO master switch off): the play-enter
                // reload re-JITs the loaded baseline (the 8118 text was never
                // imported, so ScriptAssemblies on disk still says 1).
                self.expect_output(
                    "E02 edit-mode patch dies with the play-enter domain reload",
                    "return LocusSelfTestEditorTool.Reading() == 1 ? \"editor-tool-baseline\" : \"editor-tool-patched\";",
                    "editor-tool-baseline",
                )
                .await;
            } else {
                self.expect_output(
                    "E02 edit-mode patch survives play-enter (Enter Play Mode Options enabled)",
                    "return LocusSelfTestEditorTool.Reading();",
                    "8118",
                )
                .await;
                let active = coordinator::project_active_patches(&self.project).await;
                self.log(format!(
                    "  coordinator continuity: {active} active patch(es) carried across play-enter ({})",
                    if self.domain_reload_on_play == Some(false) {
                        "domain reload disabled"
                    } else {
                        "fast enter-play, editor detour persists"
                    }
                ));
            }
            if let Err(error) = self.write_tracked(EDITOR_FILE, EDITOR_BASELINE).await {
                self.log(format!("  editor corpus revert failed: {error}"));
            }
        }
        Ok(())
    }

    async fn run_positive_tests(&mut self, subject: &mut String, helper: &mut String) {
        self.log("Phase 4/7 — hot-reloading every supported change shape");

        // P00 — the baseline really is what we wrote.
        self.expect_output(
            "P00 baseline sanity",
            "return LocusSelfTestSubject.Instance.Mult();",
            "1002",
        )
        .await;

        // P00b — a delegate captured BEFORE the edit follows the detour:
        // the redirect is method-level, so in-flight delegates (and
        // UnityEvents bound to the same method) pick up new behavior.
        let name = "P00b in-flight delegate follows detour";
        self.log(format!("— {name}"));
        match self
            .execute(
                "LocusSelfTestSubject.Captured = LocusSelfTestSubject.Instance.Snare;\nreturn \"captured\";",
            )
            .await
        {
            Ok(_) => {
                if self
                    .step_file(name, SUBJECT_FILE, subject, |s| {
                        swap(s, "public int Snare() { return 5; }", "public int Snare() { return 7667; }")
                    })
                    .await
                    .is_some()
                {
                    self.expect_output(name, "return LocusSelfTestSubject.Captured();", "7667").await;
                }
            }
            Err(error) => self.fail(name, format!("capture snippet failed: {error}")),
        }

        // P01 — method body edit.
        if self
            .step_file("P01 method body edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Mult() { return 1002; }",
                    "public int Mult() { return 4221; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P01 method body edit",
                "return LocusSelfTestSubject.Instance.Mult();",
                "4221",
            )
            .await;
            self.expect_release_immediate_output(
                "R01 release strict method body edit",
                "return LocusSelfTestSubject.Instance.Mult();",
                "4221",
            )
            .await;
        }

        // P01b — Unity message BODY edit (the engine drives the detoured
        // Update every frame; D01 later deletes it).
        if self
            .step_file("P01b Unity message body edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "void Update() { _ticks += Step(); }",
                    "void Update() { _ticks += Step(); UpdateBeats += 1; }",
                )
            })
            .await
            .is_some()
        {
            tokio::time::sleep(Duration::from_millis(700)).await;
            self.expect_output(
                "P01b Unity message body edit",
                "return LocusSelfTestSubject.UpdateBeats > 0 ? \"beating\" : \"still\";",
                "beating",
            )
            .await;
            self.expect_release_immediate_output(
                "R02 release strict Unity message body edit",
                "return LocusSelfTestSubject.UpdateBeats > 0 ? \"beating\" : \"still\";",
                "beating",
            )
            .await;
        }

        // P02 — async↔sync conversion.
        if self
            .step_file("P02 async<->sync conversion", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public Task<int> Pulse() { return Task.FromResult(2001); }",
                    "public async Task<int> Pulse() { await Task.Yield(); return 2002; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P02 async<->sync conversion",
                "return await LocusSelfTestSubject.Instance.Pulse();",
                "2002",
            )
            .await;
            self.expect_release_immediate_output(
                "R03 release strict async conversion",
                "return await LocusSelfTestSubject.Instance.Pulse();",
                "2002",
            )
            .await;
        }

        // P02b — async method BODY edit (distinct from the conversion).
        if self
            .step_file("P02b async body edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public async Task<int> Pulse() { await Task.Yield(); return 2002; }",
                    "public async Task<int> Pulse() { await Task.Yield(); return 2112; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P02b async body edit",
                "return await LocusSelfTestSubject.Instance.Pulse();",
                "2112",
            )
            .await;
        }

        // P03 — added methods (shim→shim chain) over PUBLIC original
        // surface (P34 covers the caps-gated private-surface form; N12'
        // keeps the signature-level cold verdict).
        if self
            .step_file("P03 added methods (shim chain)", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Step() { return 1; }\n",
                    "    public int Step() { return 1; }\n    public int Boost() { return BoostCore() + 7000; }\n    public int BoostCore() { return 707; }\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return Boost(); }")
            })
            .await
            .is_some()
        {
            self.expect_output("P03 added methods (shim chain)", "return LocusSelfTestSubject.Instance.Probe();", "7707").await;
            self.expect_release_immediate_output(
                "R04 release strict added shim chain",
                "return LocusSelfTestSubject.Instance.Probe();",
                "7707",
            )
            .await;
        }

        // P04 — parameter-list change with the call site OUTSIDE the batch:
        // must come back cold naming the exact caller file, then go hot once
        // the caller joins the batch.
        let name = "P04a uncovered caller named";
        self.log(format!("— {name}"));
        let helper_snapshot = helper.clone();
        let subject_snapshot = subject.clone();
        let p04 = swap(
            helper,
            "public static int Twice(int a) { return a * 2; }",
            "public static int Twice(int a, int extra) { return a * 2 + extra; }",
        );
        match p04 {
            Ok(()) => match self.write_tracked(HELPER_FILE, helper).await {
                Ok(()) => {
                    let verdict = self.hot_reload(Some(vec![HELPER_FILE.to_string()])).await;
                    let text = match &verdict {
                        Ok(summary) => summary.clone(),
                        Err(error) => error.clone(),
                    };
                    if text.contains("LocusSelfTestSubject.cs") {
                        self.pass(name, "cold verdict names the caller file");
                    } else {
                        self.fail(
                            name,
                            format!(
                                "expected the caller file in the verdict, got: {}",
                                squash(&text)
                            ),
                        );
                    }

                    // P04b — same change goes hot with the caller co-edited.
                    let p04b = swap(
                        subject,
                        "public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a); }",
                        "public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a, 100); }",
                    );
                    let applied = match p04b {
                        Ok(()) => {
                            self.log("— P04b covered batch goes hot");
                            self.apply_texts(
                                "P04b covered batch goes hot",
                                &[(SUBJECT_FILE, subject.as_str())],
                                &[
                                    (SUBJECT_FILE, subject_snapshot.as_str()),
                                    (HELPER_FILE, helper_snapshot.as_str()),
                                ],
                            )
                            .await
                        }
                        Err(error) => {
                            self.fail("P04b covered batch goes hot", error);
                            None
                        }
                    };
                    if applied.is_some() {
                        self.expect_output(
                            "P04b covered batch goes hot",
                            "return LocusSelfTestSubject.Instance.Sum(3);",
                            "109", // 3 + (3*2 + 100)
                        )
                        .await;
                    } else {
                        *subject = subject_snapshot;
                        *helper = helper_snapshot;
                    }
                }
                Err(error) => {
                    self.fail(name, error);
                    *helper = helper_snapshot;
                }
            },
            Err(error) => self.fail(name, error),
        }

        // P05 — instance field addition: a pre-existing instance reads
        // default(T); a NEW instance runs the initializer. The reading
        // member is KEPT (Probe), exercising the field-store rewrite.
        if self
            .step_file("P05 added instance field", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    private int _legacy = 3;\n",
                    "    private int _legacy = 3;\n    private int _bonus = 5050;\n",
                )?;
                swap_line(
                    s,
                    "    public int Probe()",
                    "    public int Probe() { return _bonus + 9090; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P05a existing instance reads default",
                "return LocusSelfTestSubject.Instance.Probe();",
                "9090", // default(0) + 9090
            )
            .await;
            self.expect_output(
                "P05b new instance runs the initializer",
                "var go = new UnityEngine.GameObject(\"LocusSelfTestFieldProbe\");\n\
                 var probe = go.AddComponent<LocusSelfTestSubject>();\n\
                 var value = probe.Probe();\n\
                 UnityEngine.Object.Destroy(go);\n\
                 return value;",
                "14140", // 5050 + 9090
            )
            .await;
        }

        // P05c — added field with a GENERIC argument type (the store is a
        // LocusFieldStore<List<int>>); a pre-existing instance reads
        // default(null) for reference types.
        if self
            .step_file("P05c generic-typed field added", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    private int _bonus = 5050;\n",
                    "    private int _bonus = 5050;\n    private System.Collections.Generic.List<int> _list = new System.Collections.Generic.List<int> { 41, 1 };\n",
                )?;
                swap_line(
                    s,
                    "    public int Probe()",
                    "    public int Probe() { return (_list == null ? 0 : _list.Count) + 4664; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P05c generic-typed field added",
                "return LocusSelfTestSubject.Instance.Probe();",
                "4664", // existing instance: null list + 4664
            )
            .await;
        }

        // P06 — added static field through the holder class.
        let p06_ok = self
            .step_file("P06 added static field", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public static int EvtCount;\n",
                    "    public static int EvtCount;\n    private static int s_total = 6600;\n",
                )?;
                swap_line(
                    s,
                    "    public int Probe()",
                    "    public int Probe() { s_total += 1; return s_total; }",
                )
            })
            .await
            .is_some();
        if p06_ok {
            self.expect_output(
                "P06 added static field",
                "return LocusSelfTestSubject.Instance.Probe();",
                "6601",
            )
            .await;
        }

        // P07 — using addition re-detours the whole file (M6).
        let p07_ok = self
            .step_file(
                "P07 using added (file rehook)",
                SUBJECT_FILE,
                subject,
                |s| {
                    swap(
                        s,
                        "using UnityEngine;",
                        "using UnityEngine;\nusing System.Text;",
                    )?;
                    swap(
                        s,
                        "public int Step() { return 1; }",
                        "public int Step() { return 8800 + new StringBuilder(\"ab\").Length; }",
                    )
                },
            )
            .await
            .is_some();
        if p07_ok {
            self.expect_output(
                "P07 using added (file rehook)",
                "return LocusSelfTestSubject.Instance.Step();",
                "8802",
            )
            .await;
        }

        // P07b — store-held STATIC state survives later patches: the holder
        // lives in the first batch's assembly and the whole-file rehook of
        // P07 re-detoured Probe without resetting it.
        if p06_ok && p07_ok {
            self.expect_output(
                "P07b store-held static persists across patches",
                "return LocusSelfTestSubject.Instance.Probe();",
                "6602", // second bump of the SAME s_total
            )
            .await;
        }

        // P08 — appended enum member materializes as a cast literal.
        let name = "P08 enum append";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let mode_v2 = MODE_BASELINE.replace(
            "public enum LocusSelfTestMode { A = 0, B = 1 }",
            "public enum LocusSelfTestMode { A = 0, B = 1, C = 7 }",
        );
        let p08 = swap(
            subject,
            "            case LocusSelfTestMode.B: return 22;\n",
            "            case LocusSelfTestMode.B: return 22;\n            case LocusSelfTestMode.C: return 3377;\n",
        );
        match p08 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (MODE_FILE, mode_v2.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (MODE_FILE, MODE_BASELINE),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.ModeValue((LocusSelfTestMode)7);",
                        "3377",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                }
            }
            Err(error) => self.fail(name, error),
        }

        // P09 — brand-new file with a brand-new type (TI-C visibility),
        // observed through Probe (the type is invisible to snippets).
        let name = "P09 new file with new type";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let fresh = "public static class LocusSelfTestFresh { public static int Ping() { return 4242; } }\n";
        let p09 = swap_line(
            subject,
            "    public int Probe()",
            "    public int Probe() { return LocusSelfTestFresh.Ping(); }",
        );
        match p09 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[(FRESH_FILE, fresh), (SUBJECT_FILE, subject.as_str())],
                        // Reverting a brand-new file means an empty stand-in;
                        // the deletion pass would tombstone it anyway, so
                        // keep the file with a harmless body on failure.
                        &[(SUBJECT_FILE, subject_snapshot.as_str())],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.Probe();",
                        "4242",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                }
            }
            Err(error) => self.fail(name, error),
        }

        // P10 — property getter body edit.
        if self
            .step_file("P10 property getter edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Gauge { get { return 17; } }",
                    "public int Gauge { get { return 7117; } }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P10 property getter edit",
                "return LocusSelfTestSubject.Instance.Gauge;",
                "7117",
            )
            .await;
        }

        // P10b — property SETTER body edit.
        if self
            .step_file("P10b property setter edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Stash { get { return _stash; } set { _stash = value; } }",
                    "public int Stash { get { return _stash; } set { _stash = value + 4880; } }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P10b property setter edit",
                "LocusSelfTestSubject.Instance.Stash = 8;\nreturn LocusSelfTestSubject.Instance.Stash;",
                "4888",
            )
            .await;
        }

        // P10c — expression-bodied member edit.
        if self
            .step_file(
                "P10c expression-bodied member edit",
                SUBJECT_FILE,
                subject,
                |s| swap(s, "public int Arrow => 12;", "public int Arrow => 7447;"),
            )
            .await
            .is_some()
        {
            self.expect_output(
                "P10c expression-bodied member edit",
                "return LocusSelfTestSubject.Instance.Arrow;",
                "7447",
            )
            .await;
        }

        // P11 — indexer body edit.
        if self
            .step_file("P11 indexer edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int this[int i] { get { return i; } }",
                    "public int this[int i] { get { return i + 5005; } }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P11 indexer edit",
                "return LocusSelfTestSubject.Instance[2];",
                "5007",
            )
            .await;
        }

        // P12 — event accessor body edit.
        if self
            .step_file("P12 event accessor edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public event System.Action Surge { add { EvtCount += 1; } remove { } }",
                    "public event System.Action Surge { add { EvtCount += 4400; } remove { } }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P12 event accessor edit",
                "LocusSelfTestSubject.Instance.Surge += () => { };\nreturn LocusSelfTestSubject.EvtCount;",
                "4400",
            )
            .await;
        }

        // P13 — lambda with a closure.
        if self
            .step_file("P13 lambda + closure edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Lambda() { int basis = 1; System.Func<int> f = () => basis + 1; return f(); }",
                    "public int Lambda() { int basis = 6060; System.Func<int> f = () => basis + 1; return f(); }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output("P13 lambda + closure edit", "return LocusSelfTestSubject.Instance.Lambda();", "6061").await;
        }

        // P13b — the edited lambda captures a NEW external variable (closure
        // display class fully rebuilds inside the patched body).
        if self
            .step_file("P13b lambda captures new variable", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Lambda() { int basis = 6060; System.Func<int> f = () => basis + 1; return f(); }",
                    "public int Lambda() { int basis = 6060; int extra = 1029; System.Func<int> f = () => basis + extra; return f(); }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output("P13b lambda captures new variable", "return LocusSelfTestSubject.Instance.Lambda();", "7089").await;
        }

        // P14 — local function body edit.
        if self
            .step_file("P14 local function edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Local() { int InnerFn() { return 1; } return InnerFn(); }",
                    "public int Local() { int InnerFn() { return 9119; } return InnerFn(); }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P14 local function edit",
                "return LocusSelfTestSubject.Instance.Local();",
                "9119",
            )
            .await;
        }

        // P15 — anonymous types inside an edited body.
        if self
            .step_file("P15 anonymous type edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Anon() { var a = new { V = 1 }; return a.V; }",
                    "public int Anon() { var a = new { V = 7997 }; return a.V; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P15 anonymous type edit",
                "return LocusSelfTestSubject.Instance.Anon();",
                "7997",
            )
            .await;
        }

        // P16 — pattern matching in an edited body (modern syntax).
        if self
            .step_file("P16 pattern matching edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Match(object k) { return k is int ? 1 : 0; }",
                    "public int Match(object k) { return k is int n ? n + 6770 : 0; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P16 pattern matching edit",
                "return LocusSelfTestSubject.Instance.Match(6);",
                "6776",
            )
            .await;
        }

        // P16b — edit inside an active #if block (defines parity with the
        // project's compilation).
        if self
            .step_file("P16b #if-block body edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "#if UNITY_EDITOR\n        return 1;",
                    "#if UNITY_EDITOR\n        return 8338;",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P16b #if-block body edit",
                "return LocusSelfTestSubject.Instance.Cond();",
                "8338",
            )
            .await;
        }

        // P17 — nested type member body edit.
        if self
            .step_file("P17 nested type member edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Nine() { return 1; }",
                    "public int Nine() { return 5665; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P17 nested type member edit",
                "return new LocusSelfTestSubject.Inner().Nine();",
                "5665",
            )
            .await;
        }

        // P17b — field added to a NESTED type (store chain naming +
        // constructor redirect of the nested type; the snippet constructs a
        // fresh instance so the initializer runs).
        if self
            .step_file("P17b nested type field added", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public class Inner\n    {\n",
                    "    public class Inner\n    {\n        public int W = 9;\n",
                )?;
                swap(
                    s,
                    "public int Nine() { return 5665; }",
                    "public int Nine() { return W + 5660; }",
                )
                .or_else(|_| {
                    swap(
                        s,
                        "public int Nine() { return 1; }",
                        "public int Nine() { return W + 5660; }",
                    )
                })
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P17b nested type field added",
                "return new LocusSelfTestSubject.Inner().Nine();",
                "5669", // 9 + 5660
            )
            .await;
        }

        // P17c — brand-new NESTED type inside an existing type, observed
        // through Probe.
        if self
            .step_file("P17c nested type added", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public class Inner\n    {\n",
                    "    public class Inner2 { public static int Forty() { return 4554; } }\n\n    public class Inner\n    {\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return Inner2.Forty(); }")
            })
            .await
            .is_some()
        {
            self.expect_output("P17c nested type added", "return LocusSelfTestSubject.Instance.Probe();", "4554").await;
        }

        // P18 — iterator (coroutine) body: NEW enumerations get the patch.
        if self
            .step_file("P18 coroutine body edit", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public System.Collections.IEnumerator Counting() { yield return 1; }",
                    "public System.Collections.IEnumerator Counting() { yield return 4334; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P18 coroutine body edit",
                "var e = LocusSelfTestSubject.Instance.Counting();\ne.MoveNext();\nreturn (int)e.Current;",
                "4334",
            )
            .await;
        }

        // P18b — iterator → plain method conversion (same signature, the
        // state machine disappears; the detour is method-level).
        if self
            .step_file("P18b iterator to plain conversion", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public System.Collections.IEnumerator Counting() { yield return 4334; }",
                    "public System.Collections.IEnumerator Counting() { return new int[] { 5225 }.GetEnumerator(); }",
                )
                .or_else(|_| {
                    swap(
                        s,
                        "public System.Collections.IEnumerator Counting() { yield return 1; }",
                        "public System.Collections.IEnumerator Counting() { return new int[] { 5225 }.GetEnumerator(); }",
                    )
                })
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P18b iterator to plain conversion",
                "var e = LocusSelfTestSubject.Instance.Counting();\ne.MoveNext();\nreturn (int)e.Current;",
                "5225",
            )
            .await;
        }

        // P19 — constructor BODY edit (non-generic class): new instances run
        // the new body.
        let mut ctor_ledger = CTOR_BASELINE.to_string();
        if self
            .step_file(
                "P19 constructor body edit",
                CTOR_FILE,
                &mut ctor_ledger,
                |s| {
                    swap(
                        s,
                        "public LocusSelfTestCtor() { Seed = 1; }",
                        "public LocusSelfTestCtor() { Seed = 5775; }",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "P19 constructor body edit",
                "return new LocusSelfTestCtor().Seed;",
                "5775",
            )
            .await;
        }

        // P20 — instance field initializer edit: existing instances keep
        // their value, new instances run the new initializer.
        if self
            .step_file("P20 field initializer edit", SUBJECT_FILE, subject, |s| {
                swap(s, "private int _seed = 40;", "private int _seed = 7557;")
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P20a existing instance keeps value",
                "return LocusSelfTestSubject.Instance.Seed();",
                "40",
            )
            .await;
            self.expect_output(
                "P20b new instance runs new initializer",
                "var go = new UnityEngine.GameObject(\"LocusSelfTestSeedProbe\");\n\
                 var probe = go.AddComponent<LocusSelfTestSubject>();\n\
                 var value = probe.Seed();\n\
                 UnityEngine.Object.Destroy(go);\n\
                 return value;",
                "7557",
            )
            .await;
        }

        // P21 — field deletion (last reference edited away in the batch).
        if self
            .step_file("P21 field deletion", SUBJECT_FILE, subject, |s| {
                swap(s, "    private int _legacy = 3;\n", "")?;
                swap(
                    s,
                    "public int Legacy() { return _legacy; }",
                    "public int Legacy() { return 8448; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P21 field deletion",
                "return LocusSelfTestSubject.Instance.Legacy();",
                "8448",
            )
            .await;
        }

        // P22 — method rename with the caller co-edited in the batch.
        let name = "P22 method rename (covered)";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let helper_snapshot = helper.clone();
        let p22 = (|| -> Result<(), String> {
            swap(
                helper,
                "public static int Renamed() { return 21; }",
                "public static int Thrice() { return 7227; }",
            )?;
            swap(
                subject,
                "public int CallRenamed() { return LocusSelfTestHelper.Renamed(); }",
                "public int CallRenamed() { return LocusSelfTestHelper.Thrice(); }",
            )
        })();
        match p22 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (HELPER_FILE, helper.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (HELPER_FILE, helper_snapshot.as_str()),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.CallRenamed();",
                        "7227",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                    *helper = helper_snapshot;
                }
            }
            Err(error) => self.fail(name, error),
        }

        // P23 — ref→out parameter modifier change, caller covered.
        let name = "P23 ref->out change (covered)";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let helper_snapshot = helper.clone();
        let p23 = (|| -> Result<(), String> {
            swap(
                helper,
                "public static void Bump(ref int v) { v += 1; }",
                "public static void Bump(out int v) { v = 9229; }",
            )?;
            swap(
                subject,
                "public int CallBump() { int v = 1; LocusSelfTestHelper.Bump(ref v); return v; }",
                "public int CallBump() { int v; LocusSelfTestHelper.Bump(out v); return v; }",
            )
        })();
        match p23 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (HELPER_FILE, helper.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (HELPER_FILE, helper_snapshot.as_str()),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.CallBump();",
                        "9229",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                    *helper = helper_snapshot;
                }
            }
            Err(error) => self.fail(name, error),
        }

        // P24 — instance→static flip (signature change, no outside callers),
        // observed through Probe.
        if self
            .step_file("P24 static keyword flip", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Flip() { return 3; }",
                    "public static int Flip() { return 6336; }",
                )?;
                swap_line(
                    s,
                    "    public int Probe()",
                    "    public int Probe() { return Flip(); }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P24 static keyword flip",
                "return LocusSelfTestSubject.Instance.Probe();",
                "6336",
            )
            .await;
        }

        // P25 — accessibility narrowing (public→private, no outside callers).
        if self
            .step_file("P25 accessibility narrowing", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Shrink() { return 2; }",
                    "private int Shrink() { return 4884; }",
                )?;
                swap_line(
                    s,
                    "    public int Probe()",
                    "    public int Probe() { return Shrink(); }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P25 accessibility narrowing",
                "return LocusSelfTestSubject.Instance.Probe();",
                "4884",
            )
            .await;
        }

        // P26 — static class method body edit.
        if self
            .step_file("P26 static class body edit", HELPER_FILE, helper, |s| {
                swap(
                    s,
                    "public static int Pick() { return 1; }",
                    "public static int Pick() { return 3113; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P26 static class body edit",
                "return LocusSelfTestHelper.Pick();",
                "3113",
            )
            .await;
        }

        // P27 — new type appended to an EXISTING file, observed via Probe.
        let name = "P27 new type in existing file";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let helper_snapshot = helper.clone();
        helper.push_str("\npublic class LocusSelfTestExtra\n{\n    public static int Nine() { return 9559; }\n}\n");
        let p27 = swap_line(
            subject,
            "    public int Probe()",
            "    public int Probe() { return LocusSelfTestExtra.Nine(); }",
        );
        match p27 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (HELPER_FILE, helper.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (HELPER_FILE, helper_snapshot.as_str()),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.Probe();",
                        "9559",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                    *helper = helper_snapshot;
                }
            }
            Err(error) => {
                self.fail(name, error);
                *helper = helper_snapshot;
            }
        }

        // P27b — extension method ADDED to an existing static class; the
        // extension-form call site in a kept member materializes as a
        // direct shim call.
        let name = "P27b extension method added";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let helper_snapshot = helper.clone();
        let p27b = (|| -> Result<(), String> {
            swap(
                helper,
                "    public static int Pick() { return 3113; }\n",
                "    public static int Pick() { return 3113; }\n    public static int Tripled(this int v) { return v * 3; }\n",
            )
            .or_else(|_| {
                swap(
                    helper,
                    "    public static int Pick() { return 1; }\n",
                    "    public static int Pick() { return 1; }\n    public static int Tripled(this int v) { return v * 3; }\n",
                )
            })?;
            swap_line(
                subject,
                "    public int Probe()",
                "    public int Probe() { return 1500.Tripled() + 12; }",
            )
        })();
        match p27b {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (HELPER_FILE, helper.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (HELPER_FILE, helper_snapshot.as_str()),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.Probe();",
                        "4512",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                    *helper = helper_snapshot;
                }
            }
            Err(error) => {
                self.fail(name, error);
                *subject = subject_snapshot;
                *helper = helper_snapshot;
            }
        }

        // P27c — the SAME extension call site survives the NEXT batch: the
        // re-sent shim source and the accepted patch image both provide
        // Tripled to extension lookup, and the call site must keep
        // materializing as a direct shim call (a regression here is a
        // CS0121 self-ambiguity). The body tweak proves the re-edited shim
        // is the live one.
        let name = "P27c extension re-batch (image in scope)";
        self.log(format!("— {name}"));
        let helper_snapshot = helper.clone();
        if swap(
            helper,
            "public static int Tripled(this int v) { return v * 3; }",
            "public static int Tripled(this int v) { return v * 3 + 1; }",
        )
        .is_ok()
        {
            let applied = self
                .apply_texts(
                    name,
                    &[(HELPER_FILE, helper.as_str())],
                    &[(HELPER_FILE, helper_snapshot.as_str())],
                )
                .await;
            if applied.is_some() {
                self.expect_output(
                    name,
                    "return LocusSelfTestSubject.Instance.Probe();",
                    "4513", // 1500 * 3 + 1 + 12
                )
                .await;
            } else {
                *helper = helper_snapshot;
            }
        } else {
            self.log("  (P27b did not land; skipping P27c)");
        }

        // P28 — interface-IMPLEMENTATION body edit (the interface itself is
        // untouched; dispatch through the interface sees the patch).
        let mut iface_ledger = IFACE_BASELINE.to_string();
        if self
            .step_file(
                "P28 interface impl body edit",
                IFACE_FILE,
                &mut iface_ledger,
                |s| {
                    swap(
                        s,
                        "public int Plan() { return 1; }",
                        "public int Plan() { return 6996; }",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "P28 interface impl body edit",
                "ILocusSelfTestContract c = new LocusSelfTestContractImpl();\nreturn c.Plan();",
                "6996",
            )
            .await;
        }

        // P29 — struct method body edit.
        let mut struct_ledger = STRUCT_BASELINE.to_string();
        if self
            .step_file(
                "P29 struct method body edit",
                STRUCT_FILE,
                &mut struct_ledger,
                |s| {
                    swap(
                        s,
                        "public int Get() { return 1; }",
                        "public int Get() { return 6446; }",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "P29 struct method body edit",
                "return new LocusSelfTestStruct().Get();",
                "6446",
            )
            .await;
        }

        // P30 — operator body edit (the patch copy renames the self-typed
        // parameters; unchanged operators strip from the copy).
        if self
            .step_file(
                "P30 operator body edit",
                STRUCT_FILE,
                &mut struct_ledger,
                |s| {
                    swap(
                        s,
                        "r.Value = a.Value + b.Value;",
                        "r.Value = a.Value + b.Value + 7337;",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "P30 operator body edit",
                "var a = new LocusSelfTestStruct();\na.Value = 1;\nvar b = new LocusSelfTestStruct();\nb.Value = 1;\nreturn (a + b).Value;",
                "7339",
            )
            .await;
        }

        // P31 — conversion FROM the declaring type (conversions TO it stay
        // cold — N14 asserts that verdict).
        if self
            .step_file("P31 conversion (from) body edit", STRUCT_FILE, &mut struct_ledger, |s| {
                swap(
                    s,
                    "public static implicit operator int(LocusSelfTestStruct s) { return s.Value; }",
                    "public static implicit operator int(LocusSelfTestStruct s) { return s.Value + 5885; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P31 conversion (from) body edit",
                "var s = new LocusSelfTestStruct();\ns.Value = 4;\nint x = s;\nreturn x;",
                "5889",
            )
            .await;
        }

        // P32 — five files in ONE batch (subject, helper, struct, ctor,
        // iface): batch binding, per-file rewrites and the single combined
        // patch all hold together at width.
        let name = "P32 five-file batch";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let helper_snapshot = helper.clone();
        let struct_snapshot = struct_ledger.clone();
        let ctor_snapshot = ctor_ledger.clone();
        let iface_snapshot = iface_ledger.clone();
        // Anchors tolerate any of P10/P26/P29/P19/P28 having failed (their
        // edits revert, so the baseline form is the fallback).
        let p32 = (|| -> Result<(), String> {
            swap(
                subject,
                "public int Gauge { get { return 7117; } }",
                "public int Gauge { get { return 7118; } }",
            )
            .or_else(|_| {
                swap(
                    subject,
                    "public int Gauge { get { return 17; } }",
                    "public int Gauge { get { return 7118; } }",
                )
            })?;
            swap(
                helper,
                "public static int Pick() { return 3113; }",
                "public static int Pick() { return 3114; }",
            )
            .or_else(|_| {
                swap(
                    helper,
                    "public static int Pick() { return 1; }",
                    "public static int Pick() { return 3114; }",
                )
            })?;
            swap(
                &mut struct_ledger,
                "public int Get() { return 6446; }",
                "public int Get() { return 6447; }",
            )
            .or_else(|_| {
                swap(
                    &mut struct_ledger,
                    "public int Get() { return 1; }",
                    "public int Get() { return 6447; }",
                )
            })?;
            swap(
                &mut ctor_ledger,
                "public LocusSelfTestCtor() { Seed = 5775; }",
                "public LocusSelfTestCtor() { Seed = 5776; }",
            )
            .or_else(|_| {
                swap(
                    &mut ctor_ledger,
                    "public LocusSelfTestCtor() { Seed = 1; }",
                    "public LocusSelfTestCtor() { Seed = 5776; }",
                )
            })?;
            swap(
                &mut iface_ledger,
                "public int Plan() { return 6996; }",
                "public int Plan() { return 6997; }",
            )
            .or_else(|_| {
                swap(
                    &mut iface_ledger,
                    "public int Plan() { return 1; }",
                    "public int Plan() { return 6997; }",
                )
            })
        })();
        match p32 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (SUBJECT_FILE, subject.as_str()),
                            (HELPER_FILE, helper.as_str()),
                            (STRUCT_FILE, struct_ledger.as_str()),
                            (CTOR_FILE, ctor_ledger.as_str()),
                            (IFACE_FILE, iface_ledger.as_str()),
                        ],
                        &[
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                            (HELPER_FILE, helper_snapshot.as_str()),
                            (STRUCT_FILE, struct_snapshot.as_str()),
                            (CTOR_FILE, ctor_snapshot.as_str()),
                            (IFACE_FILE, iface_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        "P32a five-file batch (subject)",
                        "return LocusSelfTestSubject.Instance.Gauge;",
                        "7118",
                    )
                    .await;
                    self.expect_output(
                        "P32b five-file batch (interface impl)",
                        "ILocusSelfTestContract c = new LocusSelfTestContractImpl();\nreturn c.Plan();",
                        "6997",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                    *helper = helper_snapshot;
                    struct_ledger = struct_snapshot;
                    ctor_ledger = ctor_snapshot;
                    iface_ledger = iface_snapshot;
                }
            }
            Err(error) => {
                self.fail(name, error);
                *subject = subject_snapshot;
                *helper = helper_snapshot;
                struct_ledger = struct_snapshot;
                ctor_ledger = ctor_snapshot;
                iface_ledger = iface_snapshot;
            }
        }

        // P33 — generic method bodies via remove+add shims (B1): Echo<T>
        // (generic METHOD) and Val (method in a generic TYPE) change bodies;
        // the compiled callers Relay/RelayVal in SUBJECT stay UNTOUCHED —
        // the file joins the batch through an unrelated Spare edit and the
        // kept callers must re-detour for the shims to take effect.
        let name = "P33 generic body via shim";
        self.log(format!("— {name}"));
        // The NEG ledger lives on past P33: P37 builds on whatever text is
        // actually on disk (re-sending a baseline-derived text would also
        // revert Echo/Val — generic bodies whose callers are outside that
        // batch — and fail closed on the M3 caller scan).
        let mut neg_ledger = NEG_BASELINE.to_string();
        let subject_snapshot = subject.clone();
        let p33 = (|| -> Result<(), String> {
            swap(
                &mut neg_ledger,
                "public T Echo<T>(T value) { return value; }",
                "public T Echo<T>(T value) { return (T)(object)(((int)(object)value) + 7000); }",
            )?;
            swap(
                &mut neg_ledger,
                "public int Val() { return 1; }",
                "public int Val() { return 4334; }",
            )?;
            swap(
                subject,
                "public int Spare() { return 1; }",
                "public int Spare() { return 2; }",
            )
        })();
        match p33 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (NEG_FILE, neg_ledger.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (NEG_FILE, NEG_BASELINE),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        "P33a generic method body (kept caller)",
                        "return LocusSelfTestSubject.Instance.Relay();",
                        "7021", // (20 + 7000) + 1
                    )
                    .await;
                    self.expect_output(
                        "P33b generic type method body (kept caller)",
                        "return LocusSelfTestSubject.Instance.RelayVal();",
                        "4334",
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                    neg_ledger = NEG_BASELINE.to_string();
                }
            }
            Err(error) => {
                self.fail(name, error);
                *subject = subject_snapshot;
                neg_ledger = NEG_BASELINE.to_string();
            }
        }

        // P34 — added member touching PRIVATE state (C2′a): with the C0
        // probe matrix green on this editor, the added member's shim reads a
        // private instance field, calls a private method and reads a private
        // static of the ORIGINAL type through IgnoresAccessChecksTo — the
        // real-machine arbiter that IACT holds for the actual corpus
        // assembly, not just the synthetic probe target.
        if self
            .step_file("P34 added member touches private state", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    public int Vault() { return _seed + SecretCore() + s_secret; }\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return Vault(); }")
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P34 added member touches private state",
                "return LocusSelfTestSubject.Instance.Probe();",
                "7640", // live instance _seed(40) + SecretCore(7000) + s_secret(600)
            )
            .await;
        }

        // P35 — compiler-generated SUB-METHODS of added members carry the
        // non-public access (C2′b): the violating IL lives in the async
        // state machine's MoveNext, the lambda's display-class method and
        // the iterator's MoveNext — nested types of the shim class, not the
        // shim method itself. These are the real-machine arbiters that
        // Mono's IACT acceptance covers state machines and closures too.
        //
        // P35a — added ASYNC method body touches private state (the await
        // completes synchronously, so the kept Probe can block on .Result).
        if self
            .step_file("P35a added async member touches private state", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    public async Task<int> VaultAsync() { await Task.CompletedTask; return _seed + SecretCore() + s_secret + 1; }\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return VaultAsync().Result; }")
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P35a added async member touches private state",
                "return LocusSelfTestSubject.Instance.Probe();",
                "7641", // _seed(40) + SecretCore(7000) + s_secret(600) + 1
            )
            .await;
        }

        // P35b — added method whose LAMBDA captures a local and `this`,
        // touching the private field and private static inside the closure.
        if self
            .step_file("P35b added member lambda captures private state", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    public int VaultLambda() { int basis = 2; System.Func<int> f = () => basis + _seed + s_secret; return f(); }\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return VaultLambda(); }")
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P35b added member lambda captures private state",
                "return LocusSelfTestSubject.Instance.Probe();",
                "642", // basis(2) + _seed(40) + s_secret(600)
            )
            .await;
        }

        // P35c — added ITERATOR (coroutine-shaped) body touches private
        // state inside MoveNext.
        if self
            .step_file("P35c added iterator member touches private state", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    public System.Collections.IEnumerator VaultIter() { yield return SecretCore() + s_secret + 3; }\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { var e = VaultIter(); e.MoveNext(); return (int)e.Current; }")
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P35c added iterator member touches private state",
                "return LocusSelfTestSubject.Instance.Probe();",
                "7603", // SecretCore(7000) + s_secret(600) + 3
            )
            .await;
        }

        // P36 — KEPT body touches a cross-file internal type (guard test):
        // the reference is pure metadata to the batch binding (the patch is
        // its own assembly), and with this editor's green caps it must keep
        // flowing hot through IgnoresAccessChecksTo after the C2′b scan
        // landed (the scan only ever runs on measured-red runtimes).
        if self
            .step_file("P36 kept body touches cross-file internal", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "public int Vaulted() { return SecretCore() + s_secret; }",
                    "public int Vaulted() { return SecretCore() + s_secret + new LocusSelfTestNegHidden().V + 44; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P36 kept body touches cross-file internal",
                "return LocusSelfTestSubject.Instance.Vaulted();",
                "7644", // SecretCore(7000) + s_secret(600) + V(0) + 44
            )
            .await;
        }

        // P37 — added member constructs its own type through a PRIVATE
        // constructor (factory pattern): the ctor symbol hangs on the `new`
        // expression, not the type name, so this exercises the C2′b
        // creation-node check (newobj_private — green on this editor). The
        // kept Spawn body routes the observation.
        let name = "P37 added member uses private ctor";
        self.log(format!("— {name}"));
        let neg_snapshot = neg_ledger.clone();
        let p37 = swap(
            &mut neg_ledger,
            "    public static int Spawn() { return 1; }\n",
            "    public static int Spawn() { return Forge(); }\n    public static int Forge() { return new LocusSelfTestLocked(4100).Worth; }\n",
        );
        match p37 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[(NEG_FILE, neg_ledger.as_str())],
                        &[(NEG_FILE, neg_snapshot.as_str())],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(name, "return LocusSelfTestLocked.Spawn();", "4100")
                        .await;
                }
            }
            Err(error) => self.fail(name, error),
        }

        // P38 — added PROPERTY (B2): write, compound assignment and read
        // all materialize as direct accessor-shim calls (get_/set_ pair).
        // The same batch also ADDS the _tally field, so the accessor bodies
        // route through an M4 store (composition of B2 + M4).
        if self
            .step_file("P38 added property write/compound/read", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    private int _tally;\n    public int Level { get { return _tally + 7; } set { _tally = value; } }\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { Level = 4000; Level += 2; return Level; }")
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P38 added property write/compound/read",
                "return LocusSelfTestSubject.Instance.Probe();",
                "4016", // set(4000): _tally=4000; compound: get=4007, set(4009); read=4016
            )
            .await;
        }

        // P39a — added INDEXER overload (the corpus already has this[int]):
        // read/write/compound through get_Item/set_Item shims with a `this`
        // receiver (repeated by the compound expansion — receiver is pure).
        if self
            .step_file("P39a added indexer read/write/compound", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    public int this[int a, int b] { get { return a + b + _stash; } set { _stash = value; } }\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { this[1, 2] = 7000; this[1, 2] += 4; return this[1, 2]; }")
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P39a added indexer read/write/compound",
                "return LocusSelfTestSubject.Instance.Probe();",
                "7010", // set: _stash=7000; compound: get=7003, set(7007); read=1+2+7007
            )
            .await;
        }

        // P39b — added accessor EVENT: += routes through add_Pump, -=
        // through remove_Pump (delta-asserted: EvtCount accumulates from
        // earlier Surge steps).
        if self
            .step_file("P39b added event subscribe/unsubscribe", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    public event System.Action Pump { add { EvtCount += 100; } remove { EvtCount += 10; } }\n",
                )?;
                swap_line(
                    s,
                    "    public int Probe()",
                    "    public int Probe() { int before = EvtCount; System.Action h = () => { }; Pump += h; Pump -= h; return EvtCount - before + 8200; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P39b added event subscribe/unsubscribe",
                "return LocusSelfTestSubject.Instance.Probe();",
                "8310", // add(+100) + remove(+10) + 8200
            )
            .await;
        }

        // P40 — added AUTO-PROPERTY: accessor shims over an M4 backing
        // store. Existing instances read default(int); the store value
        // persists across calls; NEW instances run the initializer through
        // the redirected (implicit) constructor.
        let p40_ok = self
            .step_file("P40 added auto-property (store-backed)", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Seed() { return _seed; }\n",
                    "    public int Seed() { return _seed; }\n    public int Cargo { get; set; } = 30;\n",
                )?;
                swap_line(s, "    public int Probe()", "    public int Probe() { Cargo += 5; return Cargo + 9000; }")
            })
            .await
            .is_some();
        if p40_ok {
            self.expect_output(
                "P40a existing instance reads default",
                "return LocusSelfTestSubject.Instance.Probe();",
                "9005", // default(0) + 5 + 9000
            )
            .await;
            self.expect_output(
                "P40b store value persists across calls",
                "return LocusSelfTestSubject.Instance.Probe();",
                "9010", // second bump of the SAME store slot
            )
            .await;
            self.expect_output(
                "P40c new instance runs the initializer",
                "var go = new UnityEngine.GameObject(\"LocusSelfTestAutoProbe\");\n\
                 var probe = go.AddComponent<LocusSelfTestSubject>();\n\
                 var value = probe.Probe();\n\
                 UnityEngine.Object.Destroy(go);\n\
                 return value;",
                "9035", // initializer(30) + 5 + 9000
            )
            .await;
        }

        // P40d — re-edit after the AUTO-PROPERTY landed: ++ must still
        // route through the SAME backing store slot from the earlier batch.
        if p40_ok
            && self
                .step_file(
                    "P40d auto-property increment after re-edit",
                    SUBJECT_FILE,
                    subject,
                    |s| {
                        swap_line(
                            s,
                            "    public int Probe()",
                            "    public int Probe() { Cargo++; return Cargo + 9000; }",
                        )
                    },
                )
                .await
                .is_some()
        {
            self.expect_output(
                "P40d auto-property increment after re-edit",
                "return LocusSelfTestSubject.Instance.Probe();",
                "9011", // P40a/P40b left the live instance's Cargo store at 10, then ++
            )
            .await;
        }

        // P40e — nameof(...) over an added auto-property materializes as a
        // constant; no runtime metadata for Cargo exists in the original
        // assembly.
        if p40_ok
            && self
                .step_file(
                    "P40e nameof added auto-property",
                    SUBJECT_FILE,
                    subject,
                    |s| {
                        swap_line(
                            s,
                            "    public int Probe()",
                            "    public int Probe() { return nameof(Cargo).Length + 9100; }",
                        )
                    },
                )
                .await
                .is_some()
        {
            self.expect_output(
                "P40e nameof added auto-property",
                "return LocusSelfTestSubject.Instance.Probe();",
                "9105", // "Cargo".Length(5) + 9100
            )
            .await;
        }

        // ── B3: cross-asmdef batches (the lib corpus compiles into its own
        // LocusSelfTestLib assembly; the callers live in Assembly-CSharp) ──
        let mut lib_ledger = LIB_BASELINE.to_string();

        // P41 — lib method BODY edit: the detour targets the LIB assembly's
        // type (original-type resolution is cross-assembly by name), and the
        // pre-existing Assembly-CSharp caller — compiled IL in ANOTHER
        // assembly, untouched by this batch — observes the new behavior.
        if self
            .step_file(
                "P41 lib body edit (cross-asmdef)",
                LIB_FILE,
                &mut lib_ledger,
                |s| {
                    swap(
                        s,
                        "public int LibBody() { return 5; }",
                        "public int LibBody() { return 6100; }",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "P41 lib body edit (cross-asmdef)",
                "return LocusSelfTestSubject.Instance.LibRelay();",
                "6103", // LibBody(6100) + 3, read through the Assembly-CSharp caller
            )
            .await;
            // R05 — Release-strict, cross-asmdef INSTANCE callee. LibBody is an
            // instance method Mono inlines into LibRelay in Release; the inline
            // caller refresh re-emits its changed body as a static self-shim
            // (Option A) and rewrites LibRelay's
            // `new LocusSelfTestLibType().LibBody()` to call that shim, so the
            // refreshed caller observes the new body immediately even across the
            // assembly boundary (callee+caller compile into one patch assembly).
            // This strict probe is the suite-level gate for the instance arm.
            //
            // On failure it LOCALIZES the staleness instead of just printing the
            // value: it reads LibBody() directly and dumps the P41 apply summary,
            // so a stale LibRelay can be pinned to one of:
            //   • callee detour live (LibBody direct == 6100) but LibRelay's
            //     inlined call site was not refreshed → the refresh didn't patch
            //     the caller (summary shows a note, not "Inline caller refresh
            //     patched … LocusSelfTestSubject.cs"); or
            //   • callee detour itself not in effect (LibBody direct == 5).
            let name = "R05 release strict cross-asmdef body edit";
            if self.release_mode {
                // Surface the P41 inline-refresh segment UNtruncated on EVERY run
                // (pass or fail): R05 is flaky even under confirmed Release, so
                // capturing what the refresh did on a PASSING run lets us diff it
                // against a failing run — did it patch LibRelay (methods>0, names
                // LocusSelfTestSubject.cs) or bail with a note / methods==0? The
                // dotnet repro proves the compile-level redirect is correct
                // cross-asmdef, so a live miss is caller discovery/force-detour or
                // a stale caller-index cache, not IL generation.
                let refresh_tail = self
                    .last_apply_summary
                    .find("Inline caller refresh")
                    .map(|idx| self.last_apply_summary[idx..].trim().to_string())
                    .unwrap_or_else(|| {
                        "<no 'Inline caller refresh' segment in summary>".to_string()
                    });
                self.log(format!(
                    "  R05 P41 inline-refresh segment: [{refresh_tail}]"
                ));
                match self
                    .execute("return LocusSelfTestSubject.Instance.LibRelay();")
                    .await
                {
                    Ok(ref output) if output.contains("6103") => {
                        self.pass(name, "observed 6103 immediately");
                    }
                    Ok(output) => {
                        let lib_direct = self
                            .execute("return new LocusSelfTestLibType().LibBody();")
                            .await
                            .unwrap_or_else(|error| format!("<error: {error}>"));
                        self.fail(
                            name,
                            format!(
                                "Release immediate effect missing; expected '6103' through LibRelay, got: {}. \
                                 LibBody() direct = {} (6100 ⇒ callee detour live, LibRelay's inlined call site was \
                                 NOT refreshed; 5 ⇒ callee detour not in effect). Inline-refresh segment: [{}]. \
                                 Full P41 apply summary: {}",
                                output.trim(),
                                lib_direct.trim(),
                                refresh_tail,
                                squash(&self.last_apply_summary),
                            ),
                        );
                    }
                    Err(error) => self.fail(name, format!("snippet failed: {error}")),
                }
            }
        }

        // P42 — lib method SIGNATURE change with the caller in ANOTHER
        // assembly: the M3 scan must walk Assembly-CSharp's IL and name the
        // Assembly-CSharp file first (P42a); the same change goes hot once
        // that caller joins the batch (P42b). Mirrors P04 across the
        // assembly boundary.
        let name = "P42a lib signature change names cross-assembly caller";
        self.log(format!("— {name}"));
        let lib_snapshot = lib_ledger.clone();
        let subject_snapshot = subject.clone();
        let p42 = swap(
            &mut lib_ledger,
            "public int LibSig(int x) { return x + 1; }",
            "public int LibSig(int x, int bump) { return x + bump + 200; }",
        );
        match p42 {
            Ok(()) => match self.write_tracked(LIB_FILE, &lib_ledger).await {
                Ok(()) => {
                    let verdict = self.hot_reload(Some(vec![LIB_FILE.to_string()])).await;
                    let text = match &verdict {
                        Ok(summary) => summary.clone(),
                        Err(error) => error.clone(),
                    };
                    if text.contains("Hot reload not applicable")
                        && text.contains("LocusSelfTestSubject.cs")
                    {
                        self.pass(name, "cold verdict names the Assembly-CSharp caller file");
                    } else {
                        self.fail(
                            name,
                            format!(
                                "expected cold naming the cross-assembly caller, got: {}",
                                squash(&text)
                            ),
                        );
                    }

                    // P42b — covered batch: the Assembly-CSharp caller
                    // co-edits, the scan verifies, the rewritten call site
                    // materializes as a direct cross-assembly shim call.
                    let p42b = swap(
                        subject,
                        "public int LibSigRelay() { return new LocusSelfTestLibType().LibSig(10); }",
                        "public int LibSigRelay() { return new LocusSelfTestLibType().LibSig(10, 7); }",
                    );
                    let applied = match p42b {
                        Ok(()) => {
                            self.log("— P42b lib signature change covered cross-assembly");
                            self.apply_texts(
                                "P42b lib signature change covered cross-assembly",
                                &[(SUBJECT_FILE, subject.as_str())],
                                &[
                                    (SUBJECT_FILE, subject_snapshot.as_str()),
                                    (LIB_FILE, lib_snapshot.as_str()),
                                ],
                            )
                            .await
                        }
                        Err(error) => {
                            self.fail("P42b lib signature change covered cross-assembly", error);
                            None
                        }
                    };
                    if applied.is_some() {
                        self.expect_output(
                            "P42b lib signature change covered cross-assembly",
                            "return LocusSelfTestSubject.Instance.LibSigRelay();",
                            "217", // 10 + 7 + 200
                        )
                        .await;
                    } else {
                        *subject = subject_snapshot;
                        lib_ledger = lib_snapshot;
                    }
                }
                Err(error) => {
                    self.fail(name, error);
                    lib_ledger = lib_snapshot;
                }
            },
            Err(error) => self.fail(name, error),
        }

        // P43 — method ADDED to the lib type, called from Assembly-CSharp:
        // the shim lives in the patch assembly while `self` is the LIB
        // assembly's type, and the rewritten Assembly-CSharp call site must
        // direct-call it across the boundary (the predicted B3 risk spot).
        let name = "P43 lib added method called cross-assembly";
        self.log(format!("— {name}"));
        let lib_snapshot = lib_ledger.clone();
        let subject_snapshot = subject.clone();
        let p43 = (|| -> Result<(), String> {
            swap(
                &mut lib_ledger,
                "    public int LibSeed = 8;\n",
                "    public int LibSeed = 8;\n\n    public int LibBoost() { return LibSeed + 5800; }\n",
            )?;
            swap_line(
                subject,
                "    public int Probe()",
                "    public int Probe() { return new LocusSelfTestLibType().LibBoost(); }",
            )
        })();
        match p43 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (LIB_FILE, lib_ledger.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (LIB_FILE, lib_snapshot.as_str()),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.Probe();",
                        "5808", // LibSeed(8) + 5800, through the cross-assembly shim
                    )
                    .await;
                } else {
                    // The next lib case starts from the actual disk shape.
                    *subject = subject_snapshot;
                    lib_ledger = lib_snapshot;
                }
            }
            Err(error) => {
                self.fail(name, error);
                *subject = subject_snapshot;
                lib_ledger = lib_snapshot;
            }
        }

        // P43b — PROPERTY added to the lib type and consumed from the
        // Assembly-CSharp caller: B2 accessor shims plus B3 cross-assembly
        // self binding and call-site rewrite.
        let name = "P43b lib added property called cross-assembly";
        self.log(format!("— {name}"));
        let lib_snapshot = lib_ledger.clone();
        let subject_snapshot = subject.clone();
        let p43b = (|| -> Result<(), String> {
            swap(
                &mut lib_ledger,
                "    public int LibSeed = 8;\n",
                "    public int LibSeed = 8;\n\n    public int LibScore { get { return LibSeed + 6200; } set { LibSeed = value; } }\n",
            )?;
            swap_line(
                subject,
                "    public int Probe()",
                "    public int Probe() { var lib = new LocusSelfTestLibType(); lib.LibScore = 21; return lib.LibScore; }",
            )
        })();
        match p43b {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (LIB_FILE, lib_ledger.as_str()),
                            (SUBJECT_FILE, subject.as_str()),
                        ],
                        &[
                            (LIB_FILE, lib_snapshot.as_str()),
                            (SUBJECT_FILE, subject_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.Probe();",
                        "6221", // set LibSeed=21, then get LibSeed + 6200
                    )
                    .await;
                } else {
                    *subject = subject_snapshot;
                }
            }
            Err(error) => {
                self.fail(name, error);
                *subject = subject_snapshot;
            }
        }

        // P44 — partial type split across two files (B6): editing ONE
        // part's method body goes hot. The coordinator discovers the
        // never-edited sibling part on disk, the sidecar folds it into the
        // batch as an unchanged baseline, and the complete two-part patch
        // copy — fields merged in the original assembly's order — binds the
        // OTHER part's private field (_beta) and private method (Basis).
        let mut partial_a_ledger = PARTIAL_A_BASELINE.to_string();
        let mut partial_b_ledger = PARTIAL_B_BASELINE.to_string();
        if self
            .step_file(
                "P44 partial part body edit (sibling on disk)",
                PARTIAL_A_FILE,
                &mut partial_a_ledger,
                |s| {
                    swap(
                        s,
                        "public int Combine() { return _alpha + Basis() + _beta; }",
                        "public int Combine() { return _alpha + Basis() + _beta + 7000; }",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "P44 partial part body edit (sibling on disk)",
                "return new LocusSelfTestPartial().Combine();",
                "7435", // _alpha(30) + Basis(5) + _beta(400) + 7000
            )
            .await;
        }

        // P45 — both partial part files join the same batch: the sidecar
        // should merge the two changed declarations directly, without
        // relying on an unchanged sibling copy.
        let name = "P45 partial two-part batch edit";
        self.log(format!("— {name}"));
        let partial_a_snapshot = partial_a_ledger.clone();
        let partial_b_snapshot = partial_b_ledger.clone();
        let p45 = (|| -> Result<(), String> {
            swap(
                &mut partial_a_ledger,
                "public int Combine() { return _alpha + Basis() + _beta + 7000; }",
                "public int Combine() { return _alpha + Basis() + _beta + 7100; }",
            )
            .or_else(|_| {
                swap(
                    &mut partial_a_ledger,
                    "public int Combine() { return _alpha + Basis() + _beta; }",
                    "public int Combine() { return _alpha + Basis() + _beta + 7100; }",
                )
            })?;
            swap(
                &mut partial_b_ledger,
                "private int Basis() { return 5; }",
                "private int Basis() { return 15; }",
            )
        })();
        match p45 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[
                            (PARTIAL_A_FILE, partial_a_ledger.as_str()),
                            (PARTIAL_B_FILE, partial_b_ledger.as_str()),
                        ],
                        &[
                            (PARTIAL_A_FILE, partial_a_snapshot.as_str()),
                            (PARTIAL_B_FILE, partial_b_snapshot.as_str()),
                        ],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(
                        name,
                        "return new LocusSelfTestPartial().Combine();",
                        "7545", // _alpha(30) + Basis(15) + _beta(400) + 7100
                    )
                    .await;
                } else {
                    partial_a_ledger = partial_a_snapshot;
                }
            }
            Err(error) => {
                self.fail(name, error);
                partial_a_ledger = partial_a_snapshot;
            }
        }

        // P48 — retyping an instance field decomposes into remove(placeholder)
        // + add(store): the old int slot is parked and a fresh long field is
        // virtualized, so the already-live instance reads the new field's
        // default (the documented value-loss on a hot retype).
        if self
            .step_file("P48 instance field retype", SUBJECT_FILE, subject, |s| {
                swap(s, "    private int _flux = 5;", "    private long _flux = 5;")?;
                swap(
                    s,
                    "public int Flux() { return _flux; }",
                    "public int Flux() { return (int)(_flux + 9990); }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P48 instance field retype",
                "return LocusSelfTestSubject.Instance.Flux();",
                "9990", // live instance: the freshly-virtualized long field defaults to 0
            )
            .await;
        }

        // P49 — a newly added const is inlined into the patch; a same-batch
        // reader resolves it immediately (no pre-existing call site referenced
        // it, so nothing stale survives).
        if self
            .step_file("P49 added const inlined", SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public int Spark() { return 0; }",
                    "    private const int SparkK = 88;\n    public int Spark() { return SparkK; }",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                "P49 added const inlined",
                "return LocusSelfTestSubject.Instance.Spark();",
                "88",
            )
            .await;
        }

        // Hand the per-file ledgers to the negative phase: it restores files
        // to exactly these texts after each cold probe.
        self.negative_ledgers = NegativeLedgers {
            struct_text: struct_ledger,
            ctor_text: ctor_ledger,
            iface_text: iface_ledger,
            partial_a_text: partial_a_ledger,
        };
    }

    async fn run_message_driver_tests(&mut self, messages: &mut String) {
        self.log("Phase 4b — hot-added Unity message drivers");

        self.expect_output(
            "P46 baseline message corpus",
            "return LocusSelfTestMessages.Instance != null && LocusSelfTestMessages.Instance.Marker() == 1 ? \"messages-ready\" : \"messages-missing\";",
            "messages-ready",
        )
        .await;

        // P46a — add the three parameterless per-frame messages after the type is
        // already loaded. Unity will not discover them natively; Locus must drive
        // them through its PlayerLoop driver.
        let frame_name = "P46a added Update/LateUpdate/FixedUpdate";
        if self
            .step_file(frame_name, MESSAGE_FILE, messages, |s| {
                swap(
                    s,
                    "    public int Marker() { return 1; }\n",
                    "    public int Marker() { return 1; }\n\n    void Update() { UpdateCount += 1; }\n    void LateUpdate() { LateUpdateCount += 1; }\n    void FixedUpdate() { FixedUpdateCount += 1; }\n",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                frame_name,
                "LocusSelfTestMessages.ResetFrameCounters();\n\
                 await ctx.WaitSeconds(0.8f);\n\
                 return LocusSelfTestMessages.UpdateCount > 0 && LocusSelfTestMessages.LateUpdateCount > 0 && LocusSelfTestMessages.FixedUpdateCount > 0\n\
                     ? \"frame-drivers-ok\" : (\"frame-drivers-missing u=\" + LocusSelfTestMessages.UpdateCount + \" l=\" + LocusSelfTestMessages.LateUpdateCount + \" f=\" + LocusSelfTestMessages.FixedUpdateCount);",
                "frame-drivers-ok",
            )
            .await;
        }

        let caps = self.message_driver_capabilities;
        if caps.has_physics_proxy() {
            // P46b — add physics/trigger message methods in a later patch to the
            // SAME source file. This verifies component-proxy dispatch and also
            // catches replace-by-source bugs where adding proxy drivers accidentally
            // drops the earlier PlayerLoop drivers from the same file.
            let proxy_name = "P46b added physics/trigger message proxy";
            if self
                .step_file(proxy_name, MESSAGE_FILE, messages, |s| {
                    let replacement = format!(
                        "    public int Marker() {{ return 1; }}\n\n{}",
                        initial_proxy_methods(caps)
                    );
                    swap(s, "    public int Marker() { return 1; }\n", &replacement)
                })
                .await
                .is_some()
            {
                if caps.physics3d {
                    self.expect_output(
                        "P46b 3D physics/trigger message proxy",
                        physics3d_proxy_initial_assertion(),
                        "proxy-3d-ok",
                    )
                    .await;
                }
                if caps.physics2d {
                    self.expect_output(
                        "P46b 2D physics/trigger message proxy",
                        physics2d_proxy_initial_assertion(),
                        "proxy-2d-ok",
                    )
                    .await;
                }

                // P46c/P46d — edit the same component-proxy message repeatedly.
                // The proxy registration must bind to the newest shim exactly once,
                // while unrelated proxy messages from the same source file stay live.
                let proxy_reedit_name = "P46c repeated component-proxy edit replaces prior shim";
                if self
                    .step_file(proxy_reedit_name, MESSAGE_FILE, messages, |s| {
                        if caps.physics3d {
                            swap_line(
                                s,
                                "    void OnTriggerEnter(Collider other)",
                                "    void OnTriggerEnter(Collider other) { Trigger3D += other != null ? 30 : 3; TriggerStay3D += 3000; }",
                            )?;
                        }
                        if caps.physics2d {
                            swap_line(
                                s,
                                "    void OnTriggerEnter2D(Collider2D other)",
                                "    void OnTriggerEnter2D(Collider2D other) { Trigger2D += other != null ? 40 : 4; TriggerStay2D += 4000; }",
                            )?;
                        }
                        Ok(())
                    })
                    .await
                    .is_some()
                {
                    if caps.physics3d {
                        self.expect_output(
                            "P46c 3D repeated component-proxy edit replaces prior shim",
                            physics3d_proxy_reedit_assertion(),
                            "proxy-3d-reedit-ok",
                        )
                        .await;
                    }
                    if caps.physics2d {
                        self.expect_output(
                            "P46c 2D repeated component-proxy edit replaces prior shim",
                            physics2d_proxy_reedit_assertion(),
                            "proxy-2d-reedit-ok",
                        )
                        .await;
                    }

                    let proxy_second_reedit_name =
                        "P46d second component-proxy edit remains single-bound";
                    if self
                        .step_file(proxy_second_reedit_name, MESSAGE_FILE, messages, |s| {
                            if caps.physics3d {
                                swap_line(
                                    s,
                                    "    void OnTriggerEnter(Collider other)",
                                    "    void OnTriggerEnter(Collider other) { Trigger3D += other != null ? 50 : 5; TriggerExit3D += 5000; }",
                                )?;
                            }
                            if caps.physics2d {
                                swap_line(
                                    s,
                                    "    void OnTriggerEnter2D(Collider2D other)",
                                    "    void OnTriggerEnter2D(Collider2D other) { Trigger2D += other != null ? 60 : 6; TriggerExit2D += 6000; }",
                                )?;
                            }
                            Ok(())
                        })
                        .await
                        .is_some()
                    {
                        if caps.physics3d {
                            self.expect_output(
                                "P46d 3D second component-proxy edit remains single-bound",
                                physics3d_proxy_second_reedit_assertion(),
                                "proxy-3d-second-reedit-ok",
                            )
                            .await;
                        }
                        if caps.physics2d {
                            self.expect_output(
                                "P46d 2D second component-proxy edit remains single-bound",
                                physics2d_proxy_second_reedit_assertion(),
                                "proxy-2d-second-reedit-ok",
                            )
                            .await;
                        }
                    }
                }

                self.expect_output(
                    "P46e message drivers survive same-file proxy patch",
                    "var before = LocusSelfTestMessages.FrameTotal();\n\
                     await ctx.WaitSeconds(0.5f);\n\
                     return LocusSelfTestMessages.FrameTotal() > before ? \"frame-drivers-still-live\" : \"frame-drivers-dropped\";",
                    "frame-drivers-still-live",
                )
                .await;
            }
        } else {
            self.log(
                "  skipping physics/trigger proxy message tests; no 3D/2D physics module detected",
            );
        }

        self.expect_output(
            "P46f frame message drivers honor component enabled",
            "var targets = UnityEngine.Object.FindObjectsByType<LocusSelfTestMessages>(UnityEngine.FindObjectsInactive.Include, UnityEngine.FindObjectsSortMode.InstanceID);\n\
             if (targets.Length == 0) return \"enabled-gate-missing-instance\";\n\
             var before = LocusSelfTestMessages.FrameTotal();\n\
             foreach (var target in targets) target.enabled = false;\n\
             await ctx.WaitSeconds(0.35f);\n\
             var disabled = LocusSelfTestMessages.FrameTotal();\n\
             foreach (var target in targets) target.enabled = true;\n\
             await ctx.WaitSeconds(0.35f);\n\
             return disabled == before && LocusSelfTestMessages.FrameTotal() > disabled ? \"enabled-gate-ok\" : (\"enabled-gate-leaked before=\" + before + \" disabled=\" + disabled + \" after=\" + LocusSelfTestMessages.FrameTotal());",
            "enabled-gate-ok",
        )
        .await;

        // P46g — add lifecycle messages whose native timing already passed for
        // this loaded component type. Locus should run each catch-up shim once on
        // the existing live instance.
        let lifecycle_name = "P46g added Awake/Start/OnValidate catch-up";
        match self
            .execute("LocusSelfTestLifecycleMessages.ResetLifecycleCounters(); return \"lifecycle-reset\";")
            .await
        {
            Ok(output) if output.contains("lifecycle-reset") => {}
            Ok(output) => self.fail(
                lifecycle_name,
                format!("counter reset returned unexpected output: {}", output.trim()),
            ),
            Err(error) => self.fail(lifecycle_name, format!("counter reset failed: {error}")),
        }
        if self
            .step_file(lifecycle_name, MESSAGE_FILE, messages, |s| {
                swap(
                    s,
                    "    public int Marker() { return 2; }\n",
                    "    public int Marker() { return 2; }\n\n    void Awake() { AwakeCatchUp += 1; }\n    void Start() { StartCatchUp += 1; }\n    void OnValidate() { ValidateCatchUp += 1; }\n",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(
                lifecycle_name,
                "return LocusSelfTestLifecycleMessages.AwakeCatchUp == 1 && LocusSelfTestLifecycleMessages.StartCatchUp == 1 && LocusSelfTestLifecycleMessages.ValidateCatchUp == 1\n\
                     ? \"lifecycle-catchup-ok\" : (\"lifecycle-catchup-mismatch awake=\" + LocusSelfTestLifecycleMessages.AwakeCatchUp + \" start=\" + LocusSelfTestLifecycleMessages.StartCatchUp + \" validate=\" + LocusSelfTestLifecycleMessages.ValidateCatchUp);",
                "lifecycle-catchup-ok",
            )
            .await;
        }

        // P46h — non-physics component proxies: parameterless mouse, int-argument
        // animator IK, and lifecycle OnDestroy all forward through the proxy hub.
        let utility_proxy_name = "P46h added non-physics message proxies";
        if self
            .step_file(utility_proxy_name, MESSAGE_FILE, messages, |s| {
                swap(
                    s,
                    "    public static int UtilityProxyTotal() { return MouseDownCount + AnimatorIkLayer + DestroyCount; }\n",
                    "    public static int UtilityProxyTotal() { return MouseDownCount + AnimatorIkLayer + DestroyCount; }\n\n    void OnMouseDown() { MouseDownCount += 1; }\n    void OnAnimatorIK(int layer) { AnimatorIkLayer = layer; }\n    void OnDestroy() { DestroyCount += 1; }\n",
                )
            })
            .await
            .is_some()
        {
            self.expect_output(utility_proxy_name, utility_proxy_assertion(), "utility-proxy-ok")
                .await;
        }
    }

    async fn run_hot_added_mono_behaviour_diagnostic(&mut self) {
        self.log("Phase 4c — hot-added MonoBehaviour AddComponent diagnostic");

        let name = "P47 hot-added MonoBehaviour AddComponent(Type)";
        self.log(format!("— {name}"));
        let source = r#"using UnityEngine;

public class LocusSelfTestHotAddedBehaviour : MonoBehaviour
{
    public static int AwakeCount;
    public static int EnableCount;
    public static int UpdateCount;
    public static string LastObjectName = "";

    public int marker = 4701;

    public static void ResetCounters()
    {
        AwakeCount = 0;
        EnableCount = 0;
        UpdateCount = 0;
        LastObjectName = "";
    }

    void Awake()
    {
        AwakeCount += 1;
        LastObjectName = gameObject.name;
    }

    void OnEnable()
    {
        EnableCount += 1;
    }

    void Update()
    {
        UpdateCount += 1;
    }

    public int Marker()
    {
        return marker;
    }
}
"#;

        let Some(summary) = self
            .apply_texts(name, &[(HOT_ADDED_BEHAVIOUR_FILE, source)], &[])
            .await
        else {
            return;
        };
        if summary.contains("new type") {
            self.pass(
                name,
                format!(
                    "hot patch loaded the new MonoBehaviour type ({})",
                    squash(&summary)
                ),
            );
        } else {
            self.fail(
                name,
                format!(
                    "hot patch did not report a loaded new type; summary: {}",
                    squash(&summary)
                ),
            );
        }

        match self
            .execute("return typeof(LocusSelfTestHotAddedBehaviour).FullName;")
            .await
        {
            Ok(output) => self.pass(
                "P47a unity_execute compile-scope diagnostic",
                format!("strong type reference compiled: {}", squash(&output)),
            ),
            Err(error)
                if error.contains("CS0246")
                    || error.contains("could not be found")
                    || error.contains("compilation") =>
            {
                self.pass(
                    "P47a unity_execute compile-scope diagnostic",
                    format!(
                        "strong type reference is outside the snippet compile references: {}",
                        squash(&error)
                    ),
                );
            }
            Err(error) => self.fail(
                "P47a unity_execute compile-scope diagnostic",
                format!("unexpected execute failure: {}", squash(&error)),
            ),
        }

        let diagnostic = r#"var typeName = "LocusSelfTestHotAddedBehaviour";
var assemblies = System.AppDomain.CurrentDomain.GetAssemblies();
var hotAssemblies = assemblies
    .Where(a => a.GetName().Name.StartsWith("__LocusHotPatch_"))
    .Select(a => a.GetName().Name)
    .OrderBy(n => n)
    .ToArray();
print("hot_patch_assemblies=" + string.Join(",", hotAssemblies));

var t = assemblies
    .Select(a => a.GetType(typeName, false))
    .FirstOrDefault(x => x != null);
print("type_found=" + (t != null));
if (t == null)
{
    print("hot_component_status=gap");
    return null;
}

print("type_full_name=" + t.FullName);
print("type_assembly=" + t.Assembly.GetName().Name);
print("type_location_empty=" + string.IsNullOrEmpty(t.Assembly.Location));
var isMono = typeof(UnityEngine.MonoBehaviour).IsAssignableFrom(t);
print("is_mono_behaviour=" + isMono);

var reset = t.GetMethod("ResetCounters", BindingFlags.Public | BindingFlags.Static);
reset?.Invoke(null, null);

UnityEngine.GameObject go = null;
UnityEngine.Component comp = null;
bool addOk = false;
bool markerOk = false;
int awakeCount = -1;
int enableCount = -1;
int updateCount = -1;
string lastObjectName = "";
string monoScriptPath = "<not-run>";
string serializedScript = "<not-run>";

try
{
    go = new UnityEngine.GameObject("LocusHotAddedBehaviourProbe");
    comp = go.AddComponent(t);
    addOk = comp != null;
    print("add_ok=" + addOk);
    if (comp != null)
    {
        print("component_type=" + comp.GetType().FullName);
        print("component_enabled=" + ((UnityEngine.Behaviour)comp).enabled);
        var marker = t.GetMethod("Marker", BindingFlags.Public | BindingFlags.Instance)?.Invoke(comp, null);
        markerOk = object.Equals(marker, 4701);
        print("marker=" + (marker == null ? "<null>" : marker.ToString()));

        try
        {
            var script = UnityEditor.MonoScript.FromMonoBehaviour((UnityEngine.MonoBehaviour)comp);
            monoScriptPath = script == null ? "<null>" : UnityEditor.AssetDatabase.GetAssetPath(script);
        }
        catch (System.Exception ex)
        {
            monoScriptPath = "EX:" + ex.GetType().Name + ":" + ex.Message;
        }
        print("monoscript_path=" + monoScriptPath);

        try
        {
            var serialized = new UnityEditor.SerializedObject(comp);
            var scriptProperty = serialized.FindProperty("m_Script");
            var scriptObject = scriptProperty == null ? null : scriptProperty.objectReferenceValue;
            serializedScript = scriptObject == null ? "<null>" : scriptObject.name;
        }
        catch (System.Exception ex)
        {
            serializedScript = "EX:" + ex.GetType().Name + ":" + ex.Message;
        }
        print("serialized_m_script=" + serializedScript);
    }
}
catch (System.Exception ex)
{
    print("add_exception=" + ex.GetType().Name + ":" + ex.Message);
}

await ctx.WaitFrames(4);

if (t != null)
{
    awakeCount = System.Convert.ToInt32(t.GetField("AwakeCount", BindingFlags.Public | BindingFlags.Static)?.GetValue(null) ?? -1);
    enableCount = System.Convert.ToInt32(t.GetField("EnableCount", BindingFlags.Public | BindingFlags.Static)?.GetValue(null) ?? -1);
    updateCount = System.Convert.ToInt32(t.GetField("UpdateCount", BindingFlags.Public | BindingFlags.Static)?.GetValue(null) ?? -1);
    lastObjectName = (string)(t.GetField("LastObjectName", BindingFlags.Public | BindingFlags.Static)?.GetValue(null) ?? "");
    print("lifecycle_counts=awake:" + awakeCount + ",enable:" + enableCount + ",update:" + updateCount + ",name:" + lastObjectName);
}

if (go != null)
{
    UnityEngine.Object.Destroy(go);
    await ctx.WaitFrame();
}

var status = t != null
    && isMono
    && addOk
    && markerOk
    && awakeCount > 0
    && enableCount > 0
    && updateCount > 0
        ? "ok"
        : "gap";
print("hot_component_status=" + status);
return null;"#;

        match self.execute(diagnostic).await {
            Ok(output) if output.contains("hot_component_status=ok") => {
                let mono_script = output
                    .lines()
                    .find(|line| line.contains("monoscript_path="))
                    .map(str::trim)
                    .unwrap_or("monoscript_path=<missing>");
                self.pass(
                    "P47b runtime AddComponent diagnostic",
                    format!(
                        "AddComponent(Type), Marker, Awake/OnEnable/Update all worked; {mono_script}"
                    ),
                );
            }
            Ok(output) => self.fail(
                "P47b runtime AddComponent diagnostic",
                format!("hot-added component gap: {}", squash(&output)),
            ),
            Err(error) => self.fail(
                "P47b runtime AddComponent diagnostic",
                format!("diagnostic snippet failed: {}", squash(&error)),
            ),
        }
    }

    /// Phase 2b — Release-only positive coverage for caller refresh. The
    /// callee is explicitly marked AggressiveInlining, so Unity/Mono reports it
    /// as `inlined in Release`; Locus then refreshes the caller method within
    /// the two-level hard limit and the immediate call-site read observes the
    /// new value.
    async fn run_release_inline_tests(&mut self) {
        self.log("Phase 2b — Release inline caller refresh");
        // Gate the "inlined in Release" assertions on the JIT's EFFECTIVE
        // behavior, not the codeOptimization setting: play mode has been observed
        // Debug-effective (nothing inlines) even when the setting reads release,
        // which previously failed R10/R11 spuriously. When inlining is inactive
        // the behavioral sub-asserts below still verify correctness via the direct
        // detour; only the "inlining was detected" claim is skipped.
        let (inlining_active, inlining_detail) = self.inlining_active().await;
        if inlining_active {
            self.log(format!(
                "  runtime inlining ACTIVE (Release-effective) — inline-refresh path exercised [{inlining_detail}]"
            ));
        } else {
            self.log(format!(
                "  runtime inlining INACTIVE (Debug-effective) — inline-detection asserts soft-skipped; behavior still verified [{inlining_detail}]"
            ));
        }
        let name = "R10 aggressive-inlining caller refresh";
        let edited = ADVERSARIAL_BASELINE.replace("return 5005;", "return 6116;");
        if let Some(summary) = self
            .apply_texts(
                name,
                &[(ADVERSARIAL_FILE, edited.as_str())],
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
            )
            .await
        {
            if summary.contains("inlined in Release") {
                self.pass(name, "reported inlined in Release");
            } else if inlining_active {
                self.fail(
                    name,
                    format!(
                        "expected Release inline detection in summary, got: {}",
                        squash(&summary)
                    ),
                );
            } else {
                self.pass(
                    name,
                    "Debug-effective runtime: inline detection N/A; behavior asserted below",
                );
            }
            self.expect_release_immediate_output(
                name,
                "return LocusSelfTestAdversarial.CallInlined();",
                "6116",
            )
            .await;
        }

        let _ = self
            .apply_texts(
                "R10 restore aggressive-inlining corpus",
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
            )
            .await;

        let name = "R11 cross-file inline caller refresh batch";
        let edited = INLINE_CALLEE_BASELINE
            .replace("return 101;", "return 1101;")
            .replace("return 201;", "return 1201;")
            .replace("return x + 301;", "return x + 1301;")
            .replace("return 501;", "return 1501;")
            .replace("return x > 0 ? 601 : 0;", "return x > 0 ? 1601 : 0;")
            .replace("return 701;", "return 1701;");
        if let Some(summary) = self
            .apply_texts(
                name,
                &[(INLINE_CALLEE_FILE, edited.as_str())],
                &[(INLINE_CALLEE_FILE, INLINE_CALLEE_BASELINE)],
            )
            .await
        {
            // The summary must report Release inlining was detected; the caller
            // refresh itself is proven BEHAVIORALLY by R11a–f below. The
            // "Inline caller refresh patched" line only appears when the batch's
            // refresh recompile actually patches a caller — which the large
            // cumulative batch may legitimately skip (the predicted-inlined
            // callees' direct detours deliver the values, methods==0 + a note),
            // so it is not a reliable suite gate (keying on it reddened a healthy
            // run while R11a–f all passed).
            if summary.contains("inlined in Release") {
                self.pass(
                    name,
                    "reported inlined in Release; caller refresh asserted behaviorally below",
                );
            } else if inlining_active {
                self.fail(
                    name,
                    format!(
                        "expected Release inline detection, got: {}",
                        squash(&summary)
                    ),
                );
            } else {
                self.pass(name, "Debug-effective runtime: inline detection N/A; caller refresh asserted behaviorally below");
            }
            self.expect_release_immediate_output(
                "R11a direct caller file",
                "return LocusSelfTestInlineCallerDirect.Run();",
                "1102",
            )
            .await;
            self.expect_release_immediate_output(
                "R11b nested callee caller file",
                "return LocusSelfTestInlineCallerNested.Run();",
                "1203",
            )
            .await;
            self.expect_release_immediate_output(
                "R11c overload caller file",
                "return LocusSelfTestInlineCallerOverload.Run();",
                "1307",
            )
            .await;
            self.expect_release_immediate_output(
                "R11d lambda caller file",
                "return LocusSelfTestInlineCallerLambda.Run();",
                "1505",
            )
            .await;
            self.expect_release_immediate_output(
                "R11e branch caller file",
                "return LocusSelfTestInlineCallerBranch.Run(2);",
                "1606",
            )
            .await;
            self.expect_release_immediate_output(
                "R11f collection caller file",
                "return LocusSelfTestInlineCallerArray.Run();",
                "1707",
            )
            .await;
        }

        let _ = self
            .apply_texts(
                "R11 restore cross-file inline corpus",
                &[(INLINE_CALLEE_FILE, INLINE_CALLEE_BASELINE)],
                &[(INLINE_CALLEE_FILE, INLINE_CALLEE_BASELINE)],
            )
            .await;
    }

    /// Phase 2b+ — multi-file caller-refresh coverage for the shapes R10/R11
    /// don't reach: a depth-2 static chain (R12), a cross-assembly static callee
    /// (R13), and a same-assembly INSTANCE callee (R14 — the same-assembly
    /// counterpart of the R05 suite-gate, hot since the instance self-shim
    /// redirect landed). All three are confirmed HOT on Unity, so they are real
    /// suite assertions (promoted out of the earlier diagnostic phase).
    async fn run_release_inline_multifile_tests(&mut self) {
        self.log("Phase 2b+ — multi-file inline caller refresh");

        // R12 — depth-2 static inline chain across three files: editing Leaf
        // refreshes Mid (round 1, caller of Leaf) and then Top (round 2, caller
        // of Mid), the exact INLINE_REFRESH_MAX_DEPTH ceiling. Each hop is
        // static, and a force-detoured intermediate re-enters the diff as a
        // changed method, so its own caller redirects too.
        let name = "R12 depth-2 static inline chain";
        self.log(format!("— {name}"));
        let edited = INLINE_CHAIN_LEAF_BASELINE.replace("return 3100;", "return 9100;");
        if self
            .apply_texts(
                name,
                &[(INLINE_CHAIN_LEAF_FILE, edited.as_str())],
                &[(INLINE_CHAIN_LEAF_FILE, INLINE_CHAIN_LEAF_BASELINE)],
            )
            .await
            .is_some()
        {
            self.expect_release_immediate_output(
                name,
                "return LocusSelfTestChainTop.Top();",
                "9430", // 9100 + 30 + 300, observed at the two-hops-removed caller
            )
            .await;
        }
        let _ = self
            .apply_texts(
                "R12 restore inline chain",
                &[(INLINE_CHAIN_LEAF_FILE, INLINE_CHAIN_LEAF_BASELINE)],
                &[(INLINE_CHAIN_LEAF_FILE, INLINE_CHAIN_LEAF_BASELINE)],
            )
            .await;

        // R13 — cross-ASSEMBLY static inline refresh: a static lib method
        // inlined into an Assembly-CSharp caller. The refresh recompiles
        // callee+caller into ONE patch assembly, so the static patch-copy
        // redirect erases the boundary (confirming the assembly boundary alone
        // never blocked the refresh — R05's earlier failure was instance-only).
        let name = "R13 cross-asmdef static inline refresh";
        self.log(format!("— {name}"));
        let edited = LIB_INLINE_BASELINE.replace("return 6200;", "return 7200;");
        if self
            .apply_texts(
                name,
                &[(LIB_INLINE_FILE, edited.as_str())],
                &[(LIB_INLINE_FILE, LIB_INLINE_BASELINE)],
            )
            .await
            .is_some()
        {
            self.expect_release_immediate_output(
                name,
                "return LocusSelfTestLibInlineCaller.Run();",
                "7206", // 7200 + 6, read through the Assembly-CSharp caller
            )
            .await;
        }
        let _ = self
            .apply_texts(
                "R13 restore lib inline corpus",
                &[(LIB_INLINE_FILE, LIB_INLINE_BASELINE)],
                &[(LIB_INLINE_FILE, LIB_INLINE_BASELINE)],
            )
            .await;

        // R14 — same-ASSEMBLY INSTANCE inline refresh: an instance callee
        // inlined into a static caller. This is R05 with the assembly boundary
        // removed. The instance self-shim redirect (Option A) binds the inlined
        // instance call to the changed body's static self-shim, so the immediate
        // read observes 9408 — the same-assembly companion to the R05 gate.
        let name = "R14 same-asm instance inline refresh";
        self.log(format!("— {name}"));
        let edited = INST_INLINEE_BASELINE.replace("return 8400;", "return 9400;");
        if self
            .apply_texts(
                name,
                &[(INST_INLINEE_FILE, edited.as_str())],
                &[(INST_INLINEE_FILE, INST_INLINEE_BASELINE)],
            )
            .await
            .is_some()
        {
            self.expect_release_immediate_output(
                name,
                "return LocusSelfTestInstInlineCaller.Run();",
                "9408", // 9400 + 8 — instance self-shim delivers the new body
            )
            .await;
        }
        let _ = self
            .apply_texts(
                "R14 restore instance inline corpus",
                &[(INST_INLINEE_FILE, INST_INLINEE_BASELINE)],
                &[(INST_INLINEE_FILE, INST_INLINEE_BASELINE)],
            )
            .await;
    }

    /// Phase D rollout measurement — A/B the experimental inline force-evaluation
    /// to quantify its over-refresh cost on this runtime. The SAME edit is applied
    /// with the flag OFF then ON; the delta in reported high-confidence (StubInlined)
    /// classifications is what force-evaluation ADDS — methods the static heuristic
    /// would have missed and that now drive an extra caller refresh. On this Mono
    /// the delta is expected to be ~0 (Gate A: Mono's real inline gate ≈ the ≤20-IL
    /// heuristic, and a changed method whose caller already ran has its bit set so
    /// the stub never builds), which is itself the data point that makes default-on
    /// cost-free here. Soft pass on the measurement; the behavioral read of the
    /// refreshed caller IS asserted on the ON leg. Restores the PRIOR flag value
    /// (default-agnostic) and the corpus afterward.
    async fn run_inline_force_evaluate_check(&mut self) {
        let name = "PD inline force-evaluate A/B";
        self.log("Phase D — inline force-evaluate A/B (rollout over-refresh measurement)");
        let (inlining_active, detail) = self.inlining_active().await;
        self.log(format!(
            "  runtime inlining {} [{detail}]",
            if inlining_active {
                "ACTIVE"
            } else {
                "INACTIVE"
            }
        ));
        let prior = crate::unity_hotreload::inline_force_evaluate_enabled();
        let edited = ADVERSARIAL_BASELINE.replace("return 5005;", "return 5115;");

        // OFF leg — baseline classification without force-evaluation.
        crate::unity_hotreload::set_inline_force_evaluate_enabled(false);
        let off_high = self
            .apply_texts(
                "PD off-leg",
                &[(ADVERSARIAL_FILE, edited.as_str())],
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
            )
            .await
            .map(|summary| summary.matches("(high-confidence)").count())
            .unwrap_or(0);
        let _ = self
            .apply_texts(
                "PD off-leg restore",
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
            )
            .await;

        // ON leg — force-evaluation enabled.
        crate::unity_hotreload::set_inline_force_evaluate_enabled(true);
        let on_summary = self
            .apply_texts(
                "PD on-leg",
                &[(ADVERSARIAL_FILE, edited.as_str())],
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
            )
            .await;
        if let Some(summary) = &on_summary {
            let on_high = summary.matches("(high-confidence)").count();
            self.log(format!(
                "  over-refresh delta (force-eval-attributable StubInlined): off={off_high} on={on_high} → +{}",
                on_high.saturating_sub(off_high)
            ));
            self.pass(
                name,
                format!(
                    "A/B measured; force-eval added {} high-confidence classification(s) over the heuristic",
                    on_high.saturating_sub(off_high)
                ),
            );
            // Convergence must hold with the flag on, inline verdict notwithstanding.
            self.expect_release_immediate_output(
                name,
                "return LocusSelfTestAdversarial.CallInlined();",
                "5115",
            )
            .await;
        }

        // Restore the prior (config-default) flag value, not a hardcoded one, so the
        // remaining phases run under the shipped default.
        crate::unity_hotreload::set_inline_force_evaluate_enabled(prior);
        let _ = self
            .apply_texts(
                "PD restore adversarial corpus",
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
                &[(ADVERSARIAL_FILE, ADVERSARIAL_BASELINE)],
            )
            .await;
    }

    /// Drive the editor to Release-EFFECTIVE inlining and verify it at runtime.
    ///
    /// The plain `set_code_optimization(release)` at startup has been observed not
    /// to take JIT effect — the canary shows `inlining_active=no` even in edit
    /// mode after the baseline reload (no debugger attached), so either the
    /// setting reverts across the reload or the release recompile was superseded.
    /// This re-asserts release and forces a recompile+reload to apply it, logging
    /// the codeOptimization setting and the runtime inlining verdict before and
    /// after so a stuck-Debug session is fully diagnosed. Returns whether inlining
    /// is active afterward.
    ///
    /// MUST run before the edit-mode patch tests (E01/E02): its recompile would
    /// otherwise revert E01's live editor patch before E02 asserts it survives
    /// play-enter.
    async fn ensure_release_effective(&mut self) -> bool {
        self.log("Ensuring Release-effective inlining (runtime-verified)...");
        let (active_before, detail_before) = self.inlining_active().await;
        let (_, setting_before) = coordinator::detect_code_optimization(&self.project).await;
        self.log(format!(
            "  before: codeOptimization setting={}, inlining_active={} [{detail_before}]",
            setting_before.as_deref().unwrap_or("unknown"),
            active_before
        ));
        if active_before {
            self.log("  already Release-effective; no recompile needed");
            return true;
        }

        match self
            .set_code_optimization_retrying("release", "Release-effective reassert")
            .await
        {
            Ok(value) => self.log(format!("  re-asserted codeOptimization → {value}")),
            Err(error) => self.log(format!("  re-assert failed: {error}")),
        }
        // Force a recompile + domain reload so the release setting is applied to
        // the loaded/JITed assemblies (the set alone only schedules it).
        match crate::unity_bridge::recompile_and_wait(&self.project).await {
            Ok(_) => self.log("  forced recompile+reload complete"),
            Err(error) => self.log(format!("  forced recompile failed: {error}")),
        }

        let (active_after, detail_after) = self.inlining_active().await;
        let (_, setting_after) = coordinator::detect_code_optimization(&self.project).await;
        self.log(format!(
            "  after: codeOptimization setting={}, inlining_active={} [{detail_after}]",
            setting_after.as_deref().unwrap_or("unknown"),
            active_after
        ));
        if !active_after {
            self.log(
                "  *** STILL Debug-effective after re-assert+recompile (no debugger) — \
                 inline coverage is unavailable this session; the Release-inline tests will \
                 soft-skip their inline-detection asserts. Likely codeOptimization not \
                 persisting across reloads or a Mono optimization config; needs deeper fix. ***",
            );
        }
        active_after
    }

    /// Phase A — inline-risk force-evaluation probes (DIAGNOSTIC ONLY).
    ///
    /// Sends a plugin-local command that, for a handful of method shapes, reads
    /// Mono's inline_info/inline_failure bits, force-JITs a synthetic caller stub
    /// (so Mono's inliner evaluates the callee at compile time), then re-reads the
    /// bits. It answers whether we can move the inline bit from the outside, what
    /// a refused/oversized method does, whether force-JITing runs a callee's
    /// static cctor, and which JIT-forcing API is stable — the data that gates
    /// wiring force-evaluation into IsMethodInlined (Phase B).
    ///
    /// The probe corpus lives in the plugin and is never wired into the hot-patch
    /// decision path, so this phase asserts nothing: it logs the report verbatim
    /// and records a soft pass on a successful round-trip. The probe is read
    /// cleanest on the first run after a domain reload (the callee inline bit is
    /// sticky for the domain lifetime).
    async fn run_inline_probes(&mut self) {
        let name = "PA inline-risk probes";
        self.log("Phase A — inline-risk force-evaluation probes (diagnostic, no assertions)");
        // Log the codeOptimization SETTING as seen right here (edit mode, after the
        // baseline reload). If this reads debug, the release set reverted across a
        // reload; if release while the probe below still shows inlining_active=no,
        // the setting holds but the JIT runs Debug-effective (debugger agent).
        let (_, setting_now) = coordinator::detect_code_optimization(&self.project).await;
        self.log(format!(
            "  codeOptimization setting at probe point (edit mode): {}",
            setting_now.as_deref().unwrap_or("unknown")
        ));
        match crate::unity_bridge::send_message_with_timeout(
            &self.project,
            "hot_reload_inline_probe",
            "",
            Duration::from_secs(30),
        )
        .await
        {
            Ok(resp) if resp.ok => {
                let report = resp.message.unwrap_or_default();
                for line in report.lines() {
                    self.log(format!("  {line}"));
                }
                self.pass(name, "probes ran; see report lines above");
            }
            Ok(resp) => {
                let error = resp
                    .error
                    .unwrap_or_else(|| "inline probe failed".to_string());
                if error.starts_with("unknown message type") {
                    self.log("  note: this Unity plugin predates the inline probe; skipping (rebuild the plugin to collect data)");
                } else {
                    self.fail(name, error);
                }
            }
            Err(error) => self.fail(name, squash(&error)),
        }
    }

    /// Phase 2c — adversarial C#-syntax cases that pin tricky resolver /
    /// inlining behaviour. They catch regressions and document the edges:
    ///   A1 — overloads distinct only by parameter NAMESPACE,
    ///   A2 — overloads distinct only by GENERIC ARGUMENT.
    ///        Both collapse to one reflection simple param name, so the coarse
    ///        identity is ambiguous; they resolve HOT via the enriched
    ///        per-parameter signature (namespace + closed generic argument)
    ///        the desktop now sends alongside (`param_type_sigs`).
    ///   A3 — an [AggressiveInlining] method body edit. Under the suite's
    ///        forced Release the apply reports it inlined and converges via
    ///        recompile (the behavioral assert tolerates the inline fallback).
    ///   A4 — a default parameter VALUE change. The caller is co-located in this
    ///        file and re-emitted with the new default; a caller outside the
    ///        batch would keep the baked-in old value.
    ///
    /// All four edit a dedicated `LocusSelfTestAdversarial` type, so a failure
    /// here cannot poison the other phases. Once pinned as known gaps and routed
    /// to the diagnostic tally; now confirmed HOT on Unity and promoted into the
    /// real suite (a regression here legitimately reddens the suite again).
    async fn run_adversarial_tests(&mut self) {
        self.log("Phase 2c — adversarial C# syntax edge cases");
        let mut adv = ADVERSARIAL_BASELINE.to_string();

        // A1 — body edit of an overload distinct only by parameter namespace.
        if self
            .step_file(
                "A1 namespace-distinct overload",
                ADVERSARIAL_FILE,
                &mut adv,
                |s| {
                    swap(
                        s,
                        "public static int ProbeNs(LocusSelfTestAdvA.Tag t) { return 1001; }",
                        "public static int ProbeNs(LocusSelfTestAdvA.Tag t) { return 4221; }",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "A1 namespace-distinct overload",
                "return LocusSelfTestAdversarial.ProbeNs(default(LocusSelfTestAdvA.Tag));",
                "4221",
            )
            .await;
        }

        // A2 — body edit of an overload distinct only by generic argument.
        if self
            .step_file(
                "A2 generic-arg-distinct overload",
                ADVERSARIAL_FILE,
                &mut adv,
                |s| {
                    swap(
                        s,
                        "public static int ProbeGen(System.Collections.Generic.List<int> a) { return 3003; }",
                        "public static int ProbeGen(System.Collections.Generic.List<int> a) { return 4221; }",
                    )
                },
            )
            .await
            .is_some()
        {
            self.expect_output(
                "A2 generic-arg-distinct overload",
                "return LocusSelfTestAdversarial.ProbeGen(new System.Collections.Generic.List<int>());",
                "4221",
            )
            .await;
        }

        // A3 — body edit of an aggressively-inlined method (call-site bypass).
        if self
            .step_file(
                "A3 aggressive-inlining body edit",
                ADVERSARIAL_FILE,
                &mut adv,
                |s| swap(s, "return 5005;", "return 6116;"),
            )
            .await
            .is_some()
        {
            self.expect_output(
                "A3 aggressive-inlining body edit",
                "return LocusSelfTestAdversarial.CallInlined();",
                "6116",
            )
            .await;
        }

        // A4 — default parameter value change (callers baked the old default).
        if self
            .step_file(
                "A4 default parameter value change",
                ADVERSARIAL_FILE,
                &mut adv,
                |s| swap(s, "int y = 1000", "int y = 2000"),
            )
            .await
            .is_some()
        {
            self.expect_output(
                "A4 default parameter value change",
                "return LocusSelfTestAdversarial.CallDefaulted();",
                "9000",
            )
            .await;
        }
    }

    async fn run_negative_tests(&mut self) {
        self.log("Phase 5/7 — cold classifications must carry the precise reason");

        let neg = NEG_BASELINE.to_string();

        // N01 — generic method bodies decompose into remove+add (B1), so
        // the M3 caller scan gates them: with the compiled caller (SUBJECT's
        // Relay) OUTSIDE the batch the verdict is cold and names that file.
        let name = "N01 generic body uncovered caller";
        self.log(format!("— {name}"));
        let mut text = neg.clone();
        if swap(
            &mut text,
            "public T Echo<T>(T value) { return value; }",
            "public T Echo<T>(T value) { var copy = value; return copy; }",
        )
        .is_ok()
        {
            match self.write_tracked(NEG_FILE, &text).await {
                Ok(()) => {
                    let verdict = self.hot_reload(Some(vec![NEG_FILE.to_string()])).await;
                    let vtext = match &verdict {
                        Ok(summary) => summary.clone(),
                        Err(error) => error.clone(),
                    };
                    if vtext.contains("Hot reload not applicable")
                        && vtext.contains("generic method body changed")
                        && vtext.contains("LocusSelfTestSubject.cs")
                    {
                        self.pass(name, "cold names the uncovered caller file");
                    } else {
                        self.fail(
                            name,
                            format!(
                                "expected cold naming the caller file, got: {}",
                                squash(&vtext)
                            ),
                        );
                    }
                    if let Err(error) = self.write_tracked(NEG_FILE, &neg).await {
                        self.fail(name, format!("restore after {name} failed: {error}"));
                    }
                }
                Err(error) => self.fail(name, error),
            }
        }

        // N02 — turning a method virtual is an added virtual slot.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "public int Solid() { return 1; }",
            "public virtual int Solid() { return 1; }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N02 virtual keyword added",
                NEG_FILE,
                &text,
                "virtual member added",
                &neg,
            )
            .await;
        }

        // N03 — attribute edits are immutable metadata.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Plain() { return Limit; }",
            "    [System.Obsolete]\n    public int Plain() { return Limit; }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N03 attribute added",
                NEG_FILE,
                &text,
                "member declaration changed",
                &neg,
            )
            .await;
        }

        // N04 — consts are inlined at use sites.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "public const int Limit = 3;",
            "public const int Limit = 4;",
        )
        .is_ok()
        {
            self.expect_cold(
                "N04 const value changed",
                NEG_FILE,
                &text,
                "const or static initializer changed",
                &neg,
            )
            .await;
        }

        // N05 — VIRTUAL property additions stay cold (plain property/
        // indexer/event additions are hot since B2 — P38/P39/P40 assert
        // that): a new virtual slot cannot be reproduced by a static shim.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { return 1; }\n    public virtual int NewProp { get { return 1; } }\n",
        )
        .is_ok()
        {
            self.expect_cold("N05 virtual property added", NEG_FILE, &text, "virtual member added", &neg).await;
        }

        // N06 — finalizer surface.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { return 1; }\n    ~LocusSelfTestNegative() { }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N06 finalizer added",
                NEG_FILE,
                &text,
                "finalizer added",
                &neg,
            )
            .await;
        }

        // N07 — unsupported Unity message names still stay cold. The supported
        // add-after-load families (PlayerLoop pump + component proxy: physics,
        // GUI/mouse, animator/particle/controller, lifecycle catch-ups) are
        // covered positively in P46. OnBecameVisible depends on Camera/Renderer
        // visibility timing and has no driver, so it stays cold.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n}\n",
            "    public int Solid() { return 1; }\n    void OnBecameVisible() { }\n}\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N07 unsupported Unity message added",
                NEG_FILE,
                &text,
                "new Unity message method",
                &neg,
            )
            .await;
        }

        // N08 — enum VALUE edits (only appends are hot).
        let mut text = neg.clone();
        if swap(&mut text, "Y = 2", "Y = 9").is_ok() {
            self.expect_cold(
                "N08 enum value changed",
                NEG_FILE,
                &text,
                "enum changed",
                &neg,
            )
            .await;
        }

        // N09 — struct field layout.
        let struct_text = self.negative_ledgers.struct_text.clone();
        let mut text = struct_text.clone();
        if swap(
            &mut text,
            "    public int Value;\n",
            "    public int Value;\n    public int Extra;\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N09 struct field added",
                STRUCT_FILE,
                &text,
                "struct field layout changed",
                &struct_text,
            )
            .await;
        }

        // N10 — constructor surface. The anchor chain tolerates P19/P32
        // having failed at any point.
        let ctor_text = self.negative_ledgers.ctor_text.clone();
        let mut text = ctor_text.clone();
        let mut anchored = Err(String::new());
        for body in ["Seed = 5776;", "Seed = 5775;", "Seed = 1;"] {
            text = ctor_text.clone();
            let line = format!("    public LocusSelfTestCtor() {{ {body} }}\n");
            anchored = swap(
                &mut text,
                &line,
                &format!("{line}    public LocusSelfTestCtor(int seed) {{ Seed = seed; }}\n"),
            );
            if anchored.is_ok() {
                break;
            }
        }
        if anchored.is_ok() {
            self.expect_cold(
                "N10 constructor added",
                CTOR_FILE,
                &text,
                "constructor added",
                &ctor_text,
            )
            .await;
        }

        // N11 — any interface change.
        let iface_text = self.negative_ledgers.iface_text.clone();
        let mut text = iface_text.clone();
        if swap(
            &mut text,
            "    int Plan();",
            "    int Plan();\n    int Extra();",
        )
        .is_ok()
        {
            self.expect_cold(
                "N11 interface member added",
                IFACE_FILE,
                &text,
                "interface changed",
                &iface_text,
            )
            .await;
        }

        // N12' — added member whose SIGNATURE names a non-public type: C2′a
        // relaxes BODY access only (the C0 matrix has no declaration-site
        // cell yet), so the public shim still cannot carry the internal
        // parameter type. (The old N12 — an added member touching private
        // STATE — is the positive P34 now that caps gate the body.)
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Hidden() { return _hidden; }\n",
            "    public int Hidden() { return _hidden; }\n    public int Leak(LocusSelfTestNegHidden arg) { return 1; }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N12' added member with non-public signature",
                NEG_FILE,
                &text,
                "signature-level non-public type",
                &neg,
            )
            .await;
        }

        // N13 — conversion returning the declaring type.
        let struct_text = self.negative_ledgers.struct_text.clone();
        let mut text = struct_text.clone();
        if swap(&mut text, "r.Value = v;", "r.Value = v + 1;").is_ok() {
            self.expect_cold(
                "N13 conversion to declaring type changed",
                STRUCT_FILE,
                &text,
                "conversion to the declaring type changed",
                &struct_text,
            )
            .await;
        }

        // N15 — generic-TYPE constructor bodies stay cold (B1 only covers
        // plain method bodies; ctors of generic types cannot detour).
        let mut text = neg.clone();
        if swap(
            &mut text,
            "public LocusSelfTestNegGeneric() { Marker = 1; }",
            "public LocusSelfTestNegGeneric() { Marker = 2; }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N15 generic type ctor body",
                NEG_FILE,
                &text,
                "generic type constructor changed",
                &neg,
            )
            .await;
        }

        // N16 — base-list change is type surface.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "public class LocusSelfTestNegative\n{",
            "public class LocusSelfTestNegative : System.Object\n{",
        )
        .is_ok()
        {
            self.expect_cold(
                "N16 base list changed",
                NEG_FILE,
                &text,
                "type declaration changed",
                &neg,
            )
            .await;
        }

        // N17 — partial body edits are HOT since B6 (P44); the negative is
        // the v1 boundary: FIELD layout changes on a partial type stay cold
        // (initializers/ctors can live in other parts). The mutation builds
        // on the P44 ledger so the cumulative model holds.
        let partial_a_text = self.negative_ledgers.partial_a_text.clone();
        let mut text = partial_a_text.clone();
        if swap(
            &mut text,
            "    private int _alpha = 30;\n",
            "    private int _alpha = 30;\n    private int _extraCold;\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N17 partial type field added",
                PARTIAL_A_FILE,
                &text,
                "partial type field layout changed",
                &partial_a_text,
            )
            .await;
        }

        // N18 — delegate declaration changes alter compiled signatures.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "public delegate int LocusSelfTestNegDel(int x);",
            "public delegate int LocusSelfTestNegDel(int x, int y);",
        )
        .is_ok()
        {
            self.expect_cold(
                "N18 delegate signature changed",
                NEG_FILE,
                &text,
                "delegate declarations changed",
                &neg,
            )
            .await;
        }

        // N19 — finalizer BODY edits (finalizers never detour).
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    ~LocusSelfTestNegFin() { }",
            "    ~LocusSelfTestNegFin() { System.GC.KeepAlive(this); }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N19 finalizer body changed",
                NEG_FILE,
                &text,
                "finalizer changed",
                &neg,
            )
            .await;
        }

        // N21 — field-like events need compiler-generated accessors plus a
        // backing delegate field in the original layout (B2 covers
        // accessor-style events only — P39b).
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { return 1; }\n    public event System.Action Overflow;\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N21 field-like event added",
                NEG_FILE,
                &text,
                "field-like event added",
                &neg,
            )
            .await;
        }

        // N22 — .asmdef edits are never hot-reload inputs: the coordinator
        // tracks .cs sources only, so an asmdef change cannot enter a batch
        // and the assembly-graph restructure needs unity_recompile. The
        // verdict is the untracked-input guidance, not a patch attempt.
        // (The file is restored without an import, so Unity never sees the
        // transient text.)
        let name = "N22 asmdef edit stays untracked";
        self.log(format!("— {name}"));
        let mut tweaked = LIB_ASMDEF_BASELINE.to_string();
        if swap(
            &mut tweaked,
            "\"rootNamespace\": \"\"",
            "\"rootNamespace\": \"LocusSelfTestTweak\"",
        )
        .is_ok()
        {
            match self.write_tracked(LIB_ASMDEF_FILE, &tweaked).await {
                Ok(()) => {
                    match self
                        .hot_reload(Some(vec![LIB_ASMDEF_FILE.to_string()]))
                        .await
                    {
                        Ok(summary) if summary.contains("No pending .cs edits") => {
                            self.pass(
                                name,
                                "asmdef edit never enters a hot batch (unity_recompile path)",
                            );
                        }
                        Ok(summary) => self.fail(
                            name,
                            format!("expected the untracked verdict, got: {}", squash(&summary)),
                        ),
                        Err(error) => self.fail(name, squash(&error)),
                    }
                    if let Err(error) = self
                        .write_tracked(LIB_ASMDEF_FILE, LIB_ASMDEF_BASELINE)
                        .await
                    {
                        self.fail(name, format!("restore failed: {error}"));
                    }
                }
                Err(error) => self.fail(name, error),
            }
        }

        // N23 — full added properties cannot preserve ++/-- through the
        // accessor pair without changing final compile semantics.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { Level++; return _hidden; }\n    public int Level { get { return _hidden; } set { _hidden = value; } }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N23 full added property increment",
                NEG_FILE,
                &text,
                "increment/decrement of an added property",
                &neg,
            )
            .await;
        }

        // N24 — ??= needs set-skip semantics; the B2 accessor lowering keeps
        // this shape cold instead of evaluating a setter when it should not.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { Tag ??= \"x\"; return _hidden; }\n    public string Tag { get { return null; } set { _hidden = value == null ? 0 : 1; } }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N24 added property coalesce assignment",
                NEG_FILE,
                &text,
                "set-skip semantics",
                &neg,
            )
            .await;
        }

        // N25 — even though an auto-property store is lvalue-shaped, ref/out
        // to a property would fail after the real compile, so the hot path
        // matches the eventual source semantics and stays cold.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { Bump(ref Cargo); return Cargo; }\n    public int Cargo { get; set; }\n    public static void Bump(ref int v) { v += 1; }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N25 added auto-property ref argument",
                NEG_FILE,
                &text,
                "ref/out",
                &neg,
            )
            .await;
        }

        // N26 — compound indexer lowering repeats index arguments; method
        // calls are deliberately rejected because they would run twice.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { this[Poke()] += 1; return _hidden; }\n    public int Poke() { return 1; }\n    public int this[int i] { get { return _hidden + i; } set { _hidden = value; } }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N26 added indexer non-repeatable compound",
                NEG_FILE,
                &text,
                "non-trivial index arguments",
                &neg,
            )
            .await;
        }

        // ── extra cold surface (N30+) ────────────────────────────────────
        // Each clones a dedicated baseline and mutates one type, so exactly
        // one rejection reason fires. None of these files are touched by the
        // positive phase, so the baseline IS the restore text (no ledger).

        // N30 — class↔struct is a metadata type-kind change.
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "public class LocusSelfTestKind",
            "public struct LocusSelfTestKind",
        )
        .is_ok()
        {
            self.expect_cold(
                "N30 type kind changed (class to struct)",
                COLD_FILE,
                &text,
                "type kind changed",
                COLD_BASELINE,
            )
            .await;
        }

        // N31 — the static constructor already ran in the loaded domain.
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "static LocusSelfTestStaticCtor() { Counter = 1; }",
            "static LocusSelfTestStaticCtor() { Counter = 2; }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N31 static constructor changed",
                COLD_FILE,
                &text,
                "static constructor changed",
                COLD_BASELINE,
            )
            .await;
        }

        // N32 — explicit interface implementations dispatch through the
        // interface map; a detour cannot reach them.
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "int ILocusSelfTestExplicit.Plan() { return 1; }",
            "int ILocusSelfTestExplicit.Plan() { return 2; }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N32 explicit interface impl changed",
                COLD_FILE,
                &text,
                "explicit interface implementation changed",
                COLD_BASELINE,
            )
            .await;
        }

        // N33 — appended enum members must carry an integer LITERAL; an
        // expression cannot be resolved without a real compile.
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "public enum LocusSelfTestColdEnum { P = 1, Q = 2 }",
            "public enum LocusSelfTestColdEnum { P = 1, Q = 2, R = 1 + 2 }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N33 enum appended non-literal value",
                COLD_FILE,
                &text,
                "enum member value not resolvable",
                COLD_BASELINE,
            )
            .await;
        }

        // N34 — an appended value that collides with an existing member is
        // ambiguous at the inlined use sites.
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "public enum LocusSelfTestColdEnum { P = 1, Q = 2 }",
            "public enum LocusSelfTestColdEnum { P = 1, Q = 2, R = 2 }",
        )
        .is_ok()
        {
            self.expect_cold(
                "N34 enum appended conflicting value",
                COLD_FILE,
                &text,
                "enum member value conflicts",
                COLD_BASELINE,
            )
            .await;
        }

        // N35 — a removed enum cannot be verified (values are inlined).
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "public enum LocusSelfTestColdEnum { P = 1, Q = 2 }\n\n",
            "",
        )
        .is_ok()
        {
            self.expect_cold(
                "N35 enum removed",
                COLD_FILE,
                &text,
                "enum removed",
                COLD_BASELINE,
            )
            .await;
        }

        // N36 — a removed const was inlined at every use site.
        let mut text = COLD_BASELINE.to_string();
        if swap(&mut text, "    public const int Cap = 5;\n", "").is_ok() {
            self.expect_cold(
                "N36 const removed",
                COLD_FILE,
                &text,
                "const removed",
                COLD_BASELINE,
            )
            .await;
        }

        // N37 — field modifiers live in immutable metadata.
        let mut text = COLD_BASELINE.to_string();
        if swap(&mut text, "public int Field;", "public readonly int Field;").is_ok() {
            self.expect_cold(
                "N37 field modifiers changed",
                COLD_FILE,
                &text,
                "field attributes or modifiers changed",
                COLD_BASELINE,
            )
            .await;
        }

        // N38 — constructor surface changes (the parameterless ctor stays).
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "    public LocusSelfTestCtorDrop(int seed) { Seed = seed; }\n",
            "",
        )
        .is_ok()
        {
            self.expect_cold(
                "N38 constructor removed",
                COLD_FILE,
                &text,
                "constructor removed",
                COLD_BASELINE,
            )
            .await;
        }

        // N39 — finalizer removal (N06/N19 cover added/changed).
        let mut text = COLD_BASELINE.to_string();
        if swap(&mut text, "    ~LocusSelfTestFinDrop() { }\n", "").is_ok() {
            self.expect_cold(
                "N39 finalizer removed",
                COLD_FILE,
                &text,
                "finalizer removed",
                COLD_BASELINE,
            )
            .await;
        }

        // N40 — adding an operator: its call sites live outside the batch.
        let mut text = COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "    public int Mark() { return Value; }\n",
            "    public int Mark() { return Value; }\n\n    public static LocusSelfTestOpHost operator +(LocusSelfTestOpHost a, LocusSelfTestOpHost b) { return a; }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N40 operator added",
                COLD_FILE,
                &text,
                "member kind addition not hot-reloadable",
                COLD_BASELINE,
            )
            .await;
        }

        // N41 — record types are rejected on PRESENCE. Created fresh and
        // deleted so the C# 9 syntax never reaches a real compile (a cold
        // verdict returns before the sidecar would build it).
        let name = "N41 record type rejected";
        self.log(format!("— {name}"));
        match self.write_tracked(RECORD_FILE, RECORD_BASELINE).await {
            Ok(()) => {
                let verdict = self.hot_reload(Some(vec![RECORD_FILE.to_string()])).await;
                let vtext = match &verdict {
                    Ok(summary) => summary.clone(),
                    Err(error) => error.clone(),
                };
                if vtext.contains("Hot reload not applicable")
                    && vtext.contains("record types are not hot-reloadable")
                {
                    self.pass(name, "record classified cold on presence");
                } else {
                    self.fail(
                        name,
                        format!("expected the record cold verdict, got: {}", squash(&vtext)),
                    );
                }
                if let Err(error) = self.delete_tracked(RECORD_FILE).await {
                    self.fail(name, format!("cleanup of {RECORD_FILE} failed: {error}"));
                }
            }
            Err(error) => self.fail(name, error),
        }

        // N42–N46 — using-rehook (M6) gates: toggling a using forces the
        // whole-file re-detour, which fails closed on the one member each
        // file holds that cannot be re-detoured. (C74 Burst needs the Burst
        // package; an "unsupported operator" is unreachable — every
        // overloadable C# operator has a metadata name; a default-interface-
        // method gate would need runtime DIM support. All three are left to
        // the HotDiff unit tests.)
        let mut text = USE_CONST_BASELINE.to_string();
        if swap(
            &mut text,
            "public class LocusSelfTestUseConst",
            "using System.Text;\n\npublic class LocusSelfTestUseConst",
        )
        .is_ok()
        {
            self.expect_cold(
                "N42 using-rehook gate: non-literal const",
                USE_CONST_FILE,
                &text,
                "a non-literal const value is inlined under the old bindings",
                USE_CONST_BASELINE,
            )
            .await;
        }

        let mut text = USE_STATIC_BASELINE.to_string();
        if swap(
            &mut text,
            "public class LocusSelfTestUseStatic",
            "using System.Text;\n\npublic class LocusSelfTestUseStatic",
        )
        .is_ok()
        {
            self.expect_cold(
                "N43 using-rehook gate: non-literal static initializer",
                USE_STATIC_FILE,
                &text,
                "a non-literal static initializer already ran under the old bindings",
                USE_STATIC_BASELINE,
            )
            .await;
        }

        let mut text = USE_GENERIC_BASELINE.to_string();
        if swap(
            &mut text,
            "public class LocusSelfTestUseGeneric",
            "using System.Text;\n\npublic class LocusSelfTestUseGeneric",
        )
        .is_ok()
        {
            self.expect_cold(
                "N44 using-rehook gate: generic member",
                USE_GENERIC_FILE,
                &text,
                "generic members cannot be re-detoured",
                USE_GENERIC_BASELINE,
            )
            .await;
        }

        let mut text = USE_EXPLICIT_BASELINE.to_string();
        if swap(
            &mut text,
            "public interface ILocusSelfTestUseExpl",
            "using System.Text;\n\npublic interface ILocusSelfTestUseExpl",
        )
        .is_ok()
        {
            self.expect_cold(
                "N45 using-rehook gate: explicit interface impl",
                USE_EXPLICIT_FILE,
                &text,
                "an explicit interface implementation cannot be re-detoured",
                USE_EXPLICIT_BASELINE,
            )
            .await;
        }

        let mut text = USE_FINALIZER_BASELINE.to_string();
        if swap(
            &mut text,
            "public class LocusSelfTestUseFin",
            "using System.Text;\n\npublic class LocusSelfTestUseFin",
        )
        .is_ok()
        {
            self.expect_cold(
                "N46 using-rehook gate: finalizer",
                USE_FINALIZER_FILE,
                &text,
                "a finalizer cannot be re-detoured",
                USE_FINALIZER_BASELINE,
            )
            .await;
        }

        // N47–N52 — partial-type cold boundaries (B6 v1). Body edits are HOT
        // (P44/P45); these are the structural shapes the per-file diff cannot
        // reason about across parts.

        // N47 — a vanished partial part is not a type deletion (other parts
        // may live elsewhere). Pass a replacement type, not a swap.
        self.expect_cold(
            "N47 partial part removed",
            PARTIAL_COLD_FILE,
            "public class LocusSelfTestPartialColdSpacer\n{\n    public int Mark() { return 1; }\n}\n",
            "partial type part removed",
            PARTIAL_COLD_BASELINE,
        )
        .await;

        // N48 — a new partial declaration is ambiguous from one file.
        let mut text = PARTIAL_COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "    public int Read() { return _value; }\n}\n",
            "    public int Read() { return _value; }\n}\n\npublic partial class LocusSelfTestPartialColdNew\n{\n}\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N48 new partial type declaration",
                PARTIAL_COLD_FILE,
                &text,
                "new partial type declaration",
                PARTIAL_COLD_BASELINE,
            )
            .await;
        }

        // N49 — the whole-file re-detour cannot cover a partial type's other
        // parts, so a using change in a partial file fails closed.
        let mut text = PARTIAL_COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "public partial class LocusSelfTestPartialCold",
            "using System.Text;\n\npublic partial class LocusSelfTestPartialCold",
        )
        .is_ok()
        {
            self.expect_cold(
                "N49 using changed in a partial file",
                PARTIAL_COLD_FILE,
                &text,
                "using directives changed in a file with a partial type",
                PARTIAL_COLD_BASELINE,
            )
            .await;
        }

        // N50 — adding/dropping a part within one file changes how the
        // compiler interleaves members.
        let mut text = PARTIAL_COUNT_BASELINE.to_string();
        if swap(
            &mut text,
            "\npublic partial class LocusSelfTestPartialCount\n{\n    public int B() { return 2; }\n}\n",
            "",
        )
        .is_ok()
        {
            self.expect_cold(
                "N50 partial part count changed in file",
                PARTIAL_COUNT_FILE,
                &text,
                "partial type part count changed in file",
                PARTIAL_COUNT_BASELINE,
            )
            .await;
        }

        // N51 — a partial type's initializers compile into ctors that may
        // live in other parts.
        let mut text = PARTIAL_COLD_BASELINE.to_string();
        if swap(&mut text, "private int _value = 10;", "private int _value = 20;").is_ok() {
            self.expect_cold(
                "N51 partial type initializer changed",
                PARTIAL_COLD_FILE,
                &text,
                "partial type instance initializer changed",
                PARTIAL_COLD_BASELINE,
            )
            .await;
        }

        // N52 — a partial method whose defining and implementing halves land
        // in the same (merged) file cannot be paired.
        let mut text = PARTIAL_COLD_BASELINE.to_string();
        if swap(
            &mut text,
            "    partial void Hook();\n",
            "    partial void Hook();\n    partial void Hook() { }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N52 partial method declared twice in this file",
                PARTIAL_COLD_FILE,
                &text,
                "partial method declared twice in this file",
                PARTIAL_COLD_BASELINE,
            )
            .await;
        }

        // Intentional integration-coverage gaps (covered by the HotDiff unit
        // tests, not reproducible here): [BurstCompile] surface needs the Burst
        // package; an "unsupported operator" is unreachable (every overloadable
        // C# operator has a metadata name); the default-interface-method gate
        // needs runtime DIM support the baseline corpus must not assume; and
        // explicit-interface ADD/REMOVE cannot be isolated without first
        // tripping another reason (a same-file contract change).
        self.log(
            "  note: Burst / unsupported-operator / default-interface-method gate variants are \
             covered by HotDiff unit tests, not this suite",
        );

        // N20 — an added member with NO reachable caller compiles to a shim
        // but detours nothing: the verdict must say the addition is parked,
        // not pretend nothing changed.
        let name = "N20 shim-only addition verdict";
        self.log(format!("— {name}"));
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { return 1; }\n    public int Lone() { return 6116; }\n",
        )
        .is_ok()
        {
            match self.write_tracked(NEG_FILE, &text).await {
                Ok(()) => {
                    match self.hot_reload(Some(vec![NEG_FILE.to_string()])).await {
                        Ok(summary) if summary.contains("No detourable change") => {
                            self.pass(name, "shim-only addition reported as parked new surface");
                        }
                        Ok(summary) => self.fail(
                            name,
                            format!("expected the shim-only verdict, got: {}", squash(&summary)),
                        ),
                        Err(error) => self.fail(name, squash(&error)),
                    }
                    if let Err(error) = self.write_tracked(NEG_FILE, &neg).await {
                        self.fail(name, format!("restore failed: {error}"));
                    }
                }
                Err(error) => self.fail(name, error),
            }
        }

        // N14 — accessibility WIDENING is a benign no-op (not cold, not a
        // patch): original metadata keeps working until a real compile.
        let name = "N14 accessibility widening (noop)";
        self.log(format!("— {name}"));
        let mut text = neg.clone();
        if swap(
            &mut text,
            "internal int Wide() { return 5; }",
            "public int Wide() { return 5; }",
        )
        .is_ok()
        {
            match self.write_tracked(NEG_FILE, &text).await {
                Ok(()) => {
                    match self.hot_reload(Some(vec![NEG_FILE.to_string()])).await {
                        Ok(summary) if summary.contains("No effective code change") => {
                            self.pass(name, "widening classified as a no-op");
                        }
                        Ok(summary) => self.fail(
                            name,
                            format!("expected a no-op verdict, got: {}", squash(&summary)),
                        ),
                        Err(error) => self.fail(name, squash(&error)),
                    }
                    if let Err(error) = self.write_tracked(NEG_FILE, &neg).await {
                        self.fail(name, format!("restore failed: {error}"));
                    }
                }
                Err(error) => self.fail(name, error),
            }
        }
    }

    async fn run_deletion_tests(&mut self, subject: &mut String, helper: &mut String) {
        self.log("Phase 6/7 — deletions");

        // D01 — deleting a Unity message method stops its behavior NOW
        // (empty-body stub detour). Anchor chain tolerates P01b's body edit
        // having failed.
        let name = "D01 Unity message deletion";
        if self
            .step_file(name, SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    void Update() { _ticks += Step(); UpdateBeats += 1; }\n",
                    "",
                )
                .or_else(|_| swap(s, "    void Update() { _ticks += Step(); }\n", ""))
            })
            .await
            .is_some()
        {
            match self.ticks_frozen().await {
                Ok(true) => self.pass(name, "tick counter froze after deleting Update"),
                Ok(false) => self.fail(name, "tick counter kept advancing after deleting Update"),
                Err(error) => self.fail(name, error),
            }
        }

        // D02 — non-auto property deletion (no callers): tombstone.
        let name = "D02 property deletion";
        if let Some(summary) = self
            .step_file(name, SUBJECT_FILE, subject, |s| {
                swap(s, "    public int Doomed { get { return 1; } }\n", "")
            })
            .await
        {
            self.pass(name, first_line(&summary));
        }

        // D03 — plain member deletion (no callers): tombstone. The anchor
        // chain tolerates P02/P02b having failed at any point.
        let name = "D03 method deletion";
        if let Some(summary) = self
            .step_file(name, SUBJECT_FILE, subject, |s| {
                swap(
                    s,
                    "    public async Task<int> Pulse() { await Task.Yield(); return 2112; }\n",
                    "",
                )
                .or_else(|_| {
                    swap(
                        s,
                        "    public async Task<int> Pulse() { await Task.Yield(); return 2002; }\n",
                        "",
                    )
                })
                .or_else(|_| {
                    swap(
                        s,
                        "    public Task<int> Pulse() { return Task.FromResult(2001); }\n",
                        "",
                    )
                })
            })
            .await
        {
            self.pass(name, first_line(&summary));
        }

        // D04 — using REMOVAL re-detours the whole file (M6), with behavior
        // intact afterwards. D03 normally deletes the only Task user first;
        // if it failed, drop the dangling Pulse here so the file still
        // compiles under the removed directive (no cascade).
        let name = "D04 using removed (file rehook)";
        if self
            .step_file(name, SUBJECT_FILE, subject, |s| {
                swap(s, "using System.Threading.Tasks;\n", "")?;
                // The added-member capability probes accumulate in the corpus;
                // P35a's VaultAsync is a SECOND Task<> user (async, needs the
                // directive). Drop it tolerantly so removing the using still
                // leaves a compilable file, then handle a Pulse that survived
                // D03 (the original lone Task user the cleanup was written for).
                let _ = swap_line(s, "    public async Task<int> VaultAsync()", "");
                if s.contains("Task<") {
                    swap_line(s, "    public async Task<int> Pulse()", "")
                        .or_else(|_| swap_line(s, "    public Task<int> Pulse()", ""))?;
                }
                Ok(())
            })
            .await
            .is_some()
        {
            self.expect_output(name, "return LocusSelfTestSubject.Instance.Step();", "8802")
                .await;
        }

        // D05 — whole-file deletion: first edit every compiled caller away
        // (line-anchored: earlier failures must not break the anchors), then
        // the helper file (and the type appended to it) can go.
        let name = "D05 file deletion";
        let stripped = self
            .step_file(
                "D05a helper callers edited away",
                SUBJECT_FILE,
                subject,
                |s| {
                    swap_line(
                        s,
                        "    public int Sum(int a)",
                        "    public int Sum(int a) { return a + a * 2 + 100; }",
                    )?;
                    swap_line(
                        s,
                        "    public int CallRenamed()",
                        "    public int CallRenamed() { return 7227; }",
                    )?;
                    swap_line(
                        s,
                        "    public int CallBump()",
                        "    public int CallBump() { return 9229; }",
                    )?;
                    swap_line(
                        s,
                        "    public int Probe()",
                        "    public int Probe() { return 0; }",
                    )
                },
            )
            .await;
        if stripped.is_some() {
            match self.delete_tracked(HELPER_FILE).await {
                Ok(()) => match self.hot_reload(None).await {
                    Ok(summary) => self.pass(name, first_line(&summary)),
                    Err(error) => {
                        self.fail(name, squash(&error));
                        // Put the helper back so the convergence compile has
                        // a consistent corpus on disk.
                        self.revert_files(&[(HELPER_FILE, helper.as_str())]).await;
                    }
                },
                Err(error) => self.fail(name, error),
            }
        }
    }

    async fn ticks_frozen(&self) -> Result<bool, String> {
        let before = self
            .execute("return LocusSelfTestSubject.Instance.Ticks;")
            .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let after = self
            .execute("return LocusSelfTestSubject.Instance.Ticks;")
            .await?;
        Ok(extract_int(&before) == extract_int(&after))
    }

    async fn finalize(&mut self) {
        self.log("Phase 7/7 — leaving play mode and converging");
        self.log("Requesting play mode exit...");
        match tokio::time::timeout(
            EXIT_PLAY_MODE_TIMEOUT,
            crate::unity_bridge::exit_play_mode(&self.project),
        )
        .await
        {
            Ok(Ok(())) => self.log("exit_play_mode completed"),
            Ok(Err(error)) => self.log(format!("exit_play_mode failed (continuing): {error}")),
            Err(_) => self.log(format!(
                "exit_play_mode timed out after {}ms (continuing)",
                EXIT_PLAY_MODE_TIMEOUT.as_millis()
            )),
        }
        if let Err(error) = self
            .wait_for_play_state(false, Duration::from_secs(60))
            .await
        {
            self.log(format!("warning: {error}"));
        }

        // H6 fires on the play-exit transition; wait for the convergence
        // recompile to clear the active patches.
        let converged = self.wait_for_convergence(Duration::from_secs(180)).await;
        match converged {
            Ok(()) => {
                self.pass(
                    "F01 auto-convergence",
                    "active patches cleared after leaving play mode",
                );
                if self.domain_reload_on_play == Some(false) {
                    // B5 — the trigger chain is a play-STATUS transition in
                    // the connection monitor plus a real recompile; neither
                    // leg needs a play-transition domain reload, and this
                    // run just demonstrated that in the no-reload branch.
                    self.log(
                        "  B5 note: convergence verified WITHOUT a play-transition domain reload \
                         (trigger = play-state flip; convergence = real recompile, domain-generation driven)",
                    );
                }
            }
            Err(error) => {
                self.log(format!(
                    "auto-convergence not observed ({error}); converging explicitly"
                ));
                match crate::unity_bridge::recompile_and_wait(&self.project).await {
                    Ok(_) => self.pass("F01 convergence (explicit)", "real recompile succeeded"),
                    Err(recompile_error) => self.fail("F01 convergence", recompile_error),
                }
            }
        }

        self.cleanup_corpus().await;
        self.restore_code_optimization().await;
    }

    async fn cleanup_corpus(&mut self) {
        // The corpus served its purpose. Deleting through the tracker keeps
        // the paths in the pending set, so the cleanup recompile forwards
        // them and the plugin refreshes the stale AssetDatabase entries away.
        self.log("Cleaning up the test corpus...");
        for relative in ALL_FILES {
            if let Err(error) = self.delete_tracked(relative).await {
                self.log(format!("cleanup: {error}"));
            }
        }
        let dir = self.absolute(TEST_DIR);
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let _ = tokio::fs::remove_file(self.absolute(&format!("{TEST_DIR}.meta"))).await;
        match crate::unity_bridge::recompile_and_wait(&self.project).await {
            Ok(_) => self.log("Cleanup recompile finished."),
            Err(error) => self.log(format!("cleanup recompile failed: {error}")),
        }
    }

    async fn restore_code_optimization(&mut self) {
        let Some(original) = self.original_code_optimization.take() else {
            return;
        };
        if original != "debug" && original != "release" {
            return;
        }
        if original == "release" {
            return;
        }
        match self
            .set_code_optimization_retrying(&original, "Code Optimization restore")
            .await
        {
            Ok(value) => {
                self.log(format!("Code Optimization restored to {value}."));
            }
            Err(error) => {
                self.log(format!("Code Optimization restore failed: {error}"));
            }
        }
    }

    async fn wait_for_convergence(&self, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        let mut next_progress_log = Duration::ZERO;
        loop {
            let elapsed = start.elapsed();
            let active = coordinator::project_active_patches(&self.project).await;
            if active == 0 {
                return Ok(());
            }
            if elapsed >= next_progress_log {
                self.log(format!(
                    "Waiting for convergence: {active} patch(es) active after {}s",
                    elapsed.as_secs()
                ));
                next_progress_log += Duration::from_secs(10);
            }
            if elapsed > timeout {
                return Err(format!(
                    "{active} patch(es) still active after {}s",
                    timeout.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn run(&mut self) {
        self.emit(None, false);
        self.log("Unity hot-reload self-test starting.");

        // Preconditions.
        if !super::is_enabled() || !crate::csharp_compile::is_enabled() {
            self.fail(
                "preconditions",
                "hot reload and the sidecar compiler must both be enabled",
            );
            self.emit(None, true);
            return;
        }
        let (connected, status, _) = crate::unity_bridge::query_unity_status(&self.project).await;
        if !connected {
            self.fail("preconditions", "Unity Editor is not connected");
            self.emit(None, true);
            return;
        }
        if crate::unity_bridge::is_play_mode_status(status) {
            self.fail(
                "preconditions",
                "leave play mode first: the self-test initializes in edit mode",
            );
            self.emit(None, true);
            return;
        }
        self.pass(
            "preconditions",
            "editor connected in edit mode; features enabled",
        );
        self.probe_message_driver_capabilities().await;

        // Force Release for this suite so the AggressiveInlining sample and
        // caller-refresh path run against Mono's actual inliner. Teardown
        // restores the original mode on both success and post-switch failure.
        let (_, code_optimization) = coordinator::detect_code_optimization(&self.project).await;
        self.original_code_optimization = code_optimization.clone();
        if code_optimization.as_deref() != Some("release") {
            match self
                .set_code_optimization_retrying("release", "preconditions Code Optimization")
                .await
            {
                Ok(value) => {
                    self.release_mode = value == "release";
                    self.log(format!(
                        "Code Optimization switched from {} to {} for Release inline coverage",
                        code_optimization.as_deref().unwrap_or("unknown"),
                        value,
                    ));
                }
                Err(error) => {
                    self.fail("preconditions", error);
                    self.restore_code_optimization().await;
                    self.emit(None, true);
                    return;
                }
            }
        } else {
            self.release_mode = true;
            self.log("Code Optimization = release");
        }

        if let Err(error) = self.initialize_corpus().await {
            self.fail("initialize", error);
            self.restore_code_optimization().await;
            self.cleanup_corpus().await;
            self.emit(None, true);
            return;
        }
        // Force + runtime-verify Release BEFORE the edit-mode patch tests: the
        // ensure step recompiles, which would revert E01's live editor patch if it
        // ran between E01 and the E02 play-enter assertion. The probe then reads
        // inline bits in a known Release (or explicitly-flagged-Debug) edit-mode
        // domain, so its data is never silently confounded.
        self.ensure_release_effective().await;
        self.run_inline_probes().await;
        self.run_editmode_tests().await;
        if let Err(error) = self.enter_play_mode().await {
            self.fail("enter play mode", error);
            self.finalize().await;
            self.emit(None, true);
            return;
        }

        // Evolving source ledgers: every step edits from the CURRENT text.
        let mut subject = SUBJECT_BASELINE.to_string();
        let mut messages = MESSAGE_BASELINE.to_string();
        let mut helper = HELPER_BASELINE.to_string();

        self.run_positive_tests(&mut subject, &mut helper).await;
        self.run_message_driver_tests(&mut messages).await;
        self.run_hot_added_mono_behaviour_diagnostic().await;
        self.run_release_inline_tests().await;
        self.run_release_inline_multifile_tests().await;
        self.run_inline_force_evaluate_check().await;
        self.run_adversarial_tests().await;
        self.run_negative_tests().await;
        self.run_deletion_tests(&mut subject, &mut helper).await;
        self.finalize().await;

        self.log(format!(
            "Self-test finished: {} passed, {} failed.",
            self.passed, self.failed
        ));
        self.emit(None, true);
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

fn extract_int(output: &str) -> Option<i64> {
    let digits: String = output
        .chars()
        .skip_while(|c| !c.is_ascii_digit() && *c != '-')
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().ok()
}

/// Entry point for the Tauri command. Refuses to run twice concurrently.
pub async fn run(app: tauri::AppHandle, project_path: String) -> Result<(), String> {
    if project_path.trim().is_empty() {
        return Err("select a Unity project workspace first".to_string());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("the hot-reload self-test is already running".to_string());
    }

    tauri::async_runtime::spawn(async move {
        let mut test = SelfTest {
            app,
            project: project_path,
            passed: 0,
            failed: 0,
            negative_ledgers: NegativeLedgers::default(),
            domain_reload_on_play: None,
            epmo_enabled: None,
            editor_patch_live: false,
            release_mode: false,
            original_code_optimization: None,
            last_apply_inlined: false,
            last_apply_summary: String::new(),
            message_driver_capabilities: MessageDriverCapabilities::default(),
        };
        test.run().await;
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}
