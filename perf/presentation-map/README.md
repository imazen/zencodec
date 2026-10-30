# Presentation map experiment

Private executable design for issue 187, not a published API or a playback
implementation. A caller first admits the complete edit plan, then acquires
frames per segment and projects their exact presentation intervals:

```rust,ignore
let plan = Plan::new(movie_origin, &edits)?;
for segment in plan.segments() {
    let acquisition = segment.acquisition(); // Gap, FrameAt, Span, or From
    // A gap needs the application's explicit visual/audio treatment.
    // A repeated segment needs a source seek/replay, not an unbounded cache.
    let visible = segment.project(decoded_frame_interval)?;
    // Keep rational time, or explicitly request an exactly representable clock.
}
```

Native decoding, frame acquisition, cancellation, policy decisions, codec delay,
seek pre-roll, audio resampling, output publication, and resource admission are
not implemented by this experiment. In particular it does not make the current
`VideoAnimationDecoder` obey edits. These are integration requirements before
promotion, not optional finishing work.

The plan retains segment order, explicit extent, media start, rational rate and
explicit gaps. The experiment also models a zero-rate dwell independently of
what a particular container permits. It refuses reverse playback, invalid
intervals and unrepresentable arithmetic instead of silently rounding. A frame
crossing an edit boundary is clipped in time; its pixel buffer is unchanged.
A repeated source frame can map to distinct output intervals. Callers process
segments in order; projecting an input stream once is insufficient for repeats.

