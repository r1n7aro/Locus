// Desktop entry for development; on Android the library is loaded directly.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    locus_remote_lib::run()
}
