fn main() {
    tauri_build::build();

    if target_is_windows_msvc() {
        embed_manifest_for_windows_link_targets();
    }
}

fn target_is_windows_msvc() -> bool {
    matches!(
        (
            std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref(),
            std::env::var("CARGO_CFG_TARGET_ENV").ok().as_deref(),
        ),
        (Some("windows"), Some("msvc"))
    )
}

fn embed_manifest_for_windows_link_targets() {
    let manifest = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"),
    )
    .join("comctl32-v6.manifest");

    assert!(
        manifest.is_file(),
        "missing comctl32 v6 manifest at {}",
        manifest.display()
    );

    println!("cargo:rerun-if-changed={}", manifest.display());

    let manifest = manifest
        .to_str()
        .expect("comctl32-v6.manifest path is not valid UTF-8");

    // `cargo test --lib` builds a unit-test harness executable for the library
    // target. `rustc-link-arg-tests` does not reach that harness, but the
    // generic linker args do.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{manifest}");

    // The main Tauri binary already links `resource.lib`, which contains its
    // own manifest resource. Disable the linker's extra manifest generation for
    // that binary so we only inject the additional manifest into test harnesses.
    println!("cargo:rustc-link-arg-bin=locus=/MANIFEST:NO");
}
