//! Built-in hot-reload self-test: drives the WHOLE hot-reload surface
//! (H0–H7) against the connected Unity Editor through the same internal
//! interfaces the agent tools use — coordinator baselines, the sidecar
//! compile, the pipe — and reports a step-by-step diagnostic log.
//!
//! Coverage maps to the public hot-reload feature matrix
//! (hotreload.net/zh/documentation/features), positive and negative:
//!   • positives: method/property(get+set)/indexer/event/operator/
//!     conversion/ctor body edits, expression-bodied members, lambda +
//!     closure (including NEW captures), local functions, anonymous types,
//!     pattern matching, nested types, iterator (coroutine) bodies, async
//!     body edits and async↔sync, added methods (shim→shim chains) +
//!     fields (instance and static, across separate batches),
//!     instance-initializer edits, field deletion, signature changes
//!     (params / ref→out / static flip / rename) with call-site
//!     verification, accessibility narrowing, using add/remove with
//!     whole-file rehook, enum append, new files, new types in existing
//!     files (top-level and nested), struct method bodies, interface-impl
//!     bodies, deletions (members, properties, Unity messages, whole
//!     files); plus edit-mode reloading of EDITOR-assembly code, in-flight
//!     delegates following detours, Unity message body edits, store-held
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
//!     new Unity message names, interface changes,
//!     base-list changes, partial-type FIELD layout changes (body edits
//!     are hot since B6), delegate signature changes,
//!     conversions returning the declaring type, added members whose
//!     SIGNATURE names a non-public type (C2′a relaxes body access only),
//!     uncovered call sites (named caller file); plus the shim-only
//!     "parked new surface" verdict for caller-less additions and the
//!     untracked-input verdict for .asmdef edits (assembly restructuring
//!     always needs unity_recompile), along with B2's pointed cold guards:
//!     full-property ++, ??= set-skip, auto-property ref/out, and compound
//!     indexers with non-repeatable index expressions.
//!
//! Flow: with the editor connected and NOT playing, it materializes a test
//! corpus under Assets/LocusHotReloadSelfTest, imports + recompiles it as
//! the baseline (a real domain reload), enters play mode, spawns the test
//! component, then hot-reloads one feature after another, verifying
//! observable behavior through `unity_execute_code` snippets. Added-member
//! behavior is always asserted through a PRE-EXISTING member (`Probe`)
//! re-pointed at the new surface: snippets compile against the original
//! assembly metadata, which never contains hot-added members. Every step
//! is atomic: a failed apply reverts its file(s) on disk and in the ledger
//! so one rejected patch cannot poison the following batches.
//!
//! It finishes by leaving play mode, waiting for the automatic convergence
//! (H6), and deleting the corpus.
//!
//! Triggered from Settings > Code Analysis; progress streams to the UI via
//! the `unity-hotreload-selftest` event.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::Emitter;

use super::coordinator;

static RUNNING: AtomicBool = AtomicBool::new(false);

const TEST_DIR: &str = "Assets/LocusHotReloadSelfTest";
const SUBJECT_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestSubject.cs";
const HELPER_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestHelper.cs";
const MODE_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestMode.cs";
const FRESH_FILE: &str = "Assets/LocusHotReloadSelfTest/LocusSelfTestFresh.cs";
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

const ALL_FILES: &[&str] = &[
    SUBJECT_FILE,
    HELPER_FILE,
    MODE_FILE,
    FRESH_FILE,
    STRUCT_FILE,
    CTOR_FILE,
    IFACE_FILE,
    NEG_FILE,
    EDITOR_FILE,
    PARTIAL_A_FILE,
    PARTIAL_B_FILE,
    // The .asmdef imports BEFORE the lib source so the assembly exists by
    // the time its first script imports (both flush in one batch anyway —
    // the compilation pipeline recomputes asmdef ownership per compile).
    LIB_ASMDEF_FILE,
    LIB_FILE,
];

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

    /// Snippet whose output must contain `expected` (sentinel values are
    /// chosen to be unambiguous).
    async fn expect_output(&mut self, name: &str, code: &str, expected: &str) {
        match self.execute(code).await {
            Ok(output) => {
                if output.contains(expected) {
                    self.pass(name, format!("observed {expected}"));
                } else {
                    self.fail(
                        name,
                        format!("expected '{expected}' in output, got: {output}"),
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
        match self.hot_reload(None).await {
            Ok(summary) if summary.contains("Hot reload not applicable") => {
                self.fail(
                    name,
                    format!("unexpected cold verdict: {}", squash(&summary)),
                );
                self.revert_files(reverts).await;
                None
            }
            Ok(summary) => Some(summary),
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
                .filter(|relative| **relative != FRESH_FILE) // created later, mid-play
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

        self.execute(
            "var go = new UnityEngine.GameObject(\"LocusHotReloadSelfTest\");\n\
             go.AddComponent<LocusSelfTestSubject>();\n\
             return \"spawned\";",
        )
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
                self.log(format!(
                    "  coordinator continuity: {} active patch(es) carried across play-enter ({})",
                    super::counters().active_patches,
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

        // Hand the per-file ledgers to the negative phase: it restores files
        // to exactly these texts after each cold probe.
        self.negative_ledgers = NegativeLedgers {
            struct_text: struct_ledger,
            ctor_text: ctor_ledger,
            iface_text: iface_ledger,
            partial_a_text: partial_a_ledger,
        };
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

        // N07 — Unity message names are discovered at real compiles only.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { return 1; }\n    void Update() { }\n",
        )
        .is_ok()
        {
            self.expect_cold(
                "N07 Unity message added",
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
        if let Err(error) = crate::unity_bridge::exit_play_mode(&self.project).await {
            self.log(format!("exit_play_mode failed (continuing): {error}"));
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

    async fn wait_for_convergence(&self, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        loop {
            if super::counters().active_patches == 0 {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "{} patch(es) still active after {}s",
                    super::counters().active_patches,
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

        if let Err(error) = self.initialize_corpus().await {
            self.fail("initialize", error);
            self.emit(None, true);
            return;
        }
        self.run_editmode_tests().await;
        if let Err(error) = self.enter_play_mode().await {
            self.fail("enter play mode", error);
            self.finalize().await;
            self.emit(None, true);
            return;
        }

        // Evolving source ledgers: every step edits from the CURRENT text.
        let mut subject = SUBJECT_BASELINE.to_string();
        let mut helper = HELPER_BASELINE.to_string();

        self.run_positive_tests(&mut subject, &mut helper).await;
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
        };
        test.run().await;
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}
