//! Live checks against a real Blender (ignored by default):
//!
//! ```text
//! set LOCUS_BLENDER_EXE=E:/blender/blender.exe
//! cargo test --lib blender::live_tests -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Launches a background instance through the hub's pool (bootstrap.py, no
//! add-on install), then checks exec values/output, the not-executed versus
//! uncertain timeout split, ping while busy, and shutdown.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::background::LaunchRequest;
use super::hub::{self, ExecOutcome};
use super::installations::{self, Installation, InstallationSource, ProbeState};

/// Test output is captured by the app's log redirection; timings also go to
/// `%TEMP%/locus_blender_live.txt`.
macro_rules! note {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        println!("{line}");
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("locus_blender_live.txt"))
        {
            let _ = writeln!(file, "{line}");
        }
    }};
}

fn blender_exe() -> Option<PathBuf> {
    std::env::var_os("LOCUS_BLENDER_EXE").map(PathBuf::from).filter(|path| path.is_file())
}

async fn installation(exe: &std::path::Path) -> Installation {
    let probe = installations::probe(exe).await.expect("probe");
    Installation {
        id: installations::normalize_path(exe),
        path: exe.display().to_string(),
        sources: vec![InstallationSource::Manual],
        hidden: false,
        is_default: true,
        probe_state: ProbeState::Ready,
        support: installations::support_status(Some(&probe)),
        display_version: Some(probe.display_version()),
        probe: Some(probe),
        probe_error: None,
        addon: None,
    }
}

#[test]
#[ignore = "needs LOCUS_BLENDER_EXE"]
fn background_instance_round_trip() {
    let Some(exe) = blender_exe() else {
        note!("LOCUS_BLENDER_EXE not set; skipped");
        return;
    };
    let registry = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    std::env::set_var("LOCUS_BLENDER_REGISTRY_DIR", registry.path());
    super::set_blender_integration_enabled(true);

    tauri::async_runtime::block_on(async move {
        let handle = hub::acquire().expect("hub");
        let hub = handle.hub().clone();
        let installation = installation(&exe).await;
        note!("probe: {:?}", installation.display_version);

        let started = Instant::now();
        let instance = hub
            .background
            .acquire(
                &hub,
                LaunchRequest {
                    checkout_id: "live-checkout",
                    checkout_root: checkout.path(),
                    session_id: Some("live-session"),
                    installation: &installation,
                    file: None,
                    force_new: false,
                },
            )
            .await
            .expect("background instance");
        note!("connected in {} ms: {}", started.elapsed().as_millis(), instance.key);
        let registration = instance.registration();
        assert!(registration.is_background());
        assert_eq!(registration.launched_by_checkout().as_deref(), Some("live-checkout"));

        // Value, output streaming and the result shape.
        let mut chunks = Vec::new();
        let payload = json!({"mode": "code", "code": "print('hello from blender')\nreturn (1, 2.5, {'k': None})", "readonly": true});
        let started = Instant::now();
        let outcome = instance.exec(&payload, Duration::from_secs(30), |chunk| chunks.push(chunk.text), None).await;
        note!("exec round trip {} ms", started.elapsed().as_millis());
        let ExecOutcome::Completed(result) = outcome else { panic!("exec failed: {outcome:?}") };
        assert_eq!(result["ok"], Value::Bool(true), "{result}");
        assert_eq!(result["value"], json!({"$t": [1, 2.5, {"k": null}]}));
        assert!(result["stdout"].as_str().unwrap().contains("hello from blender"));
        assert!(chunks.concat().contains("hello from blender"));

        // Exceptions come back as data, not transport errors.
        let outcome = instance
            .exec(&json!({"mode": "code", "code": "raise KeyError('Cube')", "readonly": true}), Duration::from_secs(30), |_| {}, None)
            .await;
        let ExecOutcome::Completed(result) = outcome else { panic!("exec failed: {outcome:?}") };
        assert_eq!(result["error"]["type"], "KeyError");

        // A started execution that times out is uncertain; the broker still
        // answers ping while the main thread is busy.
        let busy = instance.clone();
        let sleeper = tauri::async_runtime::spawn(async move {
            busy.exec(
                &json!({"mode": "code", "code": "import time\ntime.sleep(4)\nreturn 1", "readonly": true}),
                Duration::from_millis(1500),
                |_| {},
                None,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(400)).await;
        let started = Instant::now();
        instance.request("ping", "", Duration::from_secs(2)).await.expect("ping while busy");
        note!("ping while busy {} ms", started.elapsed().as_millis());
        let snapshot = hub::read_state_plane(&instance.key).expect("state plane");
        note!("state plane: {snapshot}");
        match sleeper.await.unwrap() {
            ExecOutcome::Unknown(message) => note!("timed out as expected: {message}"),
            other => panic!("expected an uncertain timeout, got {other:?}"),
        }

        // A longer stall: the uncertain result names the main thread's
        // native frames (symbolized when the PDBs are present).
        let outcome = instance
            .exec(
                &json!({"mode": "code", "code": "import time\ntime.sleep(6)\nreturn 1", "readonly": true}),
                Duration::from_millis(4500),
                |_| {},
                None,
            )
            .await;
        match outcome {
            ExecOutcome::Unknown(message) => {
                note!("stalled timeout: {message}");
                assert!(message.contains("native frames"), "{message}");
            }
            other => panic!("expected an uncertain timeout, got {other:?}"),
        }
        let started = Instant::now();
        let stack = instance.main_thread_stack(12).await.expect("stack");
        note!("idle stack ({} ms): {}", started.elapsed().as_millis(), stack.join(" | "));
        assert!(!stack.is_empty());

        // Shutdown through the pool.
        let pid = registration.pid;
        hub.background.close(&hub, pid).await;
        assert!(super::process::identity(pid).is_none(), "background Blender still running");
        // Dropping the last handle stops the hub (watcher, reaper, connections).
        drop(instance);
        drop(hub);
        drop(handle);
        assert!(!hub::is_running(), "hub still running after the last handle dropped");
    });
}
