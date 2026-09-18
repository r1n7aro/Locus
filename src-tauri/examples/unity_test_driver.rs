//! Same application/CLI entry point with a distinct executable, so integration
//! tests can build and run while a shared Windows development app holds locus.exe.
fn main() {
    locus_lib::run()
}
