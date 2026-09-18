use crate::view::{UnitySerializedPropertyTarget, UnitySerializedPropertyWriteResult};

#[test]
fn property_restore_wire_preserves_complete_prefab_identity() {
    let global_id = "GlobalObjectId_V1-2-01234567890123456789012345678901-9007199254740993-9223372036854775807";
    let target: UnitySerializedPropertyTarget = serde_json::from_value(serde_json::json!({
        "kind": "component", "globalObjectId": global_id,
        "targetFileId": "9007199254740993", "propertyPath": "amount"
    })).unwrap();
    assert_eq!(target.global_object_id.as_deref(), Some(global_id));
    let encoded = serde_json::to_value(target).unwrap();
    assert_eq!(encoded["globalObjectId"], global_id);
    assert_eq!(encoded["targetFileId"], "9007199254740993");
}

#[test]
fn property_restore_wire_keeps_before_and_after_state_opaque_and_exact() {
    let state = r#"{"managedId":9223372036854775807,"valueJson":"\"9223372036854775807\"","children":[]}"#;
    let result: UnitySerializedPropertyWriteResult = serde_json::from_value(serde_json::json!({
        "ok": true, "message": "ok", "saved": true,
        "target": {"kind": "asset", "path": "Assets/Fixture.asset", "propertyPath": "node"},
        "propertyPath": "node", "value": "9223372036854775807", "restoreState": state,
        "beforeSnapshot": {"propertyPath": "node", "value": "0", "restoreState": state}
    })).unwrap();
    assert_eq!(result.before_snapshot.as_ref().unwrap().restore_state.as_deref(), Some(state));
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(encoded["restoreState"], state);
    assert_eq!(encoded["beforeSnapshot"]["restoreState"], state);
    assert_eq!(encoded["value"], "9223372036854775807");
}
