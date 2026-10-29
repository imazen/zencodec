//! Select stored-orientation representation without capability coordination.
use zencodec::encode::{EncodeJob, EncodeOrientation, EncodeOutput, Encoder};
use zencodec::{Metadata, MetadataPolicy};
use zenpixels::PixelSlice;

// EXAMPLE: preserve_stored_orientation
pub fn preserve_stored<J: EncodeJob>(
    job: J,
    pixels: PixelSlice<'_>,
    metadata: Metadata,
) -> Result<EncodeOutput, Box<dyn core::error::Error + Send + Sync>>
where
    J::Enc: Encoder,
{
    // ANCHOR: preserve_stored_orientation
    let encoder = job
        .try_with_orientation_mode(EncodeOrientation::PreserveStored)?
        .with_metadata_policy(metadata, MetadataPolicy::PreserveExact)
        .encoder()?;
    let output = encoder.encode(pixels)?;
    // END: preserve_stored_orientation
    // This requires stored geometry/order, not lossless compression or a RAM bound.
    Ok(output)
}
// END EXAMPLE: preserve_stored_orientation

fn main() {}
