# Native AV1 streaming contract

`Av1Encoder` copies a checked, uncropped `YuvView` into backend-owned padded
planes. Callers can release their input immediately after `Accepted`. Code
precision, CICP, geometry, subsampling and chroma position must match the
configuration. Packing can differ; code values cannot. Conversion is a separate
operation, so encoding never implicitly expands 10-bit codes to normalized U16.

The packet clock is exact and independent of the nominal encoder frame rate.
The encoder accepts nondecreasing signed timestamps, including duplicates; it
rejects a different clock or the paired decoder's reserved `i64::MIN`. Packet
presentation identity comes from `input_frameno`, not receive order or a guessed
frame rate. Equal-picture bitstream timing is disabled for this interface.

`submit`/`receive` separates acceptance from output. `ReceivePending` means the
caller still owns input and must receive output before retrying. A queue limit
smaller than the backend's lookahead requirements produces an explicit error
instead of an unbounded queue or a stalled retry loop. `end_input` is fallible
and idempotent. Receiving after complete drain yields stable end-of-stream.

The queue limit counts accepted presentations awaiting output. Codec reference
pictures, reconstructions and lookahead storage are additional allocations;
this count is not a process-byte budget. Configure a positive pixel limit too.
The backend still performs some infallible allocations. This API does not claim
hard memory isolation for arbitrary untrusted encoder configurations.

`Av1IvfEncoder<W: Write>` combines that encoder with the sequential IVF muxer.
It writes available packets on each push, requires no seeking, records an
unknown frame count, and propagates the sink's backpressure. `finish` drains
delayed packets and reports write/flush errors. Dropping without finishing does
not finalize a stream. Validation errors before acceptance are retryable;
backend and partial-output failures poison the corresponding stream.

Cancellation retains its backend error category. Decoder polling occurs both
between packets and inside the native codec. An arbitrary blocking `Read` or
`Write` cannot be interrupted by a CPU stop token: network transports must also
supply their own read/write timeout or cancellation mechanism.

The integration tests compare every source code after native lossless encoding
and decoding across all 24 depth/layout/range combinations. Additional cases
exercise full reordered groups, variable timestamps, fragmented sinks, partial
writes, flush failures, cancellation, empty input, and retryable validation.
