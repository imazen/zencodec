# Retain the raster, not a replay token

The [detached-proof reproduction](multipass-proof-review-2026-10-27.md) showed
that a private constructor cannot prove what an arbitrary source callback reads.
The replacement candidate retains the actual immutable view:

```rust,ignore
use multipass_validation::{Input, encode_passes};

let input = Input::raster(pixels);
let result = encode_passes(encoder, input, destination, output_policy,
                           replay_policy, options, pulse)?;
```

This is an executing experiment in
[perf/multipass-validation](../perf/multipass-validation/README.md), not a new
production entry point. Its two policy arguments expose the remaining integration
work: the final API must use the existing single resolver.

## Caller evolution

Start with a complete borrowed image. `Input::raster` holds its PixelSlice and the
driver supplies those same samples on every pass. There is no raw spool or digest
reservation. The codec still receives samples in its own band buffer; this is not
a claim of zero copies inside the codec.

For a large file or decoder output, use `Input::source(source)`. A positional
source may read again, with explicit version/digest evidence. A forward-only
source can supply the first pass once; subsequent passes require admitted
staging. If exact replay is required, driver-owned memory provides the snapshot.
A caller-owned staging file plus a digest has weaker evidence and does not pass
that requirement. No unknown source promise becomes an exactness guarantee.

```rust,ignore
use multipass_validation::{Input, sources::{FileRows, ReaderRows}};
use zencodec_io::row_encode::Description;
use zenpixels::PixelDescriptor;

// Raw RGB rows from a forward reader, not an encoded JPEG/PNG stream.
let description = Description::new(width, height, PixelDescriptor::RGB8_SRGB);
let input = Input::source(ReaderRows::new(reader, stride_bytes, description));

// Raw rows already in a file: positional replay starts at the given byte offset.
let description = Description::new(width, height, PixelDescriptor::RGB8_SRGB);
let input = Input::source(FileRows::new(file, origin, stride_bytes_u64, description));
```

`stride_bytes` is the distance between source rows, including padding. It must
cover all active samples. Source padding and destination padding are independent:
the [seven-byte source-pitch example](../perf/multipass-validation/tests/sources.rs)
fills six active RGB bytes per row into a nine-byte destination pitch. The driver
does not copy padding into pixels. Malformed requests refuse before input or
destination effects; a read can still fail partway through a valid request.
Cancellation retains its typed cause. Padding discard checks cancellation between
bounded chunks; an arbitrary blocking `Read` cannot be interrupted mid-call.

A pass limit can change the route even for an immutable raster. If the encoder
needs two source passes and the caller permits one, the driver offers staging
or refuses. Staging remains a policy decision, with the reservation held while
the callback considers it. Declining reads no pixels and writes no output.

Destination access is independent. A seek-back writer can receive an index patch;
a forward writer needs a native trailing-index layout or a refusal. Output waits
are a third concern: finish-first staging and live output use the existing output
schedule resolver. Neither source repeatability nor seeking proves a memory bound.

## Adapter and implementor boundary

An encoder reports its actual pass plan and asks for a row range into its own
buffer. The driver checks pass order, complete coverage and declared pass count.
It retains source failures even if the encoder swallows the interruption, and
refuses Filled-without-fill, double fills, early Done and incomplete passes.
Only the selected source/staging mechanism supplies rows.

The raster input arm constructs a concrete reader internally. Custom sources
cannot obtain a token, claim that arm, or replace its rows after selection.
A source wrapper that forwards another source's replay declaration is still an
external source. This retains a useful trust boundary without preventing custom
network or file sources.

## Validation and remaining work

The unchanged native assertions pass for IMI's exercised 2- and 5-pass modes:
byte parity against its bounded entry point, sample parity against whole-image
encoding, strided sources, both output layouts, replay failures and cancellation.
The detached-proof regression now refuses. A positive direct-raster test passes
with zero managed bytes and objects available for digests/raw staging.

This establishes the tested replay mechanism, not release readiness. Remaining
API work includes one resolver for replay and output decisions, operation-scoped
requirement discharge, shared checked request-to-destination execution, bounded
tracing, total retained-memory evidence and async source lifetime validation.
The [precision/color](../benchmarks/multipass_declarations_2026-10-28.json) and
[source-validation](../benchmarks/multipass_sources_2026-10-28.json) receipts record
the subsequent failing-first controls and unchanged native parity gates.

## Bounded content-dependent passes

The integrated experiment now admits `PassCount::Range { min, max }` against
the maximum before reading the source. An unknown maximum cannot meet a finite
pass budget. IMI's native plan supplies the bounds; tests exercise content that
reaches each end. The driver also refuses extra passes before their source read.

```rust,ignore
PassPlan {
    passes: PassCount::Range { min: 2, max: 5 },
    output: PassOutput::FinalPass,
    timing: OutputTiming::streaming(band_rows),
}
```

This is the shape, not a universal IMI pass count: the native configuration
determines the actual range. Output phase and row lag are separate facts. The
driver rejects bytes before the declared final pass, and counts lag within that
pass. Single-band IMI output is withheld until finish. Pulse reports supplied
rows; a content-dependent pass count has an unknown total until the final pass
is identified. See the [integrated validation receipt](../perf/multipass-validation/README.md#bounded-pass-admission-and-final-pass-output).