The unit oracle covers Apple's published repeated-segment example, clipping,
negative origins, gaps, dwell boundaries, fractional rates, destination-clock
refusal and an integer-grid rational oracle. Apple's example defines movie and
media clocks separately and demonstrates repeated media and speed changes:
[Playing with edit lists](https://developer.apple.com/documentation/quicktime-file-format/playing_with_edit_lists).
This is not an ISO conformance claim or an independent player comparison.

Run `just presentation-map-check` from the repository root in an isolated export
with the checked dependency lock. Nine tests and strict all-target Clippy pass;
the [receipt](../../benchmarks/presentation_map_2026-10-30.md) records scope.
Remaining design questions include native unsupported edit
records, presentation-map epochs, decoder reordering, duration discovery,
per-segment seek/replay admission, and avoiding double application of trim/delay.

An empty edit slice in this experiment means an empty presentation; it must not
be used to stand for an absent container edit list. A production contract needs
an explicit identity route for that case. Unsupported reverse rates remain
available in the input description for remuxing, but cannot be admitted for
execution. A failed plan produces no native calls or output; it can allocate
edit descriptors before refusal, so a strict workspace policy needs admission
before construction. `project` itself allocates nothing.

## Open-ended media

`Extent::Remaining` keeps future media open without guessing the final duration;
`Acquisition::From(start)` tells the driver to acquire from that source time
through its eventual end. It must be the final segment. Open-ended gaps and
dwells refuse because neither supplies a media endpoint that can determine
their presentation length. Finite zero duration remains distinct and rejected
by this experiment.

This distinction is required for fragmented MP4: the W3C byte-stream profile
permits a single rate-one edit with duration zero to cover subsequent media.
The native parser must lower that form to `Remaining`, not `Finite(0)`.
[W3C ISO BMFF byte-stream format, initialization segments](https://w3c.github.io/mse-byte-stream-format-isobmff/#initialization-segments).
The open-ended extension is separately qualified in the same receipt; the
earlier seven-test run covers only finite segments.

## Promotion must include both directions

A new track field alone cannot establish edited playback or faithful remuxing.
The current `CodecConfig::from` intentionally copies codec configuration facts,
while the animation adapter does not retain track presentation facts. The
presentation owner must keep the full map, associate delayed frames with its
correct epoch, and apply it once after native decoding. Copying timing fields
into a codec config without defining who consumes them risks double trimming.

Likewise, an existing muxer can ignore a newly added field. Its successful
packet-timing admission is not proof that it retained an edit program. Promotion
requires checked container admission that either binds retention of the exact
program or refuses before publishing a header. Identity tracks should keep the
simple path. Missing/unknown discovery facts must not silently become identity;
declared empty presentations must not silently become an unedited whole stream.

Required native controls include parse/remux/parse preservation of ordered
segments and open ends, independent player comparison, leading gaps, repeated
source ranges, source seek/refusal before affected output, late configuration
changes with delayed frames, final frame duration, and codec priming/discard.
Preserving these data without executing them remains useful for remuxing, but
must not be reported as a successful edited render.

## Execution coverage cursor

`Cursor::new(segment)` retains only the segment and its next output time. Pass
ordered decoded frame intervals to `push`; each returned interval belongs to
that exact frame's unchanged pixels. `finish` requires complete finite coverage,
or derives an open segment's endpoint from observed frames. A missing span,
overlap, arithmetic error or premature completion poisons the cursor. Further
calls refuse. Already delivered frames remain delivered; this is not rollback.

A gap returns `GapRequiresDecision`. The application must choose and admit a
fill, omission or refusal rather than receive an invented black frame. This
experiment has no policy callback yet. Repeated edits need separate cursors and
source acquisition; cursor state never caches frames. A caller must distinguish
confirmed source EOF from temporary starvation, and must supply reliable frame
durations, especially for the final frame. Neither the cursor nor native PTS
alone can establish that endpoint.

`just presentation-native-check` exercises the cursor against real lossless AV1
frames: double-rate clipping, replay of the first frame, dwell on the second,
and a deliberately missing middle frame. The fixture retains all three decoded
frames and supplies their known 40 ms durations. It is not a streaming-memory
measurement, MP4 edit parser, replay-source implementation or independent player
oracle. Native acquisition, epoch ownership and publication remain required
before promotion to the shared API.


## Retaining maps across decoder configurations

The native epoch control uses six immutable maps while AV1 configuration changes
shape, color, sample depth and clock. The first two configurations deliberately
have identical codec settings but different maps. It observes the configuration
active when each frame is returned and requires actual delayed old-epoch frames.
Each frame selects its original map using its retained configuration epoch;
selecting the latest map instead loses earlier presentations. Source packet
ordinals deliberately repeat, so they cannot substitute for map identity.

The private `retained::PlanPool` now admits a maximum count before invoking a
plan builder. `RetainedCursor` holds a cloned immutable owner; replay holds the
same owner independently. The native control drops the original lookup list,
finishes all cursors, and verifies that the replay reference still occupies its
slot. Only the last owner dropping frees that slot. This is not a public map
registry or a byte/workspace limit. Production still needs descriptor-byte
admission and an explicit rule for presentation-only changes. A codec epoch must not be
silently reused for a different map while frames from its previous meaning may
still be pending. Unknown epochs must refuse before publishing that frame.
Source replay can return an old epoch again, so draining the codec alone does
not prove that the corresponding map can be forgotten for the whole operation.

`just presentation-epoch-check` passes with both thread configurations, along
with native harness strict Clippy. See the same qualification receipt for the
frozen source and resource record. This does not qualify mux preservation or
presentation-only epoch changes that the adapter has not explicitly declared.

## Native compressed replay witness

`just presentation-replay FILES EXPECTED_RAW RESULTS` runs the same twelve native
AV1/MP4 cases without retaining a decoded-frame vector in the execution driver.
Each media segment reacquires a fresh native decoder over an explicitly retained
compressed packet snapshot. A decoded frame goes directly to the encoder; its
projected presentation shares the same picture. Weak-reference controls require
all decoded picture owners to disappear after native session cleanup.

`Cursor::coverage_complete` permits finite presentation completion before source
EOF. It is not evidence that metadata inspection is complete. Only this generated
fixture's known source contract permits early decoder disposal; a production
route must discharge pending inspection/privacy obligations separately. Open-ended
segments still need a confirmed endpoint.

All twelve replay outputs are byte-identical to the buffered route and pass the
independent pixel/timing oracle. This is ownership and behavior evidence, not a
whole-operation RAM bound: native references, encoder storage, compressed input
and compressed output remain outside a decoded-frame-vector guarantee. Reopening
network/file sources, replay admission, seek-to-keyframe and asynchronous scheduling
remain unqualified. See [the receipt](../../benchmarks/presentation_replay_2026-10-30.md).

The companion `just presentation-replay-sources ARTIFACTS PORT` reuses the real
local HTTP fault simulator and its strict conditional `ReadAt` implementation.
It validates an admitted immutable compressed snapshot before constructing native
replay packets. File/HTTP parity, budget ownership, typed source failures and
cancellation controls are described in
[the source receipt](../../benchmarks/replay_sources_2026-10-30.md).

Unknown-length streams are also qualified through the existing capped spool:
confirm EOF, copy into admitted immutable storage, drop the spool, then replay.
The test retains the exact-cap/one-byte-over behavior and original timeout cause.
There is a real temporary overlap between spool payload and snapshot payload;
a future owner-transfer convenience must preserve admission rather than conceal
that cost. No zero-copy or whole-memory claim is made for this composition.

The source witness also qualifies eager reservation directly into OwnedStorage,
followed by freeze and a prefix view. It avoids the spool/copy overlap without
new public types, and verifies pointer preservation and retained full-cap charging.
Its tradeoff is initializing/reserving the chosen maximum before reading. The
private loop handles exact EOF, overflow, zero-cap input, failed admission and
cancellation between Interrupted retries. This is an alternative route to expose
to an operation policy, not an implicit promise that all replay uses little RAM.

### Bind policy before replay acquisition

The native replay-source test also exercises this existing boundary:

```rust,ignore
let decoder = PolicyDecoder::try_new(
    || { /* admit and freeze source storage, then construct native decoder */ },
    BoundPolicy::new(requirements, callback),
    pulse,
)?;
```

Unknown required workspace refuses before the factory runs. Default requirements
permit native decoding; the same bound requirements still filter every frame
before the callback. Frame facts cannot prove descriptive metadata removal, so
that requirement refuses publication rather than disappearing after acquisition.
Snapshot ownership remains with the caller in this witness. The workspace limit
is additional codec/conversion workspace, not source storage or total RAM.
Choosing among acquisition strategies remains an open contract; this factory
boundary does not offer such choices. See the retained-policy section of
[the source receipt](../../benchmarks/replay_sources_2026-10-30.md).


### Map lifetime prototype

```rust,ignore
let mut pool = PlanPool::new(max_retained_maps);
let map = pool.prepare(|| Plan::new(origin, &edits))?;
let replay_owner = map.clone();
let mut cursor = map.cursor(segment)?;
drop(map); // Cursor and replay still retain this immutable map.
```

Admission refuses before the builder runs, so a full pool cannot construct the
next descriptor list. Clones share one slot, and the pool itself holds only weak
references. Dropping the pool does not invalidate existing owners. Failed plan
construction leaves no occupied slot. Arc control-block allocation follows the
ordinary allocator; descriptor bytes and bookkeeping capacity are not included
in the count. A raw copied Segment is not an owned cursor and carries no lease.

This prototype changes no released codec trait or frame representation. The
native test still uses configuration epochs to select its six known maps; it
does not establish how a demuxer announces a presentation-only change. See
[qualification and remaining scope](../../benchmarks/presentation_owners_2026-10-30.md).


### Presentation identity can belong to the session

The second native owner control binds one immutable cursor to each decoder
session. Two sessions receive identical AV1 bytes, CodecConfig (epoch zero),
packet ordinal and source offset. Their maps have different presentation origins.
Input is interleaved, output drains in reverse session order, and each decoded
frame is projected only by the cursor retained beside its actual decoder.
Both native thread configurations preserve exact samples and picture sharing.

For replay or an independently started presentation, this avoids adding a map ID
to every codec: `Pass { decoder, cursor }` is the identity boundary. A map stays
immutable for that pass. Codec configuration epochs still describe codec facts;
packet ordinals and offsets still describe source provenance. They are not
repurposed as presentation IDs.

This does not authorize changing the cursor while that decoder has delayed output.
A live presentation-only update still needs a drain/new-session boundary or an
explicit, qualified native association protocol. New-session replay must start
from suitable source state and satisfy source/native resource admission. The
control retains both native contexts and output frames; it makes no low-memory
claim. See the [owner receipt](../../benchmarks/presentation_owners_2026-10-30.md).


### Drive native progress, then filter presentation

The reusable private `av1-policy-validation/src/presentation_driver.rs` owns a
native decoder and RetainedCursor for one segment:

```rust,ignore
let mut pass = Pass::new(admitted_configured_decoder, map.cursor(segment)?, output_clock);
for mut packet in source_packets {
    pass.send(&mut packet, pulse, |frame| consume(frame))?;
}
pass.finish(pulse, |frame| consume(frame))?;
```

It uses the existing native send/drain pumps, projecting frames inside their
output callback. This is deliberately not a VideoDecoder wrapper: discarding
frames for a clip must not turn a native ReceivePending promise into a caller
receive that produces no output. Native progress and visible presentation output
are distinct. No driver frame queue or pixel copy is introduced.

Every operation temporarily takes ownership of its native decoder and cursor.
Errors and unwinding drop both and leave the pass terminal. Completion also
retires them. Original native and downstream errors retain their typed causes;
projection failure is typed, and Pulse is checked by the existing pumps before
and after raw-frame callbacks, including skipped frames. Completion requires
native EOF and cursor coverage. Earlier delivered frames are not rolled back if
later coverage fails. Metadata completeness still requires separate evidence.

Native controls cover an initial skipped frame, exact clipped pixels/timing,
incomplete coverage, typed BrokenPipe winning over simultaneous timeout,
pre-cancellation, cancellation after one publication, owner release and terminal
re-entry. Admission of decoder construction remains outside this constructor;
full multi-segment orchestration, gap policy and final interpretation policy
remain integration work. The driver is private and is not a released API.
