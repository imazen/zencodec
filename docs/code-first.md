# Code-first API guide

The code-first catalog page is `code.html`, built from
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

Versioned API review uses the existing `zenutils-apidoc` runner for all eight
crates. Run `just api-doc` and commit only `docs/public-api/*.txt`; default,
additional-feature and internal surfaces remain separate. `just api-doc-check`
and the documentation CI job explicitly check those snapshots for drift.

`npm run build --prefix docs` generates the richer signature-browser catalog in
ignored `docs/.cache/code-api/`. It requires `cargo-public-api 0.52.0` and
`nightly-2026-10-06`. Its text signatures, documentation, source spans and
manifest are build data, never committed. The public site still publishes its
search JSON under `_site/code-api/` with the HTML artifact.

A cached catalog is reused only when crate source, Cargo manifests, crate
READMEs, resolved lockfiles, generator code, compiler/toolchain and generation
platform match. Every payload hash is checked too. Changed or corrupt data is
preserved beside the cache and regenerated before publishing; the renderer
retains its independent source/hash checks. CI caches only the generated catalog.
`just code-catalog EXPORT` remains an explicit prebuild option for a prepared
source export; normal site builds generate the catalog automatically.

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

## Reader-driven context

The compiler-feedback reader study records first attempts separately from repairs
and runtime checks. Imports now appear beside the focused code, before expanding
fixture setup. Signature cards identify selected methods and enumerate required
trait methods, so an excerpt cannot silently serve as an incomplete implementor
checklist. These annotations are extracted from source, including default versus
required methods; they are not handwritten copies of signatures.

Codec-implementor cards also include every required method signature, even when
the editorial selection focuses on one operation. Consumer cards retain the
smaller selection. Both identify omissions and defaults; a required-method list
is not a claim that optional/default behavior meets an application's needs.

Source signature extraction preserves array lengths and const-generic arguments;
semicolons inside `[T; N]` are not declaration terminators. Pinned dependency
recaptures retain the revision, full-source checksums and hyperlinks.

## Field and method contracts

The compiler may report a field and an inherent method at the same qualified
path (`DecodePolicy::strict`, for example). Catalog details retain rustdoc item
kinds, and rendering selects the matching kind instead of the last item emitted
by rustdoc. Field declaration colons are excluded from path lookup. Ambiguous
aliases do not acquire another declaration's documentation.
