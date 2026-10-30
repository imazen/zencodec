# Presentation map experiment

Private executable design for issue 187, not a published API or a playback
implementation. A caller first admits the complete edit plan, then acquires
frames per segment and projects their exact presentation intervals:

```rust,ignore
let plan = Plan::new(movie_origin, &edits)?;
for segment in plan.segments() {
    let acquisition = segment.acquisition(); // Gap, FrameAt, or Span
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

The plan retains segment order, finite duration, media start, rational rate and
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
with the checked dependency lock. Seven tests and strict all-target Clippy pass;
the [receipt](../../benchmarks/presentation_map_2026-10-30.md) records scope.
Remaining design questions include open-ended streams, native unsupported edit
records, presentation-map epochs, decoder reordering, duration discovery,
per-segment seek/replay admission, and avoiding double application of trim/delay.

An empty edit slice in this experiment means an empty presentation; it must not
be used to stand for an absent container edit list. A production contract needs
an explicit identity route for that case. Unsupported reverse rates remain
available in the input description for remuxing, but cannot be admitted for
execution. A failed plan produces no native calls or output; it can allocate
edit descriptors before refusal, so a strict workspace policy needs admission
before construction. `project` itself allocates nothing.
