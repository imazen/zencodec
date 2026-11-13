# The codec contract, reimagined (spike)

A from-scratch rewrite of what `zencodec` is for: the contract between a
codec and whatever drives it. It lives in its own Cargo workspace. Nothing in
the repository's real crates changes, nothing here is published, and backward
compatibility was not a goal.

Status on 2026-11-13: the fixture tests pass, clippy and rustdoc are clean
with warnings denied, and the contract crate builds for `wasm32-unknown-unknown`
and `thumbv7em-none-eabihf` without `std`. It has been proven on three small
fixture codecs written for the purpose. It has not been ported to a real
codec. Three bounded native RGB8 bindings now live under `tools/`, alongside
the synthetic driver trial. The [4096² / 8192² report](docs/optional-pulse-large-2026-11-12.md)
records wall and CPU time, A/A variation and the remaining scope. Earlier
measurements live on PR 210; see "What it costs and saves an async host".

```text
zencodec/           the contract: no_std + alloc, forbid(unsafe_code)
zencodec-io/        the loops: memory, blocking I/O, Tokio
zencodec-fixtures/  three codecs that exercise it
```

## The whole contract

A decoder names the bytes it wants and is lent a window. An encoder names the
rows it wants and is lent a window. Neither owns I/O or awaits it; the driver supplies the requested input.

| | decode | encode |
|---|---|---|
| A codec, shareable and erased | `DecodeCodec` | `EncodeCodec` |
| The request, as plain data | `DecodeSpec` | `EncodeSpec` |
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
    fn step(&mut self, input: Input<'_>, sink: &mut dyn Sink, pulse: Option<&dyn Pulse>)
        -> Result<DecodeStep>;
}

pub trait EncodeCodec: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn format(&self) -> ImageFormat;
    fn embeds(&self) -> MetadataKinds;
    fn encoder(&self, spec: &EncodeSpec) -> Result<Box<dyn Encoder>>;
}

