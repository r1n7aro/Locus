fn main() {
    println!("cargo:rerun-if-env-changed=LOCUS_BUILD_EDITION");
    println!("cargo:rerun-if-env-changed=LOCUS_VERIFY_FRONTEND");
    let internal = std::env::var_os("CARGO_FEATURE_SERVER_CLIENT").is_some();
    let edition = std::env::var("LOCUS_BUILD_EDITION").unwrap_or_else(|_| "public".into());
    assert!(matches!(edition.as_str(), "public" | "internal"), "Unknown client edition");
    assert_eq!(internal, edition == "internal", "Frontend/backend edition mismatch; use bun run internal:dev or internal:build");
    if std::env::var("LOCUS_VERIFY_FRONTEND").as_deref() == Ok("1") {
        let directory = if internal { "../dist-internal" } else { "../dist" };
        let marker = std::path::Path::new(directory).join(".locus-edition.json");
        println!("cargo:rerun-if-changed={}", marker.display());
        let contents = std::fs::read_to_string(marker).expect("Build the matching frontend first");
        let expected = format!("\"edition\":\"{edition}\"");
        assert!(contents.contains(&expected), "Frontend artifact edition mismatch");
    }
    tauri_build::build();

    #[cfg(target_os = "windows")]
    {
        // tauri-build links this resource only into binary targets. The lib
        // unit-test harness still imports comctl32!TaskDialogIndirect through
        // the desktop dependency graph, so Windows must activate Common
        // Controls v6 before resolving imports. Without the embedded manifest
        // the harness exits in the loader with STATUS_ENTRYPOINT_NOT_FOUND.
        //
        // A generic rustc link arg also reaches the lib unit-test harness.
        // Passing the same resource to the app binary twice is harmless: the
        // linker consumes the identical resource object only once. Reusing
        // tauri-build's resource keeps app and test activation metadata equal
        // and works with both rust-lld and MSVC link.exe.
        let resource_lib = std::path::PathBuf::from(
            std::env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR for build scripts"),
        )
        .join("resource.lib");
        println!("cargo:rustc-link-arg={}", resource_lib.display());
    }
}
