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

This is a bounded fixture with six retained plans, not an implemented public map
registry. Production needs an admission limit, a reliable retirement boundary,
and an explicit rule for presentation-only changes. A codec epoch must not be
silently reused for a different map while frames from its previous meaning may
still be pending. Unknown epochs must refuse before publishing that frame.
Source replay can return an old epoch again, so draining the codec alone does
not prove that the corresponding map can be forgotten for the whole operation.

`just presentation-epoch-check` passes with both thread configurations, along
with native harness strict Clippy. See the same qualification receipt for the
frozen source and resource record. This does not qualify mux preservation or
presentation-only epoch changes that the adapter has not explicitly declared.
