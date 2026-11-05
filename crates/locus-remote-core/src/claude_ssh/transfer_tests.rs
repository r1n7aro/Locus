use super::*;
use crate::claude_ssh::{archive_rpc::ArchiveRpc, errors, read_frame_traced};
use serde_json::json;
use tokio::{io::{AsyncWriteExt, BufReader}, sync::oneshot};

fn history_limits() -> Limits { Limits::history(Duration::from_secs(45)) }
fn wire(value: &Value) -> Vec<u8> { let mut bytes = serde_json::to_vec(value).unwrap(); bytes.push(b'\n'); bytes }

#[tokio::test(start_paused = true)]
async fn startup_without_a_transfer_keeps_its_original_deadline() {
    let progress = Progress::default();
    let start = Instant::now();
    let result = progress.wait(std::future::pending::<()>(), history_limits()).await;
    assert_eq!(result, Err(Timeout::Initial(Duration::from_secs(45))));
    assert_eq!(start.elapsed(), Duration::from_secs(45));
}

#[tokio::test(start_paused = true)]
async fn partial_history_frame_can_take_longer_than_the_old_handshake_deadline() {
    let progress = Arc::new(Progress::default());
    let value = json!({"type":"hello","protocol":1,"info":{"history":"x".repeat(512)}});
    let bytes = wire(&value);
    let mut frame = FrameProgress::new(progress.clone(), bytes.len(), None).unwrap();
    let (mut writer, reader) = tokio::io::duplex(32);
    let write = tokio::spawn(async move {
        for chunk in bytes.chunks(100) {
            tokio::time::sleep(Duration::from_secs(20)).await;
            writer.write_all(chunk).await.unwrap();
        }
    });
    let mut reader = BufReader::new(reader);
    let start = Instant::now();
    let result = progress.wait(read_frame_traced(&mut reader, None, Some(&mut frame)), history_limits()).await;
    assert_eq!(result.unwrap().unwrap(), Some(value));
    assert!(start.elapsed() > Duration::from_secs(45));
    write.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stalled_partial_frame_expires_since_its_last_byte_not_its_first_byte() {
    let progress = Arc::new(Progress::default());
    let mut frame = FrameProgress::new(progress.clone(), 1000, None).unwrap();
    let (mut writer, reader) = tokio::io::duplex(32);
    let write = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        writer.write_all(b"{\"type\":\"hello\"").await.unwrap();
        // Keep the stream OPEN; EOF is a different failure and must not be hidden.
        std::future::pending::<()>().await;
    });
    let mut reader = BufReader::new(reader);
    let start = Instant::now();
    let error = progress.wait(read_frame_traced(&mut reader, None, Some(&mut frame)), history_limits()).await.unwrap_err();
    assert_eq!(error, Timeout::Idle(Duration::from_secs(45)));
    assert_eq!(start.elapsed(), Duration::from_secs(65));
    write.abort();
}

#[tokio::test(start_paused = true)]
async fn continuously_dripping_bytes_cannot_bypass_the_total_limit() {
    let progress = Arc::new(Progress::default());
    let mut frame = FrameProgress::new(progress.clone(), 1000, None).unwrap();
    let (mut writer, reader) = tokio::io::duplex(32);
    let write = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            writer.write_all(b" ").await.unwrap();
        }
    });
    let mut reader = BufReader::new(reader);
    let start = Instant::now();
    let error = progress.wait(read_frame_traced(&mut reader, None, Some(&mut frame)), history_limits()).await.unwrap_err();
    assert_eq!(error, Timeout::Total(Duration::from_secs(600)));
    assert_eq!(start.elapsed(), Duration::from_secs(600));
    write.abort();
}

