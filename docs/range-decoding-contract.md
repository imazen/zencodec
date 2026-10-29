# Sans-I/O Decoding and Streaming Encoding Contract

The [2026-10-24 IO proposal](io-contract-proposal.md) records subsequently tested
driver repairs, private admission/ownership candidates and proposed extensions.
It distinguishes measured behavior from additions awaiting API review.

How a decoder that performs no I/O talks to whatever holds the bytes: a
file, an HTTP body, an object store with range requests, an FFI host that
pushes buffers, a WASM event loop. The same contract carries a decode
straight into an encoder. End-to-end bounded memory additionally requires
verified codec/configuration retention, caller ownership and driver budgets;
streaming byte availability alone does not establish it. The
[scheduling proposal](io-contract-proposal.md#5-bound-compute-batches-and-preserve-media-order)
separates CPU fairness, raw-storage release and output backpressure.

The trait lives in `zencodec::decode` (`src/traits/incremental.rs`); the
drivers live in `zencodec-io`; `zencodec-coroutine` lets a decoder be
written as straight-line `async` code; `zencodec-testkit` checks a codec
against all of it.

---

## 1. Why a dialogue, not a whole-file pass

`zencodec::inventory::Inventory` audits a file the caller already holds:
every byte from `0..len` must belong to a known part. A client streaming a
remote image holds none of it yet, may not know its length, and may need
bytes far from the start first: a TIFF's first IFD can sit at the end of a
10 GB file, an unoptimised ISOBMFF puts `moov`/`meta` last, a JPEG XL TOC
points at groups hundreds of megabytes in. Decoding as data arrives, or
fetching only the tiles a crop needs, takes a dialogue: the decoder says
what it needs next, the driver gets it, and neither knows the other's job.

| the driver knows | the decoder knows |
|---|---|
| transport: files, `Range: bytes=`, HTTP/2 multiplexing, S3 `GetObject` | container structure: IFD chains, TOCs, boxes, markers |
| latency, coalescing, retries, connection pools | which tiles or groups a crop or page needs |
| buffering and memory caps for input | entropy decoding and pixel reconstruction |
| cancellation and progress (`how_far::Pulse`) | where rows go (the sink) |

The decoder never touches a socket, a file or a URL. The driver never parses
a byte of the format.

---

## 2. The contract

```rust
// zencodec::decode
pub trait IncrementalDecode {
    type Error: core::error::Error + Send + Sync + 'static;

    /// Take bytes from the front of `input` and advance by one event.
    fn step(&mut self, input: Input<'_>, sink: &mut dyn DecodeRowSink, pulse: Option<&dyn Pulse>)
        -> Result<Progress, StepError<Self::Error>>;

    /// What the decoder needs, valid after a step returned `Event::Need`.
    fn requested(&self) -> &[InputRequest];

    fn info(&self) -> Option<&ImageInfo>;
    fn output_descriptor(&self) -> Option<PixelDescriptor>;   // default None
    fn access_pattern(&self) -> AccessPattern;                 // default Sequential
}

pub struct Input<'a> { pub offset: u64, pub bytes: &'a [u8], pub total_len: Option<u64> }
pub struct Progress { pub consumed: usize, pub event: Event }

#[non_exhaustive]
pub enum Event { Need, Rows { meta: BatchMeta, rows: u32 }, Metadata(MetadataUpdate), Done(OutputInfo) }

#[non_exhaustive]
pub enum InputRequest {
    AtLeast { offset: u64, min_len: usize },   // bytes from `offset`, at least `min_len`
    Range(ByteRange),                          // exactly this span
    Len,                                       // the total length, as Input::total_len
}

#[non_exhaustive]
pub enum StepError<E> { Codec(E), Sink(SinkError), Cancelled(StopReason), Finished }
```

The driver loop, in full:

```text
let mut dec = job.dyn_incremental_decoder(&[])?;
let mut input = Input::empty();
loop {
    let p = dec.step(input, &mut sink, &stop)?;
    // the first p.consumed bytes of `input` are gone; the rest are still the driver's
    match p.event {
        Event::Need => { /* satisfy any one of dec.requested(); build the next Input */ }
        Event::Done(info) => break,
        _ => {}  // Rows, Metadata, or an event added later: step again
    }
}
```

### The rules

**The driver owns the bytes.** `Input` is a run of bytes at an absolute
offset, possibly empty, plus the source length once the driver knows it. The
decoder takes a prefix and reports its length in `Progress::consumed`: bytes
it decoded in place or copied into its own state, plus any bytes before them
it does not need (a gap the driver read across to batch two ranges).
Consumed bytes are never offered again; the rest stay the driver's, to offer
again with more appended or to drop. A decoder is never required to buffer
input, and never fails because it was offered bytes it did not ask for.

**Requests.** After `Need`, `requested()` is non-empty and the driver
satisfies any one entry:

- `AtLeast { offset, min_len }`: an input starting at `offset` holding at
  least `min_len` bytes, or every remaining byte with `total_len` set when
  the source ends first. More is welcome. `min_len` is the contiguous span
  the decoder needs to make progress without copying: one worst-case row,
  one marker segment, one strip.
- `Range(r)`: an input holding exactly that span, or a larger input
  starting at its offset.
- `Len`: any input, even empty, carrying `total_len`.

**Progress or an error, never a spin.** A decoder makes progress whenever
its input satisfies one of its requests. Once `total_len` is known, a
request that cannot be met is a `StepError::Codec` (truncated input), not
another `Need`. Drivers check every `Need` against the input just offered
and report `ProtocolViolation::{EmptyRequest, NoProgress, NeedPastEnd}`
instead of looping.

**Output.** `sink.begin` comes before the first rows and `sink.finish`
before `Done`. Every row is written once, final, through
`provide_next_buffer`, in increasing `y`. A progressive decoder may also
write provisional rows (a pass a later one refines) through
`provide_pass_buffer`, only when the sink's `accepts_provisional()` returned
`true` at `begin`. A decoder with no provisional output ignores both, and a
final-only sink never sees a provisional row.

Sink capacity is independent of a native decoder's batch size. After `begin`,
`DecodeRowSink::max_buffer_rows` reports the admitted height per borrowed buffer.
`helpers::copy_rows_to_sink`, whole-image/frame copy helpers and coroutine
delivery split larger native batches into consecutive buffer requests. They
preserve source/destination stride and provisional pass coordinates. The copy
helpers deliver their whole batch before returning. Coroutine delivery instead
returns one `Event::Rows` per admitted buffer, retaining the rest of the batch
without polling the producer or consuming more input. A pull consumer can drain
that buffer before the next step. The held native batch and reconstruction
workspace remain outside this sink-capacity guarantee.
Legacy direct sinks must refuse oversized requests rather than grow silently.

Qualification (2026-10-28): a single native PNG batch exhausted a bounded pull
adapter's object allowance because the coroutine filled every sub-strip before
returning. `just coroutine-row-delivery-check` reproduces the same failure with
a one-slot sink, then verifies 129-row batches, different source/destination
strides, final/provisional coordinates, completion in the producer's final poll,
and cancellation between deliveries. All prior coroutine tests remain unchanged.
The native PNG consumer rerun is a separate qualification gate.

Qualification (2026-10-25): the 97x65 PNG guide input exposed a 65-row native
batch against an admitted 64-row transcode buffer. Shared helper controls use
capacities 1/2/3/7/9/unbounded with trimmed strided input, nonzero origins,
provisional passes and failure partway through copying. Controlled erasure
preserves a one-row limit across both policy suspensions. `just
sink-capacity-check` passes conformance, coroutine, strict Clippy, no-default
compilation and public API checks. Resource receipt: rc=0, 8 s; run-heavy peak
RSS 0.33 GiB, minimum available 17014 MiB, peak load 0.73. This measures the
validation process, not native workspace or throughput.

**Termination.** After `Done`, every step returns `Done` again with
nothing consumed. After any error, every step returns `Finished`. `stop` is
checked at least once per step; a tripped stop is `Cancelled`, and
terminal. `Fused<D>` enforces all three for any decoder, and
`erase_incremental(decoder)`, which a codec's `dyn_incremental_decoder`
returns, applies it.

**Events.** `Event` is non-exhaustive. Variants added later are
informational: a driver that does not recognise one steps again. Anything
that needs the driver to act is a `Need` plus a `InputRequest`.

**Crop and region intent** stay on the job; the sink's coordinate space is
the output image.

### Pull-native codecs

A third-party library that reads `Read + Seek` cannot be resumed. It binds
to the job instead, with the `push_decoder` signature and the input as a
random-access source:

```rust
// zencodec::decode::DecodeJob (default: UnsupportedOperation::SeekableDecode)
fn push_decoder_from_source(self, source: &mut dyn DynReadAt,
    sink: &mut dyn DecodeRowSink, preferred: &[PixelDescriptor])
    -> Result<OutputInfo, BoxedError>;

// zencodec::source
pub trait ReadAt {
    type Error;
    fn len(&mut self) -> Result<u64, Self::Error>;          // may be expensive
    fn len_hint(&self) -> Option<u64>;                      // default None
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error>;
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error>; // short only at the end
    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error>;
    fn as_slice(&self) -> Option<&[u8]>;                    // in memory: read without copying
}
```

| codec core | implements | other drivers reach it through |
|---|---|---|
| hand-written state machine (QOI, PNG, WebP, GIF) | `IncrementalDecode` | every `zencodec-io` driver |
| container walk written by us (TIFF, ISOBMFF, imij) | `IncrementalDecode` via `zencodec-coroutine` | every `zencodec-io` driver |
| third-party pull library | `push_decoder_from_source` | `BufferedReadAt` over a stream, `tokio_io::pull_decode_on_thread` under async |

---

## 3. A remote TIFF crop, step by step (illustrative)

```text
driver: step(Input::empty())                      -> Need; requested = [Range(0, 8)]
driver: fetch bytes 0..8, step(Input::new(0, ..)) -> consumed 8, Need; requested = [Range(450_000_000, 16_384)]
        (the header says IFD0 is at 450 MB)
driver: fetch, step                               -> consumed 16_384, Metadata(HeaderReady)
driver: step(remaining input, empty)              -> Need; requested = [Range(1_200_000, 80_000), Range(1_280_000, 80_000)]
        (the crop maps to tiles 42 and 43)
driver: coalesce into one 160_000-byte read, fetch, step(Input::new(1_200_000, ..))
                                                  -> consumed 80_000, Rows { y: 0, rows: 256 }
driver: step(the same buffer from 1_280_000)      -> consumed 80_000, Rows { y: 256, rows: 256 }
driver: step                                      -> Done
```

---

## 4. Latency and range coalescing

Over a network, round trips cost more than bytes. Tile 1 at
`10_000..20_000` and tile 2 at `26_000..36_000` leave a 6 KiB gap:

- Two requests: 20 KiB, two round trips. At 50 ms RTT, 100 ms.
- One request `10_000..36_000`: 26 KiB, one round trip; the extra 6 KiB
  take 0.9 ms at 50 Mbit/s. About 51 ms.

Drivers merge requested ranges whose gap is at most `DriveOptions::max_gap`
(`zencodec_io::coalesce_byte_ranges`); 16 to 64 KiB suits HTTP. The decoder
then receives the merged buffer and consumes the gap as part of its prefix,
so gap bytes are never stored.

## 5. CDN and server caveats

1. **`multipart/byteranges` is unreliable.** Many CDNs and proxies answer a
   multi-range request with the whole file or a 400. Use coalescing for
   small gaps and independent requests over HTTP/2 or HTTP/3 for the rest;
   `AsyncSource::read_ranges` is where a source issues them concurrently.
2. **`200 OK` instead of `206`.** A server without `Accept-Ranges: bytes`
   streams the whole file from byte 0. Detect it on the first response and
   serve the decoder sequentially from that body (`AsyncReadSource`, or
   `BufferedReadAt` for a random-access decoder).

---

## 6. Drivers (`zencodec-io`)

| input | resumable decoder | pull-native decoder |
|---|---|---|
| slice | `drive_from_slice` | `SliceReadAt` |
| `ReadAt` (file, object store, FFI `read_at` callback) | `drive_from_read_at` | direct |
| `std::io::Read` stream | `pump_read_to_sink` | `BufferedReadAt` under a `BufferPolicy` |
| async runtime (feature `tokio`) | `tokio_io::decode_async`, `transcode_async` | `tokio_io::pull_decode_on_thread` |

- `drive_from_slice` offers sub-slices of the caller's bytes; nothing is
  copied. `drive_from_read_at` takes the same path when the source reports
  `as_slice`.
- Every other driver keeps one input window. It is zeroed once as it grows
  and reused; reads go straight into its tail, and it fills to a whole
  multiple of the decoder's `min_len` no smaller than `read_size`, so a
  decoder that consumes one request per step drains it and the window
  restarts at its front instead of moving a leftover tail on every read.
- `drive_from_read_at` merges each round's `Range` requests and fetches
  them with one `read_many_at`, so a concurrent source overlaps their round
  trips; later ranges of the round are served from the fetched buffers. It
  asks for the length only when a decoder sends `Len`, because a buffered
  stream answers that by reading to its end.
- `pump_read_to_sink` reads a stream forward. A decoder emits rows as soon
  as the window holds its `min_len`. A request behind the stream position
  is `PumpError::NeedsRandomAccess`, with `BufferedReadAt` as the fix; a
  `min_len` beyond `max_window` is `PumpError::WindowLimit`.
- `BufferedReadAt` manufactures seeks over a stream: `Window` (bounded
  retention; far-back reads fail with `BeyondWindow`) or `Spool` (retain up
  to a cap; `BeyondLimit` past it). `BufferPolicy::for_pattern` picks the
  cheapest policy that cannot fail for the decoder's `AccessPattern`.
  Buffering never silently grows and never returns wrong bytes.

### Async, without async in codecs

`decode_async(decoder, sink, &mut source, compute, opts, pulse)` keeps the
decoder, the sink and the window in the async task. It awaits the
`AsyncSource` for bytes, then hands one batch (steps until the decoder needs
input or finishes) to a `Compute` and awaits that.

- While a client uploads, nothing occupies a thread. CPU work runs on the
  pool in batches as large as the input on hand.
- `AsyncSource::read_at` returns whatever has arrived, like `AsyncRead`, so
  rows come out while the upload is still running. `read_ranges` is where a
  source overlaps range requests; the default reads them in turn.
- `Compute` is any `Fn(Job)`: `|job| { tokio::task::spawn_blocking(job); }`,
  `|job| rayon::spawn(job)`, or `Inline`, which runs each batch on the
  calling task (single-threaded hosts, WASM, small images) with no handoff.
- `transcode_async` routes rows through `EncoderSink` into a streaming
  encoder and writes its pending output to an `AsyncWrite` after every
  batch.
- The returned futures are `Send` for any `Send` decoder, sink and source,
  so they go to `tokio::spawn`.
- `pull_decode_on_thread` runs a pull-native decoder on one pool thread
  with a `DynReadAt` whose reads block on the same `AsyncSource`; its
  `read_many_at` becomes one `read_ranges`. Such a codec holds a thread for
  the whole decode by design.
- `ChannelRowSink` ships finished strips to the async side over a bounded
  channel, throttling the decoder when the consumer is slow. It blocks, so
  it needs a `Compute` off the runtime (or the pull bridge).

### Cancellation and progress: one handle

Every driver takes one `how_far::Pulse` (`Pulse: Stop + Report`). A caller
with only a `Stop` passes `how_far::StopOnly::borrowed(&stop)`.

- The pulse is the `stop` of every `step`, so a codec observes it at least
  once per strip, and a tripped pulse is terminal (`Cancelled`, then
  `Finished`).
- The driver calls `set_total(Total::Exact(height))` once the header is
  known and `advance(rows)` on every `Rows`.
- Codec-side reporting inside one step goes through the job-level pulse
  (`DecodeJob::with_pulse`); the step contract exposes only `Stop`.
- Async: inside a compute batch the pulse stops the decoder within one
  step. While the driver awaits the source, cancel the tokio way: drop the
  future, or race it in `tokio::select!` against a token or a timeout.
  Nothing runs between batches, so the decoder and sink drop at once.
- `pull_decode_on_thread` re-checks the pulse while its thread waits for a
  read (a short spin, then sleeps doubling from 50 µs to 2 ms), and the
  async side stops serving a hung source as soon as the thread finishes.
  `ChannelRowSink::with_pulse` does the same while blocked on a full
  channel.

---

## 7. Streaming encode and transcode

`Encoder` gains four provided methods, all optional:

```rust
fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor)
    -> Result<OutputTiming, Self::Error>;   // declare the image; learn when bytes arrive
fn pending_output(&self) -> &[u8];          // encoded bytes ready to send
fn consume_output(&mut self, n: usize);     // mark them sent
fn take_output(&mut self, into: &mut Vec<u8>); // append them to `into` and mark them sent
```

`begin` is the negotiation point for a row-level encode. The encoder checks
the declaration against its configuration, limits and metadata and answers
with an `OutputTiming`:

- `OutputTiming::streaming(lag_rows)`: the header is written now and bytes
  appear in `pending_output` as rows arrive. Once more than `lag_rows` rows
  are pushed, every row but the last `lag_rows` is encoded and its bytes
  are pending, apart from a few bytes of coder state (bits short of a byte,
  an open run). `finish` returns only what was not consumed.
- `OutputTiming::AT_FINISH`: `pending_output` stays empty; everything comes
  from `finish`. This is also the default for encoders that do not
  override `begin`.
- An error, before any row moves: the image is out of bounds, or this
  configuration cannot encode from rows
  (`UnsupportedOperation::RowLevelEncode`).

Pushing more or fewer rows than declared is an error. Draining changes
memory, never output: the drained bytes plus `finish`'s remainder equal the
bytes of the same encode left undrained. Without `begin`, encoders behave
as before and may hold everything until `finish`.

### Rules

Written on `Encoder::begin`, enforced where marked by
`check_streaming_output_honesty`:

- **Declaration.** `begin` answers as `encode()` of that size would; a
  second `begin` is an error (checked); a failed `begin` leaves the encoder
  unchanged (checked after a zero-size refusal: the real `begin` on the
  same encoder streams the same file). `encode()`/`encode_from()` after
  `begin` is an error (checked for streaming encoders).
- **Lag.** Below the image height (checked). It counts pushed rows whose
  bytes are not yet pending, regardless of push size. Between any two
  points `lag` rows apart, bytes become pending (checked, which catches an
  understated lag and a stall after the first strip). A coder without
  row-aligned output claims a lag by flushing on a row schedule fixed at
  `begin`.
- **Declaring a zero size.** `begin(0, h)` or `begin(w, 0)` is accepted
  exactly when `encode()` of that size succeeds (checked for streaming
  encoders).
- **Bytes.** Draining never changes the file: draining all, uneven prefixes,
  more than is pending (which clamps) and `take_output` all give the
  undrained bytes (checked). `begin` may choose framing that makes the file differ from
  `encode()`'s, never the pixels (checked: same decoded pixels).
- **Errors.** After an error from `push_rows` or `finish` the encode cannot
  complete. Bytes already handed out are a prefix of a file that will never
  be finished, so a driver that sent any aborts its transport: an HTTP
  server closes the connection, an uploader aborts the multipart upload.
- **Limits.** `max_output_bytes` bounds the whole file, counting bytes
  already handed out. An undrained streaming encode holds its output like
  an `AT_FINISH` one; the memory limit may refuse it.
- **Colour.** The job's metadata and policy are authoritative; rows whose
  pixel colour context disagrees may be refused.

`EncoderConfig::with_prefer_streaming(true)` lets a codec-agnostic caller ask
for a streaming configuration: fixed tables and a sequential scan for JPEG;
no whole-image reductions for PNG. Explicit codec settings win, and `begin`
still reports the truth.

The check needs an image taller than `lag + 2 x strip` to verify a lag. It
fails on a shorter one rather than pass it unverified; `check_all` sizes
its image from the timing the default configuration reports.

### Decisions

- **The answer is per encoder, the capability is per codec.**
  `EncodeCapabilities::streaming_output` is static and says some
  configuration streams; whether this one does depends on things only the
  encoder knows. JPEG streams with fixed Huffman tables and a sequential
  scan, but optimized tables and progressive scans need every block first,
  and an MPF gain-map index needs the primary image's length. BMP, TGA and
  PFM store rows bottom-up. IMIJ streams in its row-group mode only. A
  static flag cannot express any of these; `begin` can, and it answers
  before the first row, when a driver still has every choice open: an HTTP
  response picks `Content-Length` or chunked transfer, a pipeline sizes its
  buffers from `lag_rows`.
- **Bytes are pulled, not pushed.** The encoder owns its output buffer and
  the driver drains it, as on the decode side the driver owns the input.
  The encoder does no I/O, the same encoder serves blocking writers, async
  writers and in-memory collection, and backpressure is the driver
  pushing fewer rows. The cost is one copy from the encoder's buffer to the
  destination, the same copy a `Write` adapter would make.
- **`OutputTiming` has private fields.** A later field (for example,
  "streams if the destination can seek back to patch a size") is additive.
  Encoders that write a size or index after the data and could patch it in
  place need that; today they report `AT_FINISH`.
- **An owned handoff on top of the pull.** `take_output(&mut self, into:
  &mut Vec<u8>)` appends the pending bytes to `into` and consumes them. Its
  default copies. An encoder that keeps its output in `OutputBuffer` swaps
  allocations when `into` is empty, so a driver that hands each batch on
  as an owned buffer (a hyper `Bytes` chunk, an upload part) never copies.
  A writer-provided buffer (`IBufferWriter`'s `GetMemory`/`Advance`) was
  not adopted: an encoder cannot size its output per row, so it would have
  to negotiate buffer sizes in every codec.
- **One output buffer for every codec.** `OutputBuffer` holds what each
  streaming codec had written separately: the consumed-prefix cursor, the
  compaction (drop the sent prefix once it is at least 64 KiB and half the
  buffer), the clamp on an over-large consume, the count of bytes handed
  out for `max_output_bytes`, and the swapping `take_into`. A codec that
  writes through a `Vec<u8>` uses `vec_mut()`.
- **Animation streams the same way, counted in frames.**
  `AnimationFrameEncoder::begin(width, height, descriptor, frame_count)`
  returns an `AnimationOutputTiming` (`AT_FINISH` or
  `streaming(lag_frames)`), with the same `pending_output` /
  `consume_output` / `take_output` and the same rules counted in frames.
  `frame_count` is optional because most callers know it (a transcode
  from a probed file) but a live source does not; formats that write the
  count first (APNG's `acTL`, the reference format) stream only when it is
  given, and finishing after a different number of frames is an error.
  `check_animation_streaming_output_honesty` enforces the rules, including
  a refused zero-width `begin` leaving the encoder unchanged.
- **Out of scope for now:** streaming from `encode_from` (it returns the
  whole file; drivers that stream use `push_rows`).

`EncoderSink` is the bridge: a `DecodeRowSink` that forwards `begin`, hands
the decoder its own strip buffer and pushes each strip to the encoder when
the next one is requested (no pixel is copied between them), and exposes
`pending_output`/`take_output`/`write_pending`. A decode-to-encode
transcode then holds the input window, one strip, and the encoder's
undrained output.

For an image already in memory, `zencodec_io::encode_to_writer(encoder,
pixels, &mut writer)` runs the whole loop: `begin`, strips of at least 16
rows, a write after the header and after each strip, `finish`. It falls
back to one-shot `encode` when `begin` declines row-level encode.
`write_pending` / `write_pending_animation` (and `tokio_io::write_pending_async`)
are the single drain step, which writes and consumes together so the same
bytes are never sent twice.

---

## 8. Authoring (`zencodec-coroutine`)

A hand-written resumable decoder is a state enum per parser stage.
`zencodec-coroutine` repurposes `async`/`await` as a compile-time coroutine
generator: the author writes sequential code against a `CoroutineContext`
(`read_exact_at`, `read_many_at`, `read_exact`, `len`, `write_rows`,
`announce`), `rustc` builds the state machine, and `CoroutineDecoder` polls
it behind `IncrementalDecode`. No runtime, no executor, no threads;
`no_std + alloc`, `forbid(unsafe_code)`.

- Each suspending read is one `Need` round. Sequential reads ask for the
  rest of the read as one `AtLeast`; `read_many_at` names every range in
  one round.
- Offered input is absorbed only where outstanding reads need it, under a
  byte cap; gap bytes between requested ranges are consumed but never
  stored. A single read larger than the cap is `ReadExceedsInputCap`, not a
  stall.
- Rows written before the header is announced are `RowsBeforeHeader`;
  provisional rows reach only sinks that accept them.
- Cost, inherent to safe Rust: input is copied into the coroutine's store
  and each strip once more into the sink. It suits container walks and
  headers; bulk pixel paths belong in a hand-written step.

---

## 9. Decisions and the trade-offs taken

- **The driver owns the input.** The earlier draft had `feed` (copy into
  the decoder) and `step`. Every decoder needed an input buffer, copies
  were the default, and "accepted" meant two things. Now the decoder reads
  the driver's bytes in place and reports a consumed prefix; a decoder that
  needs to keep a few bytes across a boundary copies exactly those.
- **One call, one event.** `step` both offers input and advances, so there
  is no state where bytes were fed but not acted on, and every protocol
  violation is observable from one `(input, consumed, requested)` triple.
- **`AtLeast` carries `min_len`.** The decoder states the contiguous span
  it needs to decode in place (QOI: one worst-case row, `width * 5`).
  Drivers then never offer too little, and decoders never stitch.
- **Final rows always go through `provide_next_buffer`, in order.** Every
  sink gets the simple contract; only progressive codecs and progressive
  sinks touch `provide_pass_buffer`. Settled when the testkit's provisional
  check found the reference codec and the coroutine disagreeing.
- **Errors are typed and terminal.** `StepError` keeps codec, sink,
  cancellation and misuse apart (no `wrap_sink_error`, no strings), and
  `Fused` makes termination a property of the erased decoder rather than a
  promise of each codec.
- **No `async` in codec traits.** Codecs stay synchronous, `no_std` and
  runtime-agnostic. The async driver keeps the decoder in the task between
  batches and runs batches on a pool, so it holds no thread during I/O.
  Accepted cost: a pool handoff per batch (three allocations), tuned by
  `read_size`; cancellation while awaiting I/O is by dropping the future.
- **No `IncrementalDecodeJob` trait.** The erased decoder from
  `dyn_incremental_decoder` serves every driver; a generic job trait would
  add a second path without a measured need.
- **Not done:** true incremental PNG needs a resumable inflate; zenflate's
  stream decompressor is not resumable mid-block, so PNG stays one-shot
  under this contract for now.

---

## 10. Measured

QOI (zenbitmaps PR 32) ported to this contract, against its own one-shot
paths on 8 imazen-26 photos (`zenbitmaps/benchmarks/qoi_sansio_v2_2026-10-14.md`,
mac M4 Pro):

| 2048-wide photos | vs one-shot |
|---|---|
| step decode, whole slice | -4.0 to -2.3% |
| step decode, stream pump (64 KiB reads) | -2.2 to +0.0% |
| `push_rows` encode, 16-row strips | +1.0 to +3.0% |
| decode into `EncoderSink` transcode | -0.9 to +0.8% |

Framework overhead with a memcpy-per-strip decoder, where the framework is
all there is (`zencodec-io/benchmarks/driver_overhead_v2_2026-10-14.md`):
the slice driver is within +0.3 to +5.6% of writing the sink directly with
no allocation of its own; the stream pump adds the copy out of `Read`; the
coroutine adds its store copy.

Rules learned on the way, each recorded where it was measured:

- Release consumed input by moving an index, never with `Vec::drain` per
  strip: 1,044 µs against 266 µs at 16-row strips and 7,947 µs against
  310 µs at 1-row strips (`driver_overhead_2026-10-14.md`).
- Size reads in whole requests: a 16-byte header misaligned every 64 KiB
  read and cost +257.7 to +270.3% through the pump until reads were sized
  to whole multiples of `min_len`.
- Keep a hot kernel out of line from its streaming wrapper: QOI's strip
  encode inlined into `push_rows` measured +11.0 to +13.2% against one-shot
  encode, out of line +0.6 to +2.6%.

## 11. Conformance (`zencodec-testkit`)

- `check_incremental_contract`: windows of 1, 7 and 64 bytes and the whole
  file against one-shot pixels; `begin` once and `finish`; final rows once,
  in order; `Done` repeats; unrequested input ignored; cancellation within
  one step, then `Finished`; a provisional sink still receives final rows
  through `provide_next_buffer`; truncation is a codec error, never a
  protocol violation; a step budget turns a spin into a failure.
- `check_incremental_capability_honesty` and
  `check_seekable_capability_honesty`: the capability flags match what the
  job provides, in both directions.
- `check_streaming_output_honesty`: the rules above. A streaming timing
  needs the codec's `streaming_output` and a lag below the height; bytes
  become pending across every lag window; full, uneven and over-sized
  drains leave the file byte-identical; a second `begin` and `encode` after
  `begin` fail; a refused zero-size `begin` leaves the encoder unchanged;
  the result decodes to the one-shot pixels. `AT_FINISH`
  keeps `pending_output` empty. A `RowLevelEncode` rejection at `begin`
  passes. Run it with a streaming configuration as well as the default.
- `check_read_at_contract` for sources.

`check_all` runs every codec check above against an encoder and decoder
pair; `check_read_at_contract` takes a source and runs on its own.

### Coherent input across selected reads

The source/driver must preserve one coherent representation for an opened
operation. Conditional HTTP reads can fail when the object is replaced; an
existing image identity cannot authorize mixing old directory bytes with a new
payload. The [native HTTP qualification](../perf/spatial-validation/README.md#source-identity-is-established-by-the-transport)
executes this through TIFF/SubIFDs, including faults at the last required range.
A positional file handle does not supply a disk snapshot by itself. These
obligations stay in the existing `ReadAt` source contract and its adapters;
codec trait signatures do not gain transport-specific validators.
