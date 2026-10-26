# Shared image collections

Implemented on [API review PR 168](https://github.com/imazen/zencodec/pull/168),
not released. The generic mechanism lives in
[`zencodec::collection`](../src/collection.rs); the consumer wrapper lives in
[`operation::CheckedCollection`](../src/operation/collection.rs).
Native TIFF, JXL presentation and JXL raw-layer adapters still live in the
[validation harness](../perf/spatial-validation/README.md). They import these
shared contracts. There is no `CodecSet` collection registration yet.

## Start with the selected output

Once the adapter has opened the source and the application has configured its
resolver, the consumer requests the first image:

```rust
let image = images.first_image(&ImageRequest::new())?;
```

The complete, executing [JXL example](../perf/spatial-validation/src/collection.rs)
imports `ImageRequest` from `zencodec::collection` and `CheckedCollection`,
`CollectionOperationError`, `Outcome` and `PolicyResolver` from
`zencodec::operation`. Native setup is explicit:

```rust
let mut images = CheckedCollection::open(stop, resolver, |stop| {
    JxlItems::open_with_stop(bytes, Composition::Composited, limits, stop)
})?;
```

`JxlItems` and `limits` are private adapter types in this example, not promised
published conveniences. Opening fixes the view and passes the same stop token
to native discovery, replay and publication. A raw-layer factory fixes the
uncomposited view before any IDs exist. Changing the view under retained IDs
would make their meaning ambiguous.

The checked result owns its pixels after the collection drops. Policy runs
before that result returns; consumers need no separate authorization call.
The wrapper uses the existing fallible `PolicyResolver` and `PolicySet`.
It does not introduce a parallel callback protocol.

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
