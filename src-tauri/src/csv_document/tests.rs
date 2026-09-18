use super::*;
use serde_json::json;
use std::fs;

fn fixture(content: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let csv = root.path().join("items.csv");
    fs::write(&csv, content).unwrap();
    (root, csv)
}

fn patch(value: serde_json::Value) -> CsvViewPatch {
    serde_json::from_value(value).unwrap()
}

#[test]
fn reading_defaults_is_read_only_and_ids_are_repeatable() {
    let (_root, csv) = fixture("id,name\n001,first\n");
    let a = read_view(&csv).unwrap();
    let b = read_view(&csv).unwrap();
    assert_eq!(a.view, b.view);
    assert_eq!(a.revision, b.revision);
    assert_eq!((a.row_count, a.column_count), (2, 2));
    assert!(!view_path(&csv).unwrap().exists());
    let same = patch_view(&csv, &a.revision, patch(json!({"row_height": 28}))).unwrap();
    assert_eq!(same.revision, a.revision);
    assert!(!view_path(&csv).unwrap().exists());
}

#[test]
fn layout_batch_preserves_csv_bytes_and_unity_metadata_and_retains_other_fields() {
    let original = "\u{feff}编号,名称,说明,\r\n001,\"动作,一\",\"第一行\r\n第二行\",\r\n\r\n";
    let (_root, csv) = fixture(original);
    let meta = csv.with_extension("csv.meta");
    fs::write(&meta, "guid: keep-me\n").unwrap();
    let before = read_view(&csv).unwrap();
    let after = patch_view(&csv, &before.revision, patch(json!({
        "row_height": 40, "wrap_text": true, "frozen_columns": 1,
        "columns": [{"target": {"header": "名称"}, "width": 240}, {"target": {"source_index": 3}, "hidden": true}],
        "sort": [{"target": {"header": "编号"}, "direction": "desc"}],
        "filters": [{"target": {"header": "名称"}, "value": "动作"}]
    }))).unwrap();
    assert_ne!(after.revision, before.revision);
    assert_eq!(fs::read(&csv).unwrap(), original.as_bytes());
    assert_eq!(fs::read_to_string(&meta).unwrap(), "guid: keep-me\n");
    assert_eq!(after.view.columns[&before.view.column_order[1]].width, 240);
    assert!(after.view.columns[&before.view.column_order[3]].hidden);
    let raw = read_view_file(&csv).unwrap().text.unwrap();
    assert!(!raw.starts_with("---"));
    assert_eq!(parse_view(&raw).unwrap(), after.view);
    let next = patch_view(
        &csv,
        &after.revision,
        patch(json!({"row_height": 42, "sort": []})),
    )
    .unwrap();
    assert!(next.view.sort.is_empty());
    assert_eq!(next.view.filters, after.view.filters);
    assert_eq!(next.view.columns, after.view.columns);
    assert!(next.view.wrap_text);
    assert_eq!(read_view(&csv).unwrap().revision, next.revision);
}

#[test]
fn stale_data_view_and_other_document_revisions_cannot_write() {
    let (root, csv) = fixture("id,name\n1,a\n");
    let first = read_view(&csv).unwrap();
    let other = root.path().join("other.csv");
    fs::copy(&csv, &other).unwrap();
    assert_eq!(
        patch_view(&other, &first.revision, patch(json!({"row_height": 40})))
            .unwrap_err()
            .code,
        "csv.revision_changed"
    );
    fs::write(&csv, "id,name\n2,b\n").unwrap();
    assert_eq!(
        patch_view(&csv, &first.revision, patch(json!({"row_height": 40})))
            .unwrap_err()
            .code,
        "csv.revision_changed"
    );
    assert!(!view_path(&csv).unwrap().exists());
    let current = read_view(&csv).unwrap();
    let saved = patch_view(&csv, &current.revision, patch(json!({"row_height": 42}))).unwrap();
    let bytes = fs::read(view_path(&csv).unwrap()).unwrap();
    assert!(patch_view(&csv, &current.revision, patch(json!({"wrap_text": true}))).is_err());
    assert_eq!(fs::read(view_path(&csv).unwrap()).unwrap(), bytes);
    fs::remove_file(view_path(&csv).unwrap()).unwrap();
    assert!(patch_view(&csv, &saved.revision, patch(json!({"wrap_text": true}))).is_err());
    assert!(!view_path(&csv).unwrap().exists());
}

