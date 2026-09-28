# Encoded input and output requirements

The existing zencodec image decode job still accepts a complete `Cow<[u8]>`.
That API does not become incremental merely because its caller reads a socket.
Use `source::read_bounded` when a whole encoded image fits the chosen memory
budget. It uses fallible allocation, reads at most the budget plus one detection
byte, preserves I/O failures and polls cancellation between reads.

Use `SeekableSource::read_from` for a nonseekable input that needs seeking. It
produces an immutable completed snapshot, keeps at most the configured payload
in memory, and spills to an unnamed temporary file in an explicit directory.
The directory must be on suitable disk storage. The total-byte budget is
independent of the memory threshold. Errors discard the partial snapshot and
the OS removes temporary storage when its handle closes. `into_bytes` requires
another explicit memory budget for byte-array-only codecs.

For immutable HTTP objects, `HttpRangeReader` provides bounded seeking without
first downloading the entire object, using the strong validator and response
checks documented in `SEEKING.md`. If the server does not satisfy that contract,
the caller can explicitly choose a bounded full snapshot. There is no implicit
fallback from a failed range request to an unbounded download.

AV1 packet decoding and IVF output are incremental. `Av1IvfEncoder<W: Write>`
emits packets before finalization and does not require seeking. Existing image
animation adapters retain their native buffering requirements: WebP's RIFF
length and AVIF's sample tables cannot be streamed to arbitrary `Write` by
pretending finalization has no container work. Byte-array image output must
remain identified as buffered until a native writer or bounded spool supplies
the format's required behavior.

Cooperative cancellation cannot interrupt arbitrary blocking `Read` or `Write`.
Configure network timeouts at the transport layer. `HttpRangeReader` does this
for its requests. A source snapshot stops between reads; it does not promise
an interruptible socket supplied by someone else.

## Animation/video bridges

`video::animation_to_ivf` decodes complete display canvases, explicitly converts
RGB/alpha to native components, and sends them to the incremental AV1 writer.
It requires exact positive durations on the chosen video clock. IVF cannot
record animation loops or the final presentation duration: repeated/unknown play
counts are rejected unless `LoopHandling::OneIteration` is chosen, and the
returned report records source plays and the final timeline endpoint.

`video::ivf_to_animation` keeps one presentation of lookahead to derive exact
variable durations from adjacent PTS. It requires a positive final duration
representable on the IVF clock. Duplicate/decreasing presentation timestamps
are errors. The destination animation encoder retains its own documented
buffering and representable clock limits. Zero-duration control frames require
an explicit caller policy before entering a positive-duration video timeline.

The runnable `media` example in `zencodec-media-codec-tests` supports local or
HTTP-range IVF extraction and exact animation transcoding. It writes completed
outputs through a temporary file and refuses to overwrite an existing output.
The extraction command has explicit timestamp units, selection, keyframe scope,
chroma interpretation, and integer clipping policy; source color is preserved.
