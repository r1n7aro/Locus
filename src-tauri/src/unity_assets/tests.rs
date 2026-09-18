use super::*;
fn fixture()->tempfile::TempDir {
    let root=tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("Assets")).unwrap();
    for name in ["A","B"] {std::fs::write(root.path().join(format!("Assets/{name}.asset")),b"--- !u!114 &11400000\nMonoBehaviour:\n  amount: 1\n  numbers: [1, 2]\n").unwrap();}
    root
}
fn input(path:&str,revision:&Value,value:i32)->Value {
    json!({"path":path,"expected_revision":revision,"operations":[{"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/amount","value":value}]})
}
#[tokio::test]
async fn offline_preview_and_batch_cas_without_git_or_editor() {
    let root=fixture();
    let a=execute(root.path(),json!({"action":"read","path":"Assets/A.asset"})).await.unwrap();
    let b=execute(root.path(),json!({"action":"read","path":"Assets/B.asset"})).await.unwrap();
    let entries=vec![input("Assets/A.asset",&a["revision"],7),input("Assets/B.asset",&b["revision"],9)];
    let preview=execute(root.path(),json!({"action":"preview_batch","entries":entries})).await.unwrap();
    assert_eq!(preview["persisted"],false);
    assert_eq!(execute(root.path(),json!({"action":"read","path":"Assets/A.asset"})).await.unwrap()["revision"],a["revision"]);
    let mut stale=entries.clone(); stale[1]["expected_revision"]=json!("stale");
    assert!(execute(root.path(),json!({"action":"apply_batch","entries":stale})).await.unwrap_err().contains("stale_revision"));
    assert_eq!(execute(root.path(),json!({"action":"read","path":"Assets/A.asset"})).await.unwrap()["revision"],a["revision"]);
    let applied=execute(root.path(),json!({"action":"apply_batch","entries":entries})).await.unwrap();
    assert_eq!(applied["persisted"],true);
    assert!(std::fs::read_to_string(root.path().join("Assets/A.asset")).unwrap().contains("amount: 7"));
    assert!(execute(root.path(),json!({"action":"apply_batch","entries":entries})).await.unwrap_err().contains("stale_revision"));
}
#[tokio::test]
async fn malformed_batch_never_changes_an_earlier_asset() {
    let root=fixture();
    let a=execute(root.path(),json!({"action":"read","path":"Assets/A.asset"})).await.unwrap();
    let b=execute(root.path(),json!({"action":"read","path":"Assets/B.asset"})).await.unwrap();
    let mut bad=input("Assets/B.asset",&b["revision"],9); bad["operations"][0]["property_path"]=json!("/MonoBehaviour/missing");
    assert!(execute(root.path(),json!({"action":"apply_batch","entries":[input("Assets/A.asset",&a["revision"],7),bad]})).await.is_err());
    assert_eq!(execute(root.path(),json!({"action":"read","path":"Assets/A.asset"})).await.unwrap()["revision"],a["revision"]);
    assert!(execute(root.path(),json!({"action":"read","path":"Assets/../A.asset"})).await.is_err());
}
