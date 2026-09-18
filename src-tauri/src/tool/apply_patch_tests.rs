use super::*;

fn updated(original: &str, body: &str) -> Result<String, String> {
    let patch = parse(&format!(
        "*** Begin Patch\n*** Update File: test.txt\n{body}\n*** End Patch"
    ))?;
    let Change::Update { chunks, .. } = &patch[0].change else {
        unreachable!()
    };
    update_content(original, chunks)
}

#[test]
fn multiple_files_and_move_paths_are_parsed() {
    let files = parse("*** Begin Patch\n*** Add File: a.txt\n+new\n*** Delete File: old.txt\n*** Update File: source.txt\n*** Move to: dest.txt\n@@\n-old\n+new\n*** End Patch").unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].change, Change::Add("new\n".into()));
    assert_eq!(
        files[2].targets(),
        vec![("source.txt", "edit"), ("dest.txt", "write")]
    );
}

#[test]
fn chunks_match_original_in_source_order_and_keep_context() {
    assert_eq!(
        updated(
            "class A\n  before\n  keep\nend\n",
            "@@ class A\n-  before\n+  after\n+  extra\n   keep\n@@\n-end\n+done"
        )
        .unwrap(),
        "class A\n  after\n  extra\n  keep\ndone\n"
    );
    assert!(updated("before\n", "@@\n-before\n+after\n@@\n-after\n+done").is_err());
    assert!(updated("one\ntwo\n", "@@\n-two\n+TWO\n@@\n-one\n+ONE").is_err());
}

#[test]
fn eof_and_fuzzy_unicode_matching_follow_patch_rules() {
    assert_eq!(
        updated("same\nother\nsame\n", "@@\n-same\n+last\n*** End of File").unwrap(),
        "same\nother\nlast\n"
    );
    assert_eq!(
        updated(
            "  // “quote”—dash\n  keep\n",
            "@@\n-// \"quote\"-dash\n+fixed\n keep"
        )
        .unwrap(),
        "fixed\n  keep\n"
    );
}

#[test]
fn append_empty_and_delete_content() {
    assert_eq!(updated("", "@@\n+first").unwrap(), "first\n");
    assert_eq!(updated("first", "@@\n+last").unwrap(), "first\nlast\n");
    assert_eq!(updated("first\n", "@@\n-first").unwrap(), "");
    assert_eq!(updated("first\n", "@@\n-first\n+new\n ").unwrap(), "new\n");
    assert_eq!(
        updated("first\n", "@@\n-first\n \n+new").unwrap(),
        "\nnew\n"
    );
}

#[test]
fn malformed_patches_fail_without_panicking() {
    for body in [
        "中文",
        "@@",
        "@@\n中文",
        "*** Move to: ",
        "@@\n-ok\n*** End of File\n@@\n+bad",
    ] {
        assert!(
            parse(&format!(
                "*** Begin Patch\n*** Update File: x\n{body}\n*** End Patch"
            ))
            .is_err(),
            "{body}"
        );
    }
    assert!(parse("*** Begin Patch\n*** End Patch").is_err());
    assert!(parse("*** Begin Patch\n*** Delete File: x").is_err());
}
