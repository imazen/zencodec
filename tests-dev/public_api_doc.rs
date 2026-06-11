//! Public-API surface snapshots (docs/public-api/) — shared implementation
//! in `zenutils-apidoc`; see that crate for the three-file format and the
//! `ZEN_API_DOC=off|check|regen` protocol. Lives in tests-dev/ (outside the
//! package include whitelist) so the published crate ships neither the test
//! nor its nightly-rustdoc requirement.
#[test]
fn public_api_surface_docs_are_current() {
    zenutils_apidoc::run();
}
