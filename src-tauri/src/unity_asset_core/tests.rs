use super::*;

const HEADER: &str =
    "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n";

fn asset(body: &str) -> Vec<u8> {
    format!("{HEADER}{body}").into_bytes()
}
fn include_all(session: &MergeSession) -> Vec<Decision> {
    session
        .catalog()
        .changes
        .iter()
        .map(|c| Decision {
            change_id: c.id.clone(),
            resolution: Resolution::Include,
        })
        .collect()
}
fn rendered(base: &[u8], target: &[u8], source: &[u8]) -> MergeOutput {
    let session = prepare_merge(base, target, source).unwrap();
    session.render(&include_all(&session)).unwrap()
}

fn graph() -> Vec<u8> {
    asset(concat!(
        "  m_Script: {fileID: 11500000, guid: aabbccdd00112233445566778899aabb, type: 3}\n",
        "  localValue: 10\n  incomingValue: 20\n  unknownFutureField: {x: 19, y: 23}\n",
        "  root: {rid: 101}\n  alias: {rid: 9007199254740993}\n  optional: {rid: -2}\n",
        "  references:\n    version: 2\n    RefIds:\n",
        "    - rid: 101\n      type: {class: MergeGroup, ns: Locus.MergeTesting, asm: Locus}\n      data:\n",
        "        label: Root\n        next: {rid: 9007199254740993}\n        children:\n",
        "        - rid: 9007199254740993\n        - rid: 103\n        - rid: 9007199254740993\n",
        "    - rid: 9007199254740993\n      type: {class: MergeAction, ns: Locus.MergeTesting, asm: Locus}\n      data:\n",
        "        label: Shared\n        next: {rid: 101}\n        health: 100\n        speed: 1\n",
        "    - rid: 103\n      type: {class: MergeAction, ns: Locus.MergeTesting, asm: Locus}\n      data:\n",
        "        label: Alternate\n        next: {rid: -2}\n        health: 30\n        speed: 3\n",
    ))
}

fn replace(bytes: &[u8], old: &str, new: &str) -> Vec<u8> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace(old, new)
        .into_bytes()
}

#[test]
fn no_op_is_byte_exact_with_bom_crlf_unknown_fields_and_comments() {
    let bytes=format!("\u{feff}{}",String::from_utf8(asset("  unknown: \"value: # literal\" # comment\n  empty:\n  null: null\n  list: [1, '2', {a: 3}]\n  block: |-\n    one: literal\n    # text\n\n")).unwrap().replace('\n',"\r\n")).into_bytes();
    let parsed = parse(&bytes).unwrap();
    assert_eq!(parsed.bytes.as_ref(), bytes.as_slice());
    let session = prepare_merge(&bytes, &bytes, &bytes).unwrap();
    assert!(session.catalog().changes.is_empty());
    assert_eq!(session.render(&[]).unwrap().bytes, bytes);
}

#[test]
fn parses_nested_flow_quotes_wrapped_plain_strings_and_indented_sequences() {
    let bytes=asset(concat!(
        "  nested: {first: [1, {label: 'a, b: it''s', ref: {fileID: 0}}],\n    second: \"escaped \\\" Unicode \\u4e2d\"}\n",
        "  'key: with spaces': value\n  wrapped: packed\n    continuation\n",
        "  list:\n    - name: one\n      nested:\n        - value: 1\n        - value: 2\n    - name: two\n      nested: []\n",
        "  tail: final\n",
    ));
    let parsed = parse(&bytes).unwrap();
    let body = parsed.documents[0].root.get("MonoBehaviour").unwrap();
    assert_eq!(body.get("list").unwrap().items().unwrap().len(), 2);
    assert!(body
        .get("nested")
        .unwrap()
        .get("first")
        .unwrap()
        .items()
        .is_some());
    assert_eq!(body.get("tail").unwrap().scalar(&parsed), Some("final"));
}

#[test]
fn missing_empty_null_and_quoted_empty_are_distinct() {
    let base = asset("  value:\n  emptyString: \"\"\n  explicitNull: null\n");
    let source = asset("  value: null\n  emptyString:\n");
    let session = prepare_merge(&base, &base, &source).unwrap();
    assert_eq!(session.catalog().changes.len(), 3);
    let result = session.render(&include_all(&session)).unwrap();
    assert!(result.ready);
    assert_eq!(result.bytes, source);
    let body = parse(&base).unwrap();
    let map = body.documents[0].root.get("MonoBehaviour").unwrap();
    assert_ne!(
        map.get("value").unwrap().fingerprint,
        map.get("emptyString").unwrap().fingerprint
    );
    assert_ne!(
        map.get("value").unwrap().fingerprint,
        map.get("explicitNull").unwrap().fingerprint
    );
}

