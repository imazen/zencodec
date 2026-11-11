# The codec contract, reimagined (spike)

A from-scratch rewrite of what `zencodec` is for: the contract between a
codec and whatever drives it. It lives in its own Cargo workspace. Nothing in
the repository's real crates changes, nothing here is published, and backward
compatibility was not a goal.

Status on 2026-11-11: it compiles, 33 tests pass, clippy and rustdoc are clean
with warnings denied, and the contract crate builds for `wasm32-unknown-unknown`
and `thumbv7em-none-eabihf` without `std`. It has been proven on three small
fixture codecs written for the purpose. It has not been proven on a real codec,
and nothing about its speed has been measured.

```text
zencodec/           the contract: 2,806 lines, no_std + alloc, forbid(unsafe_code)
zencodec-io/        the loops: memory, blocking I/O, Tokio (2,045 lines)
zencodec-fixtures/  three codecs that exercise it (1,507 lines)
```

## The whole contract

A decoder names the bytes it wants and is lent a window. An encoder names the
rows it wants and is lent a window. Neither ever blocks, awaits or calls back.

| | decode | encode |
|---|---|---|
| A codec, shareable and erased | `DecodeCodec` | `EncodeCodec` |
| The request, as owned data | `DecodeSpec` | `EncodeSpec` |
| What will happen | `DecodePlan` | `EncodePlan` |
| The work, as a state machine | `Decoder` | `Encoder` |
| Where the product goes | `Sink` | `OutputBuffer` |

```rust
pub trait DecodeCodec: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn formats(&self) -> &[ImageFormat];
    fn decoder(&self, spec: &DecodeSpec) -> Result<Box<dyn Decoder>>;
}

pub trait Decoder: Send + 'static {
    fn requested(&self) -> &[InputRequest];
    fn info(&self) -> Option<&ImageInfo>;
    fn plan(&self) -> Option<&DecodePlan>;
    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink) -> Result<DecodeStep>;
}

pub trait EncodeCodec: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn format(&self) -> ImageFormat;
    fn encoder(&self, spec: &EncodeSpec) -> Result<Box<dyn Encoder>>;
}

pub trait Encoder: Send + 'static {
    fn plan(&self) -> &EncodePlan;
    fn step(&mut self, input: EncodeInput<'_>) -> Result<EncodeStep>;
    fn output(&mut self) -> &mut OutputBuffer;
}
```

Those four, plus `Sink` (which the caller implements), are the contract. Every
method returns the same `Result`. Every trait is object safe, so there is no
second "dyn" family: `Box<dyn Decoder>` is what a codec returns and what a
registry of dozens of codecs holds. One trait classifies a codec's own error
type, and two optional traits in `simple` let a codec that has nothing to
stream implement two functions and get the rest.

The surface this stands in for, counted on the 0.2 review branch (`63f136f0`):
11 typed codec traits and 9 `Dyn*` mirrors, with shims between them and three
error carriers (`BoxedError`, `ErasedError`, `StepError<E>`).

## The decisions, and what each one removes

**One error type.** `Error` is `At<CodecError>`: a kind to decide on, the
codec's name, and the codec's own error as the cause. `find_cause::<T>` and
`CodecError::native_as::<T>` recover the original type through any amount of
erasure, including through a transcode. The associated `Error` type was the
only thing forcing every trait to exist twice.

**One state machine per direction.** `Decoder::step` and `Encoder::step`
replace six ways to encode (`encode`, `encode_srgba8`, `begin`/`push_rows`/
`finish`, `begin_output`/`finish_output`, `begin_declared`/`pull_rows`,
`encode_from`) and five decode traits. One-shot, push, pull, replay,
animation and range-driven decoding are the same loop under different
drivers. `zencodec-io/src/memory.rs` is that loop for bytes and pixels in
memory, and it is short.

**Row demand is data.** An encoder returns `NeedRows { pass, y }` instead of
calling a row source. A host that must await its rows answers when it has
them. This is what the pull and replay paths could not offer an async or
foreign host before: a synchronous callback cannot await.

**No setters.** A spec is plain owned data. Each negotiable knob is a
`Demand`: `Default`, `Prefer(x)` or `Require(x)`. `decoder()` and `encoder()`
are the only admission points. They refuse an unmet requirement with the
requirement named, and the plan they return states what a preference became.
There is no best-effort path beside a checked one, because there is one path.

**Gates and targets, checked once.** `ResourceLimits` are promises: the codec
stays inside them or refuses, and it refuses an unknown memory bound whenever
a memory gate is set. `ResourceTargets` are preferences. A plan carries a
`ResourceEstimate`, so the output-size gate of an encoder that knows its
exact size is settled before any row moves.

**`Info` is the admission point for decoding.** It is returned once, before
any sink call and before any allocation proportional to the image. Stepping
again accepts; dropping the decoder declines. That replaces the controlled
decode trait, boundary tickets and `resume`. Metadata a caller must see
before pixels is a spec field (`metadata_first`); a decoder whose metadata
sits after the pixels asks for those bytes first.

**Three ways to hand over packed rows.** A sink implements `lend`. `rows`
(the decoder shows rows it holds) and `frame` (the decoder gives away a
finished buffer) default to it. No decoder copies to fit the interface: a
streaming decoder writes into the sink's memory, a decoder with rows already
in hand shows them in place, and a whole-image decoder hands over its
allocation.

**The encoder owns its output.** It writes to an `OutputBuffer`; the driver
takes from it between steps. Taking moves the allocation, so an in-memory
encode returns the encoder's vector as is. `patch()` rewrites bytes still in
the buffer in place and records a `Patch` only for bytes already taken, so
the header-patching protocol is one method. An encoder that will patch a
forward-only destination calls `hold()`.

