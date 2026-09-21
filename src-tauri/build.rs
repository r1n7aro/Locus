fn main() {
    tauri_build::build();

    // Build scripts run on the host. Only link the Windows resource when the
    // application target is Windows (including cross-compilation from Windows).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
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