#[test]
fn flow_vector_fields_merge_independently_and_preserve_unselected_bytes() {
    let base = asset("  position: {x: 0, y: 0, z: 0}\n  hidden: untouched # keep\n");
    let target = replace(&base, "x: 0", "x: 3");
    let source = replace(&base, "y: 0", "y: 4");
    let result = rendered(&base, &target, &source);
    assert!(result.ready);
    assert_eq!(result.bytes, replace(&target, "y: 0", "y: 4"));
    let session = prepare_merge(&base, &target, &source).unwrap();
    assert_eq!(session.render(&[]).unwrap().bytes, target);
}

#[test]
fn shared_cyclic_serialize_reference_graph_merges_fields_without_id_rounding() {
    let base = graph();
    let target = replace(
        &replace(&base, "health: 100", "health: 75"),
        "localValue: 10",
        "localValue: 99",
    );
    let source = replace(
        &replace(
            &replace(&base, "speed: 1\n", "speed: 2.5\n"),
            "incomingValue: 20",
            "incomingValue: 88",
        ),
        "label: Alternate",
        "label: Selected alternate",
    );
    let session = prepare_merge(&base, &target, &source).unwrap();
    assert_eq!(session.catalog().changes.len(), 3);
    assert!(session
        .catalog()
        .changes
        .iter()
        .any(|c| c.property_path.contains("@rid=9007199254740993/data/speed")));
    let result = session.render(&include_all(&session)).unwrap();
    assert!(result.ready, "{:?}", result.conflicts);
    let text = String::from_utf8(result.bytes.clone()).unwrap();
    assert!(text.contains("health: 75"));
    assert!(text.contains("speed: 2.5"));
    assert!(text.contains("localValue: 99"));
    assert!(text.contains("incomingValue: 88"));
    assert!(text.contains("unknownFutureField: {x: 19, y: 23}"));
    assert_eq!(
        parse(&result.bytes)
            .unwrap()
            .references()
            .iter()
            .filter(|r| r.rid.as_deref() == Some("9007199254740993"))
            .count(),
        4
    );
    assert!(validate(&parse(&result.bytes).unwrap()).is_empty());
}

#[test]
fn agent_selects_conflict_free_subset_and_ids_do_not_depend_on_target() {
    let base = graph();
    let target = replace(&base, "localValue: 10", "localValue: 99");
    let source = replace(
        &replace(&base, "speed: 1\n", "speed: 2.5\n"),
        "incomingValue: 20",
        "incomingValue: 88",
    );
    let session = prepare_merge(&base, &target, &source).unwrap();
    let clean = prepare_merge(&base, &base, &source).unwrap();
    assert_eq!(
        session
            .catalog()
            .changes
            .iter()
            .map(|c| &c.id)
            .collect::<Vec<_>>(),
        clean
            .catalog()
            .changes
            .iter()
            .map(|c| &c.id)
            .collect::<Vec<_>>()
    );
    let selected = session
        .catalog()
        .changes
        .iter()
        .find(|c| c.property_path.ends_with("/speed"))
        .unwrap();
    let result = session
        .render(&[Decision {
            change_id: selected.id.clone(),
            resolution: Resolution::Include,
        }])
        .unwrap();
    assert_eq!(result.bytes, replace(&target, "speed: 1\n", "speed: 2.5\n"));
}

#[test]
fn conflicting_field_requires_explicit_agent_resolution() {
    let base = graph();
    let target = replace(&base, "health: 100", "health: 75");
    let source = replace(&base, "health: 100", "health: 42");
    let session = prepare_merge(&base, &target, &source).unwrap();
    let change = &session.catalog().changes[0];
    assert_eq!(change.status, ChangeStatus::Conflict);
    let blocked = session.render(&include_all(&session)).unwrap();
    assert!(!blocked.ready);
    assert_eq!(blocked.bytes, target);
    let resolved = session
        .render(&[Decision {
            change_id: change.id.clone(),
            resolution: Resolution::Source,
        }])
        .unwrap();
    assert!(resolved.ready);
    assert_eq!(resolved.bytes, source);
}