**One pulse.** Progress and cancellation arrive in the spec as a
`SharedPulse`. There is no per-step pulse and so no rule about honoring both.

**Finishing is a step.** There is no `finish`. A driver that moves steps to a
worker thread moves finalization with them.

## What it costs and saves an async host

This spike measures nothing itself. These are structural differences read off
the code:

- A batch hand-off to a worker moves one pointer. The current async driver
  moves the decoder, sink and window by value into a boxed job, per batch.
- The drivers are not generic over the codec. There is one compiled copy
  however many codecs exist.
- Reads are planned from `requested()` as owned data before the first
  `.await`, so nothing borrowed from the decoder crosses one. The current
  driver copies the request list for this reason.
- An encoder's bytes leave as an owned vector, so batch *n* is written to the
  socket while batch *n+1* computes. Two vectors alternate.
- The buffered route, which is what a callback-completion FFI host
  (`imageflow-async-ffi`) does today, is one `spawn_blocking` around the
  in-memory driver, because decoders are `'static` and borrow input per step.
- Per step, the cost is one dynamic call, the same as the erased path today.
  A concrete codec called directly would save that call; nothing prevents a
  codec from also offering inherent methods.

A separate lane did measure an earlier capture of this code against the
current drivers (PR 210, `reimagined/docs/ablation-evidence-2026-11-11.md` on
`review/reimagined-documents`). Its timing run was rejected by its own
resource gates, so its timings confirm nothing. Its allocation counts
(heaptrack, 1,000 decodes of 256x256 RGBA) are exact: the pool route here made
11,013 allocation calls against 21,013 for the current pool driver, and the
boxed in-memory route matched the current boxed route at 2,000.

It also found two costs in that capture, both fixed here and neither
re-measured:

- A positioned source that was already in memory was copied into a window.
  `ReadAt::as_slice` and `AsyncSource::as_slice` now let a driver borrow it;
  tests use sources that panic if read.
- The window filled its memory before every read. It now reuses one
  initialized buffer, and a streaming decode holds a window no larger than
  the decoder leaves unconsumed (a test decodes a 19 KB file through a
  512-byte window).

One cost it measured remains by design: with a worker and a source that is
not in memory, every read is a hand-off. Larger reads mean fewer of them
(`DriveOptions::read_size`), and bytes that are in memory should take the
buffered route.

## zenpixels

Pixels are zenpixels types throughout and are not wrapped or re-exported:
`PixelSlice` and `PixelSliceMut` for packed rows, `PixelBuffer` for an owned
frame, `PlanarSlice` for a planar strip, `PlanarLayout` and `ColorContext` in
the raster description, and `Cicp`, `Orientation` and the HDR types in
`Metadata`.

zenpixels needed no change. The planar strip view this design wanted,
`plane::PlanarSlice`, landed on zenpixels main while the design was being
worked out, and the spike pins that revision.

One gap showed up and is deliberately not filled: there is no mutable or
owned planar strip. `EncoderSink` passes planar strips from a decoder straight
to an encoder with no copy, but only when the decoder's strip height suits
the encoder; it cannot hold a planar remainder. Adding the type means public
API in zenpixels, whose rule is no public item without a consumer that is
merging. This spike is not that consumer yet.

## What is slim or missing

- **Carriers.** `ImageFormat` here is an open identity with no format table.
  `ImageInfo`, `Metadata`, `ResourceLimits` and `Fidelity` are reduced to
  what the contract needs. The current crate's versions carry over; they are
  not what this spike reimagines.
- **Decode knobs not modeled:** region, orientation handling, gain maps,
  starting frame.
- **Encode side:** color emission policy and metadata filtering are the
  caller's job by design (the encoder is handed exactly what to embed), and
  the libraries that resolve them are not ported.
- **Not touched at all:** the multi-image `Collection` contract,
  `zencodec-media`, the registry and application conveniences (which belong
  in `zencodecs`), EXIF/ICC/gain-map parsing (which belongs in `zenmeta`),
  and the conformance testkit.
- **Accounted storage.** The current crate charges retained images against
  a `StorageBudget`. Here a gate bounds what a codec may own and a driver may
  hold, and nothing tracks a buffer after it is handed over.
- **`Send + 'static` is required of every decoder and encoder.** That is what
  lets one erased type serve a registry, a worker pool and a foreign host.
  It also rules out a codec that borrows its source or is bound to a thread,
  which the current collection factories allow. Whether to keep the bound in
  the contract or move it to the drivers that offload is undecided.
- **Planar:** only borrowed strips. No owned planar hand-off.
- **Provisional passes** are in the signatures (`Pass`) and no fixture
  produces one.

## What writing it turned up

- A type that implements both codec traits made `.prepare(..)` ambiguous at
  every call site, so the admission methods are `decoder()` and `encoder()`.
  `name()` still collides on such a type when both traits are in scope.
- The first driver lent the previous window after a decoder changed its
  request without returning `NeedInput` (a range read followed by rows). The
  rule is now that `requested()` is authoritative before every step. A test
  covers it in all three drivers.
- A sink that accepts planar strips of any height needs somewhere to keep a
  remainder. See the zenpixels note.

## Running it

```sh
cd reimagined
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p zencodec --no-default-features --target wasm32-unknown-unknown
```

`zencodec-io/tests/blocking.rs` and `tests/async_io.rs` are the quickest way
to see the contract used: round trips, transcodes between every pair of
fixtures, sequences, planar strips, gates, refusals, error causes,
cancellation, late metadata, and the Tokio drivers on three executors.
