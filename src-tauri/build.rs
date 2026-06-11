fn main() {
    tauri_build::build();

    // The main binary's Windows manifest (common-controls v6 and friends)
    // comes from the `resource.lib` that tauri-build compiles; no manifest
    // linker args are needed here. This build script used to inject
    // `/MANIFEST:EMBED` + `/MANIFESTINPUT:comctl32-v6.manifest` for every link
    // target and cancel it with `/MANIFEST:NO` for the main binary so that
    // unit-test harnesses got a comctl32 v6 manifest. That arg dance only
    // works on MSVC link.exe - rust-lld rejects a dangling /MANIFESTINPUT
    // ("/manifestinput: requires /manifest:embed") - and nothing in the unit
    // tests creates common controls, so the harness manifest was dead weight.
    // If a future test really needs comctl32 v6, note that
    // `cargo:rustc-link-arg-tests` only reaches integration-test targets
    // (cargo rejects it for the lib unit-test harness); prefer activating an
    // activation context at runtime in that test instead.
}