#[test]
fn editor_and_sdk_share_version_checks_and_layout_format() {
    let (_root, csv) = fixture("id,name\n1,a");
    let first = read_view(&csv).unwrap();
    let csv_hash = blake3::hash(&fs::read(&csv).unwrap()).to_hex().to_string();
    let mut editor = first.view.clone();
    editor.row_height = 36;
    let saved = write_view_file(&csv, &editor.serialize().unwrap(), None, &csv_hash).unwrap();
    assert_eq!(read_view(&csv).unwrap().view, editor);
    assert!(patch_view(&csv, &first.revision, patch(json!({"wrap_text": true}))).is_err());
    let fresh = read_view(&csv).unwrap();
    patch_view(&csv, &fresh.revision, patch(json!({"wrap_text": true}))).unwrap();
    assert_eq!(
        write_view_file(
            &csv,
            &editor.serialize().unwrap(),
            saved.content_hash.as_deref(),
            &csv_hash
        )
        .unwrap_err()
        .code,
        "csv.view_changed"
    );
}

#[test]
fn invalid_batches_are_atomic_and_ambiguous_headers_require_explicit_identity() {
    let (_root, csv) = fixture("id,id,\n1,2,3");
    let first = read_view(&csv).unwrap();
    for value in [
        json!({"columns": [{"target": {"header": "id"}, "width": 220}]}),
        json!({"row_height": 40, "columns": [{"target": {"source_index": 0}, "width": 220}, {"target": {"source_index": 1}, "width": 0}]}),
        json!({"columns": [{"target": {"source_index": 0}, "width": 220}, {"target": {"source_index": 0}, "hidden": true}]}),
        json!({"column_order": [{"source_index": 0}]}),
        json!({"column_order": [{"source_index": 0}, {"source_index": 0}, {"source_index": 2}]}),
        json!({"frozen_columns": 4}),
        json!({"header_rows": 2}),
        json!({"sort": [{"target": {"source_index": 0}, "direction": "descending"}]}),
        json!({"columns": [{"target": {"source_index": 0, "header": "id"}, "width": 220}]}),
    ] {
        assert!(patch_view(&csv, &first.revision, patch(value)).is_err());
        assert!(!view_path(&csv).unwrap().exists());
    }
    let after = patch_view(
        &csv,
        &first.revision,
        patch(json!({"columns": [{"target": {"source_index": 1}, "width": 222}]})),
    )
    .unwrap();
    assert_eq!(after.view.columns[&first.view.column_order[1]].width, 222);
    assert_eq!(after.view.columns[&first.view.column_order[0]].width, 140);
    for value in [
        json!({"font_size": 16}),
        json!({"row_height": true}),
        json!({"columns": [{"target": {"source_index": -1}, "width": 220}]}),
    ] {
        assert!(serde_json::from_value::<CsvViewPatch>(value).is_err());
    }
}

