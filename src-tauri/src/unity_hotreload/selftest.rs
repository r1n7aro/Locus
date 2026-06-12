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
//!     five-file batch, and generic method bodies (generic methods AND
//!     methods of generic types) via the B1 remove+add shim path with
//!     their untouched callers re-detoured.
//!   • negatives (must come back COLD with the precise reason): generic
//!     bodies whose compiled caller is OUTSIDE the batch (named caller
//!     file), generic-type constructor bodies, constructor/
//!     finalizer surface and finalizer bodies, virtual members, struct
//!     field layout, enum value edits, attribute edits, const edits,
//!     property additions, new Unity message names, interface changes,
//!     base-list changes, partial types, delegate signature changes,
//!     conversions returning the declaring type, added members touching
//!     non-public state (Mono JIT access checks), uncovered call sites
//!     (named caller file); plus the shim-only "parked new surface"
//!     verdict for caller-less additions.
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
];

const SUBJECT_BASELINE: &str = r#"using UnityEngine;
using System.Threading.Tasks;

public class LocusSelfTestSubject : MonoBehaviour
{
    public static LocusSelfTestSubject Instance;
    public static int EvtCount;
    public static int UpdateBeats;
    public static System.Func<int> Captured;
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
    public int Relay() { return new LocusSelfTestNegative().Echo(20) + 1; }
    public int RelayVal() { return new LocusSelfTestNegGeneric<int>().Val(); }
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
}

