# Code-first API guide

The site entry point is `code.html`, built from
[`site/code-first-data.mjs`](site/code-first-data.mjs). Consumer, adapter and
codec implementor tabs show source-extracted calls and implementations. Imports
and surrounding code expand independently above and below each example.

Annotated examples copy their complete example scope. Implementation and native
test excerpts copy the complete source file, including its helpers; they are not
advertised as standalone application programs. Their source links preserve the
exact revision. Native test dependency manifests are alongside those files.

The catalog includes the all-feature Linux surface of zencodec, zencodec-io,
zencodec-coroutine, zencodec-media, both testkits, zenmeta and zencodec-render.
It contains public declarations, trait methods, associated types, fields and
variants. Compiler-generated blanket, auto and derived implementations are
omitted. Canonical compiler paths are reference notation, not always the imports
callers should write. Use the runnable examples for caller imports.

Every indexed declaration has its signature and defining context. Available
source documentation, Rust examples and deprecation notes stay attached.
Declarations without local documentation say so and link to the crate source.
Related-flow links identify examples using contracts from the same source file;
they do not imply that every method in that file has a dedicated walkthrough.
Private experiment crates are labelled in flows and excluded from the exported
API catalog. Dependency-only APIs are not counted as zencodec's surface; pinned
transcode and conversion snapshots appear beside the examples that use them.

## Keeping it current

Run `just code-catalog EXPORT` through the workspace's build resource wrapper,
where `EXPORT` is a prepared source export with dependencies resolved outside
workspace path overrides. It requires installed `cargo-public-api` and its
nightly rustdoc toolchain. The generator verifies each source input against this
checkout before extracting declarations and documentation.

Commit the updated `docs/code-api/` documentation with an API change. The site
build verifies source/manifest fingerprints and both generated-file checksums;
it refuses stale catalogs. Builds do not contact a documentation service.

Run `just docs-site-browser-check 3547`. This checks all source scopes,
signature preservation, keyboard tabs, deep links, independent context expansion,
full-example copy, filtering, pagination and responsive layout. The existing
documentation workflow runs the examples and deploys the same `docs/` site on
main; the review branch preview is served separately.

## Graduated examples

Each role begins with a concrete application story. Each flow opens only its first
step; later steps explain the new need before revealing its source code. Imports
and continuation still expand independently inside every example. Code remains
extracted from repository sources, and every step links its actual signatures.
The consumer story starts with one registry and one decode/transcode call;
adapter and codec stories introduce ownership and state machines separately.

## Boundaries visible to readers

The page describes the review revision, not an already released 0.3 API. A
signature does not certify codec support, a native allocation bound, or privacy.
Mandatory requirements now filter routes before the callback runs, including direct
`Decision::Choose`. Unknown required evidence refuses. This does not yet provide
operation-wide deferred-obligation handling or a whole-file privacy certificate;
remaining work is tracked in [issue 173](https://github.com/imazen/zencodec/issues/173).
Native TIFF/JXL experiments, protocol fixtures and private multipass/policy
experiments retain explicit labels.
