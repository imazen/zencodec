# Retained documents in the reimagined contract

This is an isolated candidate derived from the reimagined working tree. It does
not change the main codec API or the original experiment. The starting capture
is recorded in [source-capture.json](source-capture.json).

The finite deliverable is one document/selection contract, executable lazy
browsing and repeated spatial selection, a real native TIFF witness, and an
explicit format coverage ledger. Existing row, whole-image, animation, replay
and cancellation controls remain enabled. This does not authorize migrating
every codec, changing kernels, or publishing a crate.

## Shape

An opened document retains its parser/index state. A query discovers images,
containment, levels or channels, or describes a selected target. Queries use the
same absolute input requests and borrowed windows as decoding. After selection,
the document returns the existing decoder protocol with a retained selection
receipt. There is no second typed/erased family and no separate pixel protocol.

Levels are explicit identities with actual dimensions and optional established
registration to the parent image. They are not containment children or inferred
power-of-two reductions. Stored bounds have signed origins for formats such as
EXR. Regions are exact and never clipped silently. Channel selection either
preserves the requested native identities or refuses; named scientific channels
must not silently become display RGB.

The driver retains the same source for the opened document. Source version
consistency remains the source adapter's obligation, not a consequence of an
opaque image ID. Each query/selection retains operation controls; callbacks and
per-request preferences cannot replace mandatory requirements. Unknown native
workspace is not zero. Cancellation and native-error precedence apply across
metadata discovery as well as sample publication.

## Native pull implementations

The byte dialogue naturally fits resumable parsers. Existing synchronous native
Read/Seek parsers cannot suspend their call stack merely because the public
surface has a step method. The TIFF witness must expose that adaptation cost:
a native worker can wait for positional reads while the caller drives the same
range requests. It is qualification machinery, not a claimed allocation or
throughput improvement, and its lifetime/cancellation limits must be recorded.
Do not implement another TIFF parser merely to demonstrate the contract.

## Starting evidence

The captured standalone workspace passes its 21 blocking controls and one
doctest on Rust 1.99.1. Its first strict Clippy run fails on three mechanical
fixture lints, plus an unused test import. The isolated repairs preserve the
pre-existing assertions and zero-width behavior. Original logs are retained.

No native EXR, JXL or IMI integration is established by the synthetic browsing
controls. The final ledger must name what executes and what remains a design
mapping. EXR ripmap levels, TIFF SubIFDs, JXL frame dependencies and IMI archive
entries are different semantic relationships.
