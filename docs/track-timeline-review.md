# Complete track timelines — review draft

This branch implements shared storage and mux admission. Native full edit-list
parsing/remuxing and the presentation driver are not implemented by this change.
Do not merge or publish it as full edited-video support before those gates pass.

## Consumer: ordinary remuxing keeps one entry point

```rust
let prepared = PreparedOutput::preserving_presentation(mux, destination, &tracks, None)?;
let mut output = prepared.publish(None)?;
// Send the source packets through output.send(packet, None), then finish.
```

Demuxers supply `TrackInfo::timeline()`. The explicit preservation constructor
rejects unknown evidence before native preparation. For convenient legacy
operation, `PreparedOutput::new` keeps its existing all-unknown packet-timing
path; that path does not certify the full presentation. A known timeline automatically requires
`Muxer::begin_exact_presentation` before publication. Callers do not first query
a capability and coordinate a second potentially different preparation. A
native muxer either binds the actual request or refuses with
`UnsupportedMedia::PresentationTimeline`.

All-unknown legacy tracks retain their existing packet-timing path; this is
explicitly not proof of presentation preservation. Mixing a known declaration
with unknown track facts refuses before native preparation. The opt-in ordinary
container-timing path accepts all-`Unedited` tracks, without certifying exact
timing. It still refuses edited programs and mixed known/unknown tracks.

## Authoring an ordered program

```rust
use std::sync::Arc;
use zencodec::time::{FrameDuration, TimeBase, Timestamp};
use zencodec_media::timeline::{
    EditContent, EditExtent, EditList, EditSegment, PlaybackRate, TrackTimeline,
};

let origin = Timestamp::new(0, TimeBase::MILLIS);
let program = EditList::new(origin, vec![
    EditSegment::new(
        EditExtent::Finite(FrameDuration::new(1, 2)?),
        EditContent::Gap,
    ),
    EditSegment::new(
        EditExtent::ToEnd,
        EditContent::Media {
            start: Timestamp::new(9000, TimeBase::hz(90_000)?),
            rate: PlaybackRate::NORMAL,
        },
    ),
])?;
let timeline = TrackTimeline::Edited(Arc::new(program));
// Build the output track with builder.with_timeline(timeline).build()? .
```

The half-second gap does not choose black or silence. Rendering needs an
explicit fill/omit/refuse decision. A remuxer can preserve the gap without
rendering it. `ToEnd` preserves future fragments without inventing a duration.
Finite zero duration remains distinct. An empty list means no presentation;
`Unedited` means the inspected container has no edit program; `Unknown` is not
an identity claim.

Rate zero retains a dwell and negative rates retain reverse playback. A route
may preserve these declarations while refusing to execute them. Rate fractions
are reduced exactly, including the signed minimum; no floating point is used.
Each media start carries its own clock. Segment duration and presentation origin
are independent of those clocks.

## Implementor: bind the retained muxer

```rust
fn begin_exact_presentation(
    &mut self,
    tracks: &[TrackInfo],
    hint: MuxHint,
) -> Result<MuxTiming, SessionError<Self::Error>>;
```

The provided implementation refuses without calling legacy `begin`. An override
must validate all declared programs, priming and pre-roll before exposing any
header. Success also binds exact packet timing. Later packet discard or timing
facts must be preserved or refused before affecting pending output. The erasure
shim forwards this call; a handwritten dyn implementation also defaults to
refusal.

This is deliberately stronger than `begin_exact_timing`, which certifies packet
clocks/durations but not the container's full edit program. The MP4, WebM and IVF
implementations require their own positive and refusal controls before adopting
this hook. A successful synthetic dispatch test is not that qualification.

## Ownership and migration

`EditList::new` moves the caller's vector without allocating or cloning it.
`TrackTimeline::Edited` shares an immutable list; cloning a track shares the
same program. This is not a whole-operation memory bound. Admit descriptor
storage before a parser constructs an untrusted list.

The legacy scalar getter/setter are deprecated immediately. Supplying both a
known full timeline and a scalar fails at `build`, regardless of setter order;
zero is not treated as permission to discard one declaration. Legacy-only
builders leave the complete timeline unknown.

No zenpixels changes are required for these declarations. Native frames keep
their pixel owners. The already-qualified private cursor maps temporal intervals
without changing pixel samples, but it is not promoted into this API. Delayed
frames and source replay must retain their original map; codec drain alone is
not a map-retirement proof. A presentation-only change needs a distinct immutable
identity even when the codec settings are unchanged.

## Gates before landing

- Native MP4 parse/remux/parse preserves ordered edits, rates, independent
  clocks, explicit empty programs and fragmented open ends.
- Unknown and unsupported native records refuse without a false identity claim.
- Native mux admission binds a retained plan before header bytes; typed/erased
  paths preserve causes and no-effect refusal. Direct native begin paths must
  not silently ignore the new field.
- Independent playback comparison covers repeats, gaps, dwell, clipping and
  final duration. Native cursor tests alone do not qualify a container player.
- Bounded map ownership, replay admission, late discovery and retirement rules
  work with Pulse and operation policy. Optional evidence remains permissive;
  an explicit preservation requirement cannot turn unknown into success.
- Regenerate compiler catalogs and check the complete portable consumer graph.

## Native adoption status

