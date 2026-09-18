These DER files are a public, test-only localhost certificate and private key.
They are used exclusively by the in-memory TLS tests in `tls_tests.rs` and must
never be used by a running Locus client or server. The self-signed certificate
uses ECDSA P-256, has DNS SAN `localhost`, and is valid from 2020 through 2100.
