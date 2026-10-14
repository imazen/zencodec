# Streaming tier review: PR 146 and the long-term abstraction

Review of [PR 146](https://github.com/imazen/zencodec/pull/146)
(`feat/sansio-coroutine-source`, head `2e8d909d`) and of
`docs/streaming-transcoding-decisions.md`, against main `d116079` (PR 143).
Everything below was checked against the branch source or run on it.
Probe tests were run with `cargo test --features std` and removed.

## Verdict

Do not merge PR 146 in its current shape. The direction is right: a pull
source trait shaped like imij's `ReadAt`, straight-line authoring, and a
resumable core that async, FFI and WASM drivers can all run. The shape is
wrong in three ways that compound:

1. The interface crate now carries **two parallel codec contracts** for the
   same job (`IncrementalDecode` and `SeekableDecode`), each with its own
   dyn trait, shim, job method, set method, capability flag and testkit
   check, plus adapters that bridge them by buffering the whole file.
2. The coroutine engine is an **implementation aid, not an interface**, yet
   it lands in the public API and prelude of the "tiny, stable" crate. It
   also has four confirmed defects (below), is std-only, and copies every
   input chunk and every output row.
3. The decisions doc states unmeasured numbers as facts, links to a source
   file that does not exist, cites two APIs that do not exist, and marks
   unpublished traits that this very PR changes as "Frozen / Stable".

None of this surface is published (crates.io is 0.1.26; PR 143 is also
unpublished), so this is the cheap moment to reshape it.

## Confirmed defects in PR 146

All four reproduced with small probe tests on `2e8d909d`.

| # | Defect | Repro | Effect |
|---|---|---|---|
| 1 | Polling the coroutine after it returned `Err` panics. `poll_coroutine` sets `complete` only on `Ready(Ok)`; the next `push_*`/`pull_*` polls a finished future. | Coroutine returns `Err`; driver calls `pull_batch()` once more. | Panic (`async fn resumed after completion`) in a `forbid(unsafe)` crate. Any driver that retries after an error crashes. |
| 2 | `is_eof` is ignored (`_is_eof`). | `push_chunk(&[0;8], true)` into a coroutine that `read_exact`s 16 bytes. | Status `NeedRanges` for a range past EOF, forever. The coroutine never sees `UnexpectedEof`; a truncated stream hangs the driver loop. |
| 3 | Input retention is "last 32 chunks" with `Vec::remove(0)`. Consumed bytes are never released; early bytes are evicted while still needed. | Push offset 0, then 40 one-byte pushes, then read offset 0. | The header is re-requested; the driver re-fetches bytes it already delivered. Memory is 32 × chunk size with no `ResourceLimits` hook. |
| 4 | Sequential reads surface as `NeedRanges`, never `NeedMoreInput`. | `push_chunk(4 bytes)` into a coroutine that `read_exact`s 8. | `pump_read_to_sink` (main, `helpers/sansio.rs:202`) only continues on `NeedMoreInput`, so a coroutine codec cannot be pumped from a plain `Read`. |

Further issues found by reading, not run:

- `dyn_seekable_decoder(source, …)` takes the source at construction and
  `DynSeekableDecoder::run(source, …)` takes it again. `CodecSet::seekable_decoder`
  consumes the caller's `Box<dyn DynReadAt>` to probe, so the caller has no
  source left to pass to `run`. `SeekableIncrementalAdapter::run_once` ignores
  the constructed source and builds its own. One of the two bindings has to go.
- `SeekableIncrementalAdapter` presents a pull codec as "incremental" by
  spooling the entire input to EOF, then decoding. That is the silent
  whole-file buffer the same PR's doc calls Mistake 6. The one legitimate
  streaming bridge for a pull-native core on a push driver is a thread
  (stackful resumption), which the PR does not offer.
- `drive_ranges` fetches every coalesced range, then stops delivering on the
  first non-`NeedRanges` status and drops the remaining fetched buffers. A
  hand-written codec that prefetches will fetch those bytes twice.
- The coroutine's `yield_batch` copies the slice row by row into a fresh
  `Vec` (allocation per batch), and `pull_batch_into` copies again into the
  sink. The PR text calls this "zero-copy row transfer". `provide_chunk`
  copies every pushed chunk (`chunk.to_vec()`). `Arc<Mutex>` plus
  `Pin<Box<F>>` is not "allocation-free".
- The module is `#![cfg(feature = "std")]` because of `std::sync::Mutex`,
  while its stated purpose is WASM and `no_std` drivers. The poll is only
  ever reached through `&mut self`, so an `Arc` over an `AtomicBool` flag
  cell (safe code, no contention) makes it `no_std + alloc`.
- Cancellation surfaces as `SourceError::Io { "operation cancelled" }` and,
  on the yield path, as a boxed `std::io::Error`. The `enough::StopReason`
  is lost. `impl From<SinkError> for SourceError` exists only so the tests'
  `E = SourceError` satisfies `E: From<SinkError>`.
- Tests re-poll the decoder by pushing an empty chunk at offset 0
  (`push_range(0, &[], false, None)` appears four times). The protocol is
  missing a "step without new input" entry point, and every driver will
  reinvent this hack.
- The tokio test spawns a `JoinSet` over an in-memory `Vec`; it demonstrates
  that the decoder is synchronous, which the type already proves. It does
  not measure or verify "zero blocking threads".

## The abstraction question

### What is on the branch now

Six ways to get pixels out of a codec, four of them for non-slice input:

| Surface | Input binding | Output | Resumable | Origin |
|---|---|---|---|---|
| `Decode` | `Cow<[u8]>` | owned | no | 0.1.26 |
| `StreamingDecode` | `Cow<[u8]>` | pull `PixelSlice` | no | 0.1.26 |
| `push_decoder` | `Cow<[u8]>` | sink | no | 0.1.26 |
| `IncrementalDecode` | push bytes / ranges | pull slice or sink | yes | PR 143 |
| `SeekableDecode` | `DynReadAt` (twice) | sink | no | PR 146 |
| `CoroutineDecoder` | (implements `IncrementalDecode`) | copies | yes | PR 146 |

### What each pairing actually needs

The only real axis is **who holds the stack during I/O**. A pull codec
calls out for bytes and keeps its parse state on its stack; it needs a
source it can call synchronously. A push codec is resumed with bytes; it
keeps its state in a struct (hand-written) or in a compiler-generated
future (coroutine). Everything else is adapters.

| Source | Pull-native core (imij, hayro, image-tiff, rawloader) | Resumable core (zenpng, zenwebp, coroutine) |
|---|---|---|
| slice | direct, zero copy | feed once, zero copy via sink |
| sync `ReadAt` (file, pread, host callback) | **direct call, no overhead** | drive loop: one read per `Need` round, no thread |
| sync `Read` stream | `BufferedReadAt` (bounded, may fail far-back) | pump, bounded |
| async stream / async ranges | thread + channel-backed `ReadAt`, or spool | **native** |
| FFI push (host feeds bytes) | thread or spool | native |
| FFI pull (host `read_at` callback) | direct | drive loop |

Two cells are decisive. A pull-native third-party core on a sync seekable
source must be handed the `ReadAt` directly: any wrapper that makes it
resumable costs either a thread hop per dependency round or a whole-file
spool. And a resumable core on an async or threadless driver is the only
thing that works there at all. So both access styles stay. The question is
only how much *interface* each one needs.

### Recommendation

**One resumable contract, one source trait, one job-level binding for
pull-native cores. Drivers and the coroutine in sibling crates.**

1. **Keep `ReadAt` / `DynReadAt` / `SliceReadAt` / `SourceError` /
   `AccessPattern` in zencodec.** They are small, `no_std`, and the imij
   shape. Drop `is_empty` from the trait (noise) and keep `read_many_at`.

2. **Replace the `SeekableDecode` trait family with one `DecodeJob`
   method.** `SeekableDecode::run(source, sink, stop)` is `push_decoder`
   with the data binding changed from slice to source. Expressing it that
   way reuses an established shape (consume the job, write into the sink,
   return `OutputInfo`), has one source binding, and needs no new dyn
   trait, shim, or set wrapper:

   ```rust
   // DecodeJob<'a>, default Err(UnsupportedOperation::SeekableDecode)
   fn push_decoder_from_source(
       self,
       source: &mut dyn DynReadAt,
       sink: &mut dyn DecodeRowSink,
       preferred: &[PixelDescriptor],
   ) -> Result<OutputInfo, Self::Error>;
   ```

   `DynDecodeJob` gets the same method; `CodecSet` gets
   `push_decode_from_source` that probes offset 0 and dispatches. The
   `seekable` capability flag and `UnsupportedOperation::SeekableDecode`
   stay. A `StreamingDecode` form over a source can follow when a consumer
   asks for it; nothing in the proxy or FFI use cases needs it now.

3. **Reshape `IncrementalDecode` into a sans-I/O step machine before it
   publishes.** PR 143's trait has four push methods, three pull methods, a
   `requested_ranges()` side channel read after a status, `BlockedOnOutput`
   with partial consumption, and no way to advance without pushing. The
   minimal complete form is feed + step, output through the sink:

   ```rust
   pub trait IncrementalDecode {
       type Error: core::error::Error + Send + Sync + 'static;

       /// Offer `bytes` located at `offset`. Returns how many were accepted;
       /// re-offer the rest after `step`. `eof` means `offset + len` is the
       /// total length (an empty chunk with `eof` announces the length).
       fn feed(&mut self, offset: u64, bytes: &[u8], eof: bool)
           -> Result<usize, Self::Error>;

       /// Advance as far as fed bytes allow, writing finished rows into `sink`.
       fn step(&mut self, sink: &mut dyn DecodeRowSink, stop: Option<&dyn Stop>)
           -> Result<Step<'_>, Self::Error>;

       fn info(&self) -> Option<&ImageInfo>;
       fn output_descriptor(&self) -> Option<PixelDescriptor>;
       fn access_pattern(&self) -> AccessPattern;
   }

   #[non_exhaustive]
   pub enum Step<'a> {
       /// Needs these bytes. A sequential decoder names the cursor with
       /// `length: 0`, meaning "any amount from here".
       Need(&'a [ByteRange]),
       /// Needs the total length (containers that scan from the end).
       NeedLen,
       /// Wrote rows `meta.y .. meta.y + rows` into the sink. Call again.
       Rows { meta: BatchMeta, rows: u32 },
       /// Header, color or supplementary metadata became available. Call again.
       Metadata(MetadataUpdate),
       Done(OutputInfo),
   }
   ```

   What this removes: `push_chunk`, `push_chunk_pulse`, `push_range`,
   `push_range_pulse` (one `feed`; `Pulse: Stop` already covers the pulse
   forms), `requested_ranges`, `pull_batch`, `pull_batch_meta`,
   `pull_batch_into`, `is_complete`, `PushStatus`, `PushOutcome`,
   `BlockedOnOutput`, the empty-push re-poll hack, and the internal pending
   batch queue. Backpressure is "don't call `step` until the sink has room";
   a single-threaded driver does nothing between steps, a worker thread
   blocks in the sink. The codec writes decoded rows straight into
   `sink.provide_next_buffer`, so hand-written and pull-native cores are
   zero-copy; the coroutine pays one copy (its owned strip into the sink)
   and says so. Bounded input memory is honest through partial `feed`
   acceptance. The pull-slice shape (`StreamingDecode::next_batch`) is a
   `CollectSink` away for callers that want it.

   `MetadataUpdate`, `BatchMeta`, `ByteRange`, `AccessPattern` stay as they
   are. `DynIncrementalDecoder` becomes the same five methods over
   `BoxedError`.

4. **Move drivers and the coroutine out of the interface crate**, as
   workspace members next to `zencodec-testkit`:
   - `zencodec-io` (std): `FileReadAt`, `BufferedReadAt` + `BufferPolicy`,
     `drive_from_read_at`, `pump_read_to_sink`, `pump_async_read_to_sink`,
     `ProvisionalFinalizer`, and a `ThreadedPullAdapter` that runs a
     pull-native core on its own thread with a channel-backed `ReadAt`,
     exposing `IncrementalDecode` with bounded memory. `coalesce_byte_ranges`
     goes with the drivers. The spool-to-EOF adapter is deleted, or kept
     only under a name that says `Spool`.
   - `zencodec-coroutine` (`no_std + alloc`): `CoroutineContext`,
     `CoroutineDecoder` implementing the reshaped `IncrementalDecode`, with
     the defects above fixed: terminal state on `Err`, `UnexpectedEof` when
     a request runs past the announced length, a cursor-based input buffer
     that releases consumed bytes and honours a byte cap, `Need` with
     `length: 0` for sequential reads, typed cancellation carrying the
     `StopReason`, and an owned strip buffer handed back to the codec after
     each `Rows` step so steady state allocates nothing.

   Why out of zencodec: the interface crate's public surface is what
   `cargo semver-checks` guards and every codec pins; drivers and the
   coroutine change faster than traits, pull `std` and possibly a lock
   crate, and are chosen per host, not per codec. `helpers/sansio.rs` on
   main is unpublished and moves too.

5. **Codec side.** zenpng, zenwebp and zenbitmaps already implement PR 143's
   trait on unpublished main; they would be reworked to feed/step (zenpng's
   `PngIncrementalDecoder` is the template: a row buffer and a cursor). New
   container codecs (zentiff, imij bridge, zenavif-parse) implement
   `push_decoder_from_source` natively and get the resumable form through
   `zencodec-coroutine` when a push driver needs it.

### What this gives each driver

- **imageflow / C# FFI:** a `read_at` host callback is a `DynReadAt`;
  decode into a pinned managed buffer through `DecodeRowSink` with
  `push_decoder_from_source`. For push-style hosts, `feed` + `step`.
- **Proxy (tokio):** the bounded-channel worker from the decisions doc. On
  the worker, either `push_decoder_from_source` over a channel-backed
  `ReadAt`, or `feed`/`step` fed from the channel. No coroutine needed on
  the server path.
- **WASM, single thread:** `feed` bytes from a `fetch` stream, `step` into a
  canvas-backed sink. Only resumable cores run here; the coroutine crate is
  how a straight-line codec becomes one.
- **zenpipe:** the ring buffer is the sink; `Step::Rows` is the strip event.

## Corrections to `streaming-transcoding-decisions.md`

The doc mixes a useful design record with unverified claims. Keep sections
1 and 4 as a short rationale inside `range-decoding-contract.md`; delete or
rewrite the rest.

- Four links to `../src/incremental.rs`: the file does not exist. The trait
  is in `src/traits/decoder.rs`.
- `pulse.fraction()` (section 3): no such method in how_far 0.1.0.
  `StopTrigger` (section 2): no such type in enough or almost_enough
  (`Stopper` and `FnStop` exist). `StopReason::Cancelled` is real.
- Numbers stated as facts with no measurement behind them: 2 to 5 µs per
  thread hop, 512 threads, "< 200 microseconds" abort latency, "~50 to
  150 µs" per strip, "> 100 GB" for 1,000 requests, "~1 to 2 MB per request".
  Either measure them in a committed benchmark or remove them.
- "Working memory is bounded to (2R + 1) scanlines" holds only when the
  encoder streams rows. The AV1 encoders take whole frames, so the strip
  pipeline cannot keep that bound for AVIF output, and the deadline
  section's "switch encoder" strategy changes the memory model, not just
  the time. Say which encoders stream.
- "Allocation-free state machine" and "zero-copy row transfer": false on
  this branch, see the defect list.
- "Firezone noted in their evaluation" is cited twice without a link. Add
  the URL or drop the attribution.
- The contract table marks `IncrementalDecode` and `ReadAt`/`SeekableDecode`
  "Frozen / Stable". Both are unpublished and this PR adds
  `access_pattern` to one of them.
- Section 5 ("Mistakes Avoided") narrates the last two days of sessions as
  peer-reviewed history. The constraints it encodes are worth one line each
  in the contract doc: no whole-file inventory before decode, no async in
  the codec core, no compute on the async runtime, bounded buffering with
  explicit failure, no whole-image materialisation, early abort.

## Landing sequence

1. Land a reshaped PR 143 surface (`feed`/`step`, `Step`) on main while it
   is unpublished; update zenpng, zenwebp, zenbitmaps in their repos.
2. Land `ReadAt` + `push_decoder_from_source` + `seekable` capability +
   testkit `check_read_at_contract` and a capability-honesty check against
   the new method.
3. Create `zencodec-io` and `zencodec-coroutine` as workspace members, with
   the defect fixes as named tests (poll after error, EOF, eviction,
   sequential `Need`).
4. Fold the usable parts of `streaming-transcoding-decisions.md` into
   `range-decoding-contract.md`; delete the rest.
5. Pilot: imij bridge through `push_decoder_from_source`; zenpng through
   `feed`/`step`; one coroutine codec (the MiniTile fixture is a fine first
   one) driven by all three driver shapes in `zencodec-io` tests.
