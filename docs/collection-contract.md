# Shared image collections

Implemented on [API review PR 168](https://github.com/imazen/zencodec/pull/168),
not released. The generic mechanism lives in
[`zencodec::collection`](../src/collection.rs); the consumer wrapper lives in
[`operation::CheckedCollection`](../src/operation/collection.rs).
The JXL adapter is packaged behind `zenjxl`’s optional `collections` feature in
[PR31](https://github.com/imazen/zenjxl/pull/31). Native TIFF remains in the
[validation harness](../perf/spatial-validation/README.md). Both register through
`CodecSet::with_collection`; neither collection API is released.

## Start with the selected output

Once the adapter has opened the source and the application has configured its
resolver, the consumer requests the first image:

```rust
let image = images.first_image(&ImageRequest::new())?;
```

The complete, executing [registered example](../perf/spatial-validation/src/collection.rs)
uses the real `zenjxl::collection::JxlCollectionConfig`. Register the factory once:

```rust
let codecs = CodecSet::new()
    .with_collection(JxlCollectionConfig::new(64, 256));
```

The two native allowances bound retained image identities and processed frame
headers. Hidden frames count toward the second allowance. They do not bound
native workspace bytes or total replay cost. Opening chooses one composition
view and passes the shared stop token to discovery, replay and publication.
Changing the view under retained IDs would make their meaning ambiguous.

The checked result owns its pixels after the collection drops. Policy runs
before that result returns; consumers need no separate authorization call.
The wrapper uses the existing fallible `PolicyResolver` and `PolicySet`.
It does not introduce a parallel callback protocol.

## Register once; open a checked operation

The same [executing example](../perf/spatial-validation/src/collection.rs) now
builds a `CodecSet` with the packaged JXL factory and a private TIFF candidate. The caller uses:

```rust
let request = OpenRequest::new(Composition::Composited, 64);
let mut images = codecs.open_collection(format, source, request, resolver, stop)?;
let image = images.first_image(&ImageRequest::new())?;
```

Import `CodecSet`, `ImageFormat` and `StopToken` from `zencodec`; import
`OpenRequest`, `Composition` and `ImageRequest` from `zencodec::collection`.
`source` is a borrowed `&mut dyn zencodec::source::DynReadAt`; `ReadAtShim` erases
an existing typed source once. The returned operation can outlive the registry,
and a returned image can outlive both the operation and source. Immutable
factory templates are shared across cloned sets. Opened jobs remain local.

Opening selects the first matching factory and never retries another after a
failure. Its view is checked before discovery. `CodecSet` supplies one discovery
call allowance, one publication resolver and one latched stop combining the
set's pulse cancellation, set stop and per-call stop. Dropping the registry does
not disable any of these cancellation sources. This does not add native progress
reporting or an implicit format probe.

`OpenRequest::with_output_storage(pool)` requires a shared output payload
allowance for this opening. It makes no capability query. An adapter must admit
all returned color and auxiliary payloads before native writes, or return
`CollectionOpenError::UnsupportedOutputStorage`. Retained images keep the pool
charge after the operation ends. Native workspace and metadata need separate
bounds; selecting this pool does not satisfy `max_memory_bytes`.

Factories receive these resolved requirements through `CollectionOpenContext`.
The temporary `open_with` bridge rejects a pool before entering an older adapter;
native request-level integration is being qualified before the compatibility
method is removed. Codec-configured JXL storage remains the qualified positive
path in the [current matrix](../benchmarks/packaged_output_storage_2026-10-26.json).

Configured `ResourceLimits` and `decode::DecodePolicy` are requirements on this
path. An adapter must enforce them or return a typed refusal before affected
work. The private TIFF factory enforces `max_width`, `max_height` and
`max_pixels` against the actual selected stored dimensions before allocating
selected output or reading compressed pixels. Header-only inspection remains
available for over-limit images, and a smaller child can still decode. A tiny
ROI does not redefine the stored-image limit. The pinned native path executes
sequentially and accepts `ThreadingPolicy::Sequential`.

The packaged JXL factory checks `max_input_bytes` against the actual immutable
resident span before native parsing. Missing or misleading length hints do not
change the decision. This bounds accepted parser input; the caller already owns
that storage, and native workspace is not admitted by this check.

[Native resource-scope controls](../perf/spatial-validation/README.md#native-jxl-limit-scope)
show why the remaining mappings need more than forwarding numbers. A small
local JXL layer fits a selected-image pixel cap while its larger canvas exceeds
the native cap. Independently budgeted native decoders can also leave retained
output capacity above any one decoder's budget. Keep selected geometry, native
work and shared retained allocations explicit; publication policy happens too
late to admit allocations that already occurred.

Both factories accept the no-op native-default configurations. The explicitly
encode-only `max_output_bytes` field does not constrain a decode. Other
nontrivial resource or decode-policy requirements currently refuse before source
I/O; this includes geometry requirements on the JXL factory. Mixed supported and
unsupported fields refuse as a whole. Codec-specific templates still apply.
Limit violations retain `LimitExceeded` with the actual value and requested cap;
unimplemented enforcement retains `CollectionOpenError`. Its `UnsupportedLimits`
and `UnsupportedDecodePolicy` variants carry an `unsupported` field containing
only the unhandled requirements (other fields use no-op/default values). For a
TIFF width-plus-memory request, the error identifies memory; JXL also identifies
the unqualified width requirement. Inspect it with
`zencodec::find_cause::<CollectionOpenError>(&error)` instead of parsing text or
knowing the native adapter's error type. The data is bounded and contains no
source/metadata bytes. It is not permission to retry with weaker requirements.
Neither stripping metadata after reconstruction nor reporting this data could
establish a parsing/PII policy.
The legacy still-image `with_decode_policy` setter remains best-effort.
The [typed-refusal receipt](../benchmarks/collection_refusals_2026-10-26.json)
records the per-field, mixed-configuration and original-trace controls.

[`ErasedCollection`](../src/collection/erased.rs) retains native identities and
output ownership, checks cursor origin before native work and avoids boxing each
returned ID. It allocates at opening and per cursor; those small allocations are
still infallible. It does not promise a transferable job or native workspace
admission. TIFF adapter packaging, automatic detection, full native limit
mappings and progress integration remain open.

The [packaged JXL receipt](../benchmarks/packaged_jxl_2026-10-26.json) records
registry controls, 147 presentation selections and nine raw layers through the
codec package, with the independent-reader and native sample gates unchanged.

The [registration receipt](../benchmarks/collection_registry_2026-10-26.json) records native, HTTP, workspace, WASM, documentation and browser validation, including failed initial attempts.

The [native dimension receipt](../benchmarks/collection_dimensions_2026-10-26.json) records the three qualified decode caps, payload-read guard, per-field refusals and conditional HTTP control.

## Browse, retain an identity, request a region

An explorer needs to revisit images and their children:

```rust
let mut roots = images.images()?;
let Some(parent) = images.next(&mut roots)? else { return Ok(None) };
let mut children = images.children(&parent)?;
let Some(child) = images.next(&mut children)? else { return Ok(None) };
let output = images.decode(&child, &ImageRequest::new().with_region(rect))?;
```

`ImageId` is scoped to the opened collection. Cloning one preserves identity;
it does not serialize a source offset, retain a network revision or promise
cheap seeking. An ID from another operation fails. Native adapters also reject
foreign cursors. `Origin` provides the shared identity implementation.

A SubIFD can be shared by two parents and need not be a reduced image. A layer
can need earlier reference reconstruction. Containment, levels, placement and
video time therefore remain separate. `children` returns an unsupported error
when the adapter cannot establish the relation; it cannot report empty success.

Region requests are exact, nonempty stored coordinates. Out-of-bounds and
overflowing requests refuse without clipping. TIFF qualifies native regions;
the current JXL adapters explicitly refuse them. Returned geometry belongs to
the selected image, so differently sized pages use the same call.

## Inspect geometry before allocating samples

A viewer can lay out a page or choose a tile before decoding it:

```rust
let description = images.describe(&child)?;
```

`Collection::describe` and `CheckedCollection::describe` return the same
`ImageDescription`: nonempty local stored dimensions, optional established
orientation and optional separate placement. `None` orientation does not mean
identity. Raw layers describe their full local size; their presentation canvas
belongs to placement. Describing an issued ID must not reconstruct pixels or
advance a discovery cursor. Unsupported inspection refuses explicitly.

The [executing example](../perf/spatial-validation/src/collection.rs) keeps the
opened operation with the caller, so its returned identity remains useful for
later decoding. The checked wrapper validates geometry and placement, preserves
typed causes under `CollectionStage::Describe`, and checks cancellation through
the final return. There is no separate inspection capability negotiation or
second resolver protocol. Pixel publication policy still runs on each decode.
The [shared-description receipt](../benchmarks/public_description_2026-10-26.json)
records the native, checked-consumer, workspace and guide validation.

These facts establish no output pixel format, supported compression, complete
metadata inventory or workspace bound. TIFF can describe a directory whose
compression cannot be decoded. Discovery can traverse preceding compressed
frames, and transport buffering can read ahead; this is not a promise of zero
payload I/O. A successful description cannot authorize decoding a replaced
HTTP object. Native qualification and the initial experiment are recorded in
the [description receipt](../benchmarks/selected_description_2026-10-26.json).

## Place a layer without losing its local samples

`image.interpretation().placement` now exposes an established presentation
placement to generic consumers. A `None` field means the adapter establishes no
separate placement; it must not be inferred from a page number or timestamp.
Given a placement, `placement.visible(interpretation.geometry)` returns matching
sample-buffer and canvas rectangles. Its `Ok(None)` means a valid image lies
wholly outside the canvas. Those samples remain in the owner.

The owner validates the placement before policy publication. Invalid canvases
and signed overflow return a traced `OutputError::Placement`; valid negative
origins and partial intersections keep the complete local buffer. The policy
question and bounded report include the same placement. Native blend/reference
instructions stay in their source extension and do not become generic rendering
instructions merely because the placement is known.
The [placement receipt](../benchmarks/public_placement_2026-10-26.json) preserves
the native comparisons, synthetic signed-coordinate controls, invalid-owner
refusals, HTTP/AV1 regression checks and the borrowed-question lint fix.

## Bound repeated discovery

An interactive caller can repeatedly create cursors, revisit cached results or
retry an unsupported query. `Discovery::new(collection, attempts)` wraps the
adapter with one allowance for the whole opened operation. Put it inside the
`CheckedCollection` factory when that bound is required.

Each `images`, `children`, `next` or `describe` call charges before delegation, including
errors and successful exhaustion. Reopening a cursor cannot reset the count.
`DiscoveryExhausted` retains the method, allowance and usage as a typed cause.
Successful descriptions have a separate counter; repeating a cached description
still charges. Existing IDs remain decodable after discovery exhaustion.

This bounds delegated calls. It excludes opening, native parse work inside a
call, selected-image reconstruction, source bytes and memory. Native JXL header
limits and TIFF graph limits remain distinct. A hard workspace requirement
without qualified evidence still refuses through policy.

## Failure and publication boundaries

| Event | Scope and retained evidence |
|---|---|
| Bad region, foreign identity, unsupported request | This request fails; a corrected request can reuse the operation |
| Resolver refusal or traced application error | Held output is dropped; later requests run policy again |
| Native failure | Original concrete cause/trace retained; the adapter enforces any terminal native state |
| Observed cancellation/timeout | Exact first reason latched; collection dropped; later calls cannot enter native code |
| Caught unwind during a request | Collection dropped; already published owners stay usable |
| Successful decode | Exact held owner accepted by the resolver, then checked for stop before publication |

`CollectionOperationError` records the stage and bounded prior decisions.
Default formatting excludes arbitrary backend/callback payloads. Explicit
`Error::source` or `zencodec::find_cause` inspection retrieves them, including
an existing `whereat` trace. The portable latch stores `StopReason` intact;
it does not encode today's variants into a closed integer table. No application
callback executes while the once cell is being initialized.

Use `error.error().stop_reason()` to inspect the first reason observed by the
operation's shared token. This works across native, policy and wrapper failures,
including opening and later calls on a cancelled operation. If a native or
application failure coincides with cancellation, its original cause and trace
remain available; the accessor also reports the observed stop. `None` means the
shared token did not report a stop. It does not infer cancellation from arbitrary
backend error text or replace source-specific error classification.
The [stop-observation receipt](../benchmarks/collection_stop_2026-10-26.json)
records native seam injection and coincident-failure checks.

Publication follows reconstruction. It cannot retroactively admit native
allocation, certify all opaque/nested carriers for a PII policy or impose a
hard latency bound inside a native call. Conditional HTTP and disk snapshot
validation remain transport responsibilities. Tests inject failure at the last
required range and require refusal before policy/publication.

## Qualification and remaining work

The [native harness](../perf/spatial-validation/README.md) exercises classic and
BigTIFF, nested/shared SubIFDs, selected regions, JXL presentations and complete
local F32 layers, preserved native ownership and repeated requests. Independent
oracles check pixels and interpretation. The IMI-derived HTTP simulator checks
actual conditional requests, source changes and short final bodies. The guide's
exact Rust functions execute against these adapters.

The [promotion receipt](../benchmarks/public_collection_2026-10-26.json) records
the full native/disk/HTTP run, workspace and AV1 checks, API snapshots and browser
qualification, including failures found and corrected during promotion.

Concrete adapter packaging and registry convenience remain necessary for a
normal application to open collections without this harness. Broader header facts,
all native layer configurations, reduced-level selection, broader snapshots,
full workspace admission and complete carrier privacy remain separate work.

Core source uses `alloc` and the cancellation latch has no `std` dependency.
The full crate's freestanding build still fails on the existing
[dependency/gain-map math issue 165](https://github.com/imazen/zencodec/issues/165).
`just collection-api-bare-metal` preserves that reproducer. Passing WebAssembly
checks does not establish a std-free dependency graph. Timing measurements
rejected by Zenbench are not performance evidence for these contracts.
