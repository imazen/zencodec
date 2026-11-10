//! Versioned text snapshots use the shared zenutils format; browser JSON is build-only.
#[test]
fn public_api_surface_docs_are_current() {
    zenutils_apidoc::ApiDoc::new()
        .workspace_dir("..")
        .crates([
            "zencodec",
            "zencodec-io",
            "zencodec-coroutine",
            "zencodec-media",
            "zencodec-testkit",
            "zencodec-media-testkit",
            "zenmeta",
        ])
        .run();
    zenutils_apidoc::ApiDoc::new()
        .workspace_dir("../zencodec-render")
        .crates(["zencodec-render"])
        .out_dir("../docs/public-api")
        .run();
}