pub trait Encoder: Send + 'static {
    fn plan(&self) -> &EncodePlan;
    fn step(&mut self, input: EncodeInput<'_>, pulse: Option<&dyn Pulse>) -> Result<EncodeStep>;
    fn output(&mut self) -> &mut OutputBuffer;
}
```

Those four, plus `Sink` (which the caller implements), are the contract. Every
method returns the same `Result`. Every trait is object safe, so there is no
second "dyn" family: `Box<dyn Decoder>` is what a codec returns and what a
registry of dozens of codecs holds. One trait classifies a codec's own error
type, and the optional `simple` traits let a whole-image codec provide its native
operations and format declarations while an adapter supplies the protocol.

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

**No setters.** A spec is plain data: no handles, no lifetimes. Each
negotiable knob is a
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

**One optional pulse, lent to each step.** Progress and cancellation arrive as
`Option<&dyn Pulse>` on `step`. `None` disables observation without a no-op
adapter; `Some` retains both progress and cancellation. Worker drivers take
`Option<SharedPulse>` to retain an observer across a hand-off. Blocking
local work borrows the caller's pulse, including one that cannot be shared; a driver that moves steps to a
worker holds the owned `SharedPulse` and lends it there. A driver also checks
the pulse itself before the first step and after every step, so a codec that
never looks cannot turn a stopped operation into a success. An earlier
revision kept a `SharedPulse` in the spec, which forced sharing on every
caller and made the caller of an async driver hand over the same pulse
twice, once in the spec and once to the driver.

**Encoders say what they can carry.** `EncodeCodec::embeds` names the metadata
kinds a codec can write, so a caller choosing a color carrier knows before it
builds a spec. `EncodePlan::embedded` then states what one request will carry.

**Finishing is a step.** There is no `finish`. A driver that moves steps to a
worker thread moves finalization with them.

## What it costs and saves an async host

The [current bounded study](docs/optional-pulse-large-2026-11-12.md) measures
all listed native and synthetic driver routes at 4096² and 8192². These are
external profiler diagnostics, not zenbench Confirm. Source and binary hashes,
all five shuffled blocks, A/A controls and failures are retained.

For an existing QOI input owner at 8192², the buffered one-hop route measured
183.846 ms wall / 183.818 ms CPU, versus 302.343 / 302.180 ms through the
generic pool route over a borrowed resident source. The owner route avoids
copying its input into the generic window. The 8192² boxed slice contrasts
remain unresolved against A/A variation for all three native codecs; 4096²
BMP has a positive diagnostic contrast beyond its spread. Synthetic 4096²
still has a positive new-pool contrast against the older pool; the 8192²
contrast is unresolved. The window repair does not establish that the earlier
pool regression disappeared.

The native trials use a minimal sink. The public collecting convenience
wrapper, current-head allocation counts, native metadata/general-error
conformance, actual file/network throughput and tile browsing remain unmeasured.
InPlace runs outside a runtime worker in this study, so worker replacement
and concurrent fairness remain unqualified.

These additional structural differences are read off the code:

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

A separate lane did measure earlier revisions of this code against the
current drivers (PR 210, `reimagined/docs/ablation-evidence-2026-11-11.md` on
`review/reimagined-documents`). None of its timing runs passed its own
resource gates, so its timings are descriptive only. Its allocation counts
(heaptrack) are exact. Nothing below was measured on this revision, whose
step signatures have changed since.

- Synthetic decoder, 1,000 decodes of 256x256 RGBA: the pool route here made
  11,013 allocation calls against 21,013 for the current pool driver, and the
  boxed in-memory route matched the current boxed route at 2,000.
- Real QOI and BMP decoders bound through the whole-image adapter, 256x256
  RGB: one allocation call per decode natively, two through this contract
  (the boxed decoder), five through the current erased API. JPEG: 39, 42
  and 45. Two of JPEG's three extra calls came from that binding reading the
  header twice, which `WholeImageDecode::Header` now lets a codec avoid.
- Its process-CPU accounting of the same decodes put the boxed loop within
  0.24 microseconds of native at 256x256 on QOI (112.37 against 112.13) and
  BMP (28.40 against 28.16). Its repeated JPEG control differed by 1.67
  microseconds; that is not a noise bound for QOI or BMP. These means do not
  establish latency parity.
- On tiny images the same accounting resolves a fixed cost for the step
  path (admission, the box, the sink calls): 64 to 90 nanoseconds per decode
  over a native decode that also reads the header. At 8x8 that is 0.247
  microseconds against 0.159 for QOI. It is unresolved against run-to-run
  spread at 63x63 for QOI and JPEG. The borrowed trait-object controls at 8x8 differed from native by 4.8 ns
  for QOI, 5.7 ns for BMP and 45.8 ns for JPEG; this is not a universal
  overhead bound, and the larger cells have their own A/A variation.
- Reading the header twice cost a further 45 nanoseconds for BMP and 474
  for JPEG at 8x8. The header hand-off lets a native binding avoid that cost when its native
  API can reuse the parsed state. This revision has not established that
  removal with native measurements.

It also found two costs in its first capture, both fixed here:

- A positioned source that was already in memory was copied into a window.
  `ReadAt::as_slice` and `AsyncSource::as_slice` now let a driver borrow it;
  tests use sources that panic if read.
- The window filled its memory before every read. It now reuses one
  initialized buffer, and a streaming decode holds a window no larger than
  the decoder leaves unconsumed (a test decodes a 19 KB file through a
  512-byte window).

After those two fixes it counted 2,000 allocation calls per 1,000 decodes on
the positioned route (down from 4,000) and 3,000 on inline Tokio (down from
5,000). Its accounting at 1024x1024 RGBA with 16-row strips, in microseconds
per decode:

| Route | Current drivers | These drivers |
|---|---|---|
| Direct call, no driver | 96.8 | |
| Tokio, inline, bytes in memory | 164.4 | 97.8 |
| Positioned source, bytes in memory | 99.2 | 96.4 |
| One hand-off, bytes in memory | | 107.5 |
| Tokio worker pool, per-read hand-offs | 740.8 | 837.4 |

Both pool routes made 132 context switches per decode: a hand-off per read.
The pool route here was slower than the current one. A cause that fits the
size of the gap: with strips exactly one read long behind a header, the
window slid most of a strip forward before every read, a second pass over
the whole input. Reads are now sized to end where the decoder's units do (a
test decodes rows one read long through a window of one row). The earlier 1024² shape has not been rerun on this head. The current
4096² / 8192² study uses 64-row strips and does not establish removal of
the pool regression.

One cost it measured remains by design: with a worker and a source that is
not in memory, every read is a hand-off. Larger reads mean fewer of them
(`DriveOptions::read_size`), and bytes that are in memory should take the
buffered route.

## Borrowed input and peak memory

The slice, polling-task resident and buffered-owner routes preserve input
ownership. A generic worker route over an async source can still copy into
its owned window; use the buffered route for an existing owner, or the
polling-task route for a borrow. What a caller must hold,
by call shape, for a large file borrowed from a foreign runtime:

| Call shape | What happens to the input |
|---|---|
| The caller blocks in the call | `decode_slice` answers every request with a sub-slice of the caller's bytes. |
| The caller awaits and a worker finishes | `decode_buffered` takes any owner of the bytes that can travel to the worker: a vector, an `Arc<[u8]>`, or a handle that keeps a foreign buffer pinned until it is dropped. A foreign-function layer that copies instead adds the file's size to peak memory; that choice is its own. |
| Rows stream out as they decode | The decoder is lent the same slice at every step. It keeps positions, not references. |

The third row is where the contract can push a codec toward a copy. A
decoder is `'static`, so it cannot hold a reference to the file between
steps. One that keeps offsets and is lent the slice again copies nothing; a
port that stores the bytes inside the decoder copies the whole file, which
in a streaming transcode can be most of the footprint. zenjpeg's scanline
reader already holds a `Cow` and positions, so it fits. Other codecs have
not been surveyed.

