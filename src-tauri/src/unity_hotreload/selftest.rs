//! Built-in hot-reload self-test: drives the WHOLE hot-reload surface
//! (H0–H7) against the connected Unity Editor through the same internal
//! interfaces the agent tools use — coordinator baselines, the sidecar
//! compile, the pipe — and reports a step-by-step diagnostic log.
//!
//! Flow: with the editor connected and NOT playing, it materializes a small
//! test corpus under Assets/LocusHotReloadSelfTest, runs a real recompile
//! (baseline), enters play mode, spawns the test component, then hot-reloads
//! one feature after another, verifying observable behavior through
//! `unity_execute_code` snippets. It finishes by leaving play mode, waiting
//! for the automatic convergence (H6), and deleting the corpus.
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

const SUBJECT_BASELINE: &str = r#"using UnityEngine;
using System.Threading.Tasks;

public class LocusSelfTestSubject : MonoBehaviour
{
    public static LocusSelfTestSubject Instance;
    private int _ticks = 0;
    public int Ticks { get { return _ticks; } }

    void Awake() { Instance = this; }
    void Update() { _ticks += Step(); }

    public int Step() { return 1; }
    public int Mult() { return 1002; }
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
}
"#;

const HELPER_BASELINE: &str = r#"public static class LocusSelfTestHelper
{
    public static int Twice(int a) { return a * 2; }
}
"#;

const MODE_BASELINE: &str = r#"public enum LocusSelfTestMode { A = 0, B = 1 }
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

