use super::*;
use std::collections::{BTreeMap, HashSet};

#[test]
fn live_schema_diagnostics_are_preserved_per_asset_and_mirrored_to_snapshots() {
    let runtime =
        json!({"code":"unavailable_live_object","severity":"warning","message":"omitted object"});
    let schema =
        json!({"code":"schema_unverified","severity":"warning","message":"source unavailable"});
    let reports = BTreeMap::from([
        ("Assets/A.asset".into(), vec![schema.clone()]),
        ("Assets/B.asset".into(), vec![]),
    ]);
    let mut response = json!({"results":[
        {"path":"Assets/A.asset","diagnostics":[],"snapshot":{"objects":[],"diagnostics":[runtime]}},
        {"path":"Assets/B.asset","diagnostics":[],"snapshot":{"objects":[],"diagnostics":[]}}
    ]});
    merge_live_schema_diagnostics(&mut response, &reports);
    assert_eq!(
        response["results"][0]["diagnostics"],
        json!([runtime, schema])
    );
    assert_eq!(
        response["results"][0]["snapshot"]["diagnostics"],
        response["results"][0]["diagnostics"]
    );
    assert_eq!(response["results"][1]["diagnostics"], json!([]));
    merge_live_schema_diagnostics(&mut response, &reports);
    assert_eq!(
        response["results"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn mixed_prefab_edit_snapshots_use_the_same_materialized_object_scope_as_reads() {
    let full = json!({"revision":"revision","objects":[{"object_id":"1"},{"object_id":"1001"},{"object_id":"3"}],"diagnostics":[]});
    let ids = HashSet::from(["1".to_string()]);
    let mut read = full.clone();
    filter_materialized_snapshot(&mut read, &ids);
    let mut response = json!({"results":[
        {"path":"Assets/Mixed.prefab","snapshot":full,"diagnostics":[]},
        {"path":"Assets/Other.asset","snapshot":full,"diagnostics":[]}
    ]});
    filter_materialized_edits(
        &mut response,
        &BTreeMap::from([("Assets/Mixed.prefab".into(), ids)]),
    );
    assert_eq!(response["results"][0]["snapshot"], read);
    assert_eq!(response["results"][0]["diagnostics"], read["diagnostics"]);
    assert_eq!(
        response["results"][1]["snapshot"]["objects"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn capabilities_describe_recovery_without_claiming_atomic_multi_file_visibility() {
    let yaml = capabilities("yaml");
    let live = capabilities("live");
    assert_eq!(yaml["atomicity"], "rollback_on_failure");
    assert_eq!(yaml["multi_file_atomic_visibility"], false);
    assert_eq!(yaml["crash_recovery"], true);
    assert_eq!(live["crash_recovery"], false);
    assert_eq!(yaml["source_schema"]["registry_package_cache"], false);
}