#[test]
fn selectors_use_original_snapshot_when_header_mode_and_order_change_together() {
    let (_root, csv) = fixture("id,name\n1,a");
    let first = read_view(&csv).unwrap();
    let after = patch_view(
        &csv,
        &first.revision,
        patch(json!({"header_rows": 0,
        "columns": [{"target": {"header": "name"}, "width": 220}],
        "column_order": [{"header": "name"}, {"header": "id"}]})),
    )
    .unwrap();
    assert_eq!(
        after.view.column_order,
        first
            .view
            .column_order
            .iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(after.view.columns[&first.view.column_order[1]].width, 220);
    assert!(after.view.columns.values().all(|col| col.header.is_empty()));
    let restored = patch_view(&csv, &after.revision, patch(json!({"header_rows": 1}))).unwrap();
    assert_eq!(
        restored.view.columns[&first.view.column_order[1]].header,
        "name"
    );
    assert_eq!(read_view(&csv).unwrap().view, restored.view);
}

#[test]
fn external_header_reorder_preserves_column_ids_and_widths() {
    let (_root, csv) = fixture("id,name\n1,a");
    let first = read_view(&csv).unwrap();
    let id = first.view.column_order[0].clone();
    patch_view(
        &csv,
        &first.revision,
        patch(json!({"columns": [{"target": {"id": id}, "width": 222}]})),
    )
    .unwrap();
    fs::write(&csv, "name,id\na,1").unwrap();
    let rebound = read_view(&csv).unwrap();
    assert_eq!(rebound.view.columns[&id].source_index, 1);
    assert_eq!(rebound.view.columns[&id].width, 222);
}

#[test]
fn damaged_config_and_invalid_csv_remain_untouched() {
    let (_root, csv) = fixture("id,name\n1,a");
    let first = read_view(&csv).unwrap();
    for raw in [
        "<<<<<<< HEAD\n",
        "schema: locus.csv-view.v3\n",
        "---\ntitle: wrongly registered\n---\n",
    ] {
        fs::write(view_path(&csv).unwrap(), raw).unwrap();
        assert!(read_view(&csv).is_err());
        assert!(patch_view(&csv, &first.revision, patch(json!({"row_height": 40}))).is_err());
        assert_eq!(fs::read_to_string(view_path(&csv).unwrap()).unwrap(), raw);
    }
    fs::remove_file(view_path(&csv).unwrap()).unwrap();
    for raw in [
        "id,name\n1,\"unclosed",
        "id,name\n1,\"x\"bad",
        "id,name\n1,un\"quoted",
        "i\0d",
    ] {
        fs::write(&csv, raw).unwrap();
        assert!(read_view(&csv).is_err());
        assert_eq!(fs::read_to_string(&csv).unwrap(), raw);
    }
}

#[test]
fn schema_rejects_duplicate_keys_including_column_map() {
    let (_root, csv) = fixture("id,name\n1,a");
    let view = read_view(&csv).unwrap().view;
    let raw = view.serialize().unwrap();
    assert!(parse_view(&format!("{raw}rowHeight: 40\n")).is_err());
    let duplicate = raw.replace("  c_1:", "  c_0:");
    assert!(parse_view(&duplicate).is_err());
    assert!(parse_view(&raw.replace("sourceIndex: 1", "sourceIndex: 0")).is_err());
}

#[test]
fn companion_directories_and_non_csv_paths_are_rejected() {
    let (_root, csv) = fixture("id\n1");
    fs::create_dir(view_path(&csv).unwrap()).unwrap();
    assert_eq!(read_view(&csv).unwrap_err().code, "csv.invalid_view_path");
    assert!(read_view_file(&csv.with_extension("md")).is_err());
}

#[test]
fn shared_editor_contract_fixtures_match_shape_and_yaml() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src/__tests__/fixtures/csvViewContract.json"
    ))
    .unwrap();
    for case in fixtures["documents"].as_array().unwrap() {
        let (_root, csv) = fixture(case["source"].as_str().unwrap());
        let view = read_view(&csv).unwrap();
        assert_eq!(
            json!({"rows": view.row_count, "columns": view.column_count, "delimiter": view.delimiter}),
            case["shape"],
            "{}",
            case["name"]
        );
        let headers: Vec<_> = view
            .view
            .column_order
            .iter()
            .take(view.column_count)
            .map(|id| &view.view.columns[id].header)
            .collect();
        assert_eq!(json!(headers), case["headers"], "{}", case["name"]);
    }
    let raw = fixtures["viewYaml"].as_str().unwrap();
    assert_eq!(parse_view(raw).unwrap().serialize().unwrap(), raw);
}

#[test]
fn registered_read_only_sources_reject_style_writes() {
    let root = tempfile::tempdir().unwrap();
    let csv = root
        .path()
        .join("Library/Locus/KnowledgeSources/reference/items.csv");
    fs::create_dir_all(csv.parent().unwrap()).unwrap();
    fs::write(&csv, "id\n1").unwrap();
    assert!(read_view(&csv).is_ok());
    for ai in [true, false] {
        assert_eq!(
            ensure_write_allowed(root.path(), &csv, None, ai)
                .unwrap_err()
                .code,
            "csv.read_only"
        );
    }
    assert!(!view_path(&csv).unwrap().exists());
}