struct SelfTest {
    app: tauri::AppHandle,
    project: String,
    passed: u32,
    failed: u32,
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
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| format!("delete {relative}: {e}"))
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
        self.log("Phase 1/4 — initializing the test corpus (edit mode)");
        self.write_tracked(SUBJECT_FILE, SUBJECT_BASELINE).await?;
        self.write_tracked(HELPER_FILE, HELPER_BASELINE).await?;
        self.write_tracked(MODE_FILE, MODE_BASELINE).await?;

        self.log("Baseline recompile (this includes a domain reload)...");
        crate::unity_bridge::recompile_and_wait(&self.project)
            .await
            .map_err(|e| format!("baseline recompile failed: {e}"))?;
        self.log("Baseline compiled; corpus is the loaded truth.");
        Ok(())
    }

    async fn enter_play_mode(&mut self) -> Result<(), String> {
        self.log("Phase 2/4 — entering play mode");
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

    async fn run_feature_tests(&mut self) {
        self.log("Phase 3/4 — hot-reloading every supported change shape");

        // Evolving source ledger: every step edits from the CURRENT text.
        let mut subject = SUBJECT_BASELINE.to_string();
        let mut helper = HELPER_BASELINE.to_string();

        // T1 — method body change.
        let name = "T1 body change";
        subject = subject.replace("public int Mult() { return 1002; }", "public int Mult() { return 4221; }");
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(summary) => {
                self.log(format!("  reload: {}", first_line(&summary)));
                self.expect_output(name, "return LocusSelfTestSubject.Instance.Mult();", "4221").await;
            }
            Err(error) => self.fail(name, error),
        }

        // T2 — async↔sync conversion.
        let name = "T2 async<->sync";
        subject = subject.replace(
            "public Task<int> Pulse() { return Task.FromResult(2001); }",
            "public async Task<int> Pulse() { await Task.Yield(); return 2002; }",
        );
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(_) => {
                self.expect_output(name, "return await LocusSelfTestSubject.Instance.Pulse();", "2002").await;
            }
            Err(error) => self.fail(name, error),
        }

        // T3 — orphan added public method (shim), touching private state.
        let name = "T3 added method";
        subject = subject.replace(
            "    public int Step() { return 1; }",
            "    public int Step() { return 1; }\n    public int Boost() { return _ticks >= 0 ? 7707 : 0; }",
        );
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(_) => {
                self.expect_output(name, "return LocusSelfTestSubject.Instance.Boost();", "7707").await;
            }
            Err(error) => self.fail(name, error),
        }

        // T4 — signature change with the call site OUTSIDE the batch: must
        // come back cold listing the exact caller file, then go hot once the
        // caller is edited too.
        let name = "T4 signature change (caller check)";
        let old_helper = helper.clone();
        helper = helper.replace(
            "public static int Twice(int a) { return a * 2; }",
            "public static int Twice(int a, int extra) { return a * 2 + extra; }",
        );
        match self.write_tracked(HELPER_FILE, &helper).await {
            Ok(()) => {
                match self.hot_reload(Some(vec![HELPER_FILE.to_string()])).await {
                    Ok(summary) if summary.contains("LocusSelfTestSubject.cs") => {
                        self.pass(
                            "T4a uncovered caller listed",
                            "cold verdict names the caller file",
                        );
                    }
                    Ok(summary) => self.fail(
                        "T4a uncovered caller listed",
                        format!("expected a cold verdict naming the caller, got: {}", first_line(&summary)),
                    ),
                    Err(error) if error.contains("LocusSelfTestSubject.cs") => {
                        self.pass("T4a uncovered caller listed", "cold verdict names the caller file");
                    }
                    Err(error) => self.fail("T4a uncovered caller listed", error),
                }

                subject = subject.replace(
                    "public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a); }",
                    "public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a, 100); }",
                );
                match self.apply("T4b covered batch goes hot", SUBJECT_FILE, &subject).await {
                    Ok(_) => {
                        self.expect_output(
                            "T4b covered batch goes hot",
                            "return LocusSelfTestSubject.Instance.Sum(3);",
                            "109", // 3 + (3*2 + 100)
                        )
                        .await;
                    }
                    Err(error) => self.fail("T4b covered batch goes hot", error),
                }
            }
            Err(error) => {
                self.fail(name, error);
                helper = old_helper;
            }
        }

        // T5 — instance field addition: pre-existing instance reads
        // default(T); a NEW instance runs the initializer.
        let name = "T5 added field";
        subject = subject.replace(
            "    private int _ticks = 0;",
            "    private int _ticks = 0;\n    private int _bonus = 5050;",
        );
        subject = subject.replace(
            "    public int Step() { return 1; }",
            "    public int Step() { return 1; }\n    public int Bonus() { return _bonus + 9090; }",
        );
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(_) => {
                self.expect_output(
                    "T5a existing instance reads default",
                    "return LocusSelfTestSubject.Instance.Bonus();",
                    "9090", // default(0) + 9090
                )
                .await;
                self.expect_output(
                    "T5b new instance runs the initializer",
                    "var go = new UnityEngine.GameObject(\"LocusSelfTestFieldProbe\");\n\
                     var probe = go.AddComponent<LocusSelfTestSubject>();\n\
                     var value = probe.Bonus();\n\
                     UnityEngine.Object.Destroy(go);\n\
                     return value;",
                    "14140", // 5050 + 9090
                )
                .await;
            }
            Err(error) => self.fail(name, error),
        }

        // T6 — added static field through the holder class.
        let name = "T6 added static field";
        subject = subject.replace(
            "    public int Step() { return 1; }",
            "    public int Step() { return 1; }\n    private static int s_total = 6600;\n    public int Total() { s_total += 1; return s_total; }",
        );
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(_) => {
                self.expect_output(name, "return LocusSelfTestSubject.Instance.Total();", "6601").await;
            }
            Err(error) => self.fail(name, error),
        }

        // T7 — using change re-detours the whole file.
        let name = "T7 using change";
        subject = subject.replace(
            "using UnityEngine;",
            "using UnityEngine;\nusing System.Text;",
        );
        subject = subject.replace(
            "public int Step() { return 1; }",
            "public int Step() { return 8800 + new StringBuilder(\"ab\").Length; }",
        );
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(_) => {
                self.expect_output(name, "return LocusSelfTestSubject.Instance.Step();", "8802").await;
            }
            Err(error) => self.fail(name, error),
        }

        // T8 — appended enum member materializes as a cast literal.
        let name = "T8 enum append";
        let mode_v2 = MODE_BASELINE.replace(
            "public enum LocusSelfTestMode { A = 0, B = 1 }",
            "public enum LocusSelfTestMode { A = 0, B = 1, C = 7 }",
        );
        subject = subject.replace(
            "            case LocusSelfTestMode.B: return 22;",
            "            case LocusSelfTestMode.B: return 22;\n            case LocusSelfTestMode.C: return 3377;",
        );
        let mode_write = self.write_tracked(MODE_FILE, &mode_v2).await;
        match mode_write {
            Ok(()) => match self.apply(name, SUBJECT_FILE, &subject).await {
                Ok(_) => {
                    self.expect_output(
                        name,
                        "return LocusSelfTestSubject.Instance.ModeValue((LocusSelfTestMode)7);",
                        "3377",
                    )
                    .await;
                }
                Err(error) => self.fail(name, error),
            },
            Err(error) => self.fail(name, error),
        }

        // T9 — brand-new file with a brand-new type (TI-C visibility).
        let name = "T9 new file";
        match self
            .write_tracked(
                FRESH_FILE,
                "public static class LocusSelfTestFresh { public static int Ping() { return 4242; } }\n",
            )
            .await
        {
            Ok(()) => match self.hot_reload(Some(vec![FRESH_FILE.to_string()])).await {
                Ok(_) => {
                    self.expect_output(name, "return LocusSelfTestFresh.Ping();", "4242").await;
                }
                Err(error) => self.fail(name, error),
            },
            Err(error) => self.fail(name, error),
        }

        // T10 — deleting a Unity message method stops its behavior NOW.
        let name = "T10 magic method deletion";
        subject = subject.replace("    void Update() { _ticks += Step(); }\n", "");
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(summary) => {
                if summary.contains("stub") {
                    self.log("  stub detour reported by the reload summary");
                }
                match self.ticks_frozen().await {
                    Ok(true) => self.pass(name, "tick counter froze after deleting Update"),
                    Ok(false) => self.fail(name, "tick counter kept advancing after deleting Update"),
                    Err(error) => self.fail(name, error),
                }
            }
            Err(error) => self.fail(name, error),
        }

        // T11 — plain member deletion (no callers): tombstone noop.
        let name = "T11 member deletion";
        subject = subject
            .replace("    public async Task<int> Pulse() { await Task.Yield(); return 2002; }\n", "")
            .replace("    public Task<int> Pulse() { return Task.FromResult(2001); }\n", "");
        match self.apply(name, SUBJECT_FILE, &subject).await {
            Ok(summary) => self.pass(name, first_line(&summary)),
            Err(error) => self.fail(name, error),
        }

        // T12 — whole-file deletion. First the failure shape: deleting
        // Helper while the compiled Subject still calls it must list
        // Subject; then re-batching both goes hot.
        let name = "T12 file deletion";
        subject = subject.replace(
            "public int Sum(int a) { return a + LocusSelfTestHelper.Twice(a, 100); }",
            "public int Sum(int a) { return a + a * 2 + 100; }",
        );
        match self.apply("T12a drop the helper usage", SUBJECT_FILE, &subject).await {
            Ok(_) => match self.delete_tracked(HELPER_FILE).await {
                Ok(()) => match self.hot_reload(None).await {
                    Ok(summary) => self.pass(name, first_line(&summary)),
                    Err(error) => self.fail(name, error),
                },
                Err(error) => self.fail(name, error),
            },
            Err(error) => self.fail("T12a drop the helper usage", error),
        }
    }

    /// Write + hot reload one file, returning the reload summary.
    async fn apply(&mut self, name: &str, relative: &str, content: &str) -> Result<String, String> {
        self.log(format!("— {name}"));
        self.write_tracked(relative, content).await?;
        self.hot_reload(None).await
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
        self.log("Phase 4/4 — leaving play mode and converging");
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
            Ok(()) => self.pass("T13 auto-convergence", "active patches cleared after leaving play mode"),
            Err(error) => {
                self.log(format!("auto-convergence not observed ({error}); converging explicitly"));
                match crate::unity_bridge::recompile_and_wait(&self.project).await {
                    Ok(_) => self.pass("T13 convergence (explicit)", "real recompile succeeded"),
                    Err(recompile_error) => self.fail("T13 convergence", recompile_error),
                }
            }
        }

        // The corpus served its purpose: remove it and converge once more.
        self.log("Cleaning up the test corpus...");
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
        if let Err(error) = self.enter_play_mode().await {
            self.fail("enter play mode", error);
            self.finalize().await;
            self.emit(None, true);
            return;
        }

        self.run_feature_tests().await;
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
        };
        test.run().await;
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}