#[test]
fn type_change_and_old_type_data_edit_are_one_conflict() {
    let base = graph();
    let target = replace(&base, "health: 100", "health: 75");
    let source = replace(
        &replace(&base, "class: MergeAction", "class: MergeWeightedAction"),
        "speed: 1\n",
        "weight: 9\n",
    );
    let session = prepare_merge(&base, &target, &source).unwrap();
    let changed = session
        .catalog()
        .changes
        .iter()
        .find(|c| c.property_path.ends_with("@rid=9007199254740993"))
        .unwrap();
    assert_eq!(changed.status, ChangeStatus::Conflict);
    assert!(changed.reason.as_deref().unwrap().contains("type identity"));
    let blocked = session.render(&include_all(&session)).unwrap();
    assert!(!blocked.ready);
    let selected = session
        .render(&[Decision {
            change_id: changed.id.clone(),
            resolution: Resolution::Source,
        }])
        .unwrap();
    assert!(selected.ready);
    let text = String::from_utf8(selected.bytes).unwrap();
    assert!(text.contains("weight: 9"));
    assert!(text.contains("health: 100"));
}

#[test]
fn identity_list_reordering_cannot_silently_apply_positional_edits() {
    let base = graph();
    let target = replace(&base, "health: 100", "health: 75");
    let source = replace(
        &base,
        "        - rid: 103\n        - rid: 9007199254740993",
        "        - rid: 9007199254740993\n        - rid: 103",
    );
    let session = prepare_merge(&base, &target, &source).unwrap();
    // Shared-reference arrays contain duplicates, so list order is an atomic
    // field; editing a different registry object remains independently safe.
    let result = session.render(&include_all(&session)).unwrap();
    assert!(result.ready);
    assert!(String::from_utf8(result.bytes)
        .unwrap()
        .contains("health: 75"));
    let target_reordered = replace(
        &base,
        "        - rid: 9007199254740993\n        - rid: 103",
        "        - rid: 103\n        - rid: 9007199254740993",
    );
    assert!(!rendered(&base, &target_reordered, &source).ready);
}

#[test]
fn ordinary_duplicate_value_arrays_conflict_instead_of_index_pairing() {
    let base = asset("  values: [1, 1, 2]\n");
    let target = asset("  values: [1, 2, 1]\n");
    let source = asset("  values: [9, 1, 2]\n");
    let result = rendered(&base, &target, &source);
    assert!(!result.ready);
    assert_eq!(result.bytes, target);
}

#[test]
fn pptr_guid_and_file_id_cannot_form_a_synthetic_reference() {
    let base =
        asset("  reference: {fileID: 100, guid: aabbccdd00112233445566778899aabb, type: 3}\n");
    let target = replace(&base, "fileID: 100", "fileID: 200");
    let source = replace(
        &base,
        "aabbccdd00112233445566778899aabb",
        "112233445566778899aabbccddeeff00",
    );
    let session = prepare_merge(&base, &target, &source).unwrap();
    assert_eq!(session.catalog().changes.len(), 1);
    assert!(!session.render(&include_all(&session)).unwrap().ready);
}

#[test]
fn selecting_reference_without_its_registry_addition_returns_dependency_conflict() {
    let base = graph();
    let source = replace(&base, "optional: {rid: -2}", "optional: {rid: 104}");
    let session = prepare_merge(&base, &base, &source).unwrap();
    let result = session.render(&include_all(&session)).unwrap();
    assert!(!result.ready);
    assert!(result
        .conflicts
        .iter()
        .any(|c| c.code == "dangling_managed_reference"));
    assert_eq!(result.bytes, base);
}

#[test]
fn deleting_a_referenced_object_is_blocked_until_owner_reference_is_removed() {
    let base=b"--- !u!1 &1\nGameObject:\n  m_Component:\n  - component: {fileID: 2}\n--- !u!114 &2\nMonoBehaviour:\n  m_GameObject: {fileID: 1}\n  value: 0\n";
    let source = b"--- !u!1 &1\nGameObject:\n  m_Component: []\n";
    let session = prepare_merge(base, base, source).unwrap();
    let deletion = session
        .catalog()
        .changes
        .iter()
        .find(|c| c.kind == ChangeKind::RemoveObject)
        .unwrap();
    let blocked = session
        .render(&[Decision {
            change_id: deletion.id.clone(),
            resolution: Resolution::Include,
        }])
        .unwrap();
    assert!(!blocked.ready);
    assert!(session.render(&include_all(&session)).unwrap().ready);
}