#[test]
fn policy_is_resolved_through_the_csv_without_registering_a_markdown_companion() {
    let root = tempfile::tempdir().unwrap();
    let csv = root.path().join("Locus/knowledge/memory/items.csv");
    fs::create_dir_all(csv.parent().unwrap()).unwrap();
    fs::write(&csv, "id\n1").unwrap();
    ensure_write_allowed(root.path(), &csv, None, true).unwrap();
    let view = read_view(&csv).unwrap();
    patch_view(&csv, &view.revision, patch(json!({"row_height": 40}))).unwrap();
    assert!(!csv.with_extension("csv.view.md").exists());
    assert!(!read_view_file(&csv)
        .unwrap()
        .text
        .unwrap()
        .starts_with("---"));
    assert_eq!(fs::read_to_string(&csv).unwrap(), "id\n1");
}

#[test]
fn formatting_upgrades_v1_once_and_stores_rows_ranges_and_conditions_sparsely() {
    let (_root, csv) = fixture("id,status,value\nA,pending,12\nB,done,3");
    let original_bytes = fs::read(&csv).unwrap();
    let first = read_view(&csv).unwrap();
    let formatted = patch_view(&csv, &first.revision, patch(json!({"styles": {"upsert": [
        {"id": "row", "rows": [1, 10000], "style": {"font": "sans", "size": 16, "bold": true, "color": "accent", "background": "subtle", "border": {"style": "solid", "width": 1, "edges": ["bottom"]}}},
        {"id": "pending", "when": {"target": {"header": "status"}, "op": "eq", "value": "pending"}, "style": {"background": "warning-soft"}},
        {"id": "cell", "rows": [1, 1], "columns": [{"header": "value"}], "style": {"bold": false, "color": "#123456"}}
    ]}}))).unwrap();
    assert_eq!(first.view.schema, "locus.csv-view.v1");
    assert_eq!(formatted.view.schema, "locus.csv-view.v2");
    assert_eq!(formatted.view.styles.len(), 3);
    assert_eq!(formatted.view.styles[0].rows, Some([1, 10000]));
    assert_eq!(formatted.view.styles[0].columns, None);
    assert_eq!(formatted.view.styles[2].columns.as_ref().unwrap().len(), 1);
    assert_eq!(fs::read(&csv).unwrap(), original_bytes);
    let stored = read_view_file(&csv).unwrap();
    let raw = stored.text.unwrap();
    assert!(raw.len() < 1500, "range size must not expand storage");
    assert_eq!(parse_view(&raw).unwrap(), formatted.view);
    let same = patch_view(&csv, &formatted.revision, patch(json!({"styles": {"upsert": [
        {"id": "cell", "rows": [1, 1], "columns": [{"header": "value"}], "style": {"bold": false, "color": "#123456"}}
    ]}}))).unwrap();
    assert_eq!(same.revision, formatted.revision);
    let removed = patch_view(
        &csv,
        &same.revision,
        patch(json!({"styles": {"remove": ["cell"], "order": ["pending", "row"]}})),
    )
    .unwrap();
    assert_eq!(
        removed
            .view
            .styles
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>(),
        ["pending", "row"]
    );
    let mut migrated = first.view.clone();
    migrated.migrate_to_v2();
    migrated.migrate_to_v2();
    assert_eq!(migrated.schema, "locus.csv-view.v2");
    assert_eq!(migrated.columns, first.view.columns);
    let latest = read_view_file(&csv).unwrap();
    let csv_hash = blake3::hash(&original_bytes).to_hex().to_string();
    assert_eq!(
        write_view_file(
            &csv,
            &first.view.serialize().unwrap(),
            latest.content_hash.as_deref(),
            &csv_hash
        )
        .unwrap_err()
        .code,
        "csv.schema_downgrade"
    );
}

