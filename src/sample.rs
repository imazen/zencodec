//! Integer code representation and exact sample-depth evidence.
//!
//! Native storage, code depth and padding are independent. Use
//! `zenpixels_convert::codes` to convert native codes to canonical samples.

pub use zenpixels::sample::{
    ChromaSiting, ChromaSubsampling, CodePlane, CodePlaneError, ExactDepth, ExactDepthError,
    SampleEncoding, SampleEncodingError, full_range_grid_nests, narrow_range_grid_nests,
};
