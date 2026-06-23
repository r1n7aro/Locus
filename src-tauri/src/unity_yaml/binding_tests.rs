//! Focused coverage for serialized member-binding capture + resolution
//! (`parse_yaml_docs_with_refs_and_bindings` + `build_bindings_from_docs`).

use super::references::build_bindings_from_docs;
use crate::asset_db::types::parse_guid_hex;

/// One scene-like fixture exercising all three binding shapes:
/// - a bound UnityEvent call whose `m_Target` points at a MonoBehaviour with an
///   `m_Script` GUID and a host GameObject name,
/// - an unbound UnityEvent call (`m_Target: {fileID: 0}`) named only by
///   `m_TargetAssemblyTypeName`,
/// - an AnimationEvent `functionName` (no target at all).
const FIXTURE: &[u8] = br#"%YAML 1.1
--- !u!1 &100
GameObject:
  m_Name: Player
  m_Component:
  - component: {fileID: 200}
--- !u!114 &200
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Script: {fileID: 11500000, guid: 1111111111111111111111111111aaaa, type: 3}
  m_OnClick:
    m_PersistentCalls:
      m_Calls:
      - m_Target: {fileID: 200}
        m_TargetAssemblyTypeName: Game.UI.HealthBar, Assembly-CSharp
        m_MethodName: Refresh
        m_Mode: 1
      - m_Target: {fileID: 0}
        m_TargetAssemblyTypeName: Game.Combat.Enemy, Assembly-CSharp
        m_MethodName: TakeDamage
        m_Mode: 1
--- !u!74 &300
AnimationClip:
  m_Name: Walk
  m_Events:
  - time: 0.5
    functionName: OnFootstep
    data:
"#;

#[test]
fn build_bindings_resolves_bound_unbound_and_anim() {
    let (docs, _refs, raw) = super::parse_yaml_docs_with_refs_and_bindings(FIXTURE);
    let bindings = build_bindings_from_docs(&docs, raw);
    assert_eq!(bindings.len(), 3, "expected three captured bindings");

    // Bound UnityEvent call -> resolved script GUID + GameObject name.
    let bound = bindings
        .iter()
        .find(|b| b.method_name == "Refresh")
        .expect("Refresh binding");
    assert_eq!(bound.binding_kind, 0);
    assert_eq!(bound.target_type_full, "Game.UI.HealthBar");
    assert_eq!(bound.target_type_short_lower, "healthbar");
    assert_eq!(bound.target_file_id, Some(200));
    assert_eq!(
        bound.target_script_guid,
        parse_guid_hex("1111111111111111111111111111aaaa")
    );
    assert_eq!(bound.target_go_name, "Player");

    // Unbound UnityEvent call -> type name only, fileID 0, no guid/go.
    let unbound = bindings
        .iter()
        .find(|b| b.method_name == "TakeDamage")
        .expect("TakeDamage binding");
    assert_eq!(unbound.binding_kind, 0);
    assert_eq!(unbound.target_type_full, "Game.Combat.Enemy");
    assert_eq!(unbound.target_type_short_lower, "enemy");
    assert_eq!(unbound.target_file_id, Some(0));
    assert_eq!(unbound.target_script_guid, None);
    assert_eq!(unbound.target_go_name, "");

    // AnimationEvent -> kind 1, everything target_* empty.
    let anim = bindings
        .iter()
        .find(|b| b.method_name == "OnFootstep")
        .expect("OnFootstep binding");
    assert_eq!(anim.binding_kind, 1);
    assert_eq!(anim.target_type_full, "");
    assert_eq!(anim.target_type_short_lower, "");
    assert_eq!(anim.target_file_id, None);
    assert_eq!(anim.target_script_guid, None);
    assert_eq!(anim.target_go_name, "");
}

/// A serialized field literally named `functionName` outside an AnimationClip
/// (here a MonoBehaviour, class 114) must NOT be captured as an AnimationEvent
/// — otherwise `unity_code_usages` would misreport it as `[AnimationEvent]`,
/// since kind-1 bindings are returned class-agnostically. The class-74 clip in
/// the same fixture confirms the gate discriminates by class rather than
/// suppressing every `functionName`.
const FIXTURE_FUNCTIONNAME_FIELD: &[u8] = br#"%YAML 1.1
--- !u!114 &200
MonoBehaviour:
  m_Script: {fileID: 11500000, guid: 1111111111111111111111111111aaaa, type: 3}
  functionName: Ghost
--- !u!74 &300
AnimationClip:
  m_Name: Walk
  m_Events:
  - time: 0.5
    functionName: OnFootstep
    data:
"#;

#[test]
fn monobehaviour_functionname_field_is_not_an_anim_event() {
    let (docs, _refs, raw) =
        super::parse_yaml_docs_with_refs_and_bindings(FIXTURE_FUNCTIONNAME_FIELD);
    let bindings = build_bindings_from_docs(&docs, raw);

    // The MonoBehaviour's `functionName: Ghost` field is ignored …
    assert!(
        !bindings.iter().any(|b| b.method_name == "Ghost"),
        "a functionName field on a MonoBehaviour must not be captured as a binding"
    );
    // … while the real AnimationClip (class 74) event is still captured.
    assert_eq!(bindings.len(), 1, "only the AnimationClip event should bind");
    assert_eq!(bindings[0].binding_kind, 1);
    assert_eq!(bindings[0].method_name, "OnFootstep");
}