Two ways to deal with it, neither built: a conformance check with a counting
allocator that fails a decoder exceeding the memory bound in its plan, which
makes a hidden copy visible to the caller's memory gate; or dropping `'static`
from `Decoder` and adding one provided method that builds a decoder over
resident bytes, with the `'static` requirement moved to the drivers that use
a worker. The second costs a lifetime in driver signatures and a second
constructor for codecs that override it; the step loop and the admission
point stay single.

## zenpixels

Pixels are zenpixels types throughout and are not wrapped or re-exported:
`PixelSlice` and `PixelSliceMut` for packed rows, `PixelBuffer` for an owned
frame, `PlanarSlice` for a planar strip, `PlanarLayout` and `ColorContext` in
the raster description, and `Cicp`, `Orientation` and the HDR types in
`Metadata`.

This branch pins [zenpixels PR 159](https://github.com/imazen/zenpixels/pull/159)
at `240ed475`. Current samples have one active ICC or CICP authority in
`ColorContext`; file metadata may still retain both claims. `PixelEncoding`
borrows the descriptor and current context without retaining another Arc.

`PlanarBuffer` and `PlanarSliceMut` now exist upstream. Owned, independently
strided planes can be lent into this contract with `PlanarBuffer::as_slice()`;
the new foundation controls verify odd 4:2:0 edges through encode and decode.
The collector and fixture encoder consume the new borrowed plane iterator or
indexed plane loans directly, without a temporary descriptor vector.

`EncoderSink` still forwards borrowed strips and cannot retain a planar
remainder when strip heights disagree. The upstream ownership types are
available; an owned delivery/remainder-buffering protocol is not added here.
See [the adaptation and its evidence](docs/zenpixels-159.md).

Earlier timing tables retain their original source and pixel revisions. This
adaptation reruns correctness and compilation checks, not performance.

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
  It also rules out a codec that borrows its source across steps or is bound
  to a thread, which the current collection factories allow. See "Borrowed
  input and peak memory". Whether to keep the bound in the contract or move
  it to the drivers that offload is undecided.
- **No borrowed one-shot entry point.** The review lane proposes one that
  skips the boxed decoder. Its measurements put the whole step path at one
  allocation and 64 to 90 nanoseconds per decode: half again the cost of an
  8x8 QOI decode, about one percent at 63x63, and lost in the noise at
  256x256. Such an entry point would need the same admission guarantees as the step
  path; its measured cost control did not implement those guarantees. It is
  left out of this experiment.
- **No capability record.** What a codec can do is learned by asking:
  admission refuses with the unmet requirement named and the plan reports
  the rest. `formats()` and `embeds()` are the only standing declarations.
  A descriptive record for user interfaces is not modeled.
- **Callers' conveniences.** `probe`, `decode`, `encode`,
  `EncodeSpec::for_pixels` and `Decoded::into_still` are the short path
  here. A registry with detection, and methods on a configured codec, belong
  to the layer above and are not written.
- **Planar:** borrowed delivery, including views lent by `PlanarBuffer`. No
  owned planar hand-off or retained-remainder protocol.
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

What the review on PR 210 turned up, all fixed here with a test that fails
without the fix:

- The inline in-memory Tokio route checked the pulse only between batches, so
  a stop raised during the last batch came back as success. Every driver now
  checks after every step.
- A step quota of zero, settable through the public field, made the async
  drivers yield forever.
- The whole-image decode hook received no pulse, so a simple codec could not
  be stopped or report progress while it worked. Both hooks now receive the
  pulse and the spec.
- The whole-image adapter reported `Info` without checking that metadata the
  caller asked to see first was settled. It now asks for the whole file once
  and refuses if the codec still cannot say.
- One-call encode made the caller state the image's geometry twice, a still
  came back as a list of frames, and there was no header-only helper.

## Running it

```sh
cd reimagined
just prepare # restore the reviewed Cargo dependency graph
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p zencodec --no-default-features --target wasm32-unknown-unknown
```

`zencodec-io/tests/blocking.rs` and `tests/async_io.rs` are the quickest way
to see the contract used: round trips, transcodes between every pair of
fixtures, sequences, planar strips, gates, refusals, error causes,
cancellation (including codecs that never look at the pulse), late metadata,
header-only reads, window sizing, and the Tokio drivers on three executors.
