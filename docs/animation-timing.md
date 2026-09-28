# Exact animation timing

`animation::FrameDuration` carries a reduced, nonnegative rational number of
seconds. It preserves source timing: zero, fractions of a millisecond and large
native JXL durations. It applies no player minimum or estimated frame rate.
`AnimationFrame::duration()` and `OwnedAnimationFrame::duration()` carry that
value through borrowing and ownership changes. The legacy `duration_ms()`
accessor truncates fractional milliseconds and saturates to `u32::MAX`.

An exact transcode calls `AnimationFrameEncoder::push_frame_timed()` (also
available through `DynAnimationFrameEncoder`). An implementation must either
encode the supplied duration exactly or reject it before accepting the frame.
The default rejects with `UnsupportedOperation::AnimationTiming`, even for
integer milliseconds: older `push_frame()` implementations may round, clamp or
narrow their native timing fields. Do not fall back to that method after an
exact-timing rejection.

Native adapters validate the actual destination field: GIF has a 16-bit count
of centiseconds; WebP has a 24-bit count of milliseconds; APNG has a 16-bit
numerator and denominator (encoded denominator zero means 100); AVIF has a
track clock and 32-bit sample durations; JXL has a rational tick clock and
32-bit frame durations. AVIF/JXL may choose a common clock only when all
rescaled durations and the clock fit their native fields.

Quantization is a caller policy. A media pipeline can reject an unrepresentable
duration, or explicitly choose a destination clock and quantize cumulative
presentation endpoints. Rounding each duration independently causes drift.
Zero-delay playback clamps, loop repetition, compositing-helper frames and
end-of-stream hold behavior are separate decisions. This type does not resolve
them by changing the source duration.

This additive bridge is suitable for the remaining 0.1 release and the next
breaking zencodec release. A future breaking API can make the exact duration
the only timing input; existing image traits need not host the video transport,
seek index or network I/O abstraction to carry accurate animation timing.
