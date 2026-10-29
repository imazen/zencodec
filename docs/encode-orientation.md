# Choosing stored or baked orientation

```rust
use zencodec::encode::{EncodeJob, EncodeOrientation};
# fn prepare<J: EncodeJob>(job: J) -> Result<J, zencodec::UnsupportedOperation> {
let job = job.try_with_orientation_mode(EncodeOrientation::PreserveStored)?;
# Ok(job)
# }
```

Omit this call for the codec's existing compatibility default. Request
`PreserveStored` when the source samples must remain in their stored order and
geometry, with intrinsic orientation carried in the encoded image. Request
`Bake` for upright output with identity orientation. Compression fidelity is
separate: preservation of spatial representation does not imply lossless samples.

The orientation value comes from job metadata. The request does not introduce
a second orientation value, and later metadata changes must still honor it.
Use `with_metadata_policy` to make retention explicit. The
[complete caller example](../examples/orientation.rs) is compiled with core.

For registered configurations, use
`operation::EncodeRequest::with_orientation_mode(mode)`; both `prepare` and
`prepare_send` apply the same checked setter before offering a job. For an
existing erased job, use `DynEncodeJob::try_set_orientation_mode(mode)`.
An adapter taking ownership with `PreparedEncode::into_job` must retain the
required orientation mode; replacing it invalidates the original preparation.
Refusal retains the job; the consuming `try_with_orientation_mode` convenience
consumes it on refusal, like other checked builders.

A separate default-refusing setter is intentional. Adding a field to the old
policy could be silently ignored by existing codec overrides that inspect only
known fields. No codec implementation is required to migrate just to compile;
unqualified implementations return `UnsupportedOperation::EncodeOrientation`.
There is no new static capability flag to coordinate before making the request.

## Native qualification and remaining boundaries

The paired AVIF candidate preserves its default still-image baking behavior.
Explicit stored mode uses container orientation, keeps stored geometry and
sample order, and overrides conflicting native rotation/mirror configuration.
Its animation encoder can carry orientation but cannot yet bake frames; required
animation baking refuses before constructing the encoder.

The native controls compare all eight orientations through typed jobs, erased
jobs, regular registry preparation and Send preparation. They compare decoded
stored samples to the same encoder's identity baseline and inspect coded
geometry. Existing default-baking pixel/EXIF/XMP assertions remain unchanged.
This is same-codec parity, not independent-reader qualification.

This setter is not proof of a low-memory route. AVIF still encoding is a whole
raster operation, and its shared planning hook remains unsupported. A row or
replay driver must continue to refuse without a valid acquisition plan. Spatial
representation evidence in retained plans, native workspace bounds, additional
codecs and animation baking remain required follow-through; do not infer them
from successful one-shot encoding. No zenpixels API change is needed here.