struct SelfTest {
    app: tauri::AppHandle,
    project: String,
    passed: u32,
    failed: u32,
    /// Per-file texts as the positive phase left them; the negative phase
    /// restores files to exactly these after each cold-classification probe.
    negative_ledgers: NegativeLedgers,
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
        return Err(format!("internal corpus error: no line starts with: {line_prefix}"));
    };
    if indices.next().is_some() {
        return Err(format!("internal corpus error: multiple lines start with: {line_prefix}"));
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
                    self.fail(name, format!("expected '{expected}' in output, got: {output}"));
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
                self.fail(name, format!("unexpected cold verdict: {}", squash(&summary)));
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
            .apply_texts(name, &[(relative, ledger.as_str())], &[(relative, snapshot.as_str())])
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
                format!("expected a cold verdict with reason \"{reason_fragment}\", got: {}", squash(&text)),
            );
        }
        if let Err(error) = self.write_tracked(relative, restore).await {
            self.fail(name, format!("restore after {name} failed: {error}"));
        }
    }

    async fn wait_for_play_state(&self, playing: bool, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        loop {
            let (connected, status, _) = crate::unity_bridge::query_unity_status(&self.project).await;
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
            self.log(format!("note: edit session not started ({error}); continuing"));
        }

        self.write_tracked(SUBJECT_FILE, SUBJECT_BASELINE).await?;
        self.write_tracked(HELPER_FILE, HELPER_BASELINE).await?;
        self.write_tracked(MODE_FILE, MODE_BASELINE).await?;
        self.write_tracked(STRUCT_FILE, STRUCT_BASELINE).await?;
        self.write_tracked(CTOR_FILE, CTOR_BASELINE).await?;
        self.write_tracked(IFACE_FILE, IFACE_BASELINE).await?;
        self.write_tracked(NEG_FILE, NEG_BASELINE).await?;
        // Editor/ folder → Assembly-CSharp-Editor: edit-mode hot reload of
        // editor tooling is its own phase.
        self.write_tracked(EDITOR_FILE, EDITOR_BASELINE).await?;

        // The corpus was written behind Unity's back: the AssetDatabase must
        // import it or the compile would not include the new files at all
        // (the folder goes first so children import into an existing parent).
        let mut imports: Vec<String> = vec![TEST_DIR.to_string(), EDITOR_DIR.to_string()];
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
        self.log("Baseline compiled; corpus is the loaded truth.");
        Ok(())
    }

    /// Edit-mode hot reload, BEFORE play mode: editor tooling (custom
    /// editors, menu commands) lives in Assembly-CSharp-Editor and detours
    /// exactly like player code. The patch dies with the play-mode domain
    /// reload anyway, so the file reverts afterwards to keep the play-phase
    /// batches clean.
    async fn run_editmode_tests(&mut self) {
        self.log("Phase 2/7 — edit-mode hot reload (editor assembly, no play mode)");
        let name = "E01 editor-assembly body edit (edit mode)";
        self.log(format!("— {name}"));
        let edited = EDITOR_BASELINE.replace("return 1;", "return 8118;");
        match self.write_tracked(EDITOR_FILE, &edited).await {
            Ok(()) => match self.hot_reload(Some(vec![EDITOR_FILE.to_string()])).await {
                Ok(summary) if summary.contains("Hot reload not applicable") => {
                    self.fail(name, format!("unexpected cold verdict: {}", squash(&summary)));
                }
                Ok(_) => {
                    self.expect_output(name, "return LocusSelfTestEditorTool.Reading();", "8118").await;
                }
                Err(error) => self.fail(name, squash(&error)),
            },
            Err(error) => self.fail(name, error),
        }
        if let Err(error) = self.write_tracked(EDITOR_FILE, EDITOR_BASELINE).await {
            self.log(format!("  editor corpus revert failed: {error}"));
        }
    }

    async fn enter_play_mode(&mut self) -> Result<(), String> {
        self.log("Phase 3/7 — entering play mode");
        self.execute("UnityEditor.EditorApplication.EnterPlaymode(); return \"entering\";")
            .await
            .map_err(|e| format!("EnterPlaymode failed: {e}"))?;
        self.wait_for_play_state(true, Duration::from_secs(90)).await?;
        // The play-mode domain reload settles behind the status flip.
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
                swap(s, "public int Mult() { return 1002; }", "public int Mult() { return 4221; }")
            })
            .await
            .is_some()
        {
            self.expect_output("P01 method body edit", "return LocusSelfTestSubject.Instance.Mult();", "4221").await;
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

        // P03 — added methods (shim→shim chain). Only PUBLIC original
        // surface: Mono blocks non-public access from shims (N13 asserts
        // that exact verdict).
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
                        self.fail(name, format!("expected the caller file in the verdict, got: {}", squash(&text)));
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
                swap_line(s, "    public int Probe()", "    public int Probe() { return _bonus + 9090; }")
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
                swap_line(s, "    public int Probe()", "    public int Probe() { s_total += 1; return s_total; }")
            })
            .await
            .is_some();
        if p06_ok {
            self.expect_output("P06 added static field", "return LocusSelfTestSubject.Instance.Probe();", "6601").await;
        }

        // P07 — using addition re-detours the whole file (M6).
        let p07_ok = self
            .step_file("P07 using added (file rehook)", SUBJECT_FILE, subject, |s| {
                swap(s, "using UnityEngine;", "using UnityEngine;\nusing System.Text;")?;
                swap(
                    s,
                    "public int Step() { return 1; }",
                    "public int Step() { return 8800 + new StringBuilder(\"ab\").Length; }",
                )
            })
            .await
            .is_some();
        if p07_ok {
            self.expect_output("P07 using added (file rehook)", "return LocusSelfTestSubject.Instance.Step();", "8802").await;
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
                        &[(MODE_FILE, mode_v2.as_str()), (SUBJECT_FILE, subject.as_str())],
                        &[(MODE_FILE, MODE_BASELINE), (SUBJECT_FILE, subject_snapshot.as_str())],
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
        let p09 = swap_line(subject, "    public int Probe()", "    public int Probe() { return LocusSelfTestFresh.Ping(); }");
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
                    self.expect_output(name, "return LocusSelfTestSubject.Instance.Probe();", "4242").await;
                } else {
                    *subject = subject_snapshot;
                }
            }
            Err(error) => self.fail(name, error),
        }

        // P10 — property getter body edit.
        if self
            .step_file("P10 property getter edit", SUBJECT_FILE, subject, |s| {
                swap(s, "public int Gauge { get { return 17; } }", "public int Gauge { get { return 7117; } }")
            })
            .await
            .is_some()
        {
            self.expect_output("P10 property getter edit", "return LocusSelfTestSubject.Instance.Gauge;", "7117").await;
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
            .step_file("P10c expression-bodied member edit", SUBJECT_FILE, subject, |s| {
                swap(s, "public int Arrow => 12;", "public int Arrow => 7447;")
            })
            .await
            .is_some()
        {
            self.expect_output("P10c expression-bodied member edit", "return LocusSelfTestSubject.Instance.Arrow;", "7447").await;
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
            self.expect_output("P11 indexer edit", "return LocusSelfTestSubject.Instance[2];", "5007").await;
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
            self.expect_output("P14 local function edit", "return LocusSelfTestSubject.Instance.Local();", "9119").await;
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
            self.expect_output("P15 anonymous type edit", "return LocusSelfTestSubject.Instance.Anon();", "7997").await;
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
            self.expect_output("P16 pattern matching edit", "return LocusSelfTestSubject.Instance.Match(6);", "6776").await;
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
            self.expect_output("P16b #if-block body edit", "return LocusSelfTestSubject.Instance.Cond();", "8338").await;
        }

        // P17 — nested type member body edit.
        if self
            .step_file("P17 nested type member edit", SUBJECT_FILE, subject, |s| {
                swap(s, "public int Nine() { return 1; }", "public int Nine() { return 5665; }")
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
                swap(s, "public int Nine() { return 5665; }", "public int Nine() { return W + 5660; }")
                    .or_else(|_| swap(s, "public int Nine() { return 1; }", "public int Nine() { return W + 5660; }"))
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
            .step_file("P19 constructor body edit", CTOR_FILE, &mut ctor_ledger, |s| {
                swap(s, "public LocusSelfTestCtor() { Seed = 1; }", "public LocusSelfTestCtor() { Seed = 5775; }")
            })
            .await
            .is_some()
        {
            self.expect_output("P19 constructor body edit", "return new LocusSelfTestCtor().Seed;", "5775").await;
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
                swap(s, "public int Legacy() { return _legacy; }", "public int Legacy() { return 8448; }")
            })
            .await
            .is_some()
        {
            self.expect_output("P21 field deletion", "return LocusSelfTestSubject.Instance.Legacy();", "8448").await;
        }

        // P22 — method rename with the caller co-edited in the batch.
        let name = "P22 method rename (covered)";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let helper_snapshot = helper.clone();
        let p22 = (|| -> Result<(), String> {
            swap(helper, "public static int Renamed() { return 21; }", "public static int Thrice() { return 7227; }")?;
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
                        &[(HELPER_FILE, helper.as_str()), (SUBJECT_FILE, subject.as_str())],
                        &[(HELPER_FILE, helper_snapshot.as_str()), (SUBJECT_FILE, subject_snapshot.as_str())],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(name, "return LocusSelfTestSubject.Instance.CallRenamed();", "7227").await;
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
                        &[(HELPER_FILE, helper.as_str()), (SUBJECT_FILE, subject.as_str())],
                        &[(HELPER_FILE, helper_snapshot.as_str()), (SUBJECT_FILE, subject_snapshot.as_str())],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(name, "return LocusSelfTestSubject.Instance.CallBump();", "9229").await;
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
                swap(s, "public int Flip() { return 3; }", "public static int Flip() { return 6336; }")?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return Flip(); }")
            })
            .await
            .is_some()
        {
            self.expect_output("P24 static keyword flip", "return LocusSelfTestSubject.Instance.Probe();", "6336").await;
        }

        // P25 — accessibility narrowing (public→private, no outside callers).
        if self
            .step_file("P25 accessibility narrowing", SUBJECT_FILE, subject, |s| {
                swap(s, "public int Shrink() { return 2; }", "private int Shrink() { return 4884; }")?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return Shrink(); }")
            })
            .await
            .is_some()
        {
            self.expect_output("P25 accessibility narrowing", "return LocusSelfTestSubject.Instance.Probe();", "4884").await;
        }

        // P26 — static class method body edit.
        if self
            .step_file("P26 static class body edit", HELPER_FILE, helper, |s| {
                swap(s, "public static int Pick() { return 1; }", "public static int Pick() { return 3113; }")
            })
            .await
            .is_some()
        {
            self.expect_output("P26 static class body edit", "return LocusSelfTestHelper.Pick();", "3113").await;
        }

        // P27 — new type appended to an EXISTING file, observed via Probe.
        let name = "P27 new type in existing file";
        self.log(format!("— {name}"));
        let subject_snapshot = subject.clone();
        let helper_snapshot = helper.clone();
        helper.push_str("\npublic class LocusSelfTestExtra\n{\n    public static int Nine() { return 9559; }\n}\n");
        let p27 = swap_line(subject, "    public int Probe()", "    public int Probe() { return LocusSelfTestExtra.Nine(); }");
        match p27 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[(HELPER_FILE, helper.as_str()), (SUBJECT_FILE, subject.as_str())],
                        &[(HELPER_FILE, helper_snapshot.as_str()), (SUBJECT_FILE, subject_snapshot.as_str())],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(name, "return LocusSelfTestSubject.Instance.Probe();", "9559").await;
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
            swap_line(subject, "    public int Probe()", "    public int Probe() { return 1500.Tripled() + 12; }")
        })();
        match p27b {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[(HELPER_FILE, helper.as_str()), (SUBJECT_FILE, subject.as_str())],
                        &[(HELPER_FILE, helper_snapshot.as_str()), (SUBJECT_FILE, subject_snapshot.as_str())],
                    )
                    .await;
                if applied.is_some() {
                    self.expect_output(name, "return LocusSelfTestSubject.Instance.Probe();", "4512").await;
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
            .step_file("P28 interface impl body edit", IFACE_FILE, &mut iface_ledger, |s| {
                swap(s, "public int Plan() { return 1; }", "public int Plan() { return 6996; }")
            })
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
            .step_file("P29 struct method body edit", STRUCT_FILE, &mut struct_ledger, |s| {
                swap(s, "public int Get() { return 1; }", "public int Get() { return 6446; }")
            })
            .await
            .is_some()
        {
            self.expect_output("P29 struct method body edit", "return new LocusSelfTestStruct().Get();", "6446").await;
        }

        // P30 — operator body edit (the patch copy renames the self-typed
        // parameters; unchanged operators strip from the copy).
        if self
            .step_file("P30 operator body edit", STRUCT_FILE, &mut struct_ledger, |s| {
                swap(s, "r.Value = a.Value + b.Value;", "r.Value = a.Value + b.Value + 7337;")
            })
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
            swap(subject, "public int Gauge { get { return 7117; } }", "public int Gauge { get { return 7118; } }")
                .or_else(|_| swap(subject, "public int Gauge { get { return 17; } }", "public int Gauge { get { return 7118; } }"))?;
            swap(helper, "public static int Pick() { return 3113; }", "public static int Pick() { return 3114; }")
                .or_else(|_| swap(helper, "public static int Pick() { return 1; }", "public static int Pick() { return 3114; }"))?;
            swap(&mut struct_ledger, "public int Get() { return 6446; }", "public int Get() { return 6447; }")
                .or_else(|_| swap(&mut struct_ledger, "public int Get() { return 1; }", "public int Get() { return 6447; }"))?;
            swap(&mut ctor_ledger, "public LocusSelfTestCtor() { Seed = 5775; }", "public LocusSelfTestCtor() { Seed = 5776; }")
                .or_else(|_| swap(&mut ctor_ledger, "public LocusSelfTestCtor() { Seed = 1; }", "public LocusSelfTestCtor() { Seed = 5776; }"))?;
            swap(&mut iface_ledger, "public int Plan() { return 6996; }", "public int Plan() { return 6997; }")
                .or_else(|_| swap(&mut iface_ledger, "public int Plan() { return 1; }", "public int Plan() { return 6997; }"))
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
                    self.expect_output("P32a five-file batch (subject)", "return LocusSelfTestSubject.Instance.Gauge;", "7118").await;
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
        let mut neg_ledger = NEG_BASELINE.to_string();
        let subject_snapshot = subject.clone();
        let p33 = (|| -> Result<(), String> {
            swap(
                &mut neg_ledger,
                "public T Echo<T>(T value) { return value; }",
                "public T Echo<T>(T value) { return (T)(object)(((int)(object)value) + 7000); }",
            )?;
            swap(&mut neg_ledger, "public int Val() { return 1; }", "public int Val() { return 4334; }")?;
            swap(subject, "public int Spare() { return 1; }", "public int Spare() { return 2; }")
        })();
        match p33 {
            Ok(()) => {
                let applied = self
                    .apply_texts(
                        name,
                        &[(NEG_FILE, neg_ledger.as_str()), (SUBJECT_FILE, subject.as_str())],
                        &[(NEG_FILE, NEG_BASELINE), (SUBJECT_FILE, subject_snapshot.as_str())],
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
                }
            }
            Err(error) => {
                self.fail(name, error);
                *subject = subject_snapshot;
            }
        }

        // Hand the per-file ledgers to the negative phase: it restores files
        // to exactly these texts after each cold probe.
        self.negative_ledgers = NegativeLedgers {
            struct_text: struct_ledger,
            ctor_text: ctor_ledger,
            iface_text: iface_ledger,
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
                            format!("expected cold naming the caller file, got: {}", squash(&vtext)),
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
        if swap(&mut text, "public int Solid() { return 1; }", "public virtual int Solid() { return 1; }").is_ok() {
            self.expect_cold("N02 virtual keyword added", NEG_FILE, &text, "virtual member added", &neg).await;
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
            self.expect_cold("N03 attribute added", NEG_FILE, &text, "member declaration changed", &neg).await;
        }

        // N04 — consts are inlined at use sites.
        let mut text = neg.clone();
        if swap(&mut text, "public const int Limit = 3;", "public const int Limit = 4;").is_ok() {
            self.expect_cold("N04 const value changed", NEG_FILE, &text, "const or static initializer changed", &neg).await;
        }

        // N05 — property additions are metadata surface.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Solid() { return 1; }\n",
            "    public int Solid() { return 1; }\n    public int NewProp { get { return 1; } }\n",
        )
        .is_ok()
        {
            self.expect_cold("N05 property added", NEG_FILE, &text, "property added", &neg).await;
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
            self.expect_cold("N06 finalizer added", NEG_FILE, &text, "finalizer added", &neg).await;
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
            self.expect_cold("N07 Unity message added", NEG_FILE, &text, "new Unity message method", &neg).await;
        }

        // N08 — enum VALUE edits (only appends are hot).
        let mut text = neg.clone();
        if swap(&mut text, "Y = 2", "Y = 9").is_ok() {
            self.expect_cold("N08 enum value changed", NEG_FILE, &text, "enum changed", &neg).await;
        }

        // N09 — struct field layout.
        let struct_text = self.negative_ledgers.struct_text.clone();
        let mut text = struct_text.clone();
        if swap(&mut text, "    public int Value;\n", "    public int Value;\n    public int Extra;\n").is_ok() {
            self.expect_cold("N09 struct field added", STRUCT_FILE, &text, "struct field layout changed", &struct_text).await;
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
            self.expect_cold("N10 constructor added", CTOR_FILE, &text, "constructor added", &ctor_text).await;
        }

        // N11 — any interface change.
        let iface_text = self.negative_ledgers.iface_text.clone();
        let mut text = iface_text.clone();
        if swap(&mut text, "    int Plan();", "    int Plan();\n    int Extra();").is_ok() {
            self.expect_cold("N11 interface member added", IFACE_FILE, &text, "interface changed", &iface_text).await;
        }

        // N12 — added member touching PRIVATE state: the shim would fail
        // Mono's JIT access checks, so the classification is cold with the
        // exact reference named.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "    public int Hidden() { return _hidden; }\n",
            "    public int Hidden() { return _hidden; }\n    public int Leak() { return _hidden; }\n",
        )
        .is_ok()
        {
            self.expect_cold("N12 added member touches private state", NEG_FILE, &text, "references non-public surface", &neg).await;
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
            self.expect_cold("N15 generic type ctor body", NEG_FILE, &text, "generic type constructor changed", &neg).await;
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
            self.expect_cold("N16 base list changed", NEG_FILE, &text, "type declaration changed", &neg).await;
        }

        // N17 — partial types are out of scope entirely.
        let mut text = neg.clone();
        if swap(
            &mut text,
            "public class LocusSelfTestNegFin",
            "public partial class LocusSelfTestNegFin",
        )
        .is_ok()
        {
            self.expect_cold("N17 partial modifier added", NEG_FILE, &text, "partial type in file", &neg).await;
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
            self.expect_cold("N18 delegate signature changed", NEG_FILE, &text, "delegate declarations changed", &neg).await;
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
            self.expect_cold("N19 finalizer body changed", NEG_FILE, &text, "finalizer changed", &neg).await;
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
                        Ok(summary) => self.fail(name, format!("expected the shim-only verdict, got: {}", squash(&summary))),
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
        if swap(&mut text, "internal int Wide() { return 5; }", "public int Wide() { return 5; }").is_ok() {
            match self.write_tracked(NEG_FILE, &text).await {
                Ok(()) => {
                    match self.hot_reload(Some(vec![NEG_FILE.to_string()])).await {
                        Ok(summary) if summary.contains("No effective code change") => {
                            self.pass(name, "widening classified as a no-op");
                        }
                        Ok(summary) => self.fail(name, format!("expected a no-op verdict, got: {}", squash(&summary))),
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
                swap(s, "    void Update() { _ticks += Step(); UpdateBeats += 1; }\n", "")
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
                swap(s, "    public async Task<int> Pulse() { await Task.Yield(); return 2112; }\n", "")
                    .or_else(|_| swap(s, "    public async Task<int> Pulse() { await Task.Yield(); return 2002; }\n", ""))
                    .or_else(|_| swap(s, "    public Task<int> Pulse() { return Task.FromResult(2001); }\n", ""))
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
                if s.contains("Task<") {
                    swap_line(s, "    public async Task<int> Pulse()", "")
                        .or_else(|_| swap_line(s, "    public Task<int> Pulse()", ""))?;
                }
                Ok(())
            })
            .await
            .is_some()
        {
            self.expect_output(name, "return LocusSelfTestSubject.Instance.Step();", "8802").await;
        }

        // D05 — whole-file deletion: first edit every compiled caller away
        // (line-anchored: earlier failures must not break the anchors), then
        // the helper file (and the type appended to it) can go.
        let name = "D05 file deletion";
        let stripped = self
            .step_file("D05a helper callers edited away", SUBJECT_FILE, subject, |s| {
                swap_line(s, "    public int Sum(int a)", "    public int Sum(int a) { return a + a * 2 + 100; }")?;
                swap_line(s, "    public int CallRenamed()", "    public int CallRenamed() { return 7227; }")?;
                swap_line(s, "    public int CallBump()", "    public int CallBump() { return 9229; }")?;
                swap_line(s, "    public int Probe()", "    public int Probe() { return 0; }")
            })
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
        if let Err(error) = self.wait_for_play_state(false, Duration::from_secs(60)).await {
            self.log(format!("warning: {error}"));
        }

        // H6 fires on the play-exit transition; wait for the convergence
        // recompile to clear the active patches.
        let converged = self.wait_for_convergence(Duration::from_secs(180)).await;
        match converged {
            Ok(()) => self.pass("F01 auto-convergence", "active patches cleared after leaving play mode"),
            Err(error) => {
                self.log(format!("auto-convergence not observed ({error}); converging explicitly"));
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
            self.fail("preconditions", "hot reload and the sidecar compiler must both be enabled");
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
            self.fail("preconditions", "leave play mode first: the self-test initializes in edit mode");
            self.emit(None, true);
            return;
        }
        self.pass("preconditions", "editor connected in edit mode; features enabled");

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
        };
        test.run().await;
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}