#[test]
fn duplicate_keys_rids_objects_and_unsupported_syntax_never_silently_write() {
    let duplicate = asset("  value: 1\n  value: 2\n");
    let parsed = parse(&duplicate).unwrap();
    assert!(!parsed.is_writable());
    assert!(prepare_merge(&duplicate, &duplicate, &duplicate).is_err());
    let duplicate_rid = replace(&graph(), "rid: 103\n      type:", "rid: 101\n      type:");
    assert!(validate(&parse(&duplicate_rid).unwrap())
        .iter()
        .any(|d| d.code == "duplicate_rid"));
    assert!(parse(&asset("  value: *alias\n")).is_err());
    assert!(parse(&asset("  value: [1, 2\n")).is_err());
    assert!(parse(b"\xff\x00").is_err());
    let repeated = b"--- !u!1 &1\nGameObject:\n  value: 1\n--- !u!1 &1\nGameObject:\n  value: 2\n";
    assert!(!parse(repeated).unwrap().is_writable());
}

#[test]
fn typed_field_set_quotes_content_and_never_injects_yaml() {
    let base = asset("  name: before\n  preserve: 19\n");
    let source = asset("  name: after\n  preserve: 19\n");
    let session = prepare_merge(&base, &base, &source).unwrap();
    let result = session
        .render(&[Decision {
            change_id: session.catalog().changes[0].id.clone(),
            resolution: Resolution::Set {
                value: serde_json::json!("new\n  preserve: 0"),
            },
        }])
        .unwrap();
    assert!(result.ready);
    let text = String::from_utf8(result.bytes).unwrap();
    assert!(text.contains("name: \"new\\n  preserve: 0\""));
    assert!(text.contains("\n  preserve: 19\n"));
}

#[test]
fn additions_deletions_and_last_line_without_newline_preserve_other_fields() {
    let base = asset("  remove: 1\n  keep: 2");
    let target = replace(&base, "keep: 2", "keep: 5");
    let source = asset("  keep: 2\n  added: {nested: [1, 2]}\n");
    let result = rendered(&base, &target, &source);
    assert!(result.ready);
    assert_eq!(
        result.bytes,
        asset("  keep: 5\n  added: {nested: [1, 2]}\n")
    );
}

#[test]
fn object_and_file_selection_reject_overlapping_field_decisions() {
    let base = graph();
    let source = replace(&base, "incomingValue: 20", "incomingValue: 88");
    let session = prepare_merge(&base, &base, &source).unwrap();
    let mut decisions = include_all(&session);
    decisions.push(MergeSession::object_decision(
        "11400000",
        Resolution::Source,
    ));
    assert!(session.render(&decisions).is_err());
    assert_eq!(
        session
            .render(&[MergeSession::object_decision(
                "11400000",
                Resolution::Source
            )])
            .unwrap()
            .bytes,
        source
    );
    assert_eq!(
        session
            .render(&[MergeSession::file_decision(Resolution::Source)])
            .unwrap()
            .bytes,
        source
    );
}

#[test]
fn prefab_overrides_pair_by_target_and_property_path_and_stripped_objects_survive() {
    let base=concat!("--- !u!1001 &10\nPrefabInstance:\n  m_Modification:\n    m_Modifications:\n",
        "    - target: {fileID: 11, guid: aabbccdd00112233445566778899aabb, type: 3}\n      propertyPath: m_LocalPosition.x\n      value: 0\n      objectReference: {fileID: 0}\n",
        "    - target: {fileID: 11, guid: aabbccdd00112233445566778899aabb, type: 3}\n      propertyPath: m_LocalPosition.y\n      value: 0\n      objectReference: {fileID: 0}\n",
        "--- !u!4 &11 stripped\nTransform:\n  m_CorrespondingSourceObject: {fileID: 55, guid: aabbccdd00112233445566778899aabb, type: 3}\n  m_PrefabInstance: {fileID: 10}\n").as_bytes();
    let target = replace(
        base,
        "m_LocalPosition.x\n      value: 0",
        "m_LocalPosition.x\n      value: 3",
    );
    let source = replace(
        base,
        "m_LocalPosition.y\n      value: 0",
        "m_LocalPosition.y\n      value: 4",
    );
    let result = rendered(base, &target, &source);
    assert!(result.ready);
    assert_eq!(
        result.bytes,
        replace(
            &target,
            "m_LocalPosition.y\n      value: 0",
            "m_LocalPosition.y\n      value: 4"
        )
    );
    assert!(
        parse(&result.bytes)
            .unwrap()
            .document("11")
            .unwrap()
            .stripped
    );
}

