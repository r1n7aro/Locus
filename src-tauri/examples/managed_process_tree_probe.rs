use std::time::Duration;

use locus_lib::process_util::{
    async_command, managed_process_count, spawn_managed, terminate_managed_processes_for_session,
    ProcessOwner,
};

fn delayed_marker_command(
    marker: &std::path::Path,
) -> (tokio::process::Command, Option<std::path::PathBuf>) {
    #[cfg(target_os = "windows")]
    {
        let script_path = marker.with_extension("cmd");
        let script = format!(
            "@echo off\r\npowershell.exe -NoProfile -Command \"Start-Sleep -Milliseconds 1000; Set-Content -LiteralPath '{}' -Value survived\"\r\n",
            marker.to_string_lossy().replace('\'', "''")
        );
        std::fs::write(&script_path, script).expect("probe script should be written");
        let mut command = async_command("cmd");
        command.arg("/D").arg("/C").arg(&script_path);
        (command, Some(script_path))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let marker = marker
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('\'', "'\\''");
        let mut command = async_command("sh");
        command
            .arg("-c")
            .arg(format!("(sleep 1; printf survived > '{marker}') & wait"));
        (command, None)
    }
}

fn configure_stdio(command: &mut tokio::process::Command) {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
}

#[tokio::main]
async fn main() {
    let marker = std::env::temp_dir().join(format!(
        "locus-managed-process-probe-{}.txt",
        uuid::Uuid::new_v4().simple()
    ));

    let (mut command, script) = delayed_marker_command(&marker);
    configure_stdio(&mut command);
    let child =
        spawn_managed(command, ProcessOwner::default()).expect("managed process tree should start");
    tokio::time::sleep(Duration::from_millis(150)).await;
    drop(child);
    tokio::time::sleep(Duration::from_millis(3_000)).await;

    assert_eq!(managed_process_count(), 0, "managed registry should drain");
    assert!(
        !marker.exists(),
        "descendant process survived cancellation and wrote {}",
        marker.display()
    );
    if let Some(script) = script {
        let _ = std::fs::remove_file(script);
    }

    let cancelled_marker = std::env::temp_dir().join(format!(
        "locus-managed-process-cancelled-{}.txt",
        uuid::Uuid::new_v4().simple()
    ));
    let retained_marker = std::env::temp_dir().join(format!(
        "locus-managed-process-retained-{}.txt",
        uuid::Uuid::new_v4().simple()
    ));
    let (mut cancelled_command, cancelled_script) = delayed_marker_command(&cancelled_marker);
    let (mut retained_command, retained_script) = delayed_marker_command(&retained_marker);
    configure_stdio(&mut cancelled_command);
    configure_stdio(&mut retained_command);
    let cancelled_child = spawn_managed(
        cancelled_command,
        ProcessOwner::session("cancelled-session", "C:/probe"),
    )
    .expect("cancelled session process should start");
    let retained_child = spawn_managed(
        retained_command,
        ProcessOwner::session("retained-session", "C:/probe"),
    )
    .expect("retained session process should start");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        terminate_managed_processes_for_session("cancelled-session"),
        1
    );
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    assert!(
        !cancelled_marker.exists(),
        "cancelled session process survived"
    );
    assert!(
        retained_marker.exists(),
        "unrelated session process was terminated"
    );
    drop(cancelled_child);
    drop(retained_child);
    let _ = std::fs::remove_file(retained_marker);
    for script in [cancelled_script, retained_script].into_iter().flatten() {
        let _ = std::fs::remove_file(script);
    }
    assert_eq!(managed_process_count(), 0, "managed registry should drain");
    println!("managed process tree probe passed");
}
