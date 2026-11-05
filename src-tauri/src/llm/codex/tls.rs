//! Desktop tests exercise the shared TLS implementation.
#[cfg(test)]
use locus_remote_core::codex_transport::tls::{config_with_roots, server_name};
#[cfg(test)]
#[path = "tls_tests.rs"]
mod tests;