#[test]
fn parser_limits_are_enforced_before_unbounded_recursion() {
    let bytes = asset("  nested: {one: {two: {three: 1}}}\n");
    assert!(Asset::parse_with_limits(
        &bytes,
        Limits {
            max_depth: 2,
            ..Limits::default()
        }
    )
    .is_err());
    assert!(Asset::parse_with_limits(
        &bytes,
        Limits {
            max_nodes: 2,
            ..Limits::default()
        }
    )
    .is_err());
    assert!(Asset::parse_with_limits(
        &bytes,
        Limits {
            max_bytes: 4,
            ..Limits::default()
        }
    )
    .is_err());
}

#[test]
fn serde_contract_uses_string_ids_and_explicit_typed_resolution() {
    let resolution: Resolution =
        serde_json::from_value(serde_json::json!({"kind":"set","value":"9007199254740993"}))
            .unwrap();
    assert!(matches!(
        resolution,
        Resolution::Set {
            value: serde_json::Value::String(_)
        }
    ));
    let session = prepare_merge(
        &graph(),
        &graph(),
        &replace(&graph(), "health: 100", "health: 101"),
    )
    .unwrap();
    let json = serde_json::to_value(session.catalog()).unwrap();
    assert_eq!(json["changes"][0]["object_id"], "11400000");
}

#[test]
fn proven_formerly_serialized_as_migrates_only_selected_values() {
    let base = asset("  health: 100\n  local: 7\n");
    let target = asset("  hitPoints: 100\n  local: 99\n");
    let source = asset("  health: 120\n  local: 7\n");
    let alias = FieldAlias {
        object_id: "11400000".into(),
        old_path: "/MonoBehaviour/health".into(),
        new_path: "/MonoBehaviour/hitPoints".into(),
    };
    let session = prepare_merge_with_aliases(&base, &target, &source, &[alias.clone()]).unwrap();
    assert_eq!(session.render(&[]).unwrap().bytes, target);
    assert_eq!(session.catalog().changes.len(), 1);
    assert_eq!(
        session.catalog().changes[0].property_path,
        "/MonoBehaviour/hitPoints"
    );
    let result = session.render(&include_all(&session)).unwrap();
    assert!(result.ready);
    assert_eq!(result.bytes, asset("  hitPoints: 120\n  local: 99\n"));
    assert_eq!(
        session
            .render(&[MergeSession::file_decision(Resolution::Source)])
            .unwrap()
            .bytes,
        source
    );
    let duplicate = asset("  health: 100\n  hitPoints: 50\n  local: 7\n");
    assert!(prepare_merge_with_aliases(&duplicate, &target, &source, &[alias]).is_err());
}

#[test]
fn explicit_typed_fields_work_without_source_deltas_and_insert_only_in_existing_parent() {
    let base = asset("  value: 1\n  rows:\n  - name: old\n    amount: 3\n");
    let session = prepare_merge(&base, &base, &base).unwrap();
    let decisions = vec![
        MergeSession::field_decision(
            "11400000",
            "/MonoBehaviour/value",
            Resolution::Set {
                value: serde_json::json!(9),
            },
        ),
        MergeSession::field_decision(
            "11400000",
            "/MonoBehaviour/added",
            Resolution::Set {
                value: serde_json::json!({"x":1,"y":2}),
            },
        ),
        MergeSession::field_decision(
            "11400000",
            "/MonoBehaviour/rows/0/name",
            Resolution::Set {
                value: serde_json::json!("new"),
            },
        ),
    ];
    let result = session.render(&decisions).unwrap();
    assert!(result.ready);
    let text = String::from_utf8(result.bytes).unwrap();
    assert!(text.contains("  value: 9\n"));
    assert!(text.contains("  added: {x: 1, y: 2}\n"));
    assert!(text.contains("  - name: \"new\"\n"));
    assert!(session
        .render(&[MergeSession::field_decision(
            "11400000",
            "/MonoBehaviour/missing/child",
            Resolution::Set {
                value: serde_json::json!(1)
            }
        )])
        .is_err());
}