#[tokio::test(start_paused = true)]
async fn repeated_announcements_without_data_do_not_renew_a_lease() {
    let progress = Arc::new(Progress::default());
    progress.begin();
    let sender = progress.clone();
    let task = tokio::spawn(async move {
        loop { tokio::time::sleep(Duration::from_secs(10)).await; sender.begin(); }
    });
    assert_eq!(progress.wait(std::future::pending::<()>(), history_limits()).await,
        Err(Timeout::Idle(Duration::from_secs(45))));
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn legacy_archive_chunks_renew_only_their_own_request_beyond_120_seconds() {
    let rpc = Arc::new(ArchiveRpc::default());
    let (receiver, progress) = rpc.register("wanted").unwrap();
    let sender = rpc.clone();
    let task = tokio::spawn(async move {
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_secs(60)).await;
            assert!(sender.route(&json!({"type":"archive_chunk","id":"wanted","name":"transcript.jsonl","data":"YWJj"})));
        }
        sender.route(&json!({"type":"archive_result","id":"wanted","value":{"version":1}}));
    });
    let start = Instant::now();
    let result = progress.wait(receiver, Limits::archive()).await.unwrap().unwrap().unwrap();
    assert_eq!(result["chunks"]["transcript.jsonl"].as_array().unwrap().len(), 5);
    assert_eq!(start.elapsed(), Duration::from_secs(300));
    task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn announced_archive_result_renews_before_its_json_line_is_complete() {
    let rpc = ArchiveRpc::default();
    let (receiver, progress) = rpc.register("wanted").unwrap();
    let result = json!({"type":"archive_result","id":"wanted","value":{"padding":"x".repeat(1024)}});
    let bytes = wire(&result);
    let marker = json!({"type":"progress","protocol":1,"stage":"archive_transfer","id":"wanted","bytes":bytes.len()});
    let mut frame = rpc.frame_progress(&marker).unwrap().unwrap();
    let (mut writer, reader) = tokio::io::duplex(32);
    let write = tokio::spawn(async move {
        for chunk in bytes.chunks(100) {
            tokio::time::sleep(Duration::from_secs(20)).await;
            writer.write_all(chunk).await.unwrap();
        }
    });
    let mut reader = BufReader::new(reader);
    let start = Instant::now();
    let response = progress.wait(async {
        let value = read_frame_traced(&mut reader, None, Some(&mut frame)).await.unwrap().unwrap();
        assert!(rpc.route(&value));
        receiver.await.unwrap().unwrap()
    }, Limits::archive()).await.unwrap();
    assert_eq!(response["padding"], result["value"]["padding"]);
    assert!(start.elapsed() > Duration::from_secs(120));
    write.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn heartbeat_terminal_stale_ids_and_empty_chunks_do_not_extend_a_stalled_archive() {
    let rpc = Arc::new(ArchiveRpc::default());
    let (receiver, progress) = rpc.register("wanted").unwrap();
    rpc.route(&json!({"type":"archive_chunk","id":"wanted","name":"transcript.jsonl","data":"YWJj"}));
    let sender = rpc.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            for value in [json!({"type":"pong","id":1}), json!({"type":"terminal","data":"YWJj"}),
                json!({"type":"archive_chunk","id":"old","name":"transcript.jsonl","data":"YWJj"}),
                json!({"type":"archive_chunk","id":"wanted","name":"transcript.jsonl","data":""})] {
                assert!(sender.frame_progress(&value).unwrap().is_none());
                sender.route(&value);
            }
        }
    });
    assert_eq!(progress.wait(receiver, Limits::archive()).await.unwrap_err(), Timeout::Idle(Duration::from_secs(120)));
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn cancelled_request_data_cannot_renew_a_replacement_request() {
    let rpc = Arc::new(ArchiveRpc::default());
    let (_, old) = rpc.register("old").unwrap();
    let mut late_frame = rpc.frame_progress(&json!({"type":"progress","protocol":1,
        "stage":"archive_transfer","id":"old","bytes":100})).unwrap().unwrap();
    rpc.cancel("old");
    let (receiver, new) = rpc.register("new").unwrap();
    assert!(!Arc::ptr_eq(&old, &new));
    assert!(rpc.frame_progress(&json!({"type":"progress","protocol":1,
        "stage":"archive_transfer","id":"old","bytes":100})).unwrap().is_none());
    let task = tokio::spawn(async move {
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_secs(20)).await;
            late_frame.received(1).unwrap();
        }
    });
    assert_eq!(new.wait(receiver, Limits::archive()).await.unwrap_err(), Timeout::Initial(Duration::from_secs(120)));
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn transport_eof_and_explicit_remote_errors_are_not_relabelled_as_timeout() {
    let progress = Arc::new(Progress::default());
    let mut frame = FrameProgress::new(progress.clone(), 100, None).unwrap();
    let mut reader = &b"{\"type\":"[..];
    let error = progress.wait(read_frame_traced(&mut reader, None, Some(&mut frame)), history_limits()).await.unwrap().unwrap_err();
    assert!(errors::is_disconnected(&error) && error.contains("Truncated"));
    let rpc = ArchiveRpc::default();
    let (receiver, progress) = rpc.register("wanted").unwrap();
    rpc.route(&json!({"type":"archive_error","id":"wanted","message":"Invalid native archive cursor"}));
    let error = progress.wait(receiver, Limits::archive()).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error, "Invalid native archive cursor");
    assert!(!errors::is_disconnected(&error));
}

#[test]
fn marker_sizes_and_request_identity_are_validated() {
    for size in [json!(0), json!(-1), json!(super::super::MAX_FRAME + 1), json!("100"), Value::Null] {
        assert!(announced_size(&json!({"bytes":size})).is_err());
    }
    let progress = Arc::new(Progress::default());
    let mut frame = FrameProgress::new(progress.clone(), 10, Some("wanted".into())).unwrap();
    frame.received(10).unwrap();
    assert!(frame.complete(&json!({"type":"pong","id":"wanted"})).is_err());
    assert!(frame.complete(&json!({"type":"archive_result","id":"old"})).is_err());
    assert!(frame.complete(&json!({"type":"archive_result","id":"wanted"})).is_ok());
    assert!(frame.received(1).is_err());
}

#[tokio::test(start_paused = true)]
async fn transfer_begin_near_startup_deadline_gets_its_own_idle_window() {
    let progress = Arc::new(Progress::default());
    let sender = progress.clone();
    let (send, receive) = oneshot::channel();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(44)).await;
        sender.begin();
        tokio::time::sleep(Duration::from_secs(40)).await;
        sender.received();
        send.send(7).unwrap();
    });
    assert_eq!(progress.wait(receive, history_limits()).await.unwrap().unwrap(), 7);
}