[MP4 adoption PR79](https://github.com/imazen/zenavif/pull/79) now retains
and writes full edit programs and binds exact presentation before headers.
Its portable core `41b1357e` graph passes 38 native tests, strict all-target
Clippy and no-default WASM. Raw-record controls cover independent clocks,
ordered edits, empty/unedited/open distinctions and refusal before output.
These packet fixtures do not establish independent decoded playback.

The deprecated scalar getter derives the old first-media-start minus leading
gaps projection for compatibility. It omits later edits and rates; new
applications must inspect `timeline()`. Native writing uses the complete
program. WebM/IVF adoption, independent playback and the ownership/policy
gates above remain required; this review is still draft.

[IVF/WebM adoption PR56](https://github.com/imazen/zenextras/pull/56) closes
silent edit loss in direct writers. Both refuse full edit programs before
output, including an empty presentation and zero-start dwell; both controls
fail against the original writers. All 29 container tests, strict Clippy and
WASM pass in the isolated consumer graph. IVF reports unedited input; WebM
retains unknown until it can establish presentation completeness. Neither
yet promises exact presentation preservation. This distinction must remain
visible to ordinary callers as well as strict preservation callers.

## Explicitly accept native timing for an unedited stream

```rust,ignore
let mut output = Output::with_container_timing(mux, Forward(writer), &tracks, None)?;
for packet in packets {
    output.send(packet, None)?;
}
output.finish(None)?;
```

This opt-in remains usable when a truthful demuxer reports every track as
`Unedited`. It accepts the native container's timing rules; it does not promise
exact packet durations or full presentation preservation. Edited programs and
mixed known/unknown timelines still refuse. Use `preserving_presentation` when
that guarantee is required: it never silently falls back to the above route.
The real IVF remux control lives in
`perf/io-validation/tests/presentation_routes.rs`; packet bytes and timestamps
are checked separately from any decoded-video claims.

## Independent playback finding

An executing alternative now has a native witness: decode AV1, project frame
intervals, re-encode, and mux an explicitly unedited track. All twelve independent
pixel/timing comparisons pass, including fragmented clipping, repeats, explicit
black gaps, dwell and double speed. See
[native execution evidence](../benchmarks/presentation_execution_2026-10-30.md).
The fixture retains three decoded frames; it does not qualify a streaming driver,
replay admission, operation-wide memory bounds or cancellation during execution.

The [12-case native MP4 comparison](../benchmarks/presentation_playback_2026-10-30.md)
passes 5 cases and disagrees with FFmpeg 8.0.1 in 7. Progressive identity, clipping,
repetition and gaps agree; fragmented identity agrees. Fragmented edits, dwell
and speed changes are not independently qualified. The oracle's source explains
limitations in those paths, but this does not qualify universal playback.

Before freezing, separate retaining edit declarations, executing them into
output samples/timestamps, and meeting a selected playback target's constraints.
Keep the failing controls intact. The current remux hook must not be treated as
a certificate of arbitrary-player execution.

## Compressed replay and retirement

A second executing witness now reacquires each media segment from retained
compressed packets and feeds decoded frames directly to the encoder. It avoids
a driver-owned decoded-frame vector and proves picture cleanup after native
owners are dropped. All twelve output files match the buffered route byte for
byte and pass the independent playback comparison.

This separates presentation completion from source completion: a finite clip
can have complete frame coverage before EOF, while privacy or late metadata
inspection may still require reading further. The private cursor exposes only
coverage; it does not discharge those obligations or authorize replay storage.
A future source adapter must bind replay admission and source identity before
acquisition, then preserve pending inspection requirements through early clip
completion. File/network reacquisition, keyframe seeking and operation-wide
workspace evidence are still open. See
[replay qualification](../benchmarks/presentation_replay_2026-10-30.md).

### Source identity without another codec trait

The native replay source controls use existing `ReadAt`, `StorageBudget`,
`SharedStorage` and accounted packets. The source must maintain one coherent
representation during snapshot creation; the frozen owner then preserves that
representation across decoder instances. Creating a fresh source and accepting
its new validator on every pass would not preserve the original operation.

Known-length snapshot admission precedes body reads. Conditional HTTP failures
retain the original source cause, and cancellation releases partial storage.
File reads require caller-owned stable bytes during the copy: positional reads
alone do not detect concurrent writes. After freezing, replacing the original
file does not affect replay. Cloned packet views retain the snapshot's entire
storage charge until the last owner is dropped.

The native controls pass for stable files and HTTP, replacement and malformed
responses, cap refusal and cancellation. This qualifies a full compressed
snapshot route, not a general seek/reopen implementation or a whole-codec memory
bound. See [source-admission evidence](../benchmarks/replay_sources_2026-10-30.md).

For unknown-length sequential input, existing APIs also compose explicitly:

```rust,ignore
let mut source = BufferedReadAt::new(reader, BufferPolicy::Spool { max_bytes });
let length = source.len()?; // Consume to confirmed EOF under the spool cap.
```

The native source controls then copy the completed spool into admitted immutable
storage. Exact-cap EOF succeeds; one byte over refuses after a single-byte EOF
probe. Both native replay and original typed reader errors are preserved.
This route temporarily retains the spool and snapshot payloads together. It is
not a zero-copy owner transfer or a shared whole-operation memory allowance.
The reader must provide cancellation while reading; `len()` cannot interrupt a
stalled synchronous read on its own.