#[test]
fn empty_sequence_elements_remain_separate_and_unknown_registry_data_is_preserved() {
    let bytes = asset("  strings:\n  - first\n  - \n  - second\n  - \n  tail: done\n");
    let parsed = parse(&bytes).unwrap();
    assert_eq!(
        parsed.documents[0]
            .root
            .get("MonoBehaviour")
            .unwrap()
            .get("strings")
            .unwrap()
            .items()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        prepare_merge(&bytes, &bytes, &bytes)
            .unwrap()
            .render(&[])
            .unwrap()
            .bytes,
        bytes
    );
}

#[test]
fn shared_parse_cache_reuses_syntax_without_cross_snapshot_state() {
    let bytes = graph();
    let a = parse_shared(&bytes).unwrap();
    let b = parse_shared(&bytes).unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &b));
    let changed = parse_shared(&replace(&bytes, "health: 100", "health: 105")).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&a, &changed));
    assert!(a.text(a.documents[0].span).contains("health: 100"));
}

#[test]
fn object_reparent_requires_both_parent_lists_and_rejects_cycles() {
    let bytes=b"--- !u!4 &1\nTransform:\n  m_Father: {fileID: 0}\n  m_Children:\n  - {fileID: 3}\n--- !u!4 &2\nTransform:\n  m_Father: {fileID: 0}\n  m_Children: []\n--- !u!4 &3\nTransform:\n  m_Father: {fileID: 1}\n  m_Children: []\n";
    let session = prepare_merge(bytes, bytes, bytes).unwrap();
    let mut decisions = vec![MergeSession::field_decision(
        "3",
        "/Transform/m_Father",
        Resolution::Set {
            value: serde_json::json!({"fileID":2}),
        },
    )];
    assert!(!session.render(&decisions).unwrap().ready);
    decisions.push(MergeSession::field_decision(
        "1",
        "/Transform/m_Children",
        Resolution::Set {
            value: serde_json::json!([]),
        },
    ));
    decisions.push(MergeSession::field_decision(
        "2",
        "/Transform/m_Children",
        Resolution::Set {
            value: serde_json::json!([{"fileID":3}]),
        },
    ));
    let result = session.render(&decisions).unwrap();
    assert!(result.ready, "{:?}", result.conflicts);
    let cycle = replace(
        &replace(
            bytes,
            "m_Father: {fileID: 0}\n  m_Children:\n  - {fileID: 3}",
            "m_Father: {fileID: 3}\n  m_Children:\n  - {fileID: 3}",
        ),
        "--- !u!4 &3\nTransform:\n  m_Father: {fileID: 1}\n  m_Children: []",
        "--- !u!4 &3\nTransform:\n  m_Father: {fileID: 1}\n  m_Children:\n  - {fileID: 1}",
    );
    assert!(validate(&parse(&cycle).unwrap())
        .iter()
        .any(|d| d.code == "transform_cycle"));
}

#[test]
fn absent_git_snapshots_allow_added_and_removed_entire_files() {
    let bytes = asset("  value: 1\n");
    let addition =
        MergeSession::new(Asset::absent(), Asset::absent(), parse(&bytes).unwrap()).unwrap();
    let added = addition.render(&include_all(&addition)).unwrap();
    assert!(added.ready);
    assert!(String::from_utf8(added.bytes).unwrap().contains("value: 1"));
    let removal = MergeSession::new(
        parse(&bytes).unwrap(),
        parse(&bytes).unwrap(),
        Asset::absent(),
    )
    .unwrap();
    let removed = removal.render(&include_all(&removal)).unwrap();
    assert!(removed.ready);
    assert!(removed.bytes.is_empty());
    assert!(parse(&[]).is_err());
}

#[test]
fn independently_added_equal_ids_are_conflicts_even_with_equal_payload() {
    let bytes = asset("  value: 1\n");
    let session = MergeSession::new(
        Asset::absent(),
        parse(&bytes).unwrap(),
        parse(&bytes).unwrap(),
    )
    .unwrap();
    assert_eq!(session.catalog().changes[0].status, ChangeStatus::Conflict);
    let base = asset("  references:\n    version: 2\n    RefIds: []\n");
    let added=asset("  references:\n    version: 2\n    RefIds:\n    - rid: 1\n      type: {class: Empty, ns: Example, asm: Tests}\n      data: {}\n");
    let session = prepare_merge(&base, &added, &added).unwrap();
    assert_eq!(session.catalog().changes[0].status, ChangeStatus::Conflict);
}