#[test]
fn invalid_style_batch_never_partially_saves_or_migrates() {
    let (_root, csv) = fixture("id,status\nA,pending");
    let first = read_view(&csv).unwrap();
    for rule in [
        json!({"id": "x", "rows": [5, 2], "style": {"bold": true}}),
        json!({"id": "x", "style": {"font": "serif; display:none"}}),
        json!({"id": "x", "style": {"color": "url(x)"}}),
        json!({"id": "x", "style": {"size": 99}}),
        json!({"id": "x", "style": {"border": {"width": 6}}}),
        json!({"id": "x", "when": {"target": {"header": "status"}, "op": "empty", "value": "x"}, "style": {"bold": true}}),
        json!({"id": "x", "when": {"target": {"header": "missing"}, "op": "empty"}, "style": {"bold": true}}),
    ] {
        let batch = patch(json!({"row_height": 40, "styles": {"upsert": [rule]}}));
        assert!(patch_view(&csv, &first.revision, batch).is_err());
        assert!(!view_path(&csv).unwrap().exists());
        assert_eq!(read_view(&csv).unwrap().view.schema, "locus.csv-view.v1");
    }
}

#[test]
fn workbook_load_and_save_preserve_text_and_migrate_rich_styles_idempotently() {
    let content = "\u{feff}id,name,value\r\n001,\"a\r\nb\",0.125\r\n";
    let (_root, csv) = fixture(content);
    let loaded = read_workbook(&csv).unwrap();
    assert_eq!(loaded.content.as_deref(), Some(content));
    let change = json!({"row_dimensions": {"0": {"height": 32.5, "hidden": false}, "2": {"height": 21, "hidden": true}, "10": {"height": 24, "hidden": false}},
        "styles": {"upsert": [{"id": "font", "rows": [0, 0], "style": {"excel": {
            "font": {"name": "Arial", "size": 12.5, "bold": true, "italic": true, "strike": false, "underline": "double", "color": "#123456", "vertAlign": "baseline"},
            "fill": {"patternType": "solid", "fgColor": "#FFEEDD", "bgColor": "transparent"},
            "border": {"left": {"style": "thin", "color": "#000000"}, "bottom": {"style": "double", "color": "#FF0000"}},
            "alignment": {"horizontal": "center", "vertical": "center", "wrapText": true, "shrinkToFit": false, "textRotation": 30, "indent": 1, "readingOrder": 0},
            "numberFormat": "#,##0.00"
        }}}]}});
    let styled = save_workbook(&csv, &loaded.revision, patch(change.clone()), None).unwrap();
    assert_eq!(styled.view.schema, "locus.csv-view.v4");
    assert_eq!(fs::read_to_string(&csv).unwrap(), content);
    let yaml = read_view_file(&csv).unwrap().text.unwrap();
    assert_eq!(parse_view(&yaml).unwrap(), styled.view);
    assert!(yaml.find("\"2\":").unwrap() < yaml.find("\"10\":").unwrap());
    assert_eq!(save_workbook(&csv, &styled.revision, patch(change), None).unwrap().revision, styled.revision);
    let revised = content.replace("0.125", "0.5");
    let saved = save_workbook(&csv, &styled.revision, patch(json!({})), Some(revised.clone())).unwrap();
    assert_eq!(fs::read_to_string(&csv).unwrap(), revised);
    assert_eq!(read_view_file(&csv).unwrap().text.unwrap(), yaml);
    assert_eq!(read_workbook(&csv).unwrap().revision, saved.revision);
    assert!(save_workbook(&csv, &styled.revision, patch(json!({})), Some(content.into())).is_err());
    let mut lower = saved.view.clone();
    lower.migrate_to_v2(); lower.migrate_to_v3();
    assert_eq!(lower.schema, "locus.csv-view.v4");
}

#[test]
fn invalid_workbook_data_or_styles_write_neither_file() {
    let content = "id,name\n001,one\n";
    let (_root, csv) = fixture(content);
    let loaded = read_workbook(&csv).unwrap();
    for (value, data) in [
        (json!({"row_height": 44}), "\"invalid"),
        (json!({"styles": {"upsert": [{"id": "bad", "style": {"excel": {"numberFormat": "bad\nformat"}}}]}}), "id,name\n002,new\n"),
        (json!({"row_dimensions": {"-1": {"height": 30, "hidden": false}}}), content),
    ] {
        assert!(save_workbook(&csv, &loaded.revision, patch(value), Some(data.into())).is_err());
        assert_eq!(fs::read_to_string(&csv).unwrap(), content);
        assert!(!view_path(&csv).unwrap().exists());
    }
}
